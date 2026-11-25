//! Control's calls end to end through `Control::call`, with the configuration applied by
//! `Control::reconcile`: authorization, dispatch, provenance and revocation; and how status shows
//! a watch that can't reach the API server.

mod common;

use std::net::TcpListener;
use std::time::{Duration, Instant, SystemTime};

use common::{
    DEPLOYMENT, MODEL, NODE, OTHER_DEPLOYMENT, OTHER_NODE, denial, deployment, endpoint_slice,
    key_hash, label, model, name, names, node, operator_config, session, user,
};
use control::{
    CloseSession, CompleteRequest, CreateSession, Dispatch, ErrorCode, FailRequest, OpenRequest,
    OpenSession, ReadProvenance, ReadRevocations, ReadStatus, Reason, Rejected,
};
use control::{Control, Desired, Name};
use kube::{Client, Config};
use policy::{
    Denial, DeploymentId, ModelName, NodeId, NodeState, ProvenanceEvent, RequestId, SessionId,
    UserId,
};

struct System {
    _dir: tempfile::TempDir,
    control: Control,
}

impl System {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let control = Control::open(&dir.path().join("control.sqlite")).unwrap();
        control
            .reconcile(&operator_config(), SystemTime::now())
            .unwrap();
        Self { _dir: dir, control }
    }

    fn configure(&self, change: impl FnOnce(&mut Desired)) {
        let mut config = operator_config();
        change(&mut config);
        self.control.reconcile(&config, SystemTime::now()).unwrap();
    }

    async fn session(&self, user: &str, key: &str, compartments: &[&str]) -> u64 {
        session(&self.control, user, key, compartments).await
    }

    async fn open(&self, key: &str) -> Result<control::RequestOpened, control::ErrorReply> {
        self.control
            .call(OpenRequest {
                key_hash: key_hash(key),
                model: Some(name(MODEL)),
            })
            .await
    }

    async fn dispatch(
        &self,
        key: &str,
        request: u64,
    ) -> Result<control::Dispatched, control::ErrorReply> {
        self.control
            .call(Dispatch {
                key_hash: key_hash(key),
                request,
                deployment: name(DEPLOYMENT),
            })
            .await
    }

    async fn provenance(&self) -> control::ProvenancePage {
        self.control
            .call(ReadProvenance { since: 0 })
            .await
            .unwrap()
    }

    async fn status(&self) -> control::Status {
        self.control.call(ReadStatus {}).await.unwrap()
    }
}

#[tokio::test]
async fn a_request_runs_arrival_dispatch_and_completion_and_each_step_is_recorded() {
    let sys = System::new();
    let session = sys.session("alice", "key-a", &["finance"]).await;

    let opened = sys.open("key-a").await.unwrap();
    assert_eq!(opened.session, session);
    assert_eq!(opened.label, label(&["finance"]));
    let deployments: Vec<_> = opened.candidates.iter().map(|c| &c.deployment).collect();
    assert_eq!(deployments, [&name(OTHER_DEPLOYMENT), &name(DEPLOYMENT)]);
    let dispatched = sys.dispatch("key-a", opened.request).await.unwrap();
    assert_eq!(dispatched.node, name(NODE));
    assert_eq!(dispatched.epoch, sys.control.epoch());
    sys.control
        .call(CompleteRequest {
            key_hash: key_hash("key-a"),
            request: opened.request,
        })
        .await
        .unwrap();

    let page = sys.provenance().await;
    let events: Vec<&ProvenanceEvent> = page
        .records
        .iter()
        .filter(|r| r.session == Some(SessionId::from_raw(session)))
        .map(|r| &r.event)
        .collect();
    let [
        ProvenanceEvent::SessionCreated,
        ProvenanceEvent::RequestOpened {
            request,
            model,
            candidates,
        },
        ProvenanceEvent::Granted {
            grant: Some(_),
            deployment,
        },
        ProvenanceEvent::RequestStarted { request: started },
        ProvenanceEvent::RequestCompleted { request: completed },
    ] = events[..]
    else {
        panic!("unexpected provenance: {events:?}");
    };
    let opened_request = RequestId::from_raw(opened.request);
    assert_eq!(*request, Some(opened_request));
    assert_eq!(*model, Some(ModelName::new(MODEL)));
    assert_eq!(
        *candidates,
        [
            DeploymentId::new(OTHER_DEPLOYMENT),
            DeploymentId::new(DEPLOYMENT)
        ]
    );
    assert_eq!(*deployment, DeploymentId::new(DEPLOYMENT));
    assert_eq!((*started, *completed), (opened_request, opened_request));
    assert_eq!(page.next, page.records.last().unwrap().seq + 1);
}

