use policy::{
    Compartment, Denial, DeploymentId, Label, ModelName, NodeEvent, NodeId, Policy,
    ProvenanceEvent, RequestId, SessionId, UserId,
};

struct System {
    policy: Policy,
    alice: UserId,
    bob: UserId,
    mallory: UserId,
    finance: Label,
    legal: Label,
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
        let node_a = NodeId::new("node-a");
        let node_b = NodeId::new("node-b");
        let node_b_finance = DeploymentId::new("node-b-finance");

        let mut policy = Policy::new();
        policy.add_user(alice.clone(), finance_legal.clone());
        policy.add_user(bob.clone(), finance.clone());
        policy.add_user(mallory.clone(), Label::public());

        for node in [&node_a, &node_b] {
            policy.node_event(node, NodeEvent::Admit).unwrap();
            policy.node_event(node, NodeEvent::ModelsVerified).unwrap();
            policy.node_event(node, NodeEvent::ReadinessPassed).unwrap();
        }

        policy.add_deployment(
            DeploymentId::new("node-a-finance-legal"),
            qwen.clone(),
            finance_legal.clone(),
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
            mallory,
            finance,
            legal,
        }
    }
}

#[test]
fn a_denied_session_creation_is_recorded_with_its_denial() {
    let mut sys = System::new();
    let result = sys.policy.create_session(&sys.bob, sys.legal.clone());
    assert_eq!(
        result,
        Err(Denial::LabelExceedsClearance {
            requested: sys.legal.clone(),
            clearance: sys.finance.clone(),
        })
    );

    let records = sys.policy.provenance_since(0);
    let record = records
        .iter()
        .find(|r| matches!(r.event, ProvenanceEvent::SessionCreated))
        .expect("a SessionCreated record");
    assert_eq!(record.actor, Some(sys.bob.clone()));
    assert_eq!(record.session, None);
    assert_eq!(record.label, sys.legal);
    assert_eq!(
        record.denial,
        Some(Denial::LabelExceedsClearance {
            requested: sys.legal.clone(),
            clearance: sys.finance.clone(),
        })
    );
}

#[test]
fn provenance_seq_is_strictly_increasing() {
    let mut sys = System::new();
    let _ = sys.policy.create_session(&sys.alice, sys.finance.clone());
    let _ = sys.policy.create_session(&sys.bob, sys.legal.clone());
    let _ = sys.policy.create_session(&sys.mallory, Label::public());

    let records = sys.policy.provenance_since(0);
    let seqs: Vec<u64> = records.iter().map(|r| r.seq).collect();
    assert!(seqs.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn revocation_events_carry_the_epoch_after_the_bump() {
    let mut sys = System::new();
    assert_eq!(sys.policy.epoch(), 0);
    sys.policy.revoke_user(&sys.bob).unwrap();
    assert_eq!(sys.policy.epoch(), 1);

    let records = sys.policy.provenance_since(0);
    let record = records
        .iter()
        .rev()
        .find(|r| matches!(r.event, ProvenanceEvent::UserRevoked))
        .expect("a UserRevoked record");
    assert_eq!(record.epoch, 1);
}

#[test]
fn wire_accessors_round_trip_ids_and_list_compartments_in_order() {
    let label = Label::new([Compartment::new("legal"), Compartment::new("finance")]);
    let names: Vec<&str> = label.compartments().map(Compartment::as_str).collect();
    assert_eq!(names, ["finance", "legal"]);
    assert_eq!(SessionId::from_raw(8).get(), 8);
    assert_eq!(RequestId::from_raw(9).get(), 9);
    assert_eq!(DeploymentId::new("engine-1").as_str(), "engine-1");
    assert_eq!(NodeId::new("node-a").as_str(), "node-a");
}
