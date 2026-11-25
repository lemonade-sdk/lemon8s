use crate::KeyHash;
use policy::{DeploymentId, Label, ModelName, NodeEvent, NodeId, RequestId, SessionId, UserId};
use serde::{Deserialize, Serialize};

/// One state change, as appended to the log and replayed at startup. Each is one `Policy` call
/// (denied calls included, since they record provenance), or adding a user already revoked, with
/// the inputs control resolved before making it, so replay needs nothing but the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    /// A control process started; advances the epoch.
    Started,
    /// `revoked`: the `User` was revoked when control first saw it, so it never had access and
    /// its revocation doesn't count toward a mass revocation.
    AddUser {
        user: UserId,
        clearance: Label,
        #[serde(default)]
        revoked: bool,
    },
    SetClearance {
        user: UserId,
        clearance: Label,
    },
    RevokeUser {
        user: UserId,
    },
    AddDeployment {
        deployment: DeploymentId,
        model: ModelName,
        trusted_for: Label,
        node: NodeId,
    },
    SetDeploymentTrust {
        deployment: DeploymentId,
        trusted_for: Label,
    },
    RemoveDeployment {
        deployment: DeploymentId,
    },
    SetDeploymentReady {
        deployment: DeploymentId,
        ready: bool,
    },
    NodeTransition {
        node: NodeId,
        event: NodeEvent,
    },
    CreateSession {
        user: UserId,
        label: Label,
        key_hash: KeyHash,
    },
    OpenSession {
        user: UserId,
        session: SessionId,
        key_hash: KeyHash,
    },
    CloseSession {
        user: UserId,
        session: SessionId,
    },
    OpenRequest {
        user: UserId,
        session: SessionId,
        model: Option<ModelName>,
    },
    Dispatch {
        user: UserId,
        request: RequestId,
        deployment: DeploymentId,
    },
    CompleteRequest {
        user: UserId,
        request: RequestId,
    },
    FailRequest {
        request: RequestId,
    },
}