#[tokio::test]
async fn keys_bind_requests_to_their_session() {
    let sys = System::new();
    sys.session("alice", "key-a", &["finance"]).await;
    sys.session("bob", "key-b", &["finance"]).await;
    let alices = sys.open("key-a").await.unwrap().request;

    let unknown = sys
        .control
        .call(OpenRequest {
            key_hash: key_hash("not-a-key"),
            model: None,
        })
        .await;
    assert_eq!(unknown.unwrap_err().code, ErrorCode::InvalidApiKey);
    assert_eq!(
        denial(sys.dispatch("key-b", alices).await),
        Denial::UnknownRequest
    );
    assert_eq!(
        denial(
            sys.control
                .call(FailRequest {
                    key_hash: key_hash("key-b"),
                    request: alices,
                },)
                .await
        ),
        Denial::UnknownRequest
    );
    sys.dispatch("key-a", alices).await.unwrap();
}

#[tokio::test]
async fn denials_carry_the_policy_reason_and_are_recorded() {
    let sys = System::new();
    sys.session("alice", "key-a", &["finance", "legal"]).await;

    assert_eq!(
        denial(sys.open("key-a").await),
        Denial::NoTrustedDeployment {
            model: ModelName::new(MODEL)
        }
    );
    let page = sys.provenance().await;
    let last = page.records.last().unwrap();
    assert_eq!(
        last.denial,
        Some(Denial::NoTrustedDeployment {
            model: ModelName::new(MODEL)
        })
    );
    assert_eq!(last.label, label(&["finance", "legal"]));
}

#[tokio::test]
async fn a_bob_session_above_his_clearance_is_refused() {
    let sys = System::new();

    let refused = sys
        .control
        .call(CreateSession {
            user: name("bob"),
            label: names(&["legal"]),
            key_hash: key_hash("key-b"),
        })
        .await;

    assert!(matches!(
        denial(refused),
        Denial::LabelExceedsClearance { .. }
    ));
    assert_eq!(
        sys.open("key-b").await.unwrap_err().code,
        ErrorCode::InvalidApiKey
    );
}

#[tokio::test]
async fn cordoning_the_node_stops_dispatch_to_it() {
    let sys = System::new();
    sys.session("alice", "key-a", &["finance"]).await;
    let request = sys.open("key-a").await.unwrap().request;

    sys.configure(|c| c.nodes[0] = node(NODE, true, true));

    assert!(matches!(
        denial(sys.dispatch("key-a", request).await),
        Denial::NodeNotReady { .. }
    ));
}

#[tokio::test]
async fn a_node_gone_from_kubernetes_is_lost_even_while_a_deployment_still_names_it() {
    let sys = System::new();
    sys.session("alice", "key-a", &["finance"]).await;
    let epoch = sys.control.epoch();

    sys.configure(|c| {
        c.nodes
            .retain(|n| n.metadata.name.as_deref() != Some(OTHER_NODE));
    });

    let status = sys.status().await;
    let other = status
        .nodes
        .iter()
        .find(|n| n.node == name(OTHER_NODE))
        .unwrap();
    assert_eq!(other.state, NodeState::Lost);
    assert!(sys.control.epoch() > epoch);
    let opened = sys.open("key-a").await.unwrap();
    let candidates: Vec<_> = opened.candidates.iter().map(|c| &c.deployment).collect();
    assert_eq!(candidates, [&name(DEPLOYMENT)]);
}

