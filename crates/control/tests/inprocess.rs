//! Control as a library inside the gateway, with no listener. Operations run
//! through `Control::call`, configuration arrives through `Control::reconcile`, and engine
//! readiness through `EndpointSlices`.

mod common;

use std::time::SystemTime;

use common::{
    DEPLOYMENT, MODEL, NODE, OTHER_DEPLOYMENT, denial, deployment, endpoint_slice, key_hash, label,
    name, names, operator_config, user,
};
use control::{
    CloseSession, CompleteRequest, CreateSession, Dispatch, ErrorCode, ErrorReply, ListModels,
    OpenRequest, ReadProvenance, ReadRevocations, ReadStatus,
};
use control::{Control, Desired};
use policy::{Denial, DeploymentId};

struct LoneBox {
    _dir: tempfile::TempDir,
    control: Control,
}

impl LoneBox {
    /// The system with one engine: engine-test on node-a behind Service `lemon8s/engine-test`.
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let control = Control::open(&dir.path().join("control.sqlite")).unwrap();
        control
            .reconcile(&Self::config(true), SystemTime::now())
            .unwrap();
        Self { _dir: dir, control }
    }

    fn config(engine_ready: bool) -> Desired {
        let mut config = operator_config();
        config.deployments = vec![deployment(DEPLOYMENT, NODE, &["finance"])];
        config.endpoint_slices = vec![endpoint_slice("lemon8s", DEPLOYMENT, NODE, engine_ready)];
        config
    }

    async fn session(&self, user: &str, key: &str, compartments: &[&str]) -> u64 {
        self.control
            .call(CreateSession {
                user: name(user),
                label: names(compartments),
                key_hash: key_hash(key),
            })
            .await
            .unwrap()
            .session
    }

    async fn open(&self, key: &str) -> Result<control::RequestOpened, control::ErrorReply> {
        self.control
            .call(OpenRequest {
                key_hash: key_hash(key),
                model: Some(name(MODEL)),
            })
            .await
    }
}

#[tokio::test]
async fn a_single_host_authorizes_starts_and_completes_a_request() {
    let lone = LoneBox::new();
    lone.session("alice", "key-a", &["finance"]).await;

    let opened = lone.open("key-a").await.unwrap();
    assert_eq!(opened.candidates.len(), 1);
    let dispatched = lone
        .control
        .call(Dispatch {
            key_hash: key_hash("key-a"),
            request: opened.request,
            deployment: name(DEPLOYMENT),
        })
        .await
        .unwrap();

    assert_eq!(dispatched.node, name(NODE));
    lone.control
        .call(CompleteRequest {
            key_hash: key_hash("key-a"),
            request: opened.request,
        })
        .await
        .unwrap();
    let again = lone
        .control
        .call(Dispatch {
            key_hash: key_hash("key-a"),
            request: opened.request,
            deployment: name(DEPLOYMENT),
        })
        .await;
    assert_eq!(denial(again), Denial::RequestNotUnstarted);
}

