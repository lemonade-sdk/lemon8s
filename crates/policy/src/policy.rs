use std::collections::BTreeMap;

use crate::denial::Denial;
use crate::deployment::{Deployment, ModelName};
use crate::grant::Grant;
use crate::ids::{DeploymentId, GrantId, NodeId, RequestId, SessionId, UserId};
use crate::label::Label;
use crate::node::{Node, NodeEvent, NodeState};
use crate::provenance::{ProvenanceEvent, ProvenanceRecord};
use crate::request::{Request, RequestState};
use crate::session::{Session, SessionRevocation};
use crate::user::{User, UserStatus};

/// Every check reads current state, so revocation applies on the next check.
#[derive(Debug, Default)]
pub struct Policy {
    users: BTreeMap<UserId, User>,
    sessions: BTreeMap<SessionId, Session>,
    deployments: BTreeMap<DeploymentId, Deployment>,
    nodes: BTreeMap<NodeId, Node>,
    requests: BTreeMap<RequestId, Request>,
    provenance: Vec<ProvenanceRecord>,
    revocations: Vec<SessionRevocation>,
    next_session_id: u64,
    next_request_id: u64,
    next_grant_id: u64,
    next_provenance_seq: u64,
    epoch: u64,
}

impl Policy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_user(&mut self, id: UserId, clearance: Label) {
        self.users.insert(
            id.clone(),
            User {
                id: id.clone(),
                clearance: clearance.clone(),
                status: UserStatus::Active,
            },
        );
        self.record(
            Some(id),
            None,
            clearance,
            ProvenanceEvent::UserAdded,
            &Ok(()),
        );
    }

    pub fn set_clearance(&mut self, user: &UserId, clearance: Label) -> Result<(), Denial> {
        let label = clearance.clone();
        let result = self.set_clearance_checked(user, clearance);
        self.record(
            Some(user.clone()),
            None,
            label,
            ProvenanceEvent::ClearanceChanged,
            &result,
        );
        result
    }

    /// Bumps the epoch unconditionally (even a widening change), since telling the two apart
    /// isn't worth the complexity and a bump that revokes nothing costs nothing.
    fn set_clearance_checked(&mut self, user: &UserId, clearance: Label) -> Result<(), Denial> {
        let u = self.users.get_mut(user).ok_or(Denial::UnknownUser)?;
        u.clearance = clearance;
        self.epoch += 1;
        let clearance = u.clearance.clone();
        self.revoke_sessions(|s| s.owner == *user && !s.label.is_subset(&clearance), None);
        Ok(())
    }

    pub fn revoke_user(&mut self, user: &UserId) -> Result<(), Denial> {
        let result = self.revoke_user_checked(user);
        self.record(
            Some(user.clone()),
            None,
            Label::public(),
            ProvenanceEvent::UserRevoked,
            &result,
        );
        result
    }

    fn revoke_user_checked(&mut self, user: &UserId) -> Result<(), Denial> {
        let u = self.users.get_mut(user).ok_or(Denial::UnknownUser)?;
        u.status = UserStatus::Revoked;
        self.epoch += 1;
        self.revoke_sessions(|s| s.owner == *user, None);
        Ok(())
    }

    pub fn add_deployment(
        &mut self,
        id: DeploymentId,
        model: ModelName,
        trusted_for: Label,
        node: NodeId,
    ) {
        self.deployments.insert(
            id.clone(),
            Deployment {
                id: id.clone(),
                model: model.clone(),
                trusted_for: trusted_for.clone(),
                node: node.clone(),
                ready: true,
            },
        );
        self.record(
            None,
            None,
            trusted_for,
            ProvenanceEvent::DeploymentAdded {
                deployment: id,
                model,
                node,
            },
            &Ok(()),
        );
    }

    pub fn set_deployment_trust(
        &mut self,
        deployment: &DeploymentId,
        trusted_for: Label,
    ) -> Result<(), Denial> {
        let result = self.set_deployment_trust_checked(deployment, &trusted_for);
        self.record(
            None,
            None,
            trusted_for,
            ProvenanceEvent::DeploymentTrustChanged {
                deployment: deployment.clone(),
            },
            &result,
        );
        result
    }

    fn set_deployment_trust_checked(
        &mut self,
        deployment: &DeploymentId,
        trusted_for: &Label,
    ) -> Result<(), Denial> {
        let d = self
            .deployments
            .get_mut(deployment)
            .ok_or(Denial::UnknownDeployment)?;
        let previous = std::mem::replace(&mut d.trusted_for, trusted_for.clone());
        self.epoch += 1;
        self.revoke_sessions(
            |s| s.label.is_subset(&previous) && !s.label.is_subset(trusted_for),
            Some(deployment),
        );
        Ok(())
    }

    /// Readiness is capacity, not authorization: it records a decision but bumps no epoch, and a
    /// started stream may still complete on an engine that has since gone not ready.
    pub fn set_deployment_ready(
        &mut self,
        deployment: &DeploymentId,
        ready: bool,
    ) -> Result<(), Denial> {
        let result = self
            .deployments
            .get_mut(deployment)
            .map(|d| d.ready = ready)
            .ok_or(Denial::UnknownDeployment);
        self.record(
            None,
            None,
            Label::public(),
            ProvenanceEvent::DeploymentReadinessChanged {
                deployment: deployment.clone(),
                ready,
            },
            &result,
        );
        result
    }

    pub fn remove_deployment(&mut self, deployment: &DeploymentId) {
        let label = self
            .deployments
            .get(deployment)
            .map_or_else(Label::public, |d| d.trusted_for.clone());
        let removed = self.deployments.remove(deployment);
        self.epoch += 1;
        if removed.is_some() {
            self.revoke_sessions(|s| s.label.is_subset(&label), Some(deployment));
        }
        self.record(
            None,
            None,
            label,
            ProvenanceEvent::DeploymentRemoved {
                deployment: deployment.clone(),
            },
            &Ok(()),
        );
    }

    pub fn node_event(&mut self, node: &NodeId, event: NodeEvent) -> Result<(), Denial> {
        let result = self.node_event_checked(node, event);
        self.record(
            None,
            None,
            Label::public(),
            ProvenanceEvent::NodeEvent {
                node: node.clone(),
                event,
            },
            &result,
        );
        result
    }

    /// `Admit` registers an unknown node; every other event needs one already registered.
    /// `Rejoin` bumps the incarnation, so a request started under the old one can't complete.
    fn node_event_checked(&mut self, node: &NodeId, event: NodeEvent) -> Result<(), Denial> {
        if event == NodeEvent::Admit {
            if let Some(existing) = self.nodes.get(node) {
                return Err(Denial::InvalidTransition {
                    from: existing.state,
                    event,
                });
            }
            self.nodes.insert(
                node.clone(),
                Node {
                    id: node.clone(),
                    incarnation: 1,
                    state: NodeState::Admitted,
                },
            );
            return Ok(());
        }
        let n = self.nodes.get_mut(node).ok_or(Denial::UnknownNode)?;
        let next = match (n.state, event) {
            (NodeState::Admitted, NodeEvent::ModelsVerified) => NodeState::Loading,
            (NodeState::Loading, NodeEvent::ReadinessPassed)
            | (NodeState::Suspect, NodeEvent::Recovered) => NodeState::Ready,
            (NodeState::Ready, NodeEvent::Drain) => NodeState::Draining,
            (NodeState::Ready, NodeEvent::ReadinessLost) => NodeState::Suspect,
            // Admitted, Loading and Draining carry no new placements: a silent node skips Suspect.
            (
                NodeState::Suspect | NodeState::Admitted | NodeState::Loading | NodeState::Draining,
                NodeEvent::Timeout,
            ) => NodeState::Lost,
            (NodeState::Draining, NodeEvent::Remove) => NodeState::Removed,
            (NodeState::Removed | NodeState::Lost, NodeEvent::Rejoin) => NodeState::Admitted,
            (from, event) => return Err(Denial::InvalidTransition { from, event }),
        };
        if event == NodeEvent::Rejoin {
            n.incarnation += 1;
        }
        n.state = next;
        if matches!(
            event,
            NodeEvent::Drain
                | NodeEvent::ReadinessLost
                | NodeEvent::Timeout
                | NodeEvent::Remove
                | NodeEvent::Rejoin
        ) {
            self.epoch += 1;
        }
        Ok(())
    }

    pub fn node(&self, node: &NodeId) -> Result<&Node, Denial> {
        self.nodes.get(node).ok_or(Denial::UnknownNode)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    pub fn users(&self) -> impl Iterator<Item = &User> {
        self.users.values()
    }

    pub fn request(&self, request: &RequestId) -> Result<&Request, Denial> {
        self.requests.get(request).ok_or(Denial::UnknownRequest)
    }

    pub fn user(&self, user: &UserId) -> Result<&User, Denial> {
        self.users.get(user).ok_or(Denial::UnknownUser)
    }

    pub fn session(&self, session: &SessionId) -> Result<&Session, Denial> {
        self.sessions.get(session).ok_or(Denial::UnknownSession)
    }

    pub fn deployment(&self, deployment: &DeploymentId) -> Result<&Deployment, Denial> {
        self.deployments
            .get(deployment)
            .ok_or(Denial::UnknownDeployment)
    }

    pub fn deployments(&self) -> impl Iterator<Item = &Deployment> {
        self.deployments.values()
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Advances the epoch without changing any access decision. Control calls it each time it
    /// starts, so every process start begins a new epoch.
    pub fn advance_epoch(&mut self) {
        self.epoch += 1;
    }

    pub fn create_session(&mut self, user: &UserId, label: Label) -> Result<SessionId, Denial> {
        let requested_label = label.clone();
        let result = self.create_session_checked(user, label);
        self.record(
            Some(user.clone()),
            result.as_ref().ok().copied(),
            requested_label,
            ProvenanceEvent::SessionCreated,
            &result,
        );
        result
    }

    fn create_session_checked(&mut self, user: &UserId, label: Label) -> Result<SessionId, Denial> {
        let u = self.active_user(user)?;
        if !label.is_subset(&u.clearance) {
            return Err(Denial::LabelExceedsClearance {
                requested: label,
                clearance: u.clearance.clone(),
            });
        }
        let id = SessionId(self.next_session_id);
        self.next_session_id += 1;
        self.sessions.insert(
            id,
            Session {
                id,
                owner: user.clone(),
                label,
                closed: false,
            },
        );
        Ok(id)
    }

    pub fn open_session(&mut self, user: &UserId, session: &SessionId) -> Result<(), Denial> {
        let label = self
            .sessions
            .get(session)
            .map_or_else(Label::public, |s| s.label.clone());
        let result = self.session_access(user, *session);
        self.record(
            Some(user.clone()),
            Some(*session),
            label,
            ProvenanceEvent::SessionOpened,
            &result,
        );
        result
    }

    /// The non-logging check reused by every other method that needs "can this user currently
    /// open this session" as a precondition, so a nested use doesn't also emit its own
    /// `SessionOpened` record.
    fn session_access(&self, user: &UserId, session: SessionId) -> Result<(), Denial> {
        let u = self.active_user(user)?;
        let s = self.sessions.get(&session).ok_or(Denial::UnknownSession)?;
        if s.owner != *user {
            return Err(Denial::NotSessionOwner);
        }
        if s.closed {
            return Err(Denial::SessionClosed);
        }
        if !s.label.is_subset(&u.clearance) {
            return Err(Denial::LabelExceedsClearance {
                requested: s.label.clone(),
                clearance: u.clearance.clone(),
            });
        }
        Ok(())
    }

    /// Ends the session for good: every later check on it fails with `SessionClosed`, and it is
    /// listed in the revocations at the new epoch, so its streams are cut.
    pub fn close_session(&mut self, user: &UserId, session: &SessionId) -> Result<(), Denial> {
        let label = self
            .sessions
            .get(session)
            .map_or_else(Label::public, |s| s.label.clone());
        let result = self.close_session_checked(user, *session);
        self.record(
            Some(user.clone()),
            Some(*session),
            label,
            ProvenanceEvent::SessionClosed,
            &result,
        );
        result
    }

    fn close_session_checked(&mut self, user: &UserId, session: SessionId) -> Result<(), Denial> {
        self.session_access(user, session)?;
        let s = self
            .sessions
            .get_mut(&session)
            .ok_or(Denial::UnknownSession)?;
        s.closed = true;
        self.epoch += 1;
        self.revocations.push(SessionRevocation {
            epoch: self.epoch,
            session,
            deployment: None,
        });
        Ok(())
    }

    pub fn authorize_placement(
        &self,
        user: &UserId,
        session: &SessionId,
        deployment: &DeploymentId,
    ) -> Result<(), Denial> {
        self.session_access(user, *session)?;
        let s = self.sessions.get(session).ok_or(Denial::UnknownSession)?;
        let d = self
            .deployments
            .get(deployment)
            .ok_or(Denial::UnknownDeployment)?;
        if !s.label.is_subset(&d.trusted_for) {
            return Err(Denial::DeploymentNotTrusted {
                session: s.label.clone(),
                trusted_for: d.trusted_for.clone(),
            });
        }
        let node = self.nodes.get(&d.node).ok_or(Denial::UnknownNode)?;
        if node.state != NodeState::Ready {
            return Err(Denial::NodeNotReady {
                node: d.node.clone(),
                state: node.state,
            });
        }
        if !d.ready {
            return Err(Denial::DeploymentNotReady {
                deployment: deployment.clone(),
            });
        }
        Ok(())
    }

    /// An empty result under automatic routing (`model: None`) is a capacity condition, not a
    /// denial; only an explicit, unmet model request is denied outright (no silent substitution).
    pub fn eligible_deployments(
        &self,
        user: &UserId,
        session: &SessionId,
        model: Option<&ModelName>,
    ) -> Result<Vec<DeploymentId>, Denial> {
        self.session_access(user, *session)?;
        let s = self.sessions.get(session).ok_or(Denial::UnknownSession)?;
        let eligible: Vec<DeploymentId> = self
            .deployments
            .values()
            .filter(|d| s.label.is_subset(&d.trusted_for) && d.ready)
            .filter(|d| {
                self.nodes
                    .get(&d.node)
                    .is_some_and(|n| n.state == NodeState::Ready)
            })
            .filter(|d| model.is_none_or(|m| &d.model == m))
            .map(|d| d.id.clone())
            .collect();
        if let Some(m) = model
            && eligible.is_empty()
        {
            return Err(Denial::NoTrustedDeployment { model: m.clone() });
        }
        Ok(eligible)
    }

    /// The deployments serving an explicit `model` that are trusted for the session's label, when
    /// the model lacks only readiness: the session may be used, but none of them is ready on a
    /// Ready node. Readiness is capacity, not authorization, so this is no denial; it records
    /// nothing. Empty when one is ready, none is trusted, or the session may not be used.
    pub fn unready_deployments(
        &self,
        user: &UserId,
        session: &SessionId,
        model: &ModelName,
    ) -> Vec<DeploymentId> {
        let Some(s) = self.sessions.get(session) else {
            return Vec::new();
        };
        if !matches!(
            self.eligible_deployments(user, session, Some(model)),
            Err(Denial::NoTrustedDeployment { .. })
        ) {
            return Vec::new();
        }
        self.deployments
            .values()
            .filter(|d| &d.model == model && s.label.is_subset(&d.trusted_for))
            .map(|d| d.id.clone())
            .collect()
    }

    pub fn complete_request(&mut self, request: &RequestId, user: &UserId) -> Result<(), Denial> {
        let session = self.requests.get(request).map(|r| r.session);
        let label = session
            .and_then(|s| self.sessions.get(&s))
            .map_or_else(Label::public, |s| s.label.clone());
        let result = self.complete_request_checked(*request, user);
        self.record(
            Some(user.clone()),
            session,
            label,
            ProvenanceEvent::RequestCompleted { request: *request },
            &result,
        );
        result
    }

    /// A started stream must stop once the user loses access or the deployment loses trust, so
    /// this re-runs the open-session check and the trust check (but not node readiness: Draining
    /// and Suspect still let an in-flight stream finish). A cut stream (node gone, or rejoined
    /// under a new incarnation) must never silently become `Done`.
    fn complete_request_checked(
        &mut self,
        request: RequestId,
        user: &UserId,
    ) -> Result<(), Denial> {
        let (session, deployment, incarnation) = {
            let req = self.requests.get(&request).ok_or(Denial::UnknownRequest)?;
            match &req.state {
                RequestState::Started {
                    deployment,
                    incarnation,
                } => (req.session, deployment.clone(), *incarnation),
                _ => return Err(Denial::RequestNotStarted),
            }
        };
        self.session_access(user, session)?;
        let d = self
            .deployments
            .get(&deployment)
            .ok_or(Denial::UnknownDeployment)?;
        let s = self.sessions.get(&session).ok_or(Denial::UnknownSession)?;
        if !s.label.is_subset(&d.trusted_for) {
            return Err(Denial::DeploymentNotTrusted {
                session: s.label.clone(),
                trusted_for: d.trusted_for.clone(),
            });
        }
        let node = self.nodes.get(&d.node).ok_or(Denial::UnknownNode)?;
        let gone = matches!(node.state, NodeState::Lost | NodeState::Removed);
        if gone || node.incarnation != incarnation {
            return Err(Denial::RequestCut);
        }
        self.requests
            .get_mut(&request)
            .ok_or(Denial::UnknownRequest)?
            .state = RequestState::Done;
        Ok(())
    }

    pub fn fail_request(&mut self, request: &RequestId) -> Result<(), Denial> {
        let session = self.requests.get(request).map(|r| r.session);
        let label = session
            .and_then(|s| self.sessions.get(&s))
            .map_or_else(Label::public, |s| s.label.clone());
        let result = self.fail_request_checked(*request);
        self.record(
            None,
            session,
            label,
            ProvenanceEvent::RequestFailed { request: *request },
            &result,
        );
        result
    }

    fn fail_request_checked(&mut self, request: RequestId) -> Result<(), Denial> {
        let req = self
            .requests
            .get_mut(&request)
            .ok_or(Denial::UnknownRequest)?;
        match req.state {
            RequestState::Done | RequestState::FailedExplicit => {
                return Err(Denial::RequestAlreadyTerminal);
            }
            _ => {}
        }
        req.state = RequestState::FailedExplicit;
        Ok(())
    }

    /// Arrival-time authorization, before any prompt leaves the gateway: the request starts
    /// unbound (`Unstarted`) and the result lists the deployments it may
    /// run on. An explicit model with none eligible is denied (no substitution); automatic
    /// routing with none eligible is a capacity condition and returns an empty list.
    pub fn open_request(
        &mut self,
        user: &UserId,
        session: &SessionId,
        model: Option<&ModelName>,
    ) -> Result<(RequestId, Vec<DeploymentId>), Denial> {
        let label = self
            .sessions
            .get(session)
            .map_or_else(Label::public, |s| s.label.clone());
        let result = self.open_request_checked(user, *session, model);
        let (request, candidates) = match &result {
            Ok((request, candidates)) => (Some(*request), candidates.clone()),
            Err(_) => (None, Vec::new()),
        };
        self.record(
            Some(user.clone()),
            Some(*session),
            label,
            ProvenanceEvent::RequestOpened {
                request,
                model: model.cloned(),
                candidates,
            },
            &result,
        );
        result
    }

    fn open_request_checked(
        &mut self,
        user: &UserId,
        session: SessionId,
        model: Option<&ModelName>,
    ) -> Result<(RequestId, Vec<DeploymentId>), Denial> {
        let candidates = self.eligible_deployments(user, &session, model)?;
        let id = RequestId(self.next_request_id);
        self.next_request_id += 1;
        self.requests.insert(
            id,
            Request {
                id,
                session,
                state: RequestState::Unstarted,
            },
        );
        Ok((id, candidates))
    }

    /// Binds an unstarted request to `deployment`, starts it and records its grant under one
    /// `&mut self`, so no epoch change can fall between the authorization and the grant, and
    /// every started request has exactly one grant. On a denial nothing changes.
    pub fn dispatch(
        &mut self,
        request: &RequestId,
        user: &UserId,
        deployment: &DeploymentId,
    ) -> Result<Grant, Denial> {
        let session = self.requests.get(request).map(|r| r.session);
        let label = session
            .and_then(|s| self.sessions.get(&s))
            .map_or_else(Label::public, |s| s.label.clone());
        let result = self.dispatch_checked(*request, user, deployment);
        self.record(
            Some(user.clone()),
            session,
            label.clone(),
            ProvenanceEvent::Granted {
                grant: result.as_ref().ok().map(|g| g.id),
                deployment: deployment.clone(),
            },
            &result,
        );
        if result.is_ok() {
            self.record(
                Some(user.clone()),
                session,
                label,
                ProvenanceEvent::RequestStarted { request: *request },
                &result,
            );
        }
        result
    }

    fn dispatch_checked(
        &mut self,
        request: RequestId,
        user: &UserId,
        deployment: &DeploymentId,
    ) -> Result<Grant, Denial> {
        let req = self.requests.get(&request).ok_or(Denial::UnknownRequest)?;
        let session = req.session;
        if req.state != RequestState::Unstarted {
            return Err(Denial::RequestNotUnstarted);
        }
        self.authorize_placement(user, &session, deployment)?;
        let d = self
            .deployments
            .get(deployment)
            .ok_or(Denial::UnknownDeployment)?;
        let node = self.nodes.get(&d.node).ok_or(Denial::UnknownNode)?;
        let grant = Grant {
            id: GrantId(self.next_grant_id),
            node: d.node.clone(),
            incarnation: node.incarnation,
            epoch: self.epoch,
        };
        self.next_grant_id += 1;
        self.requests
            .get_mut(&request)
            .ok_or(Denial::UnknownRequest)?
            .state = RequestState::Started {
            deployment: deployment.clone(),
            incarnation: grant.incarnation,
        };
        Ok(grant)
    }

    /// Every record with `seq >= seq`, in order: what a caller that has stored everything
    /// below `seq` still has to store.
    pub fn provenance_since(&self, seq: u64) -> &[ProvenanceRecord] {
        let start = self.provenance.partition_point(|r| r.seq < seq);
        self.provenance.get(start..).unwrap_or_default()
    }

    /// Sessions that lost access (closed, owner revoked, clearance narrowed below the session
    /// label) or lost trust on a deployment, at an epoch above `epoch`. Each entry is decided
    /// when the change happens, so a later widening does not withdraw it.
    pub fn sessions_affected_since(&self, epoch: u64) -> &[SessionRevocation] {
        let start = self.revocations.partition_point(|r| r.epoch <= epoch);
        self.revocations.get(start..).unwrap_or_default()
    }

    fn revoke_sessions(
        &mut self,
        affected: impl Fn(&Session) -> bool,
        deployment: Option<&DeploymentId>,
    ) {
        let epoch = self.epoch;
        self.revocations.extend(
            self.sessions
                .values()
                .filter(|s| !s.closed && affected(s))
                .map(|s| SessionRevocation {
                    epoch,
                    session: s.id,
                    deployment: deployment.cloned(),
                }),
        );
    }

    fn active_user(&self, user: &UserId) -> Result<&User, Denial> {
        let u = self.users.get(user).ok_or(Denial::UnknownUser)?;
        if u.status != UserStatus::Active {
            return Err(Denial::UserRevoked);
        }
        Ok(u)
    }

    fn record<T>(
        &mut self,
        actor: Option<UserId>,
        session: Option<SessionId>,
        label: Label,
        event: ProvenanceEvent,
        result: &Result<T, Denial>,
    ) {
        let denial = result.as_ref().err().cloned();
        let seq = self.next_provenance_seq;
        self.next_provenance_seq += 1;
        self.provenance.push(ProvenanceRecord {
            seq,
            epoch: self.epoch,
            actor,
            session,
            label,
            event,
            denial,
        });
    }
}