/// An engine is ready only while its Node reports Ready, in status and in the `ModelDeployment`'s
/// `Ready` condition, whatever its `EndpointSlice` still says.
#[tokio::test]
async fn an_engine_on_a_deleted_or_unready_node_is_not_ready_though_its_endpoint_is() {
    let sys = System::new();
    let ready = |status: &control::Status| -> Vec<(String, bool)> {
        status
            .deployments
            .iter()
            .map(|d| (d.deployment.as_str().to_owned(), d.ready))
            .collect()
    };
    let both = vec![
        (OTHER_DEPLOYMENT.to_owned(), true),
        (DEPLOYMENT.to_owned(), true),
    ];
    let first_only = vec![
        (OTHER_DEPLOYMENT.to_owned(), false),
        (DEPLOYMENT.to_owned(), true),
    ];
    assert_eq!(ready(&sys.status().await), both);

    let mut deleted = operator_config();
    deleted
        .nodes
        .retain(|n| n.metadata.name.as_deref() != Some(OTHER_NODE));
    let mut not_ready = operator_config();
    not_ready.nodes[1] = node(OTHER_NODE, false, false);
    for (case, config) in [("deleted", deleted), ("not ready", not_ready)] {
        let reconciled = sys.control.reconcile(&config, SystemTime::now()).unwrap();
        assert_eq!(
            reconciled.engine_ready.get(OTHER_DEPLOYMENT),
            Some(&false),
            "{case}"
        );
        assert_eq!(ready(&sys.status().await), first_only, "{case}");

        let reconciled = sys
            .control
            .reconcile(&operator_config(), SystemTime::now())
            .unwrap();
        assert_eq!(
            reconciled.engine_ready.get(OTHER_DEPLOYMENT),
            Some(&true),
            "{case}, then back"
        );
        assert_eq!(ready(&sys.status().await), both, "{case}, then back");
    }
}

#[tokio::test]
async fn an_engine_whose_ready_endpoint_is_on_another_node_is_not_ready() {
    let sys = System::new();

    let reconciled = sys
        .control
        .reconcile(
            &Desired {
                endpoint_slices: vec![
                    endpoint_slice("lemon8s", DEPLOYMENT, OTHER_NODE, true),
                    endpoint_slice("lemon8s", OTHER_DEPLOYMENT, OTHER_NODE, true),
                ],
                ..operator_config()
            },
            SystemTime::now(),
        )
        .unwrap();

    assert_eq!(reconciled.engine_ready.get(DEPLOYMENT), Some(&false));
    assert_eq!(reconciled.engine_ready.get(OTHER_DEPLOYMENT), Some(&true));
}

#[tokio::test]
async fn only_the_nodes_a_deployment_names_are_followed() {
    let sys = System::new();
    let recorded = sys.provenance().await.records.len();

    for ready in [true, false, true] {
        sys.configure(|c| c.nodes.push(node("node-c", ready, false)));
    }

    let nodes: Vec<Name> = sys
        .status()
        .await
        .nodes
        .into_iter()
        .map(|n| n.node)
        .collect();
    assert_eq!(nodes, [name(NODE), name(OTHER_NODE)]);
    assert_eq!(sys.provenance().await.records.len(), recorded);

    sys.configure(|c| {
        c.deployments.retain(|d| d.spec.node != OTHER_NODE);
        c.nodes.push(node("node-c", true, false));
    });
    let other = sys
        .status()
        .await
        .nodes
        .into_iter()
        .find(|n| n.node == name(OTHER_NODE))
        .unwrap();
    assert_eq!(
        other.state,
        NodeState::Lost,
        "no longer named, so no longer followed"
    );
}

