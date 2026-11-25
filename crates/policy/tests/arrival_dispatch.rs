use policy::{
    Compartment, Denial, DeploymentId, Label, ModelName, NodeEvent, NodeId, Policy,
    ProvenanceEvent, RequestState, SessionId, UserId,
};

struct System {
    policy: Policy,
    alice: UserId,
    bob: UserId,
    finance: Label,
    legal: Label,
    qwen: ModelName,
    node_a: NodeId,
    node_a_finance: DeploymentId,
    node_a_finance_legal: DeploymentId,
    node_a_small: DeploymentId,
}

impl System {
    fn new() -> Self {
        let finance = Label::new([Compartment::new("finance")]);
        let legal = Label::new([Compartment::new("legal")]);
        let finance_legal = Label::new([Compartment::new("finance"), Compartment::new("legal")]);
        let alice = UserId::new("alice");
        let bob = UserId::new("bob");
        let qwen = ModelName::new("qwen3-30b");
        let small = ModelName::new("qwen3-0.6b");
        let node_a = NodeId::new("node-a");
        let node_a_finance = DeploymentId::new("node-a-finance");
        let node_a_finance_legal = DeploymentId::new("node-a-finance-legal");
        let node_a_small = DeploymentId::new("node-a-small");

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
        policy.add_deployment(
            node_a_finance.clone(),
            qwen.clone(),
            finance.clone(),
            node_a.clone(),
        );
        policy.add_deployment(
            node_a_finance_legal.clone(),
            qwen.clone(),
            finance_legal,
            node_a.clone(),
        );
        policy.add_deployment(node_a_small.clone(), small, Label::public(), node_a.clone());

        Self {
            policy,
            alice,
            bob,
            finance,
            legal,
            qwen,
            node_a,
            node_a_finance,
            node_a_finance_legal,
            node_a_small,
        }
    }

    fn session(&mut self, user: &UserId, label: &Label) -> SessionId {
        self.policy.create_session(user, label.clone()).unwrap()
    }
}

#[test]
fn open_request_lists_the_eligible_deployments_of_the_model_and_leaves_the_request_unbound() {
    let mut sys = System::new();
    let alice = sys.alice.clone();
    let session = sys.session(&alice, &sys.finance.clone());

    let (request, candidates) = sys
        .policy
        .open_request(&alice, &session, Some(&sys.qwen))
        .unwrap();

    assert_eq!(
        candidates,
        vec![sys.node_a_finance.clone(), sys.node_a_finance_legal.clone()]
    );
    let req = sys.policy.request(&request).unwrap();
    assert_eq!(req.session, session);
    assert_eq!(req.state, RequestState::Unstarted);
    let record = sys.policy.provenance_since(0).last().unwrap();
    assert_eq!(
        record.event,
        ProvenanceEvent::RequestOpened {
            request: Some(request),
            model: Some(sys.qwen.clone()),
            candidates,
        }
    );
    assert_eq!(record.label, sys.finance);
    assert_eq!(record.denial, None);
}

#[test]
fn open_request_with_automatic_routing_lists_every_model() {
    let mut sys = System::new();
    let alice = sys.alice.clone();
    let session = sys.session(&alice, &sys.finance.clone());

    let (_, candidates) = sys.policy.open_request(&alice, &session, None).unwrap();

    assert_eq!(
        candidates,
        vec![sys.node_a_finance.clone(), sys.node_a_finance_legal.clone()]
    );
}

#[test]
fn an_explicit_model_with_no_trusted_deployment_is_denied_without_substitution() {
    let mut sys = System::new();
    let alice = sys.alice.clone();
    let session = sys.session(&alice, &sys.legal.clone());
    let small = ModelName::new("qwen3-0.6b");

    let denied = sys.policy.open_request(&alice, &session, Some(&small));

    assert_eq!(
        denied,
        Err(Denial::NoTrustedDeployment {
            model: small.clone()
        })
    );
    let record = sys.policy.provenance_since(0).last().unwrap();
    assert_eq!(
        record.event,
        ProvenanceEvent::RequestOpened {
            request: None,
            model: Some(small),
            candidates: Vec::new(),
        }
    );
}

#[test]
fn automatic_routing_with_nothing_eligible_is_capacity_not_a_denial() {
    let mut sys = System::new();
    let alice = sys.alice.clone();
    let session = sys.session(&alice, &sys.finance.clone());
    sys.policy
        .set_deployment_trust(&sys.node_a_finance.clone(), Label::public())
        .unwrap();
    sys.policy
        .set_deployment_trust(&sys.node_a_finance_legal.clone(), Label::public())
        .unwrap();

    let (request, candidates) = sys.policy.open_request(&alice, &session, None).unwrap();

    assert_eq!(candidates, []);
    assert_eq!(
        sys.policy.request(&request).unwrap().state,
        RequestState::Unstarted
    );
}

