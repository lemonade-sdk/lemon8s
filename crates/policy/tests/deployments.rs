use policy::{
    Compartment, Denial, DeploymentId, Label, ModelName, NodeEvent, NodeId, Policy, UserId,
};

struct System {
    policy: Policy,
    alice: UserId,
    bob: UserId,
    mallory: UserId,
    finance: Label,
    legal: Label,
    qwen: ModelName,
    big_model: ModelName,
    node_a_finance_legal: DeploymentId,
    node_b_finance: DeploymentId,
    remote: DeploymentId,
    rack_big: DeploymentId,
}

impl System {
    fn new() -> Self {
        let finance = Label::new([Compartment::new("finance")]);
        let legal = Label::new([Compartment::new("legal")]);
        let finance_legal = Label::new([Compartment::new("finance"), Compartment::new("legal")]);
        let alice = UserId::new("alice");
        let bob = UserId::new("bob");
        let mallory = UserId::new("mallory");
        let qwen = ModelName::new("qwen3-30b");
        let big_model = ModelName::new("big-model");
        let node_a_finance_legal = DeploymentId::new("node-a-finance-legal");
        let node_b_finance = DeploymentId::new("node-b-finance");
        let remote = DeploymentId::new("remote");
        let rack_big = DeploymentId::new("rack-big");

        let mut policy = Policy::new();
        policy.add_user(alice.clone(), finance_legal.clone());
        policy.add_user(bob.clone(), finance.clone());
        policy.add_user(mallory.clone(), Label::public());

        let node_a = NodeId::new("node-a");
        let node_b = NodeId::new("node-b");
        let remote_node = NodeId::new("remote-host");
        let rack_node = NodeId::new("rack");
        for node in [&node_a, &node_b, &remote_node, &rack_node] {
            policy.node_event(node, NodeEvent::Admit).unwrap();
            policy.node_event(node, NodeEvent::ModelsVerified).unwrap();
            policy.node_event(node, NodeEvent::ReadinessPassed).unwrap();
        }

        policy.add_deployment(
            node_a_finance_legal.clone(),
            qwen.clone(),
            finance_legal,
            node_a,
        );
        policy.add_deployment(
            node_b_finance.clone(),
            qwen.clone(),
            finance.clone(),
            node_b,
        );
        policy.add_deployment(remote.clone(), qwen.clone(), Label::public(), remote_node);
        policy.add_deployment(
            rack_big.clone(),
            big_model.clone(),
            finance.clone(),
            rack_node,
        );

        Self {
            policy,
            alice,
            bob,
            mallory,
            finance,
            legal,
            qwen,
            big_model,
            node_a_finance_legal,
            node_b_finance,
            remote,
            rack_big,
        }
    }
}

#[test]
fn alice_legal_session_is_eligible_only_for_node_a() {
    let mut sys = System::new();
    let session = sys
        .policy
        .create_session(&sys.alice, sys.legal.clone())
        .unwrap();

    assert_eq!(
        sys.policy
            .eligible_deployments(&sys.alice, &session, Some(&sys.qwen)),
        Ok(vec![sys.node_a_finance_legal.clone()])
    );

    assert!(
        sys.policy
            .authorize_placement(&sys.alice, &session, &sys.node_a_finance_legal)
            .is_ok()
    );
    assert_eq!(
        sys.policy
            .authorize_placement(&sys.alice, &session, &sys.node_b_finance),
        Err(Denial::DeploymentNotTrusted {
            session: sys.legal.clone(),
            trusted_for: sys.finance.clone(),
        })
    );
    assert_eq!(
        sys.policy
            .authorize_placement(&sys.alice, &session, &sys.remote),
        Err(Denial::DeploymentNotTrusted {
            session: sys.legal.clone(),
            trusted_for: Label::public(),
        })
    );
}

