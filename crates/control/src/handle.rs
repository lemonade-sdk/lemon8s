use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::SystemTime;

use crate::{ErrorCode, ErrorReply};
use parking_lot::Mutex;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::desired::{Desired, Reconciled};
use crate::operation::Operation;
use crate::state::{ControlError, State};
use crate::store::Store;
use crate::watch::Watches;

/// The policy decision point, run inside the gateway's process: answers the gateway and the
/// operator through `call`.
#[derive(Clone)]
pub struct Control {
    /// `State::watches`, for the watch to write without taking the state lock.
    pub(crate) watches: Watches,
    state: Arc<Mutex<State>>,
    applied: Arc<AtomicBool>,
    /// `State::mass_revocations` as of the last reconcile, readable without waiting for the
    /// state lock.
    mass_revocations: Arc<AtomicU64>,
    /// The epoch as of the last call or reconcile, sent while the state lock is held so the
    /// values go out in order.
    epochs: watch::Sender<u64>,
    /// `State::poisoned`, readable without waiting for the state lock, and awaitable.
    poisoned: watch::Sender<bool>,
}

/// A background task of control's; dropping it stops the task.
pub struct Running {
    pub(crate) task: JoinHandle<()>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Running {
    /// Waits for the task to end or panic, which it doesn't while it runs: a watch that stopped
    /// would leave changes in Kubernetes, revocations among them, unapplied. Returns how it ended,
    /// for the caller to exit so that the process is restarted.
    pub async fn ended(&mut self) -> &'static str {
        match (&mut self.task).await {
            Ok(()) => "ended",
            Err(error) if error.is_panic() => "panicked",
            Err(_) => "was cancelled",
        }
    }
}

impl Control {
    /// Opens (or creates) the SQLite file at `database` holding the event log and provenance, and
    /// replays it, refusing to start if the replay does not reproduce the stored provenance.
    /// Starting bumps the epoch.
    pub fn open(database: &Path) -> Result<Self, ControlError> {
        let store = Store::open(database).map_err(ControlError::Store)?;
        let state = State::open(store)?;
        Ok(Self {
            epochs: watch::Sender::new(state.epoch()),
            watches: state.watches.clone(),
            mass_revocations: Arc::new(AtomicU64::new(0)),
            state: Arc::new(Mutex::new(state)),
            applied: Arc::new(AtomicBool::new(false)),
            poisoned: watch::Sender::new(false),
        })
    }

    pub fn epoch(&self) -> u64 {
        self.state.lock().epoch()
    }

    /// The policy epoch, changing as soon as a call or reconcile advances it. Every revocation
    /// advances it, so a reader of the revocation feed that reads again on each change, from the
    /// epoch it last read, misses none: changes that come faster than it reads coalesce into one
    /// read that lists them all.
    pub fn epochs(&self) -> watch::Receiver<u64> {
        self.epochs.subscribe()
    }

    /// Each kind `watch` watches, with when its watch began failing (seconds since the Unix
    /// epoch), or `None` while its cache matches the API server. Empty until `watch` starts.
    pub fn watches(&self) -> BTreeMap<String, Option<u64>> {
        self.watches
            .lock()
            .iter()
            .map(|(kind, failing)| (kind.clone(), failing.map(|f| f.since)))
            .collect()
    }

    /// Applies the operator's configuration as observed `now` (see `State::reconcile`); blocks on
    /// the database.
    pub fn reconcile(&self, desired: &Desired, now: SystemTime) -> Result<Reconciled, ErrorReply> {
        let mut state = self.state.lock();
        let reconciled = state.reconcile(desired, now);
        let epoch = state.epoch();
        self.mass_revocations
            .store(state.mass_revocations(), Ordering::Release);
        self.epochs
            .send_if_modified(|published| std::mem::replace(published, epoch) != epoch);
        if state.poisoned() {
            self.poisoned
                .send_if_modified(|poisoned| !std::mem::replace(poisoned, true));
        }
        drop(state);
        let reconciled = reconciled?;
        self.applied.store(true, Ordering::Release);
        Ok(reconciled)
    }

    /// How many mass revocations (`Status::mass_revocation`) this process has seen begin, for the
    /// gateway's metrics.
    pub fn mass_revocations(&self) -> u64 {
        self.mass_revocations.load(Ordering::Acquire)
    }

    /// Whether a reconcile since this process started has applied the operator's configuration:
    /// until then, control decides from its event log alone.
    pub fn policy_applied(&self) -> bool {
        self.applied.load(Ordering::Acquire)
    }

    /// Whether a write to the event log failed or a call or reconcile panicked: memory may be
    /// ahead of disk, so control refuses every call until a restart replays the log.
    pub fn poisoned(&self) -> bool {
        *self.poisoned.borrow()
    }

    /// Refuses every call from now on, until a restart replays the log.
    pub(crate) fn poison(&self) {
        self.state.lock().poison();
        self.poisoned
            .send_if_modified(|poisoned| !std::mem::replace(poisoned, true));
    }

    /// Resolves once control is poisoned (at once if it is), for the process to stop and
    /// restart.
    pub async fn until_poisoned(&self) {
        let mut poisoned = self.poisoned.subscribe();
        drop(poisoned.wait_for(|poisoned| *poisoned).await);
    }

    /// Runs one operation against control's state.
    pub async fn call<R: Operation>(&self, call: R) -> Result<R::Reply, ErrorReply> {
        let state = self.state.clone();
        let epochs = self.epochs.clone();
        let poisoned = self.poisoned.clone();
        tokio::task::spawn_blocking(move || {
            let mut state = state.lock();
            if state.poisoned() {
                return Err(ErrorReply::from(ErrorCode::Unavailable));
            }
            let reply = R::run(&mut state, call);
            let epoch = state.epoch();
            epochs.send_if_modified(|published| std::mem::replace(published, epoch) != epoch);
            if state.poisoned() {
                poisoned.send_if_modified(|poisoned| !std::mem::replace(poisoned, true));
            }
            reply
        })
        .await
        .unwrap_or_else(|error| {
            if error.is_panic() {
                self.poison();
            }
            Err(ErrorCode::Internal.into())
        })
    }
}
