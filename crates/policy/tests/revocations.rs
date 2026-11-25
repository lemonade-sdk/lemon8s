use policy::{
    Compartment, Denial, DeploymentId, Label, ModelName, NodeEvent, NodeId, Policy,
    ProvenanceEvent, SessionId, SessionRevocation, UserId,
};

struct System {
    policy: Policy,
    alice: UserId,
    bob: UserId,
    finance: Label,
    finance_legal: Label,
    engine_a: DeploymentId,
    engine_b: DeploymentId,
}

impl System {
    fn new() -> Self {
        let finance = Label::new([Compartment::new("finance")]);
        let finance_legal = Label::new([Compartment::new("finance"), Compartment::new("legal")]);
        let alice = UserId::new("alice");
        let bob = UserId::new("bob");
        let node_a = NodeId::new("node-a");
        let engine_a = DeploymentId::new("engine-a");
        let engine_b = DeploymentId::new("engine-b");
        let mut policy = Policy::new();
        policy.add_user(alice.clone(), finance_legal.clone());
        policy.add_user(bob.clone(), finance.clone());
        policy.node_event(&node_a, NodeEvent::Admit).unwrap();
        policy
            .node_event(&node_a, NodeEvent::ModelsVerified)
            .unwrap();
        policy
            .node_event(&node_a, NodeEvent::ReadinessPassed)
            .unwrap();
        for engine in [&engine_a, &engine_b] {
            policy.add_deployment(
                engine.clone(),
                ModelName::new("qwen3-30b"),
                finance_legal.clone(),
                node_a.clone(),
            );
        }
        Self {
            policy,
            alice,
            bob,
            finance,
            finance_legal,
            engine_a,
            engine_b,
        }
    }

    fn session(&mut self, user: &UserId, label: &Label) -> SessionId {
        self.policy.create_session(user, label.clone()).unwrap()
    }
}

fn everywhere(epoch: u64, session: SessionId) -> SessionRevocation {
    SessionRevocation {
        epoch,
        session,
        deployment: None,
    }
}

#[test]
fn revoking_a_user_affects_all_of_their_open_sessions_and_nobody_elses() {
    let mut sys = System::new();
    let (alice, bob) = (sys.alice.clone(), sys.bob.clone());
    let a1 = sys.session(&alice, &sys.finance.clone());
    let a2 = sys.session(&alice, &sys.finance_legal.clone());
    let _b1 = sys.session(&bob, &sys.finance.clone());
    let since = sys.policy.epoch();

    sys.policy.revoke_user(&alice).unwrap();

    let epoch = sys.policy.epoch();
    assert_eq!(epoch, since + 1);
    assert_eq!(
        sys.policy.sessions_affected_since(since),
        &[everywhere(epoch, a1), everywhere(epoch, a2)]
    );
    assert_eq!(sys.policy.sessions_affected_since(epoch), []);
}

#[test]
fn narrowing_a_clearance_affects_only_sessions_above_it_and_widening_affects_none() {
    let mut sys = System::new();
    let alice = sys.alice.clone();
    let low = sys.session(&alice, &sys.finance.clone());
    let high = sys.session(&alice, &sys.finance_legal.clone());
    let since = sys.policy.epoch();

    sys.policy
        .set_clearance(&alice, sys.finance.clone())
        .unwrap();
    let narrowed = sys.policy.epoch();
    sys.policy
        .set_clearance(&alice, sys.finance_legal.clone())
        .unwrap();

    assert_eq!(
        sys.policy.sessions_affected_since(since),
        &[everywhere(narrowed, high)]
    );
    assert_eq!(sys.policy.sessions_affected_since(narrowed), []);
    assert_eq!(sys.policy.epoch(), narrowed + 1);
    assert!(sys.policy.open_session(&alice, &low).is_ok());
}

#[test]
fn narrowing_trust_names_the_deployment_and_only_the_sessions_it_no_longer_covers() {
    let mut sys = System::new();
    let (alice, bob) = (sys.alice.clone(), sys.bob.clone());
    let high = sys.session(&alice, &sys.finance_legal.clone());
    let _low = sys.session(&bob, &sys.finance.clone());
    let since = sys.policy.epoch();
    let engine_a = sys.engine_a.clone();

    sys.policy
        .set_deployment_trust(&engine_a, sys.finance.clone())
        .unwrap();

    assert_eq!(
        sys.policy.sessions_affected_since(since),
        &[SessionRevocation {
            epoch: since + 1,
            session: high,
            deployment: Some(engine_a),
        }]
    );
}