#[test]
fn open_request_refuses_a_revoked_user_and_another_users_session() {
    let mut sys = System::new();
    let (alice, bob) = (sys.alice.clone(), sys.bob.clone());
    let session = sys.session(&alice, &sys.finance.clone());

    assert_eq!(
        sys.policy.open_request(&bob, &session, None),
        Err(Denial::NotSessionOwner)
    );
    sys.policy.revoke_user(&alice).unwrap();
    assert_eq!(
        sys.policy.open_request(&alice, &session, None),
        Err(Denial::UserRevoked)
    );
}

#[test]
fn dispatch_binds_starts_and_grants_the_request_in_one_step() {
    let mut sys = System::new();
    let alice = sys.alice.clone();
    let session = sys.session(&alice, &sys.finance.clone());
    let (request, _) = sys
        .policy
        .open_request(&alice, &session, Some(&sys.qwen))
        .unwrap();
    let before = sys.policy.provenance_since(0).len();

    let grant = sys
        .policy
        .dispatch(&request, &alice, &sys.node_a_finance_legal)
        .unwrap();

    assert_eq!(grant.node, sys.node_a);
    assert_eq!(grant.incarnation, 1);
    assert_eq!(grant.epoch, sys.policy.epoch());
    assert_eq!(sys.policy.request(&request).unwrap().session, session);
    assert_eq!(
        sys.policy.request(&request).unwrap().state,
        RequestState::Started {
            deployment: sys.node_a_finance_legal.clone(),
            incarnation: 1,
        }
    );
    let events: Vec<ProvenanceEvent> = sys.policy.provenance_since(0)[before..]
        .iter()
        .map(|r| r.event.clone())
        .collect();
    assert_eq!(
        events,
        vec![
            ProvenanceEvent::Granted {
                grant: Some(grant.id),
                deployment: sys.node_a_finance_legal.clone(),
            },
            ProvenanceEvent::RequestStarted { request },
        ]
    );
}

#[test]
fn a_request_is_dispatched_at_most_once() {
    let mut sys = System::new();
    let alice = sys.alice.clone();
    let session = sys.session(&alice, &sys.finance.clone());
    let (request, _) = sys.policy.open_request(&alice, &session, None).unwrap();
    sys.policy
        .dispatch(&request, &alice, &sys.node_a_finance)
        .unwrap();

    assert_eq!(
        sys.policy
            .dispatch(&request, &alice, &sys.node_a_finance_legal),
        Err(Denial::RequestNotUnstarted)
    );
}

#[test]
fn dispatch_reauthorizes_and_a_denial_leaves_the_request_unstarted() {
    let mut sys = System::new();
    let alice = sys.alice.clone();
    let session = sys.session(
        &alice,
        &Label::new([Compartment::new("finance"), Compartment::new("legal")]),
    );
    let (request, candidates) = sys.policy.open_request(&alice, &session, None).unwrap();
    assert_eq!(candidates, vec![sys.node_a_finance_legal.clone()]);
    let before = sys.policy.provenance_since(0).len();

    sys.policy
        .set_clearance(&alice, sys.finance.clone())
        .unwrap();
    let denied = sys
        .policy
        .dispatch(&request, &alice, &sys.node_a_finance_legal);

    assert!(matches!(denied, Err(Denial::LabelExceedsClearance { .. })));
    assert_eq!(
        sys.policy.request(&request).unwrap().state,
        RequestState::Unstarted
    );
    let last = sys.policy.provenance_since(0).last().unwrap();
    assert_eq!(
        last.event,
        ProvenanceEvent::Granted {
            grant: None,
            deployment: sys.node_a_finance_legal.clone(),
        }
    );
    assert_eq!(last.denial, denied.err());
    assert_eq!(sys.policy.provenance_since(0).len(), before + 2);
}

#[test]
fn dispatch_refuses_an_untrusted_deployment_and_a_draining_node() {
    let mut sys = System::new();
    let alice = sys.alice.clone();
    let session = sys.session(&alice, &sys.finance.clone());
    let (request, _) = sys.policy.open_request(&alice, &session, None).unwrap();

    assert!(matches!(
        sys.policy.dispatch(&request, &alice, &sys.node_a_small),
        Err(Denial::DeploymentNotTrusted { .. })
    ));
    sys.policy
        .node_event(&sys.node_a.clone(), NodeEvent::Drain)
        .unwrap();
    assert!(matches!(
        sys.policy.dispatch(&request, &alice, &sys.node_a_finance),
        Err(Denial::NodeNotReady { .. })
    ));
}

#[test]
fn only_the_session_owner_can_dispatch_its_request() {
    let mut sys = System::new();
    let (alice, bob) = (sys.alice.clone(), sys.bob.clone());
    let session = sys.session(&alice, &sys.finance.clone());
    let (request, _) = sys.policy.open_request(&alice, &session, None).unwrap();

    assert_eq!(
        sys.policy.dispatch(&request, &bob, &sys.node_a_finance),
        Err(Denial::NotSessionOwner)
    );
}
