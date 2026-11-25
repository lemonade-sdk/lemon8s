//! Control watching its custom resources on a real API server; ignored by default. Run with
//!
//!     LEMON8S_KUBECONFIG=<a test cluster's kubeconfig> cargo test -p control --test kube -- --ignored
//!
//! Applies control's CRDs from the code (server-side apply), and each test works in its own new
//! namespace `lemon8s-test-*`, deleted at the end. The first Ready node stands in for the
//! deployments' node and runs each engine as a pause pod behind a real Service, made ready or not
//! through a readiness gate, so the `EndpointSlice` controller writes what control reads.

mod common;

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use common::{DEPLOYMENT, MODEL, denial, deployment, key_hash, model, name, names, session, user};
use control::crd::{self, Model, ModelDeployment, User};
use control::{CheckKey, Control, ErrorCode, NodeTarget};
use control::{Dispatch, ErrorReply, OpenRequest, ReadRevocations, ReadStatus};
use k8s_openapi::api::core::v1::{
    ConfigMap, Namespace, Node, Pod, Secret, Service, ServiceAccount,
};
use k8s_openapi::api::discovery::v1::EndpointSlice;
use k8s_openapi::api::rbac::v1::{ClusterRole, ClusterRoleBinding, Role, RoleBinding};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::{DeleteParams, ListParams, Patch, PatchParams, PostParams};
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Api, Client, Config, ResourceExt};
use policy::{Denial, Label, ModelName};
use serde_json::{Value, json};

const SYNC_TIMEOUT: Duration = Duration::from_secs(30);
const RBAC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/rbac.json");
/// The namespace `rbac.json` names, which each test's namespace replaces.
const RBAC_NAMESPACE: &str = "lemonade";
const READY_GATE: &str = "lemon8s.io/test-ready";
/// Set, to `<namespace>/<service account>`, in the copy of this test binary a test runs to read
/// control's log.
const CHILD: &str = "LEMON8S_TEST_CHILD";

/// A client for the test cluster, the CRDs installed, a fresh namespace and a Ready node.
struct Cluster {
    client: Client,
    config: Config,
    namespace: String,
    node: String,
}

async fn test_config() -> Config {
    let path = std::env::var("LEMON8S_KUBECONFIG")
        .expect("set LEMON8S_KUBECONFIG to the test cluster's kubeconfig");
    Config::from_custom_kubeconfig(
        Kubeconfig::read_from(&path).unwrap(),
        &KubeConfigOptions::default(),
    )
    .await
    .unwrap()
}

/// A client acting as `service_account` in `namespace`.
fn impersonating(mut config: Config, namespace: &str, service_account: &str) -> Client {
    config.auth_info.impersonate = Some(format!(
        "system:serviceaccount:{namespace}:{service_account}"
    ));
    Client::try_from(config).unwrap()
}