#[tokio::test]
async fn configuration_that_cannot_be_enforced_is_rejected_and_reported() {
    let sys = System::new();
    sys.configure(|c| c.users[1] = user("bob", &["finance"], true));

    sys.configure(|c| {
        c.users[1] = user("bob", &["finance"], false);
        let mut orphan = deployment("engine-orphan", NODE, &["finance"]);
        orphan.spec.model = "missing".to_owned();
        c.deployments.push(orphan);
    });

    let status = sys.status().await;
    let rejected_as = |kind: &str, object: &str, reason: Reason| Rejected {
        kind: name(kind),
        name: name(object),
        reason,
    };
    assert_eq!(
        status.rejected,
        [
            rejected_as("ModelDeployment", "engine-orphan", Reason::ModelNotFound),
            rejected_as("User", "bob", Reason::RevokedPermanently),
        ]
    );
    let deployments: Vec<&str> = status
        .deployments
        .iter()
        .map(|d| d.deployment.as_str())
        .collect();
    assert_eq!(deployments, [OTHER_DEPLOYMENT, DEPLOYMENT]);
    assert!(
        sys.control
            .call(CreateSession {
                user: name("bob"),
                label: names(&["finance"]),
                key_hash: key_hash("key-b"),
            },)
            .await
            .is_err()
    );
    sys.session("alice", "key-a", &["finance"]).await;
    let reused = sys
        .control
        .call(OpenSession {
            user: name("alice"),
            session: 0,
            key_hash: key_hash("key-a"),
        })
        .await;
    assert_eq!(reused.unwrap_err().code, ErrorCode::Conflict);
}

#[tokio::test]
async fn deleting_a_user_revokes_it_for_good() {
    let sys = System::new();
    sys.session("bob", "key-b", &["finance"]).await;
    let epoch = sys.control.epoch();

    sys.configure(|c| {
        c.users
            .retain(|u| u.metadata.name.as_deref() != Some("bob"));
    });
    assert!(sys.control.epoch() > epoch);
    sys.configure(|_| {});

    assert_eq!(
        sys.open("key-b").await.unwrap_err().code,
        ErrorCode::InvalidApiKey,
        "revoking bob dropped his keys"
    );
    let again = sys
        .control
        .call(CreateSession {
            user: name("bob"),
            label: names(&["finance"]),
            key_hash: key_hash("key-b2"),
        })
        .await;
    assert_eq!(denial(again), Denial::UserRevoked);
}

#[tokio::test]
async fn effective_trust_is_the_deployment_trust_within_the_model_approval() {
    let sys = System::new();
    sys.configure(|c| c.deployments[0] = deployment(DEPLOYMENT, NODE, &["finance", "legal"]));
    let alice = sys.session("alice", "key-a", &["finance", "legal"]).await;
    sys.open("key-a").await.unwrap();
    let before = sys.control.epoch();

    sys.configure(|c| {
        c.deployments[0] = deployment(DEPLOYMENT, NODE, &["finance", "legal"]);
        c.models[0] = model("qwen3", MODEL, &["finance"]);
    });

    let status = sys.status().await;
    let ours = status
        .deployments
        .iter()
        .find(|d| d.deployment == name(DEPLOYMENT))
        .unwrap();
    assert_eq!(ours.trusted_for, label(&["finance"]));
    let feed = sys
        .control
        .call(ReadRevocations { since: before })
        .await
        .unwrap();
    let revoked: Vec<(u64, Option<control::Name>)> = feed
        .revoked
        .iter()
        .map(|r| (r.session, r.deployment.clone()))
        .collect();
    assert_eq!(revoked, [(alice, Some(name(DEPLOYMENT)))]);
    assert_eq!(
        denial(sys.open("key-a").await),
        Denial::NoTrustedDeployment {
            model: ModelName::new(MODEL)
        }
    );
}