#[tokio::test]
async fn an_engine_whose_endpoints_are_not_ready_is_not_placeable() {
    let lone = LoneBox::new();
    lone.session("alice", "key-a", &["finance"]).await;
    let opened = lone.open("key-a").await.unwrap();

    lone.control
        .reconcile(&LoneBox::config(false), SystemTime::now())
        .unwrap();

    let refused = lone
        .control
        .call(Dispatch {
            key_hash: key_hash("key-a"),
            request: opened.request,
            deployment: name(DEPLOYMENT),
        })
        .await;
    assert_eq!(
        denial(refused),
        Denial::DeploymentNotReady {
            deployment: DeploymentId::new(DEPLOYMENT)
        }
    );
    let next = lone
        .control
        .call(ReadProvenance { since: 0 })
        .await
        .unwrap()
        .next;
    assert_eq!(
        lone.open("key-a").await.unwrap_err(),
        ErrorReply {
            code: ErrorCode::NotReady,
            denial: None,
            unready: vec![DeploymentId::new(DEPLOYMENT)],
        },
        "a model the session may use whose engine isn't ready is no denial, and names the engine"
    );
    let since = lone
        .control
        .call(ReadProvenance { since: next })
        .await
        .unwrap();
    assert_eq!(since.records, [], "and records nothing");
    let status = lone.control.call(ReadStatus {}).await.unwrap();
    assert!(!status.deployments[0].ready);

    lone.control
        .reconcile(&LoneBox::config(true), SystemTime::now())
        .unwrap();
    lone.control
        .call(Dispatch {
            key_hash: key_hash("key-a"),
            request: opened.request,
            deployment: name(DEPLOYMENT),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn an_engine_with_no_readiness_source_is_never_placeable() {
    let lone = LoneBox::new();
    let mut config = LoneBox::config(true);
    let mut unwatched = deployment(OTHER_DEPLOYMENT, NODE, &["finance"]);
    unwatched.spec.engine_service = None;
    config.deployments.push(unwatched);

    lone.control.reconcile(&config, SystemTime::now()).unwrap();

    let status = lone.control.call(ReadStatus {}).await.unwrap();
    let ready: Vec<(&str, bool)> = status
        .deployments
        .iter()
        .map(|d| (d.deployment.as_str(), d.ready))
        .collect();
    assert_eq!(ready, [(OTHER_DEPLOYMENT, false), (DEPLOYMENT, true)]);
}

#[tokio::test]
async fn the_revocation_feed_lists_sessions_that_lost_access_since_an_epoch() {
    let lone = LoneBox::new();
    let alice = lone.session("alice", "key-a", &["finance"]).await;
    let bob = lone.session("bob", "key-b", &["finance"]).await;
    let start = lone
        .control
        .call(ReadRevocations { since: 0 })
        .await
        .unwrap();
    assert_eq!(start.revoked, []);

    lone.control
        .call(CloseSession {
            user: name("alice"),
            session: alice,
        })
        .await
        .unwrap();
    let mut config = LoneBox::config(true);
    config.users[1] = user("bob", &["finance"], true);
    lone.control.reconcile(&config, SystemTime::now()).unwrap();

    let feed = lone
        .control
        .call(ReadRevocations { since: start.epoch })
        .await
        .unwrap();
    let sessions: Vec<(u64, Option<control::Name>)> = feed
        .revoked
        .iter()
        .map(|r| (r.session, r.deployment.clone()))
        .collect();
    assert_eq!(sessions, [(alice, None), (bob, None)]);
    assert!(feed.revoked.iter().all(|r| r.epoch > start.epoch));
    assert_eq!(feed.epoch, lone.control.epoch());
    let caught_up = lone
        .control
        .call(ReadRevocations { since: feed.epoch })
        .await
        .unwrap();
    assert_eq!(caught_up.revoked, []);
}

/// The gateway reads the feed each time `epochs` changes, so every revocation must change it
/// by the time the call or reconcile that made it returns.
#[tokio::test]
async fn the_epoch_channel_moves_with_every_revocation_and_only_with_the_epoch() {
    let lone = LoneBox::new();
    let mut epochs = lone.control.epochs();
    assert_eq!(*epochs.borrow_and_update(), lone.control.epoch());

    let alice = lone.session("alice", "key-a", &["finance"]).await;
    lone.session("bob", "key-b", &["finance"]).await;
    lone.control
        .reconcile(&LoneBox::config(true), SystemTime::now())
        .unwrap();
    assert!(
        !epochs.has_changed().unwrap(),
        "sessions and an unchanged configuration leave the epoch"
    );

    lone.control
        .call(CloseSession {
            user: name("alice"),
            session: alice,
        })
        .await
        .unwrap();
    assert!(epochs.has_changed().unwrap());
    assert_eq!(*epochs.borrow_and_update(), lone.control.epoch());

    let mut config = LoneBox::config(true);
    config.users[1] = user("bob", &["finance"], true);
    lone.control.reconcile(&config, SystemTime::now()).unwrap();
    assert!(epochs.has_changed().unwrap());
    assert_eq!(*epochs.borrow_and_update(), lone.control.epoch());
}

#[tokio::test]
async fn listing_models_reads_eligibility_and_records_nothing() {
    let lone = LoneBox::new();
    lone.session("alice", "key-a", &["finance"]).await;
    let recorded = lone
        .control
        .call(ReadStatus {})
        .await
        .unwrap()
        .provenance_records;

    let listed = lone
        .control
        .call(ListModels {
            key_hash: key_hash("key-a"),
        })
        .await
        .unwrap();
    assert_eq!(listed.models, [name(MODEL)]);
    assert_eq!(listed.label, label(&["finance"]));
    lone.control
        .reconcile(&LoneBox::config(false), SystemTime::now())
        .unwrap();
    let unready = lone
        .control
        .call(ListModels {
            key_hash: key_hash("key-a"),
        })
        .await
        .unwrap();
    assert_eq!(unready.models, []);

    let after = lone
        .control
        .call(ReadStatus {})
        .await
        .unwrap()
        .provenance_records;
    assert_eq!(after, recorded + 1);
    let unknown = lone
        .control
        .call(ListModels {
            key_hash: key_hash("nope"),
        })
        .await;
    assert_eq!(unknown.unwrap_err().code, ErrorCode::InvalidApiKey);
}
