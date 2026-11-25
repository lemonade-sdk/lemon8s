use crate::{
    CheckKey, CloseSession, CompleteRequest, CreateSession, Dispatch, Dispatched, Done, ErrorReply,
    FailRequest, ListModels, ModelList, OpenRequest, OpenSession, ProvenancePage, ReadProvenance,
    ReadRevocations, ReadStatus, RequestOpened, Revocations, SessionCreated, Status,
};

use crate::state::State;

/// A control operation: one api call type, its reply, and the `State` method that answers it.
pub trait Operation: Send + 'static {
    type Reply: Send + 'static;

    fn run(state: &mut State, call: Self) -> Result<Self::Reply, ErrorReply>;
}

impl Operation for CreateSession {
    type Reply = SessionCreated;

    fn run(state: &mut State, call: Self) -> Result<Self::Reply, ErrorReply> {
        state.create_session(&call)
    }
}

impl Operation for OpenSession {
    type Reply = Done;

    fn run(state: &mut State, call: Self) -> Result<Self::Reply, ErrorReply> {
        state.open_session(&call)
    }
}

impl Operation for CloseSession {
    type Reply = Done;

    fn run(state: &mut State, call: Self) -> Result<Self::Reply, ErrorReply> {
        state.close_session(&call)
    }
}

impl Operation for CheckKey {
    type Reply = Done;

    fn run(state: &mut State, call: Self) -> Result<Self::Reply, ErrorReply> {
        state.check_key(&call)
    }
}

impl Operation for OpenRequest {
    type Reply = RequestOpened;

    fn run(state: &mut State, call: Self) -> Result<Self::Reply, ErrorReply> {
        state.open_request(&call)
    }
}

impl Operation for Dispatch {
    type Reply = Dispatched;

    fn run(state: &mut State, call: Self) -> Result<Self::Reply, ErrorReply> {
        state.dispatch(&call)
    }
}

impl Operation for CompleteRequest {
    type Reply = Done;

    fn run(state: &mut State, call: Self) -> Result<Self::Reply, ErrorReply> {
        state.complete_request(&call)
    }
}

impl Operation for FailRequest {
    type Reply = Done;

    fn run(state: &mut State, call: Self) -> Result<Self::Reply, ErrorReply> {
        state.fail_request(&call)
    }
}

impl Operation for ReadRevocations {
    type Reply = Revocations;

    fn run(state: &mut State, call: Self) -> Result<Self::Reply, ErrorReply> {
        state.read_revocations(&call)
    }
}

impl Operation for ReadProvenance {
    type Reply = ProvenancePage;

    fn run(state: &mut State, call: Self) -> Result<Self::Reply, ErrorReply> {
        Ok(state.read_provenance(&call))
    }
}

impl Operation for ReadStatus {
    type Reply = Status;

    fn run(state: &mut State, call: Self) -> Result<Self::Reply, ErrorReply> {
        state.read_status(&call)
    }
}

impl Operation for ListModels {
    type Reply = ModelList;

    fn run(state: &mut State, call: Self) -> Result<Self::Reply, ErrorReply> {
        state.list_models(&call)
    }
}
