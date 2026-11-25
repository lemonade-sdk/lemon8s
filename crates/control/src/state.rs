use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    Candidate, CheckKey, CloseSession, CompleteRequest, CreateSession, DeploymentStatus, Dispatch,
    Dispatched, Done, ErrorCode, ErrorReply, FailRequest, KeyHash, ListModels, ModelList, Name,
    NodeStatus, OpenRequest, OpenSession, ProvenancePage, ReadProvenance, ReadRevocations,
    ReadStatus, Reason, Rejected, RequestOpened, Revocations, RevokedSession, SessionCreated,
    StaleWatch, Status,
};
use kube::ResourceExt;
use policy::{
    Compartment, Denial, DeploymentId, Grant, Label, ModelName, NodeId, Policy, RequestId,
    SessionId, UserId, UserStatus,
};

use crate::crd;
use crate::desired::{Desired, Engine, NodeTarget, REVOCATION_WINDOW, Reconciled, Rejections};
use crate::event::Event;
use crate::store::{Store, StoreError, millis};
use crate::watch::Watches;

#[derive(Debug)]
pub enum ControlError {
    Store(StoreError),
    /// Replaying the log produced different provenance than was recorded at `seq`.
    ReplayDiverged {
        seq: u64,
    },
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ControlError::Store(e) => write!(f, "store: {e}"),
            ControlError::ReplayDiverged { seq } => {
                write!(
                    f,
                    "replaying the event log diverges at provenance seq {seq}"
                )
            }
        }
    }
}

impl std::error::Error for ControlError {}

/// What one event did, in the shape of the `Policy` call it made.
enum Applied {
    Done(Result<(), Denial>),
    Session(Result<SessionId, Denial>),
    Opened(Result<(RequestId, Vec<DeploymentId>), Denial>),
    Grant(Result<Grant, Denial>),
}

/// Everything behind control's lock: the policy, the session-key hashes that authenticate into
/// it and the durable log.
pub struct State {
    policy: Policy,
    store: Store,
    stored_provenance: u64,
    poisoned: bool,
    keys: BTreeMap<KeyHash, (UserId, SessionId)>,
    rejected: Vec<Rejected>,
    /// When each revoked user was revoked (`millis`), except those added already revoked.
    revoked_at: BTreeMap<UserId, i64>,
    /// `Status::mass_revocation` as of the last reconcile.
    mass_revocation: Vec<Name>,
    /// How many mass revocations this process has seen begin.
    mass_revocations: u64,
    /// Shared with `Control` and written by the watch, outside this lock.
    pub(crate) watches: Watches,
}

impl State {
    /// Replays the log into a fresh `Policy`, checks the replay reproduces the stored provenance
    /// exactly, and only then records this start (an epoch bump).
    pub fn open(store: Store) -> Result<Self, ControlError> {
        let mut state = Self {
            policy: Policy::new(),
            store,
            stored_provenance: 0,
            poisoned: false,
            keys: BTreeMap::new(),
            rejected: Vec::new(),
            revoked_at: BTreeMap::new(),
            mass_revocation: Vec::new(),
            mass_revocations: 0,
            watches: Watches::default(),
        };
        state.replay()?;
        state
            .persist(&Event::Started, SystemTime::now())
            .map_err(ControlError::Store)?;
        Ok(state)
    }

    fn replay(&mut self) -> Result<(), ControlError> {
        for (recorded_at_ms, event) in self.store.events().map_err(ControlError::Store)? {
            self.apply(&event, recorded_at_ms);
        }
        let stored = self.store.provenance().map_err(ControlError::Store)?;
        let replayed = self.policy.provenance_since(0);
        if let Some(seq) =
            (0..replayed.len().max(stored.len())).find(|&i| replayed.get(i) != stored.get(i))
        {
            return Err(ControlError::ReplayDiverged {
                seq: u64::try_from(seq).unwrap_or(u64::MAX),
            });
        }
        self.stored_provenance = u64::try_from(stored.len()).unwrap_or(u64::MAX);
        Ok(())
    }

    pub fn epoch(&self) -> u64 {
        self.policy.epoch()
    }

    pub fn mass_revocations(&self) -> u64 {
        self.mass_revocations
    }

