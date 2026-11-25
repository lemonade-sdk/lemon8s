use crate::ids::{DeploymentId, RequestId, SessionId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestState {
    Unstarted,
    Started {
        deployment: DeploymentId,
        incarnation: u64,
    },
    Done,
    FailedExplicit,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub id: RequestId,
    pub session: SessionId,
    pub state: RequestState,
}
