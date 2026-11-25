use policy::{
    Compartment, Denial, DeploymentId, Label, ModelName, NodeEvent, NodeId, NodeState, Policy,
    RequestState, UserId,
};

struct System {
    policy: Policy,
    alice: UserId,
    bob: UserId,
    finance: Label,
    legal: Label,
    qwen: ModelName,
    node_a: NodeId,
    node_b: NodeId,
    node_a_finance_legal: DeploymentId,
    node_b_finance: DeploymentId,
    node_c_finance_legal: DeploymentId,
}

impl System {
    fn new() -> Self {
        let finance = Label::new([Compartment::new("finance")]);
        let legal = Label::new([Compartment::new("legal")]);
        let finance_legal = Label::new([Compartment::new("finance"), Compartment::new("legal")]);
        let alice = UserId::new("alice");
        let bob = UserId::new("bob");
        let qwen = ModelName::new("qwen3-30b");
        let node_a = NodeId::new("node-a");
        let node_b = NodeId::new("node-b");
        let node_c = NodeId::new("node-c");
        let deployment_a = DeploymentId::new("node-a-finance-legal");
        let deployment_b = DeploymentId::new("node-b-finance");
        let deployment_c = DeploymentId::new("node-c-finance-legal");

        let mut policy = Policy::new();
        policy.add_user(alice.clone(), finance_legal.clone());
        policy.add_user(bob.clone(), finance.clone());

        for node in [&node_a, &node_b, &node_c] {
            policy.node_event(node, NodeEvent::Admit).unwrap();
            policy.node_event(node, NodeEvent::ModelsVerified).unwrap();
            policy.node_event(node, NodeEvent::ReadinessPassed).unwrap();
        }

        policy.add_deployment(
            deployment_a.clone(),
            qwen.clone(),
            finance_legal.clone(),
            node_a.clone(),
        );
        policy.add_deployment(
            deployment_b.clone(),
            qwen.clone(),
            finance.clone(),
            node_b.clone(),
        );
        policy.add_deployment(
            deployment_c.clone(),
            qwen.clone(),
            finance_legal,
            node_c.clone(),
        );

        Self {
            policy,
            alice,
            bob,
            finance,
            legal,
            qwen,
            node_a,
            node_b,
            node_a_finance_legal: deployment_a,
            node_b_finance: deployment_b,
            node_c_finance_legal: deployment_c,
        }
    }
}

#[test]
fn every_valid_node_transition_works_and_invalid_ones_are_rejected() {
    let mut sys = System::new();
    let node = NodeId::new("spare");

    assert!(sys.policy.node_event(&node, NodeEvent::Admit).is_ok());
    assert_eq!(
        sys.policy.node_event(&node, NodeEvent::Drain),
        Err(Denial::InvalidTransition {
            from: NodeState::Admitted,
            event: NodeEvent::Drain,
        })
    );

    assert!(
        sys.policy
            .node_event(&node, NodeEvent::ModelsVerified)
            .is_ok()
    );
    assert!(
        sys.policy
            .node_event(&node, NodeEvent::ReadinessPassed)
            .is_ok()
    );
    assert_eq!(
        sys.policy.node_event(&node, NodeEvent::ModelsVerified),
        Err(Denial::InvalidTransition {
            from: NodeState::Ready,
            event: NodeEvent::ModelsVerified,
        })
    );

    assert!(
        sys.policy
            .node_event(&node, NodeEvent::ReadinessLost)
            .is_ok()
    );
    assert!(sys.policy.node_event(&node, NodeEvent::Recovered).is_ok());
    assert!(sys.policy.node_event(&node, NodeEvent::Drain).is_ok());
    assert!(sys.policy.node_event(&node, NodeEvent::Remove).is_ok());

    // Only Rejoin applies to an already-registered node; Admit is not a re-admission path.
    assert_eq!(
        sys.policy.node_event(&node, NodeEvent::Admit),
        Err(Denial::InvalidTransition {
            from: NodeState::Removed,
            event: NodeEvent::Admit,
        })
    );

    assert!(sys.policy.node_event(&node, NodeEvent::Rejoin).is_ok());
    assert_eq!(sys.policy.node(&node).unwrap().state, NodeState::Admitted);
    assert_eq!(sys.policy.node(&node).unwrap().incarnation, 2);
}

