use serde::{Deserialize, Serialize};

use crate::denial::Denial;
use crate::deployment::ModelName;
use crate::ids::{DeploymentId, GrantId, NodeId, RequestId, SessionId, UserId};
use crate::label::Label;
use crate::node::NodeEvent;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProvenanceEvent {
    SessionCreated,
    SessionOpened,
    SessionClosed,
    /// Arrival-time authorization: the request exists, unbound, with these eligible
    /// deployments (`model: None` is automatic routing).
    RequestOpened {
        request: Option<RequestId>,
        model: Option<ModelName>,
        candidates: Vec<DeploymentId>,
    },
    RequestStarted {
        request: RequestId,
    },
    RequestCompleted {
        request: RequestId,
    },
    RequestFailed {
        request: RequestId,
    },
    /// The dispatch decision for a request on `deployment`: its grant when the request started,
    /// none when it was refused.
    Granted {
        grant: Option<GrantId>,
        deployment: DeploymentId,
    },
    UserAdded,
    ClearanceChanged,
    UserRevoked,
    DeploymentAdded {
        deployment: DeploymentId,
        model: ModelName,
        node: NodeId,
    },
    DeploymentTrustChanged {
        deployment: DeploymentId,
    },
    DeploymentReadinessChanged {
        deployment: DeploymentId,
        ready: bool,
    },
    DeploymentRemoved {
        deployment: DeploymentId,
    },
    NodeEvent {
        node: NodeId,
        event: NodeEvent,
    },
}

/// `label` is the label of the data the decision concerns (the session's label for session and
/// request events), so a record about legal data is itself legal. Carries IDs, labels and denials
/// only, matching `Denial`: never prompt text, model output or document content. Control stores it
/// as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceRecord {
    pub seq: u64,
    pub epoch: u64,
    pub actor: Option<UserId>,
    pub session: Option<SessionId>,
    pub label: Label,
    pub event: ProvenanceEvent,
    /// `None` when the decision allowed the action.
    pub denial: Option<Denial>,
}