    /// A write failed or a call panicked: memory may be ahead of disk, so every call is refused
    /// until a restart.
    pub fn poisoned(&self) -> bool {
        self.poisoned
    }

    pub fn poison(&mut self) {
        self.poisoned = true;
    }

    fn apply(&mut self, event: &Event, recorded_at_ms: i64) -> Applied {
        let policy = &mut self.policy;
        match event {
            Event::Started => {
                policy.advance_epoch();
                Applied::Done(Ok(()))
            }
            Event::AddUser {
                user,
                clearance,
                revoked,
            } => {
                policy.add_user(user.clone(), clearance.clone());
                Applied::Done(if *revoked {
                    policy.revoke_user(user)
                } else {
                    Ok(())
                })
            }
            Event::SetClearance { user, clearance } => {
                Applied::Done(policy.set_clearance(user, clearance.clone()))
            }
            Event::RevokeUser { user } => {
                let revoked = policy.revoke_user(user);
                if revoked.is_ok() {
                    self.keys.retain(|_, (holder, _)| holder != user);
                    self.revoked_at.insert(user.clone(), recorded_at_ms);
                }
                Applied::Done(revoked)
            }
            Event::AddDeployment {
                deployment,
                model,
                trusted_for,
                node,
            } => {
                policy.add_deployment(
                    deployment.clone(),
                    model.clone(),
                    trusted_for.clone(),
                    node.clone(),
                );
                Applied::Done(Ok(()))
            }
            Event::SetDeploymentTrust {
                deployment,
                trusted_for,
            } => Applied::Done(policy.set_deployment_trust(deployment, trusted_for.clone())),
            Event::RemoveDeployment { deployment } => {
                policy.remove_deployment(deployment);
                Applied::Done(Ok(()))
            }
            Event::SetDeploymentReady { deployment, ready } => {
                Applied::Done(policy.set_deployment_ready(deployment, *ready))
            }
            Event::NodeTransition { node, event } => Applied::Done(policy.node_event(node, *event)),
            Event::CreateSession {
                user,
                label,
                key_hash,
            } => {
                let created = policy.create_session(user, label.clone());
                if let Ok(session) = created {
                    self.keys.insert(*key_hash, (user.clone(), session));
                }
                Applied::Session(created)
            }
            Event::OpenSession {
                user,
                session,
                key_hash,
            } => {
                let opened = policy.open_session(user, session);
                if opened.is_ok() {
                    self.keys.insert(*key_hash, (user.clone(), *session));
                }
                Applied::Done(opened)
            }
            Event::CloseSession { user, session } => {
                let closed = policy.close_session(user, session);
                if closed.is_ok() {
                    self.keys.retain(|_, (_, s)| s != session);
                }
                Applied::Done(closed)
            }
            Event::OpenRequest {
                user,
                session,
                model,
            } => Applied::Opened(policy.open_request(user, session, model.as_ref())),
            Event::Dispatch {
                user,
                request,
                deployment,
            } => Applied::Grant(policy.dispatch(request, user, deployment)),
            Event::CompleteRequest { user, request } => {
                Applied::Done(policy.complete_request(request, user))
            }
            Event::FailRequest { request } => Applied::Done(policy.fail_request(request)),
        }
    }

    /// Applies `event` and makes it and its provenance durable, recorded as happening `at`,
    /// before anyone hears the outcome.
    fn persist(&mut self, event: &Event, at: SystemTime) -> Result<Applied, StoreError> {
        let recorded_at_ms = millis(at);
        let applied = self.apply(event, recorded_at_ms);
        let records = self
            .policy
            .provenance_since(self.stored_provenance)
            .to_vec();
        self.store.append(event, &records, recorded_at_ms)?;
        self.stored_provenance += u64::try_from(records.len()).unwrap_or(u64::MAX);
        Ok(applied)
    }

    /// If the write fails, memory is ahead of disk, so control refuses everything until a restart
    /// replays the log.
    fn commit(&mut self, event: &Event, at: SystemTime) -> Result<Applied, ErrorReply> {
        if self.poisoned {
            return Err(ErrorCode::Unavailable.into());
        }
        self.persist(event, at).map_err(|e| {
            eprintln!("control: persisting an event failed, refusing all calls: {e}");
            self.poisoned = true;
            ErrorReply::from(ErrorCode::Unavailable)
        })
    }