#[test]
fn rejoin_bumps_incarnation_and_requires_re_verification_before_ready() {
    let mut sys = System::new();
    assert_eq!(sys.policy.node(&sys.node_a).unwrap().incarnation, 1);

    sys.policy
        .node_event(&sys.node_a, NodeEvent::ReadinessLost)
        .unwrap();
    sys.policy
        .node_event(&sys.node_a, NodeEvent::Timeout)
        .unwrap();
    assert_eq!(sys.policy.node(&sys.node_a).unwrap().state, NodeState::Lost);

    sys.policy
        .node_event(&sys.node_a, NodeEvent::Rejoin)
        .unwrap();
    assert_eq!(
        sys.policy.node(&sys.node_a).unwrap().state,
        NodeState::Admitted
    );
    assert_eq!(sys.policy.node(&sys.node_a).unwrap().incarnation, 2);

    let session = sys
        .policy
        .create_session(&sys.alice, sys.finance.clone())
        .unwrap();
    assert_eq!(
        sys.policy
            .authorize_placement(&sys.alice, &session, &sys.node_a_finance_legal),
        Err(Denial::NodeNotReady {
            node: sys.node_a.clone(),
            state: NodeState::Admitted,
        })
    );

    // Can't skip straight to Ready: artifacts must be re-verified after a rejoin.
    assert_eq!(
        sys.policy
            .node_event(&sys.node_a, NodeEvent::ReadinessPassed),
        Err(Denial::InvalidTransition {
            from: NodeState::Admitted,
            event: NodeEvent::ReadinessPassed,
        })
    );

    sys.policy
        .node_event(&sys.node_a, NodeEvent::ModelsVerified)
        .unwrap();
    sys.policy
        .node_event(&sys.node_a, NodeEvent::ReadinessPassed)
        .unwrap();
    assert!(
        sys.policy
            .authorize_placement(&sys.alice, &session, &sys.node_a_finance_legal)
            .is_ok()
    );
}

#[test]
fn draining_node_blocks_new_placements_but_lets_started_requests_complete() {
    let mut sys = System::new();
    let session = sys
        .policy
        .create_session(&sys.alice, sys.finance.clone())
        .unwrap();
    let (request, _) = sys.policy.open_request(&sys.alice, &session, None).unwrap();
    sys.policy
        .dispatch(&request, &sys.alice, &sys.node_a_finance_legal)
        .unwrap();

    sys.policy
        .node_event(&sys.node_a, NodeEvent::Drain)
        .unwrap();

    let (later, _) = sys.policy.open_request(&sys.alice, &session, None).unwrap();
    assert_eq!(
        sys.policy
            .dispatch(&later, &sys.alice, &sys.node_a_finance_legal),
        Err(Denial::NodeNotReady {
            node: sys.node_a.clone(),
            state: NodeState::Draining,
        })
    );
    assert!(sys.policy.complete_request(&request, &sys.alice).is_ok());
}

#[test]
fn a_stream_cut_by_a_lost_node_never_completes_and_ends_failed_explicit() {
    let mut sys = System::new();
    let session = sys
        .policy
        .create_session(&sys.alice, sys.legal.clone())
        .unwrap();
    let (request, _) = sys.policy.open_request(&sys.alice, &session, None).unwrap();
    sys.policy
        .dispatch(&request, &sys.alice, &sys.node_a_finance_legal)
        .unwrap();

    sys.policy
        .node_event(&sys.node_a, NodeEvent::ReadinessLost)
        .unwrap();
    sys.policy
        .node_event(&sys.node_a, NodeEvent::Timeout)
        .unwrap();

    assert_eq!(
        sys.policy.complete_request(&request, &sys.alice),
        Err(Denial::RequestCut)
    );
    assert!(sys.policy.fail_request(&request).is_ok());
    assert_eq!(
        sys.policy.complete_request(&request, &sys.alice),
        Err(Denial::RequestNotStarted)
    );
    assert_eq!(
        sys.policy.fail_request(&request),
        Err(Denial::RequestAlreadyTerminal)
    );
}

#[test]
fn suspect_node_blocks_new_placements_until_it_recovers() {
    let mut sys = System::new();
    let session = sys
        .policy
        .create_session(&sys.bob, sys.finance.clone())
        .unwrap();

    sys.policy
        .node_event(&sys.node_b, NodeEvent::ReadinessLost)
        .unwrap();
    assert_eq!(
        sys.policy
            .authorize_placement(&sys.bob, &session, &sys.node_b_finance),
        Err(Denial::NodeNotReady {
            node: sys.node_b.clone(),
            state: NodeState::Suspect,
        })
    );
    // node-b is omitted from the eligible list while Suspect; node-a and node-c (finance+legal-trusted)
    // remain.
    assert_eq!(
        sys.policy
            .eligible_deployments(&sys.bob, &session, Some(&sys.qwen)),
        Ok(vec![
            sys.node_a_finance_legal.clone(),
            sys.node_c_finance_legal.clone()
        ])
    );

    sys.policy
        .node_event(&sys.node_b, NodeEvent::Recovered)
        .unwrap();
    assert!(
        sys.policy
            .authorize_placement(&sys.bob, &session, &sys.node_b_finance)
            .is_ok()
    );
}