impl Cluster {
    async fn connect() -> Self {
        let config = test_config().await;
        let config_for_impersonation = config.clone();
        let client = Client::try_from(config).unwrap();
        let crds = Api::<CustomResourceDefinition>::all(client.clone());
        for definition in crd::definitions() {
            let name = definition.name_any();
            crds.patch(
                &name,
                &PatchParams::apply("lemon8s-control-test").force(),
                &Patch::Apply(definition),
            )
            .await
            .unwrap();
            let deadline = Instant::now() + SYNC_TIMEOUT;
            loop {
                let established = crds
                    .get(&name)
                    .await
                    .unwrap()
                    .status
                    .and_then(|s| s.conditions)
                    .is_some_and(|c| {
                        c.iter()
                            .any(|c| c.type_ == "Established" && c.status == "True")
                    });
                if established {
                    break;
                }
                assert!(Instant::now() < deadline, "{name} not established");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
        let mut suffix = [0u8; 4];
        SystemRandom::new().fill(&mut suffix).unwrap();
        let namespace = format!("lemon8s-test-{}", hex::encode(suffix));
        Api::<Namespace>::all(client.clone())
            .create(
                &PostParams::default(),
                &serde_json::from_value(json!({"metadata": {"name": namespace}})).unwrap(),
            )
            .await
            .unwrap();
        let node = Api::<Node>::all(client.clone())
            .list(&ListParams::default())
            .await
            .unwrap()
            .items
            .into_iter()
            .find(|n| NodeTarget::of(n) == NodeTarget::Ready)
            .expect("a Ready node")
            .name_any();
        Self {
            client,
            config: config_for_impersonation,
            namespace,
            node,
        }
    }

    fn api<K>(&self) -> Api<K>
    where
        K: kube::Resource<Scope = kube::core::NamespaceResourceScope, DynamicType = ()>,
    {
        Api::namespaced(self.client.clone(), &self.namespace)
    }

    async fn create<K>(&self, object: &K)
    where
        K: kube::Resource<Scope = kube::core::NamespaceResourceScope, DynamicType = ()>
            + Clone
            + serde::Serialize
            + serde::de::DeserializeOwned
            + std::fmt::Debug,
    {
        self.api::<K>()
            .create(&PostParams::default(), object)
            .await
            .unwrap();
    }

    /// Creates control's `rbac.json` in this namespace, the cluster-scoped objects named for it.
    /// Returns the `ServiceAccount`'s name.
    async fn apply_rbac(&self) -> String {
        let rbac: Value = serde_json::from_slice(&std::fs::read(RBAC).unwrap()).unwrap();
        let mut service_account = String::new();
        for mut item in rbac["items"].as_array().unwrap().clone() {
            if item["metadata"].get("namespace").is_some() {
                item["metadata"]["namespace"] = json!(self.namespace);
            }
            for subject in item["subjects"].as_array_mut().into_iter().flatten() {
                subject["namespace"] = json!(self.namespace);
            }
            let kind = item["kind"].as_str().unwrap().to_owned();
            if kind.starts_with("Cluster") {
                let name = item["metadata"]["name"].as_str().unwrap();
                let prefix = name.strip_suffix(RBAC_NAMESPACE).unwrap();
                item["metadata"]["name"] = json!(format!("{prefix}{}", self.namespace));
            }
            if kind == "ClusterRoleBinding" {
                item["roleRef"]["name"] = item["metadata"]["name"].clone();
            }
            match kind.as_str() {
                "ServiceAccount" => {
                    let account: ServiceAccount = serde_json::from_value(item).unwrap();
                    service_account = account.name_any();
                    self.create(&account).await;
                }
                "Role" => {
                    self.create(&serde_json::from_value::<Role>(item).unwrap())
                        .await;
                }
                "RoleBinding" => {
                    self.create(&serde_json::from_value::<RoleBinding>(item).unwrap())
                        .await;
                }
                "ClusterRole" => {
                    Api::<ClusterRole>::all(self.client.clone())
                        .create(
                            &PostParams::default(),
                            &serde_json::from_value(item).unwrap(),
                        )
                        .await
                        .unwrap();
                }
                "ClusterRoleBinding" => {
                    Api::<ClusterRoleBinding>::all(self.client.clone())
                        .create(
                            &PostParams::default(),
                            &serde_json::from_value(item).unwrap(),
                        )
                        .await
                        .unwrap();
                }
                other => panic!("rbac.json holds a {other}"),
            }
        }
        service_account
    }

    /// An engine on the test node: Service `name`, labeled as control's watch expects, in front
    /// of a pause pod whose readiness gate starts `ready` or not.
    async fn engine(&self, name: &str, ready: bool) {
        self.create(
            &serde_json::from_value::<Service>(json!({
                "metadata": {"name": name, "labels": {"app.kubernetes.io/name": "engine"}},
                "spec": {
                    "selector": {"lemon8s.io/test-engine": name},
                    "ports": [{"port": 8080}],
                },
            }))
            .unwrap(),
        )
        .await;
        self.create(
            &serde_json::from_value::<Pod>(json!({
                "metadata": {"name": name, "labels": {"lemon8s.io/test-engine": name}},
                "spec": {
                    "nodeName": self.node,
                    "readinessGates": [{"conditionType": READY_GATE}],
                    "containers": [{
                        "name": "engine",
                        // Every kind node holds its sandbox image, so nothing is pulled.
                        "image": "registry.k8s.io/pause:3.10",
                        "imagePullPolicy": "IfNotPresent",
                    }],
                },
            }))
            .unwrap(),
        )
        .await;
        self.set_ready(name, ready).await;
    }

    /// Sets the engine pod's readiness gate, which makes the pod Ready or not once it runs.
    async fn set_ready(&self, name: &str, ready: bool) {
        self.api::<Pod>()
            .patch_status(
                name,
                &PatchParams::default(),
                &Patch::Strategic(json!({"status": {"conditions": [{
                    "type": READY_GATE,
                    "status": if ready { "True" } else { "False" },
                }]}})),
            )
            .await
            .unwrap();
    }

    /// The system: model qwen3 (finance, legal), deployment engine-test (finance) on the Ready
    /// node behind engine `engine-test`, ready or not as `engine_ready`; users alice (finance,
    /// legal) and bob (finance).
    async fn configure(&self, engine_ready: bool) {
        self.create(&model("qwen3", MODEL, &["finance", "legal"]))
            .await;
        self.create(&deployment(DEPLOYMENT, &self.node, &["finance"]))
            .await;
        self.engine(DEPLOYMENT, engine_ready).await;
        self.create(&user("alice", &["finance", "legal"], false))
            .await;
        self.create(&user("bob", &["finance"], false)).await;
    }

    async fn delete_namespace(self) {
        Api::<Namespace>::all(self.client)
            .delete(&self.namespace, &DeleteParams::default())
            .await
            .unwrap();
    }

    /// Revokes Bob, then reads the feed as the gateway does, on each change of the epoch.
    async fn revoke_bob(&self, control: &Control, applied: u64) -> control::Revocations {
        let mut epochs = control.epochs();
        self.api::<User>()
            .patch(
                "bob",
                &PatchParams::default(),
                &Patch::Merge(json!({"spec": {"revoked": true}})),
            )
            .await
            .unwrap();
        let patched = Instant::now();
        let feed = loop {
            epochs.mark_unchanged();
            let feed = control
                .call(ReadRevocations { since: applied })
                .await
                .unwrap();
            if !feed.revoked.is_empty() {
                break feed;
            }
            tokio::time::timeout(
                SYNC_TIMEOUT.saturating_sub(patched.elapsed()),
                epochs.changed(),
            )
            .await
            .expect("the revocation never arrived")
            .unwrap();
        };
        eprintln!(
            "kubectl patch to the session in the revocation feed: {:?}",
            patched.elapsed()
        );
        feed
    }

    async fn until_verdicts_written(&self) {
        let deadline = Instant::now() + SYNC_TIMEOUT;
        loop {
            let engine = conditions(self, DEPLOYMENT).await;
            let orphaned = conditions(self, "engine-orphan").await;
            let expected_engine = [
                (
                    "Accepted".to_owned(),
                    "True".to_owned(),
                    "Applied".to_owned(),
                ),
                (
                    "Ready".to_owned(),
                    "True".to_owned(),
                    "EngineReady".to_owned(),
                ),
            ];
            let expected_orphan = [
                (
                    "Accepted".to_owned(),
                    "False".to_owned(),
                    "ModelNotFound".to_owned(),
                ),
                (
                    "Ready".to_owned(),
                    "False".to_owned(),
                    "NotAdmitted".to_owned(),
                ),
            ];
            if engine == expected_engine && orphaned == expected_orphan {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "statuses never written: {engine:?} {orphaned:?}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

fn refusal(result: Result<impl std::fmt::Debug, kube::Error>) -> (u16, String) {
    match result {
        Err(kube::Error::Api(status)) => (status.code, status.message),
        other => panic!("expected the API server to refuse, got {other:?}"),
    }
}

/// Polls control's operator status until `ready` holds.
async fn until(control: &Control, ready: impl Fn(&control::Status) -> bool) -> control::Status {
    let deadline = Instant::now() + SYNC_TIMEOUT;
    loop {
        let status = control.call(ReadStatus {}).await.unwrap();
        if ready(&status) {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "control never synced: {status:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
#[ignore = "needs a Kubernetes API server (LEMON8S_KUBECONFIG)"]
async fn the_api_server_enforces_the_crd_rules() {
    let cluster = Cluster::connect().await;

    cluster.create(&user("bob", &["finance"], true)).await;
    let (code, message) = refusal(
        cluster
            .api::<User>()
            .patch(
                "bob",
                &PatchParams::default(),
                &Patch::Merge(json!({"spec": {"revoked": false}})),
            )
            .await,
    );
    assert_eq!(code, 422);
    assert!(message.contains("revocation is permanent"), "{message}");

    let (code, message) = refusal(
        cluster
            .api::<User>()
            .create(
                &PostParams::default(),
                &user("mallory", &["top secret"], false),
            )
            .await,
    );
    assert_eq!(code, 422);
    assert!(message.contains("spec.clearance[0]"), "{message}");

    cluster
        .create(&deployment(DEPLOYMENT, &cluster.node, &["finance"]))
        .await;
    let (code, message) = refusal(
        cluster
            .api::<ModelDeployment>()
            .patch(
                DEPLOYMENT,
                &PatchParams::default(),
                &Patch::Merge(json!({"spec": {"node": "elsewhere"}})),
            )
            .await,
    );
    assert_eq!(code, 422);
    assert!(message.contains("immutable"), "{message}");
    let (code, message) = refusal(
        cluster
            .api::<ModelDeployment>()
            .patch(
                DEPLOYMENT,
                &PatchParams::default(),
                &Patch::Merge(json!({"spec": {"model": "another"}})),
            )
            .await,
    );
    assert_eq!(code, 422);
    assert!(message.contains("immutable"), "{message}");
    cluster
        .api::<ModelDeployment>()
        .patch(
            DEPLOYMENT,
            &PatchParams::default(),
            &Patch::Merge(json!({"spec": {"trustedFor": ["finance", "legal"]}})),
        )
        .await
        .unwrap();

    cluster.delete_namespace().await;
}

#[tokio::test]
#[ignore = "needs a Kubernetes API server (LEMON8S_KUBECONFIG)"]
async fn the_api_server_enforces_the_model_rules() {
    let cluster = Cluster::connect().await;

    let mut unpinned = model("unpinned", MODEL, &["finance"]);
    unpinned.spec.sha256 = "latest".to_owned();
    let (code, message) = refusal(
        cluster
            .api::<Model>()
            .create(&PostParams::default(), &unpinned)
            .await,
    );
    assert_eq!(code, 422);
    assert!(message.contains("spec.sha256"), "{message}");
    let mut escaping = model("escaping", MODEL, &["finance"]);
    escaping.spec.source.file = "../secrets.gguf".to_owned();
    let (code, message) = refusal(
        cluster
            .api::<Model>()
            .create(&PostParams::default(), &escaping)
            .await,
    );
    assert_eq!(code, 422);
    assert!(message.contains("spec.source.file"), "{message}");

    cluster.create(&model("qwen3", MODEL, &["finance"])).await;
    for change in [
        json!({"spec": {"sha256": "cd".repeat(32)}}),
        json!({"spec": {"source": {"file": "another.gguf"}}}),
    ] {
        let (code, message) = refusal(
            cluster
                .api::<Model>()
                .patch("qwen3", &PatchParams::default(), &Patch::Merge(change))
                .await,
        );
        assert_eq!(code, 422);
        assert!(message.contains("immutable"), "{message}");
    }
    cluster
        .api::<Model>()
        .patch(
            "qwen3",
            &PatchParams::default(),
            &Patch::Merge(json!({"spec": {"approvedFor": ["finance", "legal"]}})),
        )
        .await
        .unwrap();

    cluster.delete_namespace().await;
}

#[tokio::test]
#[ignore = "needs a Kubernetes API server (LEMON8S_KUBECONFIG)"]
async fn a_user_revoked_in_kubernetes_is_refused_and_listed_for_cutting() {
    let cluster = Cluster::connect().await;
    let dir = tempfile::tempdir().unwrap();
    let control = Control::open(&dir.path().join("control.sqlite")).unwrap();
    let _watching = control.watch(cluster.client.clone(), &cluster.namespace);

    cluster.configure(true).await;
    until(&control, |s| {
        s.deployments
            .iter()
            .any(|d| d.deployment == name(DEPLOYMENT) && d.ready)
    })
    .await;
    let alice = session(&control, "alice", "key-a", &["finance"]).await;
    let bob = session(&control, "bob", "key-b", &["finance"]).await;
    let opened = control
        .call(OpenRequest {
            key_hash: key_hash("key-b"),
            model: Some(name(MODEL)),
        })
        .await
        .unwrap();
    control
        .call(Dispatch {
            key_hash: key_hash("key-b"),
            request: opened.request,
            deployment: name(DEPLOYMENT),
        })
        .await
        .unwrap();
    let applied = control.epoch();

    let feed = cluster.revoke_bob(&control, applied).await;
    let revoked: Vec<(u64, Option<control::Name>)> = feed
        .revoked
        .iter()
        .map(|r| (r.session, r.deployment.clone()))
        .collect();
    assert_eq!(revoked, [(bob, None)]);
    let refused = control
        .call(CheckKey {
            key_hash: key_hash("key-b"),
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::InvalidApiKey);

    cluster
        .api::<Model>()
        .patch(
            "qwen3",
            &PatchParams::default(),
            &Patch::Merge(json!({"spec": {"approvedFor": []}})),
        )
        .await
        .unwrap();
    until(&control, |s| {
        s.deployments
            .iter()
            .any(|d| d.deployment == name(DEPLOYMENT) && d.trusted_for == Label::public())
    })
    .await;
    let narrowed = control
        .call(ReadRevocations { since: feed.epoch })
        .await
        .unwrap();
    let revoked: Vec<(u64, Option<control::Name>)> = narrowed
        .revoked
        .iter()
        .map(|r| (r.session, r.deployment.clone()))
        .collect();
    assert_eq!(
        revoked,
        [
            (alice, Some(name(DEPLOYMENT))),
            (bob, Some(name(DEPLOYMENT)))
        ],
        "the narrowing lists every session it covers, Bob's revoked one too: cutting it again is a no-op"
    );
    let refused = control
        .call(OpenRequest {
            key_hash: key_hash("key-a"),
            model: Some(name(MODEL)),
        })
        .await;
    assert_eq!(
        denial(refused),
        Denial::NoTrustedDeployment {
            model: ModelName::new(MODEL)
        }
    );

    cluster
        .api::<ModelDeployment>()
        .delete(DEPLOYMENT, &DeleteParams::default())
        .await
        .unwrap();
    until(&control, |s| s.deployments.is_empty()).await;

    cluster.delete_namespace().await;
}

#[tokio::test]
#[ignore = "needs a Kubernetes API server (LEMON8S_KUBECONFIG)"]
async fn a_change_made_while_control_was_down_applies_when_it_returns() {
    let cluster = Cluster::connect().await;
    let dir = tempfile::tempdir().unwrap();
    let first = Control::open(&dir.path().join("control.sqlite")).unwrap();
    let watching = first.watch(cluster.client.clone(), &cluster.namespace);
    cluster.configure(true).await;
    until(&first, |s| !s.deployments.is_empty()).await;
    session(&first, "alice", "key-a", &["finance", "legal"]).await;
    let epoch = first.epoch();
    drop(watching);
    drop(first);

    cluster
        .api::<User>()
        .patch(
            "alice",
            &PatchParams::default(),
            &Patch::Merge(json!({"spec": {"clearance": ["finance"]}})),
        )
        .await
        .unwrap();

    let second = Control::open(&dir.path().join("control.sqlite")).unwrap();
    let open = || {
        second.call(OpenRequest {
            key_hash: key_hash("key-a"),
            model: None,
        })
    };
    open().await.unwrap();
    let _watching = second.watch(cluster.client.clone(), &cluster.namespace);
    let deadline = Instant::now() + SYNC_TIMEOUT;
    loop {
        match open().await {
            Err(ErrorReply {
                denial: Some(denial),
                ..
            }) => {
                assert!(matches!(denial, Denial::LabelExceedsClearance { .. }));
                break;
            }
            Ok(_) => assert!(Instant::now() < deadline, "the narrowing never arrived"),
            Err(other) => panic!("{other:?}"),
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(second.epoch() > epoch + 1);

    cluster.delete_namespace().await;
}

/// The conditions control wrote into a `ModelDeployment`'s status, as (type, status, reason).
async fn conditions(cluster: &Cluster, name: &str) -> Vec<(String, String, String)> {
    cluster
        .api::<ModelDeployment>()
        .get_status(name)
        .await
        .unwrap()
        .status
        .map_or_default(|s| s.conditions)
        .into_iter()
        .map(|c| (c.type_, c.status, c.reason))
        .collect()
}

#[tokio::test]
#[ignore = "needs a Kubernetes API server (LEMON8S_KUBECONFIG)"]
async fn control_runs_on_its_rbac_alone_and_writes_every_verdict_into_status() {
    let cluster = Cluster::connect().await;
    let service_account = cluster.apply_rbac().await;
    let control_client =
        impersonating(cluster.config.clone(), &cluster.namespace, &service_account);
    let dir = tempfile::tempdir().unwrap();
    let control = Control::open(&dir.path().join("control.sqlite")).unwrap();
    let _watching = control.watch(control_client.clone(), &cluster.namespace);

    cluster.configure(true).await;
    let mut orphan = deployment("engine-orphan", &cluster.node, &["finance"]);
    orphan.spec.model = "missing".to_owned();
    cluster.create(&orphan).await;
    until(&control, |s| s.deployments.iter().any(|d| d.ready)).await;
    cluster.until_verdicts_written().await;

    assert_eq!(
        refusal(
            Api::<Secret>::namespaced(control_client.clone(), &cluster.namespace)
                .list(&ListParams::default())
                .await
        )
        .0,
        403
    );
    assert_eq!(
        refusal(
            Api::<Pod>::all(control_client.clone())
                .list(&ListParams::default())
                .await
        )
        .0,
        403
    );
    assert_eq!(
        refusal(
            Api::<EndpointSlice>::all(control_client.clone())
                .list(&ListParams::default())
                .await
        )
        .0,
        403,
        "EndpointSlices only in its namespace"
    );
    assert_eq!(
        refusal(
            Api::<User>::namespaced(control_client.clone(), &cluster.namespace)
                .patch(
                    "bob",
                    &PatchParams::default(),
                    &Patch::Merge(json!({"spec": {"clearance": ["finance", "legal"]}})),
                )
                .await
        )
        .0,
        403
    );
    assert_eq!(
        refusal(
            Api::<ConfigMap>::namespaced(control_client, &cluster.namespace)
                .get("kube-root-ca.crt")
                .await
        )
        .0,
        403
    );

    let cluster_name = format!("lemon8s-control-{}", cluster.namespace);
    Api::<ClusterRoleBinding>::all(cluster.client.clone())
        .delete(&cluster_name, &DeleteParams::default())
        .await
        .unwrap();
    Api::<ClusterRole>::all(cluster.client.clone())
        .delete(&cluster_name, &DeleteParams::default())
        .await
        .unwrap();
    cluster.delete_namespace().await;
}

#[tokio::test]
#[ignore = "needs a Kubernetes API server (LEMON8S_KUBECONFIG)"]
async fn an_engine_is_ready_only_while_its_pod_is() {
    let cluster = Cluster::connect().await;
    let dir = tempfile::tempdir().unwrap();
    let control = Control::open(&dir.path().join("control.sqlite")).unwrap();
    let _watching = control.watch(cluster.client.clone(), &cluster.namespace);
    let ready = |want: bool| {
        move |s: &control::Status| {
            s.deployments
                .iter()
                .any(|d| d.deployment == name(DEPLOYMENT) && d.ready == want)
        }
    };

    cluster.configure(false).await;
    until(&control, ready(false)).await;
    cluster.set_ready(DEPLOYMENT, true).await;
    until(&control, ready(true)).await;
    cluster.set_ready(DEPLOYMENT, false).await;
    until(&control, ready(false)).await;

    cluster.delete_namespace().await;
}

#[tokio::test]
#[ignore = "needs a Kubernetes API server (LEMON8S_KUBECONFIG)"]
async fn a_namespace_without_the_users_revokes_them_at_once_and_raises_the_alert() {
    let cluster = Cluster::connect().await;
    let dir = tempfile::tempdir().unwrap();
    let first = Control::open(&dir.path().join("control.sqlite")).unwrap();
    let watching = first.watch(cluster.client.clone(), &cluster.namespace);
    cluster.configure(true).await;
    until(&first, |s| !s.deployments.is_empty()).await;
    session(&first, "alice", "key-a", &["finance"]).await;
    session(&first, "bob", "key-b", &["finance"]).await;
    drop(watching);
    drop(first);

    let wrong = Cluster::connect().await;
    let control = Control::open(&dir.path().join("control.sqlite")).unwrap();
    let _watching = control.watch(wrong.client.clone(), &wrong.namespace);

    until(&control, |s| s.mass_revocation == names(&["alice", "bob"])).await;
    assert_eq!(control.mass_revocations(), 1);
    for key in ["key-a", "key-b"] {
        let refused = control
            .call(CheckKey {
                key_hash: key_hash(key),
            })
            .await
            .unwrap_err();
        assert_eq!(refused.code, ErrorCode::InvalidApiKey, "{key}");
    }

    wrong.delete_namespace().await;
    cluster.delete_namespace().await;
}

#[tokio::test]
#[ignore = "needs a Kubernetes API server (LEMON8S_KUBECONFIG)"]
async fn a_failed_status_write_is_logged_without_names() {
    if let Ok(as_control) = std::env::var(CHILD) {
        let (namespace, service_account) = as_control.split_once('/').unwrap();
        let client = impersonating(test_config().await, namespace, service_account);
        let dir = tempfile::tempdir().unwrap();
        let control = Control::open(&dir.path().join("control.sqlite")).unwrap();
        let _watching = control.watch(client, namespace);
        tokio::time::sleep(SYNC_TIMEOUT).await;
        return;
    }
    let cluster = Cluster::connect().await;
    let service_account = cluster.apply_rbac().await;
    let roles = cluster.api::<Role>();
    let mut no_status = roles.get("lemon8s-control").await.unwrap();
    no_status.rules.as_mut().unwrap().retain(|r| {
        !r.resources
            .iter()
            .flatten()
            .any(|resource| resource.ends_with("/status"))
    });
    roles
        .replace("lemon8s-control", &PostParams::default(), &no_status)
        .await
        .unwrap();
    cluster.configure(true).await;

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "a_failed_status_write_is_logged_without_names",
            "--include-ignored",
            "--nocapture",
        ])
        .env(CHILD, format!("{}/{service_account}", cluster.namespace))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let log = tokio::task::spawn_blocking(move || {
        let mut log = Vec::new();
        for line in BufReader::new(stderr).lines() {
            let line = line.unwrap();
            let done = line.contains("User objects failed");
            log.push(line);
            if done {
                break;
            }
        }
        log
    })
    .await
    .unwrap();
    child.kill().unwrap();
    child.wait().unwrap();

    let failed = log.iter().find(|l| l.contains("User objects failed"));
    assert_eq!(
        failed.map(String::as_str),
        Some("control: writing the status of 2 User objects failed: 403 Forbidden"),
        "{log:?}"
    );
    for who in ["alice", "bob"] {
        assert!(log.iter().all(|l| !l.contains(who)), "{log:?}");
    }

    let cluster_name = format!("lemon8s-control-{}", cluster.namespace);
    Api::<ClusterRoleBinding>::all(cluster.client.clone())
        .delete(&cluster_name, &DeleteParams::default())
        .await
        .unwrap();
    Api::<ClusterRole>::all(cluster.client.clone())
        .delete(&cluster_name, &DeleteParams::default())
        .await
        .unwrap();
    cluster.delete_namespace().await;
}