    fn done(&mut self, event: &Event) -> Result<Done, ErrorReply> {
        match self.commit(event, SystemTime::now())? {
            Applied::Done(Ok(())) => Ok(Done {}),
            Applied::Done(Err(denial)) => Err((&denial).into()),
            _ => Err(ErrorCode::Internal.into()),
        }
    }

    /// The key's session, and `request` must belong to it: a gateway can act only on requests
    /// of sessions whose keys pass through it.
    fn request_of(
        &self,
        key_hash: &KeyHash,
        request: u64,
    ) -> Result<(UserId, RequestId), ErrorReply> {
        let (user, session) = self
            .keys
            .get(key_hash)
            .cloned()
            .ok_or(ErrorCode::InvalidApiKey)?;
        let request = RequestId::from_raw(request);
        match self.policy.request(&request) {
            Ok(r) if r.session == session => Ok((user, request)),
            _ => Err((&Denial::UnknownRequest).into()),
        }
    }

    pub fn create_session(&mut self, call: &CreateSession) -> Result<SessionCreated, ErrorReply> {
        if self.keys.contains_key(&call.key_hash) {
            return Err(ErrorCode::Conflict.into());
        }
        match self.commit(
            &Event::CreateSession {
                user: UserId::new(call.user.as_str()),
                label: Label::new(call.label.iter().map(|c| Compartment::new(c.as_str()))),
                key_hash: call.key_hash,
            },
            SystemTime::now(),
        )? {
            Applied::Session(Ok(session)) => Ok(SessionCreated {
                session: session.get(),
            }),
            Applied::Session(Err(denial)) => Err((&denial).into()),
            _ => Err(ErrorCode::Internal.into()),
        }
    }

    pub fn open_session(&mut self, call: &OpenSession) -> Result<Done, ErrorReply> {
        if self.keys.contains_key(&call.key_hash) {
            return Err(ErrorCode::Conflict.into());
        }
        self.done(&Event::OpenSession {
            user: UserId::new(call.user.as_str()),
            session: SessionId::from_raw(call.session),
            key_hash: call.key_hash,
        })
    }

    pub fn close_session(&mut self, call: &CloseSession) -> Result<Done, ErrorReply> {
        self.done(&Event::CloseSession {
            user: UserId::new(call.user.as_str()),
            session: SessionId::from_raw(call.session),
        })
    }

    pub fn check_key(&mut self, call: &CheckKey) -> Result<Done, ErrorReply> {
        if self.keys.contains_key(&call.key_hash) {
            Ok(Done {})
        } else {
            Err(ErrorCode::InvalidApiKey.into())
        }
    }

    pub fn open_request(&mut self, call: &OpenRequest) -> Result<RequestOpened, ErrorReply> {
        let (user, session) = self
            .keys
            .get(&call.key_hash)
            .cloned()
            .ok_or(ErrorCode::InvalidApiKey)?;
        let model = call.model.as_ref().map(|m| ModelName::new(m.as_str()));
        // Readiness, not a policy decision: refused before logging, so logs replay as recorded.
        let unready = model
            .as_ref()
            .map_or_default(|m| self.policy.unready_deployments(&user, &session, m));
        if !unready.is_empty() {
            return Err(ErrorReply {
                code: ErrorCode::NotReady,
                denial: None,
                unready,
            });
        }
        let opened = self.commit(
            &Event::OpenRequest {
                user,
                session,
                model,
            },
            SystemTime::now(),
        )?;
        let (request, deployments) = match opened {
            Applied::Opened(Ok(opened)) => opened,
            Applied::Opened(Err(denial)) => return Err((&denial).into()),
            _ => return Err(ErrorCode::Internal.into()),
        };
        let label = self
            .policy
            .session(&session)
            .map(|s| s.label.clone())
            .map_err(|d| ErrorReply::from(&d))?;
        let candidates = deployments
            .iter()
            .filter_map(|id| {
                let d = self.policy.deployment(id).ok()?;
                Some(Candidate {
                    deployment: Name::new(id.as_str()).ok()?,
                    model: Name::new(d.model.as_str()).ok()?,
                    node: Name::new(d.node.as_str()).ok()?,
                })
            })
            .collect();
        Ok(RequestOpened {
            request: request.get(),
            session: session.get(),
            label,
            candidates,
        })
    }

