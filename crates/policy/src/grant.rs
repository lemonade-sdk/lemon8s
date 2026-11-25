use crate::ids::{GrantId, NodeId};

/// What `Policy::dispatch` granted: the request was started on a node, under that node's
/// incarnation and the policy epoch of the moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub id: GrantId,
    pub node: NodeId,
    pub incarnation: u64,
    pub epoch: u64,
}
