//! Revocations apply at once, however many there are, and a revoked user's keys stop working.
//! Revoking within ten minutes more users than remain active, as a wrong namespace or a mistaken
//! delete would, raises an alert: a status entry and a count for the gateway's metrics.

mod common;

use std::time::{Duration, SystemTime};

use common::{key_hash, name, names, operator_config, user};
use control::{CheckKey, Control, CreateSession, Desired, ErrorCode, Name, ReadStatus};
use policy::Denial;

const MINUTE: Duration = Duration::from_mins(1);

const TEAM_OF_TEN: [&str; 10] = [
    "alice", "bob", "carol", "dave", "erin", "frank", "grace", "heidi", "ivan", "judy",
];

struct System {
    dir: tempfile::TempDir,
    control: Control,
    /// When the first sync happens; each test counts its syncs from here.
    t0: SystemTime,
}

impl System {
    /// `operator_config`'s Alice and Bob plus `others` (cleared for finance), each with a finance
    /// session under key `key-<initial>`.
    async fn with(others: &[&str]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let control = Control::open(&dir.path().join("control.sqlite")).unwrap();
        // Far from the wall clock, so only the times reconcile was given count.
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_hours(500_000);
        let mut config = operator_config();
        config
            .users
            .extend(others.iter().map(|who| user(who, &["finance"], false)));
        control.reconcile(&config, t0).unwrap();
        for who in ["alice", "bob"].iter().chain(others) {
            control
                .call(CreateSession {
                    user: name(who),
                    label: names(&["finance"]),
                    key_hash: key_hash(&format!("key-{}", &who[..1])),
                })
                .await
                .unwrap();
        }
        Self { dir, control, t0 }
    }

    /// The same system after control restarts and replays its log.
    fn restarted(self) -> Self {
        let Self { dir, control, t0 } = self;
        drop(control);
        let control = Control::open(&dir.path().join("control.sqlite")).unwrap();
        Self { dir, control, t0 }
    }

    async fn revoked(&self, key: &str) -> bool {
        match self
            .control
            .call(CheckKey {
                key_hash: key_hash(key),
            })
            .await
        {
            Ok(_) => false,
            Err(refused) if refused.code == ErrorCode::InvalidApiKey => true,
            Err(other) => panic!("unexpected refusal: {other:?}"),
        }
    }

    async fn mass_revocation(&self) -> Vec<Name> {
        self.control
            .call(ReadStatus {})
            .await
            .unwrap()
            .mass_revocation
    }
}

/// The configuration with only the users `present`, cleared as `System::with` added them, those
/// in `flagged` flagged revoked.
fn config(present: &[&str], flagged: &[&str]) -> Desired {
    let mut config = operator_config();
    config.users = present
        .iter()
        .map(|who| {
            let clearance: &[&str] = if *who == "alice" {
                &["finance", "legal"]
            } else {
                &["finance"]
            };
            user(who, clearance, flagged.contains(who))
        })
        .collect();
    config
}

#[tokio::test]
async fn deleting_every_user_revokes_them_at_once_and_raises_the_alert() {
    let sys = System::with(&["carol"]).await;

    let reconciled = sys
        .control
        .reconcile(&config(&[], &[]), sys.t0 + MINUTE)
        .unwrap();

    for key in ["key-a", "key-b", "key-c"] {
        assert!(sys.revoked(key).await, "{key}");
    }
    assert_eq!(
        sys.mass_revocation().await,
        names(&["alice", "bob", "carol"])
    );
    assert_eq!(sys.control.mass_revocations(), 1);
    assert_eq!(
        reconciled.engine_ready.len(),
        2,
        "the rest of the sync applies"
    );
}

#[tokio::test]
async fn revoking_a_minority_raises_no_alert() {
    let sys = System::with(&["carol"]).await;

    sys.control
        .reconcile(&config(&["alice", "bob", "carol"], &["carol"]), sys.t0)
        .unwrap();

    assert!(sys.revoked("key-c").await);
    assert!(!sys.revoked("key-b").await);
    assert_eq!(sys.mass_revocation().await, []);
    assert_eq!(sys.control.mass_revocations(), 0);
}