#[tokio::test]
async fn closing_a_session_ends_its_keys_and_lists_it_for_cutting() {
    let sys = System::new();
    sys.session("alice", "key-a", &["finance"]).await;
    let carol = sys.session("alice", "key-c", &["finance"]).await;
    let before = sys.control.epoch();

    sys.control
        .call(CloseSession {
            user: name("alice"),
            session: carol,
        })
        .await
        .unwrap();

    let feed = sys
        .control
        .call(ReadRevocations { since: before })
        .await
        .unwrap();
    let revoked: Vec<(u64, Option<control::Name>)> = feed
        .revoked
        .iter()
        .map(|r| (r.session, r.deployment.clone()))
        .collect();
    assert_eq!(revoked, [(carol, None)]);
    assert_eq!(
        sys.open("key-c").await.unwrap_err().code,
        ErrorCode::InvalidApiKey
    );
    sys.open("key-a").await.unwrap();
}

#[tokio::test]
async fn configuration_changes_are_recorded_for_the_operator() {
    let sys = System::new();

    sys.configure(|c| c.users[1] = user("bob", &[], false));

    let page = sys.provenance().await;
    let events: Vec<&ProvenanceEvent> = page.records.iter().map(|r| &r.event).collect();
    assert!(events.contains(&&ProvenanceEvent::DeploymentAdded {
        deployment: DeploymentId::new(DEPLOYMENT),
        model: ModelName::new(MODEL),
        node: NodeId::new(NODE),
    }));
    let users: Vec<(&ProvenanceEvent, Option<&str>)> = page
        .records
        .iter()
        .filter(|r| {
            matches!(
                r.event,
                ProvenanceEvent::UserAdded | ProvenanceEvent::ClearanceChanged
            )
        })
        .map(|r| (&r.event, r.actor.as_ref().map(UserId::as_str)))
        .collect();
    assert_eq!(
        users,
        [
            (&ProvenanceEvent::UserAdded, Some("alice")),
            (&ProvenanceEvent::UserAdded, Some("bob")),
            (&ProvenanceEvent::ClearanceChanged, Some("bob")),
        ]
    );
}

#[tokio::test]
async fn every_watch_failing_before_its_first_sync_is_in_status() {
    let sys = System::new();
    let closed = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let client =
        Client::try_from(Config::new(format!("http://{closed}").parse().unwrap())).unwrap();

    let _watching = sys.control.watch(client, "lemon8s");

    let deadline = Instant::now() + Duration::from_secs(10);
    let stale = loop {
        let stale = sys.status().await.stale_watches;
        if stale.len() == 5 {
            break stale;
        }
        assert!(Instant::now() < deadline, "{stale:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let kinds: Vec<&str> = stale.iter().map(|s| s.kind.as_str()).collect();
    assert_eq!(
        kinds,
        ["EndpointSlice", "Model", "ModelDeployment", "Node", "User"]
    );
    assert!(stale.iter().all(|s| !s.synced), "{stale:?}");
    let watches = sys.control.watches();
    assert_eq!(
        watches.keys().map(String::as_str).collect::<Vec<_>>(),
        kinds
    );
    assert!(watches.values().all(Option::is_some), "{watches:?}");
}

#[tokio::test]
async fn watches_that_keep_failing_start_over_without_ending_the_watch_task() {
    let sys = System::new();
    let closed = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let client =
        Client::try_from(Config::new(format!("http://{closed}").parse().unwrap())).unwrap();
    let mut watching = sys.control.watch(client, "lemon8s");

    // kube's backoff (0.8 s, then 1.6 s) has each watch fail and start over at least twice.
    let ended = tokio::time::timeout(Duration::from_secs(3), watching.ended()).await;

    assert!(ended.is_err(), "the watch task {ended:?}");
    assert_eq!(sys.status().await.stale_watches.len(), 5);
}