    pub fn list_models(&mut self, call: &ListModels) -> Result<ModelList, ErrorReply> {
        let (user, session) = self
            .keys
            .get(&call.key_hash)
            .cloned()
            .ok_or(ErrorCode::InvalidApiKey)?;
        let deployments = self
            .policy
            .eligible_deployments(&user, &session, None)
            .map_err(|d| ErrorReply::from(&d))?;
        let models = deployments
            .iter()
            .filter_map(|id| self.policy.deployment(id).ok())
            .map(|d| Name::new(d.model.as_str()))
            .collect::<Result<BTreeSet<Name>, _>>()
            .map_err(|_| ErrorCode::Internal)?;
        let label = self
            .policy
            .session(&session)
            .map(|s| s.label.clone())
            .map_err(|d| ErrorReply::from(&d))?;
        Ok(ModelList {
            session: session.get(),
            label,
            models: models.into_iter().collect(),
        })
    }

    /// Re-authorizes the request, binds it to the deployment and starts it, recording the grant.
    pub fn dispatch(&mut self, call: &Dispatch) -> Result<Dispatched, ErrorReply> {
        let (user, request) = self.request_of(&call.key_hash, call.request)?;
        let grant = match self.commit(
            &Event::Dispatch {
                user,
                request,
                deployment: DeploymentId::new(call.deployment.as_str()),
            },
            SystemTime::now(),
        )? {
            Applied::Grant(Ok(grant)) => grant,
            Applied::Grant(Err(denial)) => return Err((&denial).into()),
            _ => return Err(ErrorCode::Internal.into()),
        };
        Ok(Dispatched {
            node: Name::new(grant.node.as_str()).map_err(|_| ErrorCode::Internal)?,
            incarnation: grant.incarnation,
            epoch: grant.epoch,
        })
    }

    pub fn complete_request(&mut self, call: &CompleteRequest) -> Result<Done, ErrorReply> {
        let (user, request) = self.request_of(&call.key_hash, call.request)?;
        self.done(&Event::CompleteRequest { user, request })
    }

    pub fn fail_request(&mut self, call: &FailRequest) -> Result<Done, ErrorReply> {
        let (_, request) = self.request_of(&call.key_hash, call.request)?;
        self.done(&Event::FailRequest { request })
    }

    /// Brings the policy to the operator's configuration as observed in Kubernetes `now`. Every
    /// change is one logged event, like a user call, recorded as happening `now`, so replay
    /// reproduces it; a narrowing change bumps the epoch through the policy as before. Objects that
    /// can't be applied are listed in status.
    pub fn reconcile(
        &mut self,
        desired: &Desired,
        now: SystemTime,
    ) -> Result<Reconciled, ErrorReply> {
        if self.poisoned {
            return Err(ErrorCode::Unavailable.into());
        }
        let mut rejected = Vec::new();
        self.reconcile_nodes(desired, now)?;
        let engines = desired.cluster_engines(&mut rejected);
        let engine_ready = self.reconcile_deployments(engines, now)?;
        self.reconcile_users(desired, now, &mut rejected)?;

        self.rejected = rejected
            .into_iter()
            .filter_map(|(kind, name, reason)| {
                Some(Rejected {
                    kind: Name::new(kind).ok()?,
                    name: Name::new(name).ok()?,
                    reason,
                })
            })
            .collect();
        Ok(Reconciled {
            rejected: self.rejected.clone(),
            engine_ready,
        })
    }

    /// Follows only the Nodes some `ModelDeployment` names, so the log doesn't record every other
    /// Node's changes; a node no longer named reads as gone. Node names are DNS subdomains, so
    /// they are always valid `Name`s and need no check.
    fn reconcile_nodes(&mut self, desired: &Desired, now: SystemTime) -> Result<(), ErrorReply> {
        let named: BTreeSet<&str> = desired
            .deployments
            .iter()
            .map(|d| d.spec.node.as_str())
            .collect();
        let mut targets: BTreeMap<String, NodeTarget> = desired
            .nodes
            .iter()
            .filter(|n| named.contains(n.name_any().as_str()))
            .map(|n| (n.name_any(), NodeTarget::of(n)))
            .collect();
        for node in self.policy.nodes() {
            targets
                .entry(node.id.as_str().to_owned())
                .or_insert(NodeTarget::Gone);
        }
        for (name, target) in targets {
            let node = NodeId::new(name);
            let current = self.policy.node(&node).ok().map(|n| n.state);
            for event in target.transitions(current) {
                self.commit(
                    &Event::NodeTransition {
                        node: node.clone(),
                        event,
                    },
                    now,
                )?;
            }
        }
        Ok(())
    }