#[test]
fn removing_a_deployment_affects_the_sessions_it_could_serve() {
    let mut sys = System::new();
    let alice = sys.alice.clone();
    let session = sys.session(&alice, &sys.finance.clone());
    let since = sys.policy.epoch();
    let engine_b = sys.engine_b.clone();

    sys.policy.remove_deployment(&engine_b);

    assert_eq!(
        sys.policy.sessions_affected_since(since),
        &[SessionRevocation {
            epoch: since + 1,
            session,
            deployment: Some(engine_b),
        }]
    );
}

#[test]
fn closing_a_session_ends_it_bumps_the_epoch_and_reports_it_once() {
    let mut sys = System::new();
    let alice = sys.alice.clone();
    let session = sys.session(&alice, &sys.finance.clone());
    let since = sys.policy.epoch();

    sys.policy.close_session(&alice, &session).unwrap();

    assert_eq!(sys.policy.epoch(), since + 1);
    assert_eq!(
        sys.policy.sessions_affected_since(since),
        &[everywhere(since + 1, session)]
    );
    assert_eq!(
        sys.policy.open_session(&alice, &session),
        Err(Denial::SessionClosed)
    );
    assert_eq!(
        sys.policy.open_request(&alice, &session, None),
        Err(Denial::SessionClosed)
    );
    assert_eq!(
        sys.policy.close_session(&alice, &session),
        Err(Denial::SessionClosed)
    );

    sys.policy.revoke_user(&alice).unwrap();
    assert_eq!(sys.policy.sessions_affected_since(since).len(), 1);
}

#[test]
fn only_the_owner_can_close_a_session() {
    let mut sys = System::new();
    let (alice, bob) = (sys.alice.clone(), sys.bob.clone());
    let session = sys.session(&alice, &sys.finance.clone());
    let epoch = sys.policy.epoch();

    assert_eq!(
        sys.policy.close_session(&bob, &session),
        Err(Denial::NotSessionOwner)
    );
    assert_eq!(sys.policy.epoch(), epoch);
    assert_eq!(sys.policy.sessions_affected_since(0), []);
    let last = sys.policy.provenance_since(0).last().unwrap();
    assert_eq!(last.event, ProvenanceEvent::SessionClosed);
    assert_eq!(last.denial, Some(Denial::NotSessionOwner));
}

#[test]
fn a_closed_session_cannot_complete_its_started_request() {
    let mut sys = System::new();
    let alice = sys.alice.clone();
    let session = sys.session(&alice, &sys.finance.clone());
    let (request, _) = sys.policy.open_request(&alice, &session, None).unwrap();
    sys.policy
        .dispatch(&request, &alice, &sys.engine_a.clone())
        .unwrap();

    sys.policy.close_session(&alice, &session).unwrap();

    assert_eq!(
        sys.policy.complete_request(&request, &alice),
        Err(Denial::SessionClosed)
    );
}

#[test]
fn provenance_since_returns_exactly_the_records_not_yet_stored() {
    let mut sys = System::new();
    let alice = sys.alice.clone();
    let stored = sys.policy.provenance_since(0).len() as u64;

    let session = sys.session(&alice, &sys.finance.clone());
    let _denied = sys
        .policy
        .open_request(&alice, &session, Some(&ModelName::new("none")));

    let fresh = sys.policy.provenance_since(stored);
    assert_eq!(fresh.len(), 2);
    assert_eq!(fresh[0].seq, stored);
    assert_eq!(fresh[0].event, ProvenanceEvent::SessionCreated);
    assert!(matches!(
        fresh[1].event,
        ProvenanceEvent::RequestOpened { request: None, .. }
    ));
    assert_eq!(sys.policy.provenance_since(0).len() as u64, stored + 2);
    assert_eq!(sys.policy.provenance_since(stored + 2), []);
    assert_eq!(sys.policy.provenance_since(u64::MAX), []);
}

#[test]
fn advancing_the_epoch_revokes_no_session() {
    let mut sys = System::new();
    let alice = sys.alice.clone();
    let session = sys.session(&alice, &sys.finance.clone());
    let (request, _) = sys.policy.open_request(&alice, &session, None).unwrap();
    let grant = sys
        .policy
        .dispatch(&request, &alice, &sys.engine_a.clone())
        .unwrap();

    sys.policy.advance_epoch();

    assert_eq!(sys.policy.sessions_affected_since(0), []);
    assert!(grant.epoch < sys.policy.epoch());
    assert!(sys.policy.open_session(&alice, &session).is_ok());
}
