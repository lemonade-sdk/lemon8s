#![allow(dead_code)]

use aws_lc_rs::digest::{SHA256, digest};
use control::crd::{
    Model, ModelDeployment, ModelDeploymentSpec, ModelSource, ModelSpec, ServiceRef, User, UserSpec,
};
use control::{Control, Desired};
use control::{CreateSession, ErrorCode, ErrorReply, KeyHash, Name};
use k8s_openapi::api::core::v1::{Node, NodeCondition, NodeSpec, NodeStatus};
use k8s_openapi::api::discovery::v1::{Endpoint, EndpointConditions, EndpointSlice};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use policy::{Compartment, Denial, Label};

pub const DEPLOYMENT: &str = "engine-test";
pub const OTHER_DEPLOYMENT: &str = "engine-other";
pub const NODE: &str = "node-a";
pub const OTHER_NODE: &str = "node-b";
pub const MODEL: &str = "qwen3-30b-a3b";

pub fn name(s: &str) -> Name {
    Name::new(s).unwrap()
}

pub fn label(compartments: &[&str]) -> Label {
    Label::new(compartments.iter().map(|c| Compartment::new(*c)))
}

/// A label's compartments as a call carries them.
pub fn names(compartments: &[&str]) -> Vec<Name> {
    compartments.iter().map(|c| name(c)).collect()
}

/// What a gateway sends for an API key: its SHA-256.
pub fn key_hash(key: &str) -> KeyHash {
    KeyHash(digest(&SHA256, key.as_bytes()).as_ref().try_into().unwrap())
}

pub fn user(name: &str, clearance: &[&str], revoked: bool) -> User {
    User::new(
        name,
        UserSpec {
            clearance: clearance.iter().map(|c| (*c).to_owned()).collect(),
            revoked,
        },
    )
}

pub fn model(name: &str, served: &str, approved_for: &[&str]) -> Model {
    Model::new(
        name,
        ModelSpec {
            name: served.to_owned(),
            source: ModelSource {
                repository: "lemon8s-test/models".to_owned(),
                file: format!("{name}.gguf"),
            },
            sha256: "ab".repeat(32),
            approved_for: approved_for.iter().map(|c| (*c).to_owned()).collect(),
        },
    )
}

/// An engine of model qwen3 behind Service `<name>`.
pub fn deployment(name: &str, node: &str, trusted_for: &[&str]) -> ModelDeployment {
    ModelDeployment::new(
        name,
        ModelDeploymentSpec {
            node: node.to_owned(),
            model: "qwen3".to_owned(),
            trusted_for: trusted_for.iter().map(|c| (*c).to_owned()).collect(),
            engine_service: Some(ServiceRef {
                name: name.to_owned(),
            }),
        },
    )
}

/// The `EndpointSlice` the `EndpointSlice` controller keeps for engine Service `namespace/service`,
/// with the Service's labels and one endpoint on `node` whose readiness the kubelet decides.
pub fn endpoint_slice(namespace: &str, service: &str, node: &str, ready: bool) -> EndpointSlice {
    EndpointSlice {
        metadata: ObjectMeta {
            name: Some(format!("{service}-abcde")),
            namespace: Some(namespace.to_owned()),
            labels: Some(
                [
                    ("kubernetes.io/service-name".to_owned(), service.to_owned()),
                    ("app.kubernetes.io/name".to_owned(), "engine".to_owned()),
                    (
                        "endpointslice.kubernetes.io/managed-by".to_owned(),
                        "endpointslice-controller.k8s.io".to_owned(),
                    ),
                ]
                .into(),
            ),
            ..ObjectMeta::default()
        },
        address_type: "IPv4".to_owned(),
        endpoints: Some(vec![Endpoint {
            addresses: vec!["10.42.0.17".to_owned()],
            conditions: Some(EndpointConditions {
                ready: Some(ready),
                ..EndpointConditions::default()
            }),
            node_name: Some(node.to_owned()),
            ..Endpoint::default()
        }]),
        ports: None,
    }
}

/// A Kubernetes Node as the kubelet and `kubectl cordon` leave it.
pub fn node(name: &str, ready: bool, cordoned: bool) -> Node {
    Node {
        metadata: ObjectMeta {
            name: Some(name.to_owned()),
            ..ObjectMeta::default()
        },
        spec: Some(NodeSpec {
            unschedulable: Some(cordoned),
            ..NodeSpec::default()
        }),
        status: Some(NodeStatus {
            conditions: Some(vec![NodeCondition {
                type_: "Ready".to_owned(),
                status: if ready { "True" } else { "False" }.to_owned(),
                ..NodeCondition::default()
            }]),
            ..NodeStatus::default()
        }),
    }
}

/// The system's operator configuration: users alice (finance, legal) and bob (finance); nodes node-a and node-b
/// Ready; model qwen3 (served as `MODEL`, approved for finance and legal); deployment engine-test (finance)
/// on node-a and engine-other (finance) on node-b, both engines' `EndpointSlices` ready.
pub fn operator_config() -> Desired {
    Desired {
        users: vec![
            user("alice", &["finance", "legal"], false),
            user("bob", &["finance"], false),
        ],
        deployments: vec![
            deployment(DEPLOYMENT, NODE, &["finance"]),
            deployment(OTHER_DEPLOYMENT, OTHER_NODE, &["finance"]),
        ],
        models: vec![model("qwen3", MODEL, &["finance", "legal"])],
        nodes: vec![node(NODE, true, false), node(OTHER_NODE, true, false)],
        endpoint_slices: vec![
            endpoint_slice("lemon8s", DEPLOYMENT, NODE, true),
            endpoint_slice("lemon8s", OTHER_DEPLOYMENT, OTHER_NODE, true),
        ],
    }
}

/// Opens a session for `user` the way the gateway does, through `Control::call`.
pub async fn session(control: &Control, user: &str, key: &str, l: &[&str]) -> u64 {
    control
        .call(CreateSession {
            user: name(user),
            label: names(l),
            key_hash: key_hash(key),
        })
        .await
        .unwrap()
        .session
}

/// The policy's reason for a refused call.
pub fn denial<T: std::fmt::Debug>(result: Result<T, ErrorReply>) -> Denial {
    match result {
        Err(ErrorReply {
            code: ErrorCode::Denied,
            denial: Some(denial),
            ..
        }) => denial,
        other => panic!("expected a denial, got {other:?}"),
    }
}