    /// Makes the policy's deployments exactly `engines`, and returns each one's readiness, by name.
    fn reconcile_deployments(
        &mut self,
        engines: BTreeMap<DeploymentId, Engine>,
        now: SystemTime,
    ) -> Result<BTreeMap<String, bool>, ErrorReply> {
        let stale: Vec<DeploymentId> = self
            .policy
            .deployments()
            .filter(|d| match engines.get(&d.id) {
                None => true,
                Some(e) => d.model != e.served || d.node != e.node,
            })
            .map(|d| d.id.clone())
            .collect();
        for deployment in stale {
            self.commit(&Event::RemoveDeployment { deployment }, now)?;
        }
        let engine_ready: BTreeMap<String, bool> = engines
            .iter()
            .map(|(id, e)| (id.as_str().to_owned(), e.ready))
            .collect();
        for (deployment, engine) in engines {
            let held = self
                .policy
                .deployment(&deployment)
                .map(|d| (d.trusted_for == engine.trust, d.ready));
            let ready = if let Ok((same_trust, ready)) = held {
                if !same_trust {
                    self.commit(
                        &Event::SetDeploymentTrust {
                            deployment: deployment.clone(),
                            trusted_for: engine.trust,
                        },
                        now,
                    )?;
                }
                ready
            } else {
                self.commit(
                    &Event::AddDeployment {
                        deployment: deployment.clone(),
                        model: engine.served,
                        trusted_for: engine.trust,
                        node: engine.node,
                    },
                    now,
                )?;
                true
            };
            if ready != engine.ready {
                self.commit(
                    &Event::SetDeploymentReady {
                        deployment,
                        ready: engine.ready,
                    },
                    now,
                )?;
            }
        }
        Ok(engine_ready)
    }

    fn reconcile_users(
        &mut self,
        desired: &Desired,
        now: SystemTime,
        rejected: &mut Rejections,
    ) -> Result<(), ErrorReply> {
        let mut configured = BTreeSet::new();
        let mut changes = Vec::new();
        let mut revocations = Vec::new();
        let mut users: Vec<&crd::User> = desired.users.iter().collect();
        users.sort_by_key(|u| u.name_any());
        for user in users {
            let name = user.name_any();
            let clearance = user
                .spec
                .clearance
                .iter()
                .map(Name::new)
                .collect::<Result<Vec<_>, _>>();
            let (Ok(id), Ok(clearance)) = (Name::new(name.as_str()), clearance) else {
                rejected.push(("User", name, Reason::InvalidName));
                continue;
            };
            let id = UserId::new(id.as_str());
            let clearance = Label::new(clearance.iter().map(|c| Compartment::new(c.as_str())));
            configured.insert(id.clone());
            let held = self
                .policy
                .user(&id)
                .map(|u| (u.status, u.clearance.clone()));
            match held {
                Err(_) => changes.push(Event::AddUser {
                    user: id,
                    clearance,
                    revoked: user.spec.revoked,
                }),
                Ok((UserStatus::Revoked, _)) => {
                    if !user.spec.revoked {
                        rejected.push(("User", name, Reason::RevokedPermanently));
                    }
                }
                Ok((UserStatus::Active, _)) if user.spec.revoked => {
                    revocations.push(Event::RevokeUser { user: id });
                }
                Ok((UserStatus::Active, held)) => {
                    if held != clearance {
                        changes.push(Event::SetClearance {
                            user: id,
                            clearance,
                        });
                    }
                }
            }
        }
        revocations.extend(
            self.policy
                .users()
                .filter(|u| u.status == UserStatus::Active && !configured.contains(&u.id))
                .map(|u| Event::RevokeUser { user: u.id.clone() }),
        );
        for change in changes.iter().chain(&revocations) {
            self.commit(change, now)?;
        }
        self.watch_for_mass_revocation(now)
    }