#[test]
fn bob_finance_session_is_eligible_for_node_a_and_node_b_but_not_remote() {
    let mut sys = System::new();
    let session = sys
        .policy
        .create_session(&sys.bob, sys.finance.clone())
        .unwrap();

    assert_eq!(
        sys.policy
            .eligible_deployments(&sys.bob, &session, Some(&sys.qwen)),
        Ok(vec![
            sys.node_a_finance_legal.clone(),
            sys.node_b_finance.clone()
        ])
    );
    assert_eq!(
        sys.policy
            .authorize_placement(&sys.bob, &session, &sys.remote),
        Err(Denial::DeploymentNotTrusted {
            session: sys.finance.clone(),
            trusted_for: Label::public(),
        })
    );
}

#[test]
fn mallory_public_session_is_eligible_for_every_deployment_of_the_model() {
    let mut sys = System::new();
    let session = sys
        .policy
        .create_session(&sys.mallory, Label::public())
        .unwrap();

    assert_eq!(
        sys.policy
            .eligible_deployments(&sys.mallory, &session, Some(&sys.qwen)),
        Ok(vec![
            sys.node_a_finance_legal.clone(),
            sys.node_b_finance.clone(),
            sys.remote.clone(),
        ])
    );
}

#[test]
fn explicit_model_never_substitutes_while_automatic_routing_considers_all_models() {
    let mut sys = System::new();
    let session = sys
        .policy
        .create_session(&sys.bob, sys.finance.clone())
        .unwrap();

    let explicit = sys
        .policy
        .eligible_deployments(&sys.bob, &session, Some(&sys.qwen))
        .unwrap();
    assert!(!explicit.contains(&sys.rack_big));

    let automatic = sys
        .policy
        .eligible_deployments(&sys.bob, &session, None)
        .unwrap();
    assert!(automatic.contains(&sys.rack_big));
}

#[test]
fn explicit_model_with_no_trusted_deployment_is_denied_not_substituted() {
    let mut sys = System::new();
    let session = sys
        .policy
        .create_session(&sys.alice, sys.legal.clone())
        .unwrap();

    assert_eq!(
        sys.policy
            .eligible_deployments(&sys.alice, &session, Some(&sys.big_model)),
        Err(Denial::NoTrustedDeployment {
            model: sys.big_model.clone(),
        })
    );
    assert_eq!(
        sys.policy
            .unready_deployments(&sys.alice, &session, &sys.big_model),
        [],
        "no deployment of it is trusted for the session, ready or not"
    );
}

#[test]
fn authorize_placement_rechecks_trust_on_every_call() {
    let mut sys = System::new();
    let session = sys
        .policy
        .create_session(&sys.bob, sys.finance.clone())
        .unwrap();

    assert!(
        sys.policy
            .authorize_placement(&sys.bob, &session, &sys.node_b_finance)
            .is_ok()
    );

    sys.policy
        .set_deployment_trust(&sys.node_b_finance, Label::public())
        .unwrap();

    assert_eq!(
        sys.policy
            .authorize_placement(&sys.bob, &session, &sys.node_b_finance),
        Err(Denial::DeploymentNotTrusted {
            session: sys.finance.clone(),
            trusted_for: Label::public(),
        })
    );
}

#[test]
fn authorize_placement_fails_on_revoked_or_under_cleared_actor_before_the_deployment_check() {
    let mut sys = System::new();
    let finance_legal = Label::new([Compartment::new("finance"), Compartment::new("legal")]);
    let session = sys
        .policy
        .create_session(&sys.alice, finance_legal.clone())
        .unwrap();

    sys.policy
        .set_clearance(&sys.alice, sys.finance.clone())
        .unwrap();
    assert_eq!(
        sys.policy
            .authorize_placement(&sys.alice, &session, &sys.node_a_finance_legal),
        Err(Denial::LabelExceedsClearance {
            requested: finance_legal.clone(),
            clearance: sys.finance.clone(),
        })
    );

    sys.policy.set_clearance(&sys.alice, finance_legal).unwrap();
    sys.policy.revoke_user(&sys.alice).unwrap();
    assert_eq!(
        sys.policy
            .authorize_placement(&sys.alice, &session, &sys.node_a_finance_legal),
        Err(Denial::UserRevoked)
    );
}