#[test]
fn dispatch_reauthorizes_the_node_the_trust_and_the_user_after_arrival() {
    // The node is drained after arrival.
    let mut sys = System::new();
    let session = sys
        .policy
        .create_session(&sys.bob, sys.finance.clone())
        .unwrap();
    let (request, _) = sys.policy.open_request(&sys.bob, &session, None).unwrap();
    sys.policy
        .node_event(&sys.node_b, NodeEvent::Drain)
        .unwrap();
    assert_eq!(
        sys.policy.dispatch(&request, &sys.bob, &sys.node_b_finance),
        Err(Denial::NodeNotReady {
            node: sys.node_b.clone(),
            state: NodeState::Draining,
        })
    );
    assert_eq!(
        sys.policy.request(&request).unwrap().state,
        RequestState::Unstarted
    );

    // The deployment's trust drops below the session label.
    let mut sys = System::new();
    let session = sys
        .policy
        .create_session(&sys.bob, sys.finance.clone())
        .unwrap();
    let (request, _) = sys.policy.open_request(&sys.bob, &session, None).unwrap();
    sys.policy
        .set_deployment_trust(&sys.node_b_finance, Label::public())
        .unwrap();
    assert_eq!(
        sys.policy.dispatch(&request, &sys.bob, &sys.node_b_finance),
        Err(Denial::DeploymentNotTrusted {
            session: sys.finance.clone(),
            trusted_for: Label::public(),
        })
    );
    assert_eq!(
        sys.policy.request(&request).unwrap().state,
        RequestState::Unstarted
    );

    // The user is revoked.
    let mut sys = System::new();
    let session = sys
        .policy
        .create_session(&sys.bob, sys.finance.clone())
        .unwrap();
    let (request, _) = sys.policy.open_request(&sys.bob, &session, None).unwrap();
    sys.policy.revoke_user(&sys.bob).unwrap();
    assert_eq!(
        sys.policy.dispatch(&request, &sys.bob, &sys.node_b_finance),
        Err(Denial::UserRevoked)
    );
    assert_eq!(
        sys.policy.request(&request).unwrap().state,
        RequestState::Unstarted
    );
}

#[test]
fn complete_request_rechecks_clearance_and_trust_while_in_flight() {
    // Bob's finance clearance is removed mid-stream.
    let mut sys = System::new();
    let session = sys
        .policy
        .create_session(&sys.bob, sys.finance.clone())
        .unwrap();
    let (request, _) = sys.policy.open_request(&sys.bob, &session, None).unwrap();
    sys.policy
        .dispatch(&request, &sys.bob, &sys.node_b_finance)
        .unwrap();
    sys.policy.set_clearance(&sys.bob, Label::public()).unwrap();
    assert_eq!(
        sys.policy.complete_request(&request, &sys.bob),
        Err(Denial::LabelExceedsClearance {
            requested: sys.finance.clone(),
            clearance: Label::public(),
        })
    );
    assert!(sys.policy.fail_request(&request).is_ok());
    assert_eq!(
        sys.policy.complete_request(&request, &sys.bob),
        Err(Denial::RequestNotStarted)
    );

    // node-b-finance's trust drops to public mid-stream.
    let mut sys = System::new();
    let session = sys
        .policy
        .create_session(&sys.bob, sys.finance.clone())
        .unwrap();
    let (request, _) = sys.policy.open_request(&sys.bob, &session, None).unwrap();
    sys.policy
        .dispatch(&request, &sys.bob, &sys.node_b_finance)
        .unwrap();
    sys.policy
        .set_deployment_trust(&sys.node_b_finance, Label::public())
        .unwrap();
    assert_eq!(
        sys.policy.complete_request(&request, &sys.bob),
        Err(Denial::DeploymentNotTrusted {
            session: sys.finance.clone(),
            trusted_for: Label::public(),
        })
    );
    assert!(sys.policy.fail_request(&request).is_ok());
    assert_eq!(
        sys.policy.request(&request).unwrap().state,
        RequestState::FailedExplicit
    );
}

#[test]
fn admitted_loading_and_draining_nodes_go_lost_directly_on_timeout() {
    let mut sys = System::new();

    let admitted = NodeId::new("spare-admitted");
    sys.policy.node_event(&admitted, NodeEvent::Admit).unwrap();
    sys.policy
        .node_event(&admitted, NodeEvent::Timeout)
        .unwrap();
    assert_eq!(sys.policy.node(&admitted).unwrap().state, NodeState::Lost);

    let loading = NodeId::new("spare-loading");
    sys.policy.node_event(&loading, NodeEvent::Admit).unwrap();
    sys.policy
        .node_event(&loading, NodeEvent::ModelsVerified)
        .unwrap();
    sys.policy.node_event(&loading, NodeEvent::Timeout).unwrap();
    assert_eq!(sys.policy.node(&loading).unwrap().state, NodeState::Lost);

    // A Draining node's started stream is cut once the node is lost.
    let session = sys
        .policy
        .create_session(&sys.alice, sys.finance.clone())
        .unwrap();
    let (request, _) = sys.policy.open_request(&sys.alice, &session, None).unwrap();
    sys.policy
        .dispatch(&request, &sys.alice, &sys.node_a_finance_legal)
        .unwrap();
    sys.policy
        .node_event(&sys.node_a, NodeEvent::Drain)
        .unwrap();
    sys.policy
        .node_event(&sys.node_a, NodeEvent::Timeout)
        .unwrap();
    assert_eq!(sys.policy.node(&sys.node_a).unwrap().state, NodeState::Lost);
    assert_eq!(
        sys.policy.complete_request(&request, &sys.alice),
        Err(Denial::RequestCut)
    );
}
