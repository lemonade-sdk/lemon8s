//! The policy epoch: every operation that narrows access advances it, and nothing else does, so
//! the revocation feed can be read from any epoch onward.

use policy::{Compartment, DeploymentId, Label, ModelName, NodeEvent, NodeId, Policy, UserId};

struct System {
    policy: Policy,
    alice: UserId,
    bob: UserId,
    finance: Label,
    node_a: NodeId,
    node_b: NodeId,
    node_b_finance: DeploymentId,
}

impl System {
    fn new() -> Self {
        let finance = Label::new([Compartment::new("finance")]);
        let finance_legal = Label::new([Compartment::new("finance"), Compartment::new("legal")]);
        let alice = UserId::new("alice");
        let bob = UserId::new("bob");
        let qwen = ModelName::new("qwen3-30b");
        let node_a = NodeId::new("node-a");
        let node_b = NodeId::new("node-b");
        let node_b_finance = DeploymentId::new("node-b-finance");

        let mut policy = Policy::new();
        policy.add_user(alice.clone(), finance_legal.clone());
        policy.add_user(bob.clone(), finance.clone());

        for node in [&node_a, &node_b] {
            policy.node_event(node, NodeEvent::Admit).unwrap();
            policy.node_event(node, NodeEvent::ModelsVerified).unwrap();
            policy.node_event(node, NodeEvent::ReadinessPassed).unwrap();
        }

        policy.add_deployment(
            DeploymentId::new("node-a-finance-legal"),
            qwen.clone(),
            finance_legal,
            node_a.clone(),
        );
        policy.add_deployment(
            DeploymentId::new("node-a-finance"),
            qwen.clone(),
            finance.clone(),
            node_a.clone(),
        );
        policy.add_deployment(
            node_b_finance.clone(),
            qwen,
            finance.clone(),
            node_b.clone(),
        );

        Self {
            policy,
            alice,
            bob,
            finance,
            node_a,
            node_b,
            node_b_finance,
        }
    }
}

#[test]
fn narrowing_operations_bump_the_epoch_and_others_dont() {
    let mut sys = System::new();
    assert_eq!(sys.policy.epoch(), 0);

    // None of these narrow access, so none of them should move the epoch.
    sys.policy.add_user(UserId::new("mallory"), Label::public());
    sys.policy
        .create_session(&sys.alice, sys.finance.clone())
        .unwrap();
    sys.policy.add_deployment(
        DeploymentId::new("node-a-extra"),
        ModelName::new("qwen3-30b"),
        sys.finance.clone(),
        sys.node_a.clone(),
    );
    let spare = NodeId::new("spare");
    sys.policy.node_event(&spare, NodeEvent::Admit).unwrap();
    sys.policy
        .node_event(&spare, NodeEvent::ModelsVerified)
        .unwrap();
    sys.policy
        .node_event(&spare, NodeEvent::ReadinessPassed)
        .unwrap();
    assert_eq!(sys.policy.epoch(), 0);

    let mut expected_epoch = 0;

    sys.policy
        .set_clearance(&sys.bob, sys.finance.clone())
        .unwrap();
    expected_epoch += 1;
    assert_eq!(sys.policy.epoch(), expected_epoch);

    sys.policy.revoke_user(&UserId::new("mallory")).unwrap();
    expected_epoch += 1;
    assert_eq!(sys.policy.epoch(), expected_epoch);

    sys.policy
        .set_deployment_trust(&sys.node_b_finance, sys.finance.clone())
        .unwrap();
    expected_epoch += 1;
    assert_eq!(sys.policy.epoch(), expected_epoch);

    sys.policy
        .remove_deployment(&DeploymentId::new("node-a-extra"));
    expected_epoch += 1;
    assert_eq!(sys.policy.epoch(), expected_epoch);

    sys.policy
        .node_event(&sys.node_b, NodeEvent::Drain)
        .unwrap();
    expected_epoch += 1;
    assert_eq!(sys.policy.epoch(), expected_epoch);

    sys.policy
        .node_event(&sys.node_b, NodeEvent::Remove)
        .unwrap();
    expected_epoch += 1;
    assert_eq!(sys.policy.epoch(), expected_epoch);

    sys.policy
        .node_event(&sys.node_b, NodeEvent::Rejoin)
        .unwrap();
    expected_epoch += 1;
    assert_eq!(sys.policy.epoch(), expected_epoch);

    sys.policy
        .node_event(&spare, NodeEvent::ReadinessLost)
        .unwrap();
    expected_epoch += 1;
    assert_eq!(sys.policy.epoch(), expected_epoch);

    sys.policy.node_event(&spare, NodeEvent::Timeout).unwrap();
    expected_epoch += 1;
    assert_eq!(sys.policy.epoch(), expected_epoch);
}
