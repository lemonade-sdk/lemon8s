//! lemon8s `control`, the policy decision point: decides on metadata only and keeps its state as
//! an append-only SQLite event log replayed at startup. Every field its calls carry is a `Name`,
//! a `KeyHash`, a number or a list of those (`Name` refuses free text), so no call can carry
//! prompt or output content to control.

#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
        clippy::dbg_macro,
        clippy::indexing_slicing
    )
)]

mod call;
pub mod crd;
mod desired;
mod event;
mod handle;
mod operation;
mod state;
mod store;
mod watch;
mod wire;

pub use call::{
    Candidate, CheckKey, CloseSession, CompleteRequest, CreateSession, DeploymentStatus, Dispatch,
    Dispatched, Done, ErrorCode, ErrorReply, FailRequest, ListModels, ModelList, NodeStatus,
    OpenRequest, OpenSession, ProvenancePage, ReadProvenance, ReadRevocations, ReadStatus, Reason,
    Rejected, RequestOpened, Revocations, RevokedSession, SessionCreated, StaleWatch, Status,
};
pub use desired::{Desired, NodeTarget, REVOCATION_WINDOW, Reconciled};
pub use handle::{Control, Running};
pub use operation::Operation;
pub use state::ControlError;
pub use store::{LOG_FORMAT, StoreError};
pub use wire::{KeyHash, Name, WireError};
