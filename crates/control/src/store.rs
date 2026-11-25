use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use policy::ProvenanceRecord;
use rusqlite::{Connection, params};

use crate::event::Event;

/// The event log's format, kept in SQLite's `user_version`. A change that a log written before
/// can't be replayed under, or a new event kind or field, raises it, so that an older lemon8s
/// refuses the log instead of failing on the event.
pub const LOG_FORMAT: i64 = 1;

/// How an operator starts over when control can't open its log.
const START_FRESH: &str = "then `helm uninstall` the release, delete the \
    PersistentVolumeClaim holding the log, and install again: that starts a fresh log";

/// Marks a SQLite file as control's log in its header's `application_id`: "lem8" in ASCII.
const APPLICATION_ID: i32 = 0x6c65_6d38;

const PRAGMAS: &str = "
PRAGMA journal_mode = WAL;
PRAGMA synchronous = FULL;
PRAGMA foreign_keys = ON;
";

const SCHEMA: &str = "
CREATE TABLE events (
    seq INTEGER PRIMARY KEY,
    recorded_at_ms INTEGER NOT NULL,
    body TEXT NOT NULL
) STRICT;
CREATE TABLE provenance (
    seq INTEGER PRIMARY KEY,
    event_seq INTEGER NOT NULL REFERENCES events (seq),
    body TEXT NOT NULL
) STRICT;
CREATE TRIGGER events_no_update BEFORE UPDATE ON events
    BEGIN SELECT RAISE(ABORT, 'events are append-only'); END;
CREATE TRIGGER events_no_delete BEFORE DELETE ON events
    BEGIN SELECT RAISE(ABORT, 'events are append-only'); END;
CREATE TRIGGER provenance_no_update BEFORE UPDATE ON provenance
    BEGIN SELECT RAISE(ABORT, 'provenance is append-only'); END;
CREATE TRIGGER provenance_no_delete BEFORE DELETE ON provenance
    BEGIN SELECT RAISE(ABORT, 'provenance is append-only'); END;
";

#[derive(Debug)]
pub enum StoreError {
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    /// The log at `path` is in format `found`, which a newer lemon8s wrote.
    Format {
        path: PathBuf,
        found: i64,
    },
    /// The file at `path` isn't control's log.
    Foreign {
        path: PathBuf,
    },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Sqlite(e) => write!(f, "sqlite: {e}"),
            StoreError::Json(e) => write!(f, "stored record: {e}"),
            StoreError::Format { path, found } => write!(
                f,
                "{} is in event-log format {found}, which a newer lemon8s wrote; this lemon8s \
                 reads format {LOG_FORMAT}, so it can't replay it. That lemon8s or a newer one \
                 keeps its provenance history. To start over instead, copy this file off the \
                 gateway's volume first, {START_FRESH}",
                path.display()
            ),
            StoreError::Foreign { path } => write!(
                f,
                "{} isn't control's event log. Copy it off the gateway's volume if you need it, \
                 {START_FRESH}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sqlite(e)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        StoreError::Json(e)
    }
}

/// `at` in milliseconds since the Unix epoch, as the log records when each event happened.
pub(crate) fn millis(at: SystemTime) -> i64 {
    at.duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Control's durable state: the append-only event log and the provenance each event produced,
/// written in one transaction per event.
pub struct Store {
    connection: Connection,
}

impl Store {
    /// Opens the log at `path`, creating it when the file is new or empty. A log in a newer
    /// format, or a file that isn't control's log, is refused before anything in it changes.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let mut connection = Connection::open(path)?;
        // Control is the only writer, so a lock means something foreign: fail now, not in 5 s.
        connection.busy_timeout(Duration::ZERO)?;
        let application: i32 =
            connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
        let found: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let objects: i64 =
            connection.query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))?;
        if application == 0 && found == 0 && objects == 0 {
            connection.execute_batch(PRAGMAS)?;
            let transaction = connection.transaction()?;
            transaction.execute_batch(SCHEMA)?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.pragma_update(None, "user_version", LOG_FORMAT)?;
            transaction.commit()?;
            return Ok(Self { connection });
        }
        match (application, found) {
            (APPLICATION_ID, LOG_FORMAT) => {
                connection.execute_batch(PRAGMAS)?;
                Ok(Self { connection })
            }
            (APPLICATION_ID, found) if found > LOG_FORMAT => Err(StoreError::Format {
                path: path.to_owned(),
                found,
            }),
            _ => Err(StoreError::Foreign {
                path: path.to_owned(),
            }),
        }
    }

    /// Every event in order, with when it was recorded (`millis`).
    pub fn events(&self) -> Result<Vec<(i64, Event)>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT recorded_at_ms, body FROM events ORDER BY seq")?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<(i64, String)>, _>>()?;
        Ok(rows
            .iter()
            .map(|(recorded_at_ms, body)| Ok((*recorded_at_ms, serde_json::from_str(body)?)))
            .collect::<Result<Vec<(i64, Event)>, serde_json::Error>>()?)
    }

    pub fn provenance(&self) -> Result<Vec<ProvenanceRecord>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT body FROM provenance ORDER BY seq")?;
        let bodies = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(bodies
            .iter()
            .map(|body| serde_json::from_str(body))
            .collect::<Result<Vec<ProvenanceRecord>, _>>()?)
    }

    /// Durable on return (`synchronous = FULL`), or nothing was written.
    pub fn append(
        &mut self,
        event: &Event,
        records: &[ProvenanceRecord],
        recorded_at_ms: i64,
    ) -> Result<(), StoreError> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO events (recorded_at_ms, body) VALUES (?1, ?2)",
            params![recorded_at_ms, serde_json::to_string(event)?],
        )?;
        let event_seq = transaction.last_insert_rowid();
        for record in records {
            transaction.execute(
                "INSERT INTO provenance (seq, event_seq, body) VALUES (?1, ?2, ?3)",
                params![record.seq, event_seq, serde_json::to_string(record)?],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
}