    /// Revocation is permanent and applies at once, so revoking more users within
    /// `REVOCATION_WINDOW` than remain active (a wrong namespace, a mistaken delete) raises an
    /// alert: a log line when it begins, `Status::mass_revocation`, and a count for the gateway's
    /// metrics.
    fn watch_for_mass_revocation(&mut self, now: SystemTime) -> Result<(), ErrorReply> {
        let since = millis(now.checked_sub(REVOCATION_WINDOW).unwrap_or(UNIX_EPOCH));
        let recent: Vec<Name> = self
            .revoked_at
            .iter()
            .filter(|&(_, &at)| at > since)
            .map(|(user, _)| Name::new(user.as_str()))
            .collect::<Result<_, _>>()
            .map_err(|_| ErrorCode::Internal)?;
        let active = self
            .policy
            .users()
            .filter(|u| u.status == UserStatus::Active)
            .count();
        let mass = if recent.len() > active {
            recent
        } else {
            Vec::new()
        };
        if self.mass_revocation.is_empty() && !mass.is_empty() {
            self.mass_revocations += 1;
            eprintln!(
                "control: revoked {} users within {} minutes, more than the {active} still \
                 active, as a wrong namespace or a mistaken delete would; status lists them",
                mass.len(),
                REVOCATION_WINDOW.as_secs() / 60
            );
        }
        self.mass_revocation = mass;
        Ok(())
    }

    pub fn read_provenance(&mut self, call: &ReadProvenance) -> ProvenancePage {
        let records: Vec<_> = self
            .policy
            .provenance_since(call.since)
            .iter()
            .take(ProvenancePage::MAX)
            .cloned()
            .collect();
        let next = records.last().map_or(call.since, |r| r.seq + 1);
        ProvenancePage { records, next }
    }

    pub fn read_status(&mut self, _call: &ReadStatus) -> Result<Status, ErrorReply> {
        let nodes = self
            .policy
            .nodes()
            .map(|n| {
                Ok(NodeStatus {
                    node: Name::new(n.id.as_str())?,
                    state: n.state,
                    incarnation: n.incarnation,
                })
            })
            .collect::<Result<Vec<_>, crate::WireError>>()
            .map_err(|_| ErrorCode::Internal)?;
        let deployments = self
            .policy
            .deployments()
            .map(|d| {
                Ok(DeploymentStatus {
                    deployment: Name::new(d.id.as_str())?,
                    model: Name::new(d.model.as_str())?,
                    trusted_for: d.trusted_for.clone(),
                    node: Name::new(d.node.as_str())?,
                    ready: d.ready,
                })
            })
            .collect::<Result<Vec<_>, crate::WireError>>()
            .map_err(|_| ErrorCode::Internal)?;
        let stale_watches = self
            .watches
            .lock()
            .iter()
            .filter_map(|(kind, failing)| Some((kind, (*failing)?)))
            .map(|(kind, failing)| {
                Ok(StaleWatch {
                    kind: Name::new(kind.as_str())?,
                    since: failing.since,
                    synced: failing.synced,
                })
            })
            .collect::<Result<Vec<_>, crate::WireError>>()
            .map_err(|_| ErrorCode::Internal)?;
        Ok(Status {
            epoch: self.policy.epoch(),
            nodes,
            deployments,
            rejected: self.rejected.clone(),
            mass_revocation: self.mass_revocation.clone(),
            provenance_records: self.stored_provenance,
            stale_watches,
        })
    }

    pub fn read_revocations(&mut self, call: &ReadRevocations) -> Result<Revocations, ErrorReply> {
        let revoked = self
            .policy
            .sessions_affected_since(call.since)
            .iter()
            .map(|r| {
                Ok(RevokedSession {
                    epoch: r.epoch,
                    session: r.session.get(),
                    deployment: r
                        .deployment
                        .as_ref()
                        .map(|d| Name::new(d.as_str()))
                        .transpose()?,
                })
            })
            .collect::<Result<Vec<_>, crate::WireError>>()
            .map_err(|_| ErrorCode::Internal)?;
        Ok(Revocations {
            epoch: self.policy.epoch(),
            revoked,
        })
    }
}
