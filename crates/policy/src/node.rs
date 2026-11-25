use serde::{Deserialize, Serialize};

use crate::ids::NodeId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    Admitted,
    Loading,
    Ready,
    Draining,
    Suspect,
    Lost,
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeEvent {
    Admit,
    ModelsVerified,
    ReadinessPassed,
    Drain,
    /// The Node's Ready condition went false or unknown.
    ReadinessLost,
    Recovered,
    Timeout,
    Remove,
    Rejoin,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    pub incarnation: u64,
    pub state: NodeState,
}