#[tokio::test]
async fn a_revoked_users_keys_stop_working_and_stay_gone_after_a_restart() {
    let sys = System::with(&["carol"]).await;
    sys.control
        .reconcile(&config(&["alice", "bob", "carol"], &["bob"]), sys.t0)
        .unwrap();
    assert!(sys.revoked("key-b").await);

    let sys = sys.restarted();

    assert!(sys.revoked("key-b").await);
    assert!(!sys.revoked("key-a").await);
    let refused = sys
        .control
        .call(CreateSession {
            user: name("bob"),
            label: names(&["finance"]),
            key_hash: key_hash("key-bob-again"),
        })
        .await
        .unwrap_err();
    assert_eq!(refused.denial, Some(Denial::UserRevoked));
}

#[tokio::test]
async fn a_team_of_ten_flagging_one_a_minute_raises_the_alert_at_the_sixth() {
    let sys = System::with(&TEAM_OF_TEN[2..]).await;

    let mut flagged = Vec::new();
    let mut alerts = Vec::new();
    for (minutes, who) in (1..).zip(["judy", "ivan", "heidi", "grace", "frank", "erin"]) {
        flagged.push(who);
        sys.control
            .reconcile(&config(&TEAM_OF_TEN, &flagged), sys.t0 + minutes * MINUTE)
            .unwrap();
        assert!(sys.revoked(&format!("key-{}", &who[..1])).await, "{who}");
        alerts.push(sys.mass_revocation().await);
    }

    assert!(alerts[..5].iter().all(Vec::is_empty), "{alerts:?}");
    assert_eq!(alerts[5], names(&TEAM_OF_TEN[4..]));
    assert_eq!(sys.control.mass_revocations(), 1);
}

#[tokio::test]
async fn a_revocation_counts_for_ten_minutes() {
    let everyone = ["alice", "bob", "carol"];
    for (after, alert) in [
        (Duration::from_secs(10 * 60 - 1), true),
        (Duration::from_secs(10 * 60 + 1), false),
    ] {
        let sys = System::with(&["carol"]).await;
        sys.control
            .reconcile(&config(&everyone, &["carol"]), sys.t0)
            .unwrap();

        sys.control
            .reconcile(&config(&everyone, &["bob", "carol"]), sys.t0 + after)
            .unwrap();

        assert!(sys.revoked("key-b").await);
        assert_eq!(
            sys.mass_revocation().await,
            if alert {
                names(&["bob", "carol"])
            } else {
                Vec::new()
            },
            "bob flagged {after:?} after carol"
        );
    }
}

#[tokio::test]
async fn the_alert_ends_once_its_revocations_age_out_and_a_restart_rebuilds_it() {
    let sys = System::with(&["carol"]).await;
    let nobody = config(&[], &[]);
    sys.control.reconcile(&nobody, sys.t0 + MINUTE).unwrap();
    let sys = sys.restarted();

    sys.control.reconcile(&nobody, sys.t0 + 2 * MINUTE).unwrap();
    let rebuilt = sys.mass_revocation().await;
    sys.control
        .reconcile(&nobody, sys.t0 + 12 * MINUTE)
        .unwrap();

    assert_eq!(rebuilt, names(&["alice", "bob", "carol"]));
    assert_eq!(sys.mass_revocation().await, []);
}

#[tokio::test]
async fn users_created_already_revoked_never_had_access_and_never_count() {
    let sys = System::with(&["carol"]).await;
    let mut with_former = config(&["alice", "bob", "carol"], &[]);
    with_former.users.extend(
        ["erin", "frank", "grace"]
            .iter()
            .map(|who| user(who, &["finance"], true)),
    );
    sys.control
        .reconcile(&with_former, sys.t0 + MINUTE)
        .unwrap();
    let mut carol_flagged = with_former.clone();
    carol_flagged.users[2] = user("carol", &["finance"], true);

    sys.control
        .reconcile(&carol_flagged, sys.t0 + 2 * MINUTE)
        .unwrap();

    assert!(sys.revoked("key-c").await);
    assert_eq!(sys.mass_revocation().await, [], "one of three");
    let refused = sys
        .control
        .call(CreateSession {
            user: name("erin"),
            label: names(&[]),
            key_hash: key_hash("key-erin"),
        })
        .await
        .unwrap_err();
    assert_eq!(refused.denial, Some(Denial::UserRevoked));
}
