use crate::ids::{DeploymentId, SessionId, UserId};
use crate::label::Label;

#[derive(Debug, Clone)]
pub struct Session {
    pub id: SessionId,
    pub owner: UserId,
    pub label: Label,
    pub closed: bool,
}

/// A session that lost access, or lost trust on `deployment` (`None`: everywhere), at `epoch`.
/// Enforcement points erase its cached state and cut its in-flight streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRevocation {
    pub epoch: u64,
    pub session: SessionId,
    pub deployment: Option<DeploymentId>,
}
