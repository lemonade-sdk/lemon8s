use std::collections::BTreeMap;
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::Rejected;
use futures::StreamExt;
use k8s_openapi::api::core::v1::Node;
use k8s_openapi::api::discovery::v1::EndpointSlice;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{Condition, Time};
use k8s_openapi::jiff::Timestamp;
use kube::api::{Patch, PatchParams};
use kube::core::object::HasStatus;
use kube::runtime::reflector::{self, Store, store::Writer};
use kube::runtime::watcher;
use kube::{Api, Client, Resource, ResourceExt};
use parking_lot::Mutex;
use serde::de::DeserializeOwned;
use serde_json::json;
use tokio::sync::{Notify, watch};
use tokio::task::JoinSet;

use crate::crd::{Model, ModelDeployment, ResourceStatus, User};
use crate::desired::{Desired, Reconciled};
use crate::handle::{Control, Running};

const FIELD_MANAGER: &str = "lemon8s-control";

/// A watch that is failing: since when, in seconds since the Unix epoch, and whether its kind
/// had synced before, in which case its cache holds the API server's view as of `since`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Failing {
    pub(crate) since: u64,
    pub(crate) synced: bool,
}

/// Each watched kind, `Some` while its watch fails. The mirrors write it and status reads it
/// without the state lock, so a failure shows even before the first reconcile.
pub(crate) type Watches = Arc<Mutex<BTreeMap<String, Option<Failing>>>>;

/// Keeps `writer`'s cache equal to the API server's view of `api`, signals every change, and
/// keeps its kind's entry in `watches` current. kube's watcher resumes a failed watch without a
/// sign once it succeeds, so each failure ends the watch, and after a backoff a new one starts
/// from a list, which shows the recovery even for a kind in which nothing changes. The backoff
/// starts over only after two minutes without a failure, so a list that succeeds before each
/// failed watch can't make it relist at the first step forever. 410 Gone (an expired resource
/// version) isn't a failure: kube's watcher lists again at once.
async fn mirror<K>(
    api: Api<K>,
    config: watcher::Config,
    mut writer: Writer<K>,
    changed: Arc<Notify>,
    watches: Watches,
) where
    K: Resource + Clone + DeserializeOwned + Debug + Send + Sync + 'static,
    K::DynamicType: Default + Eq + Hash + Clone,
{
    let kind = K::kind(&K::DynamicType::default()).into_owned();
    watches.lock().insert(kind.clone(), None);
    let mut synced = false;
    let mut backoff = Control::watch_backoff();
    loop {
        let mut events = Box::pin(watcher(api.clone(), config.clone()));
        let failure = loop {
            match events.next().await {
                Some(Ok(event)) => {
                    writer.apply_watcher_event(&event);
                    if !matches!(event, watcher::Event::Init | watcher::Event::InitApply(_)) {
                        synced = true;
                        watches.lock().insert(kind.clone(), None);
                    }
                    changed.notify_one();
                }
                Some(Err(watcher::Error::WatchError(e))) if e.code == 410 => {}
                Some(Err(
                    watcher::Error::InitialListFailed(e)
                    | watcher::Error::WatchStartFailed(e)
                    | watcher::Error::WatchFailed(e),
                )) => break api_failure(&e),
                Some(Err(watcher::Error::WatchError(e))) => {
                    break format!("{} {}", e.code, e.reason);
                }
                Some(Err(e)) => break e.to_string(),
                None => break "the watch ended".to_owned(),
            }
        };
        eprintln!("control: watching {kind}: {failure}");
        let since = u64::try_from(Timestamp::now().as_second()).unwrap_or_default();
        watches
            .lock()
            .entry(kind.clone())
            .or_default()
            .get_or_insert(Failing { since, synced });
        if let Some(delay) = backoff.next() {
            tokio::time::sleep(delay).await;
        }
    }
}

/// A failed call to the API server, without its message, which can name a person: the object
/// the call was about, or the user it ran as.
fn api_failure(error: &kube::Error) -> String {
    match error {
        kube::Error::Api(status) => format!("{} {}", status.code, status.reason),
        kube::Error::SerdeError(e) => format!("a response didn't parse ({:?})", e.classify()),
        other => other.to_string(),
    }
}

/// One condition as control means it; `last_transition_time` is filled in when written.
struct Wanted {
    kind: &'static str,
    status: bool,
    reason: String,
    message: String,
}

