use policy::{
    Compartment, DeploymentId, Label, ModelName, NodeId, Policy, ProvenanceEvent, UserId,
};

#[test]
fn admitting_a_user_is_visible_to_that_user_and_the_operator_only() {
    let finance = Label::new([Compartment::new("finance")]);
    let alice = UserId::new("alice");
    let bob = UserId::new("bob");
    let mut policy = Policy::new();

    policy.add_user(alice.clone(), finance.clone());
    policy.add_user(bob.clone(), finance.clone());

    let operator: Vec<(ProvenanceEvent, Option<UserId>)> = policy
        .provenance_since(0)
        .iter()
        .map(|r| (r.event.clone(), r.actor.clone()))
        .collect();
    assert_eq!(
        operator,
        [
            (ProvenanceEvent::UserAdded, Some(alice.clone())),
            (ProvenanceEvent::UserAdded, Some(bob.clone())),
        ]
    );
}

#[test]
fn admitting_a_deployment_is_an_operator_record_carrying_its_trust() {
    let finance = Label::new([Compartment::new("finance")]);
    let alice = UserId::new("alice");
    let mut policy = Policy::new();
    policy.add_user(alice.clone(), finance.clone());

    policy.add_deployment(
        DeploymentId::new("engine-1"),
        ModelName::new("qwen3-30b"),
        finance.clone(),
        NodeId::new("node-a"),
    );

    let record = policy.provenance_since(0).last().unwrap();
    assert_eq!(
        record.event,
        ProvenanceEvent::DeploymentAdded {
            deployment: DeploymentId::new("engine-1"),
            model: ModelName::new("qwen3-30b"),
            node: NodeId::new("node-a"),
        }
    );
    assert_eq!(record.label, finance);
}

#[test]
fn meet_keeps_only_the_shared_compartments() {
    let finance = Compartment::new("finance");
    let legal = Compartment::new("legal");
    let both = Label::new([finance.clone(), legal.clone()]);

    assert_eq!(both.meet(&Label::new([legal.clone()])), Label::new([legal]));
    assert_eq!(both.meet(&Label::public()), Label::public());
    assert_eq!(both.meet(&both), both);
    assert_eq!(
        Label::new([finance.clone()]).meet(&Label::new([Compartment::new("compliance")])),
        Label::public()
    );
}

#[test]
fn an_unready_deployment_is_neither_a_candidate_nor_placeable_and_costs_no_epoch() {
    let finance = Label::new([Compartment::new("finance")]);
    let alice = UserId::new("alice");
    let node_a = NodeId::new("node-a");
    let engine = DeploymentId::new("engine-1");
    let mut policy = Policy::new();
    policy.add_user(alice.clone(), finance.clone());
    for event in [
        policy::NodeEvent::Admit,
        policy::NodeEvent::ModelsVerified,
        policy::NodeEvent::ReadinessPassed,
    ] {
        policy.node_event(&node_a, event).unwrap();
    }
    policy.add_deployment(
        engine.clone(),
        ModelName::new("qwen3-30b"),
        finance.clone(),
        node_a,
    );
    let session = policy.create_session(&alice, finance).unwrap();
    let (request, candidates) = policy.open_request(&alice, &session, None).unwrap();
    assert_eq!(candidates, std::slice::from_ref(&engine));
    let epoch = policy.epoch();

    policy.set_deployment_ready(&engine, false).unwrap();

    assert_eq!(policy.epoch(), epoch);
    assert_eq!(
        policy.dispatch(&request, &alice, &engine),
        Err(policy::Denial::DeploymentNotReady {
            deployment: engine.clone()
        })
    );
    assert_eq!(
        policy.unready_deployments(&alice, &session, &ModelName::new("qwen3-30b")),
        std::slice::from_ref(&engine)
    );
    assert_eq!(
        policy.open_request(&alice, &session, Some(&ModelName::new("qwen3-30b"))),
        Err(policy::Denial::NoTrustedDeployment {
            model: ModelName::new("qwen3-30b")
        })
    );
    policy.set_deployment_ready(&engine, true).unwrap();
    assert_eq!(
        policy.unready_deployments(&alice, &session, &ModelName::new("qwen3-30b")),
        []
    );
    assert!(policy.dispatch(&request, &alice, &engine).is_ok());
    assert_eq!(
        policy.set_deployment_ready(&DeploymentId::new("nope"), true),
        Err(policy::Denial::UnknownDeployment)
    );
}
