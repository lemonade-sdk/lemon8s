//! How a Kubernetes Node's state drives the policy's node lifecycle: every planned transition is
//! one the policy accepts, and it lands where the Node says.

mod common;

use common::node;
use control::NodeTarget;
use policy::{NodeEvent, NodeId, NodeState, Policy};

const STARTS: [Option<NodeState>; 8] = [
    None,
    Some(NodeState::Admitted),
    Some(NodeState::Loading),
    Some(NodeState::Ready),
    Some(NodeState::Suspect),
    Some(NodeState::Draining),
    Some(NodeState::Lost),
    Some(NodeState::Removed),
];

const TARGETS: [NodeTarget; 4] = [
    NodeTarget::Ready,
    NodeTarget::NotReady,
    NodeTarget::Cordoned,
    NodeTarget::Gone,
];

/// A policy holding node `node-a` in `start`, reached along the lifecycle's own transitions.
fn policy_at(start: Option<NodeState>) -> Policy {
    use NodeEvent::*;
    let path: &[NodeEvent] = match start {
        None => &[],
        Some(NodeState::Admitted) => &[Admit],
        Some(NodeState::Loading) => &[Admit, ModelsVerified],
        Some(NodeState::Ready) => &[Admit, ModelsVerified, ReadinessPassed],
        Some(NodeState::Suspect) => &[Admit, ModelsVerified, ReadinessPassed, ReadinessLost],
        Some(NodeState::Draining) => &[Admit, ModelsVerified, ReadinessPassed, Drain],
        Some(NodeState::Lost) => &[
            Admit,
            ModelsVerified,
            ReadinessPassed,
            ReadinessLost,
            Timeout,
        ],
        Some(NodeState::Removed) => &[Admit, ModelsVerified, ReadinessPassed, Drain, Remove],
    };
    let mut policy = Policy::new();
    for event in path {
        policy.node_event(&NodeId::new("node-a"), *event).unwrap();
    }
    policy
}

fn expected(start: Option<NodeState>, target: NodeTarget) -> Option<NodeState> {
    match (start, target) {
        (_, NodeTarget::Ready) => Some(NodeState::Ready),
        (None, NodeTarget::Gone) => None,
        (
            Some(NodeState::Admitted | NodeState::Loading | NodeState::Ready | NodeState::Suspect),
            NodeTarget::Gone,
        ) => Some(NodeState::Lost),
        (Some(NodeState::Ready | NodeState::Suspect), NodeTarget::NotReady) => {
            Some(NodeState::Suspect)
        }
        (Some(NodeState::Ready | NodeState::Suspect), NodeTarget::Cordoned) => {
            Some(NodeState::Draining)
        }
        (Some(NodeState::Draining), NodeTarget::Gone) => Some(NodeState::Removed),
        (Some(NodeState::Lost | NodeState::Removed), NodeTarget::Gone) => start,
        (None | Some(NodeState::Lost | NodeState::Removed), _) => Some(NodeState::Admitted),
        (start, _) => start,
    }
}

#[test]
fn every_transition_is_accepted_and_lands_where_the_node_says() {
    let node_a = NodeId::new("node-a");
    for start in STARTS {
        for target in TARGETS {
            let mut policy = policy_at(start);
            for event in target.transitions(start) {
                assert_eq!(
                    policy.node_event(&node_a, event),
                    Ok(()),
                    "{start:?} -> {target:?} at {event:?}"
                );
            }
            assert_eq!(
                policy.node(&node_a).ok().map(|n| n.state),
                expected(start, target),
                "{start:?} -> {target:?}"
            );
            assert!(
                target
                    .transitions(policy.node(&node_a).ok().map(|n| n.state))
                    .is_empty(),
                "{start:?} -> {target:?} is not a fixed point"
            );
        }
    }
}

#[test]
fn uncordoning_a_drained_node_rejoins_it_under_a_new_incarnation() {
    let node_a = NodeId::new("node-a");
    let mut policy = policy_at(Some(NodeState::Draining));
    let epoch = policy.epoch();

    for event in NodeTarget::Ready.transitions(Some(NodeState::Draining)) {
        policy.node_event(&node_a, event).unwrap();
    }

    let node = policy.node(&node_a).unwrap();
    assert_eq!(node.state, NodeState::Ready);
    assert_eq!(node.incarnation, 2);
    assert!(policy.epoch() > epoch);
}

#[test]
fn a_node_object_reads_as_ready_not_ready_or_cordoned() {
    assert_eq!(
        NodeTarget::of(&node("node-a", true, false)),
        NodeTarget::Ready
    );
    assert_eq!(
        NodeTarget::of(&node("node-a", false, false)),
        NodeTarget::NotReady
    );
    assert_eq!(
        NodeTarget::of(&node("node-a", true, true)),
        NodeTarget::Cordoned
    );
    let mut silent = node("node-a", true, false);
    silent.status = None;
    assert_eq!(NodeTarget::of(&silent), NodeTarget::NotReady);
}