/// Writes `wanted` into the object's status conditions with a server-side apply, unless they
/// already say exactly that (so writing a status doesn't wake the watch into another write).
async fn write_conditions<K>(api: &Api<K>, object: &K, wanted: Vec<Wanted>) -> Result<(), String>
where
    K: Resource<DynamicType = ()>
        + HasStatus<Status = ResourceStatus>
        + Clone
        + DeserializeOwned
        + Debug,
{
    let held = object.status().map_or_default(|s| s.conditions.as_slice());
    let generation = object.meta().generation;
    let conditions: Vec<Condition> = wanted
        .into_iter()
        .map(|w| {
            let status = if w.status { "True" } else { "False" }.to_owned();
            let previous = held.iter().find(|c| c.type_ == w.kind);
            Condition {
                last_transition_time: previous.filter(|c| c.status == status).map_or_else(
                    || Time(Timestamp::now()),
                    |c| c.last_transition_time.clone(),
                ),
                message: w.message,
                observed_generation: generation,
                reason: w.reason,
                status,
                type_: w.kind.to_owned(),
            }
        })
        .collect();
    if conditions.as_slice() == held {
        return Ok(());
    }
    let patch = json!({
        "apiVersion": K::api_version(&()),
        "kind": K::kind(&()),
        "status": {"conditions": conditions},
    });
    api.patch_status(
        &object.name_any(),
        &PatchParams::apply(FIELD_MANAGER).force(),
        &Patch::Apply(patch),
    )
    .await
    .map(drop)
    .map_err(|e| api_failure(&e))
}

/// The `Accepted` condition for an object of `kind` named `name`.
fn accepted(reconciled: &Reconciled, kind: &str, name: &str, applied: &str) -> Wanted {
    let rejection = reconciled
        .rejected
        .iter()
        .find(|r: &&Rejected| r.kind.as_str() == kind && r.name.as_str() == name);
    let Some(rejection) = rejection else {
        return Wanted {
            kind: "Accepted",
            status: true,
            reason: applied.to_owned(),
            message: String::new(),
        };
    };
    Wanted {
        kind: "Accepted",
        status: false,
        reason: rejection.reason.as_str().to_owned(),
        message: rejection.reason.message().to_owned(),
    }
}

/// Every object's verdict into its status subresource, so `kubectl get -o yaml` shows it. Failures
/// are logged by kind and count, since a `User`'s name is a person's.
async fn write_statuses(
    client: &Client,
    namespace: &str,
    desired: &Desired,
    reconciled: &Reconciled,
) {
    let mut failed: BTreeMap<(&str, String), usize> = BTreeMap::new();
    let users = Api::<User>::namespaced(client.clone(), namespace);
    for user in &desired.users {
        let wanted = vec![accepted(reconciled, "User", &user.name_any(), "Applied")];
        if let Err(why) = write_conditions(&users, user, wanted).await {
            *failed.entry(("User", why)).or_default() += 1;
        }
    }
    let models = Api::<Model>::namespaced(client.clone(), namespace);
    for model in &desired.models {
        let wanted = vec![accepted(reconciled, "Model", &model.name_any(), "Applied")];
        if let Err(why) = write_conditions(&models, model, wanted).await {
            *failed.entry(("Model", why)).or_default() += 1;
        }
    }
    let deployments = Api::<ModelDeployment>::namespaced(client.clone(), namespace);
    for deployment in &desired.deployments {
        let name = deployment.name_any();
        let ready = match reconciled.engine_ready.get(&name) {
            Some(true) => ("EngineReady", true),
            Some(false) => ("EngineNotReady", false),
            None => ("NotAdmitted", false),
        };
        let wanted = vec![
            accepted(reconciled, "ModelDeployment", &name, "Applied"),
            Wanted {
                kind: "Ready",
                status: ready.1,
                reason: ready.0.to_owned(),
                message: String::new(),
            },
        ];
        if let Err(why) = write_conditions(&deployments, deployment, wanted).await {
            *failed.entry(("ModelDeployment", why)).or_default() += 1;
        }
    }
    for ((kind, why), count) in failed {
        eprintln!("control: writing the status of {count} {kind} objects failed: {why}");
    }
}

/// The caches of every watched kind, filled by `mirror`.
struct Caches {
    users: Store<User>,
    deployments: Store<ModelDeployment>,
    models: Store<Model>,
    nodes: Store<Node>,
    endpoint_slices: Store<EndpointSlice>,
}

impl Caches {
    async fn wait_until_ready(&self) -> bool {
        self.users.wait_until_ready().await.is_ok()
            && self.deployments.wait_until_ready().await.is_ok()
            && self.models.wait_until_ready().await.is_ok()
            && self.nodes.wait_until_ready().await.is_ok()
            && self.endpoint_slices.wait_until_ready().await.is_ok()
    }

