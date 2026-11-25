use serde::{Deserialize, Serialize};

use crate::deployment::ModelName;
use crate::ids::{DeploymentId, NodeId};
use crate::label::Label;
use crate::node::{NodeEvent, NodeState};

/// Carries labels/IDs, never content, so a decision stays explainable without exposing restricted
/// data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Denial {
    UnknownUser,
    UnknownSession,
    UnknownDeployment,
    UnknownNode,
    UnknownRequest,
    UserRevoked,
    LabelExceedsClearance { requested: Label, clearance: Label },
    NotSessionOwner,
    SessionClosed,
    DeploymentNotTrusted { session: Label, trusted_for: Label },
    NoTrustedDeployment { model: ModelName },
    NodeNotReady { node: NodeId, state: NodeState },
    DeploymentNotReady { deployment: DeploymentId },
    InvalidTransition { from: NodeState, event: NodeEvent },
    RequestNotUnstarted,
    RequestNotStarted,
    RequestAlreadyTerminal,
    RequestCut,
}
