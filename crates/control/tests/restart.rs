//! Control restarted from its SQLite file: revocations stay in force, the epoch only rises,
//! session keys and provenance survive, the log can't be rewritten, and a file this build can't
//! read is refused.

mod common;

use std::time::{Duration, SystemTime};

use common::{DEPLOYMENT, key_hash, name, operator_config, session, user};
use control::{Control, ControlError, LOG_FORMAT, StoreError};
use control::{Dispatch, ErrorCode, OpenRequest, ReadProvenance, ReadStatus};
use policy::ProvenanceRecord;
use rusqlite::Connection;

/// The `application_id` control stamps on a log it creates.
fn controls_mark() -> i32 {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.sqlite");
    drop(Control::open(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .unwrap()
}

async fn all_provenance(control: &Control) -> Vec<ProvenanceRecord> {
    let mut records = Vec::new();
    let mut since = 0;
    loop {
        let page = control.call(ReadProvenance { since }).await.unwrap();
        if page.records.is_empty() {
            return records;
        }
        since = page.next;
        records.extend(page.records);
    }
}

fn open_request(key: &str) -> OpenRequest {
    OpenRequest {
        key_hash: key_hash(key),
        model: None,
    }
}

#[tokio::test]
async fn a_revocation_survives_a_restart_and_the_epoch_does_not_go_back() {
    let dir = tempfile::tempdir().unwrap();
    let first = Control::open(&dir.path().join("control.sqlite")).unwrap();
    first
        .reconcile(&operator_config(), SystemTime::now())
        .unwrap();
    session(&first, "alice", "key-a", &["finance"]).await;
    session(&first, "bob", "key-b", &["finance"]).await;
    let alices = first.call(open_request("key-a")).await.unwrap();
    first
        .call(Dispatch {
            key_hash: key_hash("key-a"),
            request: alices.request,
            deployment: name(DEPLOYMENT),
        })
        .await
        .unwrap();
    let mut revoked = operator_config();
    revoked.users[1] = user("bob", &["finance"], true);
    first.reconcile(&revoked, SystemTime::now()).unwrap();
    let epoch_before = first.epoch();
    let provenance_before = all_provenance(&first).await;
    drop(first);

    let second = Control::open(&dir.path().join("control.sqlite")).unwrap();

    assert!(second.epoch() > epoch_before);
    assert_eq!(
        second.call(open_request("key-b")).await.unwrap_err().code,
        ErrorCode::InvalidApiKey,
        "a revoked user's keys are gone"
    );
    let alices_again = second.call(open_request("key-a")).await.unwrap();
    assert!(alices_again.request > alices.request);
    let provenance_after = all_provenance(&second).await;
    assert_eq!(
        provenance_after[..provenance_before.len()],
        provenance_before[..]
    );
    let status = second.call(ReadStatus {}).await.unwrap();
    assert_eq!(status.epoch, second.epoch());
}

#[tokio::test]
async fn the_log_is_append_only() {
    let dir = tempfile::tempdir().unwrap();
    let control = Control::open(&dir.path().join("control.sqlite")).unwrap();
    let mut revoked = operator_config();
    revoked.users[1] = user("bob", &["finance"], true);
    control.reconcile(&revoked, SystemTime::now()).unwrap();

    let connection = Connection::open(dir.path().join("control.sqlite")).unwrap();
    for statement in [
        "DELETE FROM events WHERE body LIKE '%\"revoked\":true%'",
        "UPDATE events SET body = '{\"kind\":\"started\"}'",
        "DELETE FROM provenance",
        "UPDATE provenance SET body = '{}'",
    ] {
        let refused = connection.execute(statement, []).unwrap_err();
        assert!(refused.to_string().contains("append-only"), "{refused}");
    }
}

#[tokio::test]
async fn a_log_that_replays_differently_than_recorded_refuses_to_start() {
    let dir = tempfile::tempdir().unwrap();
    let control = Control::open(&dir.path().join("control.sqlite")).unwrap();
    control
        .reconcile(&operator_config(), SystemTime::now())
        .unwrap();
    drop(control);
    let connection = Connection::open(dir.path().join("control.sqlite")).unwrap();
    let (seq, body): (i64, String) = connection
        .query_row(
            "SELECT seq, body FROM provenance ORDER BY seq DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let mut forged: ProvenanceRecord = serde_json::from_str(&body).unwrap();
    forged.seq += 1;
    connection
        .execute(
            "INSERT INTO provenance (seq, event_seq, body) VALUES (?1, 1, ?2)",
            rusqlite::params![seq + 1, serde_json::to_string(&forged).unwrap()],
        )
        .unwrap();

    match Control::open(&dir.path().join("control.sqlite")) {
        Err(ControlError::ReplayDiverged { seq: at }) => {
            assert_eq!(at, u64::try_from(seq + 1).unwrap());
        }
        Err(other) => panic!("expected a replay divergence, got {other}"),
        Ok(_) => panic!("control started on a forged log"),
    }
}

#[tokio::test]
async fn a_failed_write_fails_closed_until_a_restart_replays_the_log() {
    let dir = tempfile::tempdir().unwrap();
    let first = Control::open(&dir.path().join("control.sqlite")).unwrap();
    first
        .reconcile(&operator_config(), SystemTime::now())
        .unwrap();
    session(&first, "alice", "key-a", &["finance"]).await;

    let locker = Connection::open(dir.path().join("control.sqlite")).unwrap();
    locker.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let mut revoked = operator_config();
    revoked.users[0] = user("alice", &["finance", "legal"], true);
    assert!(!first.poisoned());
    assert_eq!(
        first
            .reconcile(&revoked, SystemTime::now())
            .unwrap_err()
            .code,
        ErrorCode::Unavailable
    );
    assert!(first.poisoned(), "the gateway's readiness reads this");
    tokio::time::timeout(Duration::from_secs(1), first.until_poisoned())
        .await
        .expect("the gateway's command waits on this to stop and restart");
    locker.execute_batch("ROLLBACK").unwrap();

    assert_eq!(
        first.call(ReadStatus {}).await.unwrap_err().code,
        ErrorCode::Unavailable
    );
    assert_eq!(
        first.call(open_request("key-a")).await.unwrap_err().code,
        ErrorCode::Unavailable
    );
    drop(locker);
    drop(first);

    let second = Control::open(&dir.path().join("control.sqlite")).unwrap();
    assert!(!second.poisoned());
    second.call(open_request("key-a")).await.unwrap();
    second.reconcile(&revoked, SystemTime::now()).unwrap();
    assert_eq!(
        second.call(open_request("key-a")).await.unwrap_err().code,
        ErrorCode::InvalidApiKey,
        "a revoked user's keys are gone"
    );
}

#[tokio::test]
async fn a_new_log_records_its_format_and_replays() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.sqlite");
    let first = Control::open(&path).unwrap();
    first
        .reconcile(&operator_config(), SystemTime::now())
        .unwrap();
    let provenance_before = all_provenance(&first).await;
    drop(first);

    let format: i64 = Connection::open(&path)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(format, LOG_FORMAT);
    assert_ne!(controls_mark(), 0, "a new log carries no application id");
    let second = Control::open(&path).unwrap();
    assert_eq!(
        all_provenance(&second).await[..provenance_before.len()],
        provenance_before[..]
    );
}

#[tokio::test]
async fn a_file_this_build_cant_read_is_refused_untouched_with_how_to_start_fresh() {
    let newer = LOG_FORMAT + 1;
    let mark = controls_mark();
    let notes = "CREATE TABLE notes (body TEXT NOT NULL); INSERT INTO notes VALUES ('a list');";
    // `true`: starts as control's own log; `Some(format)`: control's log in a newer format;
    // `None`: not control's log.
    for (from_control, file, newer_format) in [
        (true, format!("PRAGMA user_version = {newer};"), Some(newer)),
        (false, notes.to_owned(), None),
        // Another program's file, at the version programs that set one most often use.
        (false, format!("{notes} PRAGMA user_version = 1;"), None),
        // Control's tables under another program's mark.
        (true, format!("PRAGMA application_id = {};", mark + 1), None),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.sqlite");
        if from_control {
            drop(Control::open(&path).unwrap());
        }
        Connection::open(&path)
            .unwrap()
            .execute_batch(&file)
            .unwrap();
        let before = std::fs::read(&path).unwrap();

        let Err(refused) = Control::open(&path) else {
            panic!("control started on {file}");
        };
        let said = if let Some(found) = newer_format {
            assert!(
                matches!(&refused, ControlError::Store(StoreError::Format { found: f, .. }) if *f == found),
                "{refused}"
            );
            format!("is in event-log format {newer}, which a newer lemon8s wrote")
        } else {
            assert!(
                matches!(&refused, ControlError::Store(StoreError::Foreign { .. })),
                "{refused}"
            );
            "isn't control's event log".to_owned()
        };
        let message = refused.to_string();
        for part in [
            &path.display().to_string(),
            &said,
            "then `helm uninstall` the release, delete the PersistentVolumeClaim holding the log",
        ] {
            assert!(message.contains(part), "{message}");
        }
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "refusing changed the file"
        );
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "refusing left a copy or a journal"
        );
    }
}