    fn desired(&self) -> Desired {
        Desired {
            users: self.users.state().iter().map(|o| User::clone(o)).collect(),
            deployments: self
                .deployments
                .state()
                .iter()
                .map(|o| ModelDeployment::clone(o))
                .collect(),
            models: self
                .models
                .state()
                .iter()
                .map(|o| Model::clone(o))
                .collect(),
            nodes: self.nodes.state().iter().map(|o| Node::clone(o)).collect(),
            endpoint_slices: self
                .endpoint_slices
                .state()
                .iter()
                .map(|o| EndpointSlice::clone(o))
                .collect(),
        }
    }
}

impl Control {
    /// The waits `watch` takes between a failed watch and the next.
    pub fn watch_backoff() -> impl Iterator<Item = Duration> {
        watcher::DefaultBackoff::default()
    }

    /// Watches the operator's custom resources and the engine `EndpointSlices` in `namespace`, and
    /// the cluster's Nodes; reconciles the policy to them
    /// after every change and writes each object's verdict into its status. Status is written by
    /// a task of its own, from the latest verdicts, so a reconcile never waits on the API server's
    /// writes. Nothing is applied until every cache holds a complete list, since a missing object
    /// reads as deleted (and a deleted User as revoked). A watch that fails starts over in its own
    /// task, which never ends; if one watch or the status writer stops all the same (a panic), the
    /// whole task ends, for `Running::ended` to report.
    pub fn watch(&self, client: Client, namespace: &str) -> Running {
        let (users, users_writer) = reflector::store::<User>();
        let (deployments, deployments_writer) = reflector::store::<ModelDeployment>();
        let (models, models_writer) = reflector::store::<Model>();
        let (nodes, nodes_writer) = reflector::store::<Node>();
        let (endpoint_slices, endpoint_slices_writer) = reflector::store::<EndpointSlice>();
        let caches = Caches {
            users,
            deployments,
            models,
            nodes,
            endpoint_slices,
        };
        let changed = Arc::new(Notify::new());
        let namespace = namespace.to_owned();
        let control = self.clone();
        let task = tokio::spawn(async move {
            let mut mirrors = JoinSet::new();
            mirrors.spawn(mirror(
                Api::<User>::namespaced(client.clone(), &namespace),
                watcher::Config::default(),
                users_writer,
                changed.clone(),
                control.watches.clone(),
            ));
            mirrors.spawn(mirror(
                Api::<ModelDeployment>::namespaced(client.clone(), &namespace),
                watcher::Config::default(),
                deployments_writer,
                changed.clone(),
                control.watches.clone(),
            ));
            mirrors.spawn(mirror(
                Api::<Model>::namespaced(client.clone(), &namespace),
                watcher::Config::default(),
                models_writer,
                changed.clone(),
                control.watches.clone(),
            ));
            mirrors.spawn(mirror(
                Api::<Node>::all(client.clone()),
                watcher::Config::default(),
                nodes_writer,
                changed.clone(),
                control.watches.clone(),
            ));
            mirrors.spawn(mirror(
                Api::<EndpointSlice>::namespaced(client.clone(), &namespace),
                watcher::Config::default().labels(
                    "app.kubernetes.io/name=engine,endpointslice.kubernetes.io/managed-by=endpointslice-controller.k8s.io",
                ),
                endpoint_slices_writer,
                changed.clone(),
                control.watches.clone(),
            ));
            if !caches.wait_until_ready().await {
                return;
            }
            let (verdicts, mut latest) = watch::channel(None::<(Arc<Desired>, Reconciled)>);
            mirrors.spawn(async move {
                while latest.changed().await.is_ok() {
                    let verdict = latest.borrow_and_update().clone();
                    if let Some((desired, reconciled)) = verdict {
                        write_statuses(&client, &namespace, &desired, &reconciled).await;
                    }
                }
            });
            loop {
                let desired = Arc::new(caches.desired());
                let reconciling = control.clone();
                let snapshot = desired.clone();
                let reconciled = tokio::task::spawn_blocking(move || {
                    reconciling.reconcile(&snapshot, SystemTime::now())
                });
                match reconciled.await {
                    Ok(Ok(reconciled)) => {
                        verdicts.send_replace(Some((desired, reconciled)));
                    }
                    Ok(Err(error)) => eprintln!("control: reconcile refused: {:?}", error.code),
                    Err(e) if e.is_panic() => {
                        eprintln!(
                            "control: reconcile panicked, refusing every call until a restart"
                        );
                        control.poison();
                    }
                    Err(_) => eprintln!("control: reconcile was cancelled"),
                }
                tokio::select! {
                    () = changed.notified() => {}
                    stopped = mirrors.join_next() => {
                        let panicked = matches!(stopped, Some(Err(e)) if e.is_panic());
                        let how = if panicked { "panicked" } else { "stopped" };
                        eprintln!("control: a watch or the status writer {how}");
                        return;
                    }
                }
            }
        });
        Running { task }
    }
}
