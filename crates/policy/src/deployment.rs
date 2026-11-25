use serde::{Deserialize, Serialize};

use crate::ids::{DeploymentId, NodeId};
use crate::label::Label;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModelName(String);

impl ModelName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct Deployment {
    pub id: DeploymentId,
    pub model: ModelName,
    pub trusted_for: Label,
    pub node: NodeId,
    /// Whether its engine can take work (capacity, not trust: no epoch change either way).
    pub ready: bool,
}
