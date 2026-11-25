use policy::{Denial, DeploymentId, Label, NodeState, ProvenanceRecord};
use serde::{Deserialize, Serialize};

use crate::wire::{KeyHash, Name};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// The policy refused; `denial` says why.
    Denied,
    /// No open session has this key hash.
    InvalidApiKey,
    /// The user, deployment or key is already registered.
    Conflict,
    /// The session may use the model it named, but no deployment serving it is ready now; nothing
    /// is recorded. `unready` lists those deployments.
    NotReady,
    /// Control could not persist a decision and refuses everything until it restarts.
    Unavailable,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorReply {
    pub code: ErrorCode,
    pub denial: Option<Denial>,
    /// For `NotReady`: the deployments serving the model that are trusted for the session.
    pub unready: Vec<DeploymentId>,
}

impl From<ErrorCode> for ErrorReply {
    fn from(code: ErrorCode) -> Self {
        Self {
            code,
            denial: None,
            unready: Vec::new(),
        }
    }
}

impl From<&Denial> for ErrorReply {
    fn from(denial: &Denial) -> Self {
        Self {
            code: ErrorCode::Denied,
            denial: Some(denial.clone()),
            unready: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {}

/// Opens a session for `user` at the label made of `label`'s compartments; the gateway generated
/// the API key and sends only its hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSession {
    pub user: Name,
    pub label: Vec<Name>,
    pub key_hash: KeyHash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCreated {
    pub session: u64,
}

/// Binds one more API key to an existing session, if `user` may open it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenSession {
    pub user: Name,
    pub session: u64,
    pub key_hash: KeyHash,
}

/// Ends a session and every key bound to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseSession {
    pub user: Name,
    pub session: u64,
}

/// Whether `key_hash` is bound to an open session; records nothing. The gateway asks before it
/// reads a request body, so a request with an unknown key is refused with nothing buffered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckKey {
    pub key_hash: KeyHash,
}

/// Arrival-time authorization (`Policy::open_request`); `model: None` is automatic routing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRequest {
    pub key_hash: KeyHash,
    pub model: Option<Name>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub deployment: Name,
    pub model: Name,
    pub node: Name,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestOpened {
    pub request: u64,
    pub session: u64,
    pub label: Label,
    pub candidates: Vec<Candidate>,
}

/// The models the key's session may use now (a deployment trusted for its label, on a Ready node,
/// with a ready engine), without opening a request: a read, so nothing is recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListModels {
    pub key_hash: KeyHash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelList {
    pub session: u64,
    pub label: Label,
    /// Ascending, without duplicates.
    pub models: Vec<Name>,
}

/// Re-authorizes, starts the request on `deployment` and records its grant
/// (`Policy::dispatch`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dispatch {
    pub key_hash: KeyHash,
    pub request: u64,
    pub deployment: Name,
}

/// Where the request was started, under which incarnation of its node and policy epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dispatched {
    pub node: Name,
    pub incarnation: u64,
    pub epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteRequest {
    pub key_hash: KeyHash,
    pub request: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailRequest {
    pub key_hash: KeyHash,
    pub request: u64,
}

/// The operator's provenance view, from `since` (a record `seq`), at most `ProvenancePage::MAX`
/// records per call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadProvenance {
    pub since: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenancePage {
    pub records: Vec<ProvenanceRecord>,
    /// The `since` of the next page; equal to the request's when there is nothing new.
    pub next: u64,
}

impl ProvenancePage {
    pub const MAX: usize = 1000;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadStatus {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub epoch: u64,
    pub nodes: Vec<NodeStatus>,
    pub deployments: Vec<DeploymentStatus>,
    /// Custom resources control did not apply, and why.
    pub rejected: Vec<Rejected>,
    /// Users revoked within `REVOCATION_WINDOW` when they outnumber the users still active, as a
    /// wrong namespace or a mistaken delete would; empty otherwise. They are revoked all the same.
    pub mass_revocation: Vec<Name>,
    pub provenance_records: u64,
    /// Watched kinds whose watch keeps failing, so changes to them since aren't applied.
    #[serde(default)]
    pub stale_watches: Vec<StaleWatch>,
}

/// A watched kind (`User`, `Node`, ...) whose watch has failed since `since`, in seconds since
/// the Unix epoch. If it had `synced` before, control enforces it as the API server had it then;
/// if not, control has applied no configuration since it started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaleWatch {
    pub kind: Name,
    pub since: u64,
    pub synced: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rejected {
    pub kind: Name,
    pub name: Name,
    pub reason: Reason,
}

/// Why control didn't apply an object, as its `Accepted` condition's reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reason {
    InvalidName,
    ModelNotFound,
    RevokedPermanently,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::InvalidName => "InvalidName",
            Reason::ModelNotFound => "ModelNotFound",
            Reason::RevokedPermanently => "RevokedPermanently",
        }
    }

    /// The `Accepted` condition's message.
    pub(crate) fn message(self) -> &'static str {
        match self {
            Reason::InvalidName => "a name or tag is not a valid identifier",
            Reason::ModelNotFound => "spec.model names no Model in this namespace",
            Reason::RevokedPermanently => "this user was revoked, and revocation is permanent",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeStatus {
    pub node: Name,
    pub state: NodeState,
    pub incarnation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentStatus {
    pub deployment: Name,
    pub model: Name,
    pub trusted_for: Label,
    pub node: Name,
    /// Placeable as far as the engine goes (its `EndpointSlice`).
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevokedSession {
    pub epoch: u64,
    pub session: u64,
    /// Lost trust on this deployment only; `None`: lost access everywhere.
    pub deployment: Option<Name>,
}

/// Sessions that lost access or trust above `since` (a previous reply's `epoch`), for cutting
/// their streams and erasing their cached state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadRevocations {
    pub since: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revocations {
    pub epoch: u64,
    pub revoked: Vec<RevokedSession>,
}
