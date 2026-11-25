//! The configuration control should be enforcing, as last observed in Kubernetes, and how
//! Kubernetes objects map onto the policy: a Node's state onto the node lifecycle, and
//! `ModelDeployments` onto engines.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::{Name, Reason, Rejected};
use k8s_openapi::api::core::v1::Node;
use k8s_openapi::api::discovery::v1::EndpointSlice;
use kube::ResourceExt;
use policy::{Compartment, DeploymentId, Label, ModelName, NodeEvent, NodeId, NodeState};

use crate::crd::{Model, ModelDeployment, User};

/// The operator's configuration and the cluster's state, as one consistent snapshot. Control
/// enforces whatever it is given (`Control::reconcile`), so this is where its inputs cross from
/// their source: the Kubernetes watch (`Control::watch`) fills it from the lemon8s custom
/// resources, Nodes and `EndpointSlices`; tests build it directly.
#[derive(Debug, Clone, Default)]
pub struct Desired {
    pub users: Vec<User>,
    pub deployments: Vec<ModelDeployment>,
    pub models: Vec<Model>,
    pub nodes: Vec<Node>,
    /// `EndpointSlices` the `EndpointSlice` controller keeps for the engines' Services
    /// (`ModelDeploymentSpec::engine_service`).
    pub endpoint_slices: Vec<EndpointSlice>,
}

/// How long a revocation counts toward a mass revocation (`Status::mass_revocation`).
pub const REVOCATION_WINDOW: Duration = Duration::from_mins(10);

/// Objects control can't apply, as (kind, name, reason), for status.
pub(crate) type Rejections = Vec<(&'static str, String, Reason)>;

/// One engine control may place work on: the model it serves, the node it runs on, the tags it
/// is trusted for and whether it can take work now.
pub(crate) struct Engine {
    pub(crate) served: ModelName,
    pub(crate) trust: Label,
    pub(crate) node: NodeId,
    pub(crate) ready: bool,
}

impl Desired {
    /// The cluster's engines: each `ModelDeployment`, trusted for its `trustedFor` met with its
    /// `Model`'s `approvedFor`, and ready when its Service's `EndpointSlices` list a ready
    /// endpoint on its node, but only while that Node reports Ready. A deployment that can't be
    /// applied goes into `rejected` instead.
    pub(crate) fn cluster_engines(
        &self,
        rejected: &mut Rejections,
    ) -> BTreeMap<DeploymentId, Engine> {
        let models: BTreeMap<String, &Model> =
            self.models.iter().map(|m| (m.name_any(), m)).collect();
        let mut engines: BTreeMap<DeploymentId, Engine> = BTreeMap::new();
        let mut deployments: Vec<&ModelDeployment> = self.deployments.iter().collect();
        deployments.sort_by_key(|d| d.name_any());
        for deployment in deployments {
            let name = deployment.name_any();
            let spec = &deployment.spec;
            let Some(model) = models.get(&spec.model) else {
                rejected.push(("ModelDeployment", name, Reason::ModelNotFound));
                continue;
            };
            let trusted_for = spec
                .trusted_for
                .iter()
                .map(Name::new)
                .collect::<Result<Vec<_>, _>>();
            let approved_for = model
                .spec
                .approved_for
                .iter()
                .map(Name::new)
                .collect::<Result<Vec<_>, _>>();
            let fields = (
                Name::new(name.as_str()),
                Name::new(model.spec.name.as_str()),
                Name::new(spec.node.as_str()),
                trusted_for,
                approved_for,
            );
            let (Ok(id), Ok(served), Ok(node), Ok(trusted_for), Ok(approved_for)) = fields else {
                rejected.push(("ModelDeployment", name, Reason::InvalidName));
                continue;
            };
            let trust = Label::new(trusted_for.iter().map(|c| Compartment::new(c.as_str()))).meet(
                &Label::new(approved_for.iter().map(|c| Compartment::new(c.as_str()))),
            );
            let node_ready = self
                .nodes
                .iter()
                .any(|n| n.name_any() == spec.node && reports_ready(n));
            let ready = node_ready
                && match &spec.engine_service {
                    Some(service) => self.endpoint_slices.iter().any(|slice| {
                        slice
                            .metadata
                            .labels
                            .as_ref()
                            .and_then(|l| l.get("kubernetes.io/service-name"))
                            .is_some_and(|n| *n == service.name)
                            && slice.endpoints.iter().flatten().any(|e| {
                                e.conditions.as_ref().and_then(|c| c.ready) == Some(true)
                                    && e.node_name.as_deref() == Some(spec.node.as_str())
                            })
                    }),
                    None => false,
                };
            engines.insert(
                DeploymentId::new(id.as_str()),
                Engine {
                    served: ModelName::new(served.as_str()),
                    trust,
                    node: NodeId::new(node.as_str()),
                    ready,
                },
            );
        }
        engines
    }
}

/// What a reconcile made of the configuration, for writing back into each object's status.
#[derive(Debug, Clone, PartialEq)]
pub struct Reconciled {
    pub rejected: Vec<Rejected>,
    /// Engine readiness of every admitted `ModelDeployment`, by name.
    pub engine_ready: BTreeMap<String, bool>,
}

/// Whether the Node's `Ready` condition is `True`, as its kubelet last reported it.
fn reports_ready(node: &Node) -> bool {
    node.status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .is_some_and(|conditions| {
            conditions
                .iter()
                .any(|c| c.type_ == "Ready" && c.status == "True")
        })
}

/// What a Kubernetes Node says about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeTarget {
    /// `Ready=True` and schedulable.
    Ready,
    /// `Ready` is not `True`.
    NotReady,
    /// Cordoned (`spec.unschedulable`), as by `kubectl cordon` or `kubectl drain`.
    Cordoned,
    /// No such Node object.
    Gone,
}

impl NodeTarget {
    pub fn of(node: &Node) -> Self {
        let cordoned = node
            .spec
            .as_ref()
            .and_then(|s| s.unschedulable)
            .unwrap_or(false);
        if cordoned {
            NodeTarget::Cordoned
        } else if reports_ready(node) {
            NodeTarget::Ready
        } else {
            NodeTarget::NotReady
        }
    }

    /// The policy node events that take a node from `current` (`None`: not yet admitted) to
    /// this target. A drained node that is uncordoned rejoins under a new incarnation, so a
    /// request started before the drain can't complete.
    pub fn transitions(self, current: Option<NodeState>) -> Vec<NodeEvent> {
        use NodeEvent::{
            Admit, Drain, ModelsVerified, ReadinessLost, ReadinessPassed, Recovered, Rejoin,
            Remove, Timeout,
        };
        match (current, self) {
            (None, NodeTarget::Ready) => vec![Admit, ModelsVerified, ReadinessPassed],
            (None, NodeTarget::NotReady | NodeTarget::Cordoned) => vec![Admit],
            (Some(NodeState::Admitted), NodeTarget::Ready) => vec![ModelsVerified, ReadinessPassed],
            (Some(NodeState::Loading), NodeTarget::Ready) => vec![ReadinessPassed],
            (
                Some(NodeState::Admitted | NodeState::Loading | NodeState::Suspect),
                NodeTarget::Gone,
            ) => vec![Timeout],
            (Some(NodeState::Ready), NodeTarget::NotReady) => vec![ReadinessLost],
            (Some(NodeState::Ready), NodeTarget::Cordoned) => vec![Drain],
            (Some(NodeState::Ready), NodeTarget::Gone) => vec![ReadinessLost, Timeout],
            (Some(NodeState::Suspect), NodeTarget::Ready) => vec![Recovered],
            (Some(NodeState::Suspect), NodeTarget::Cordoned) => vec![Recovered, Drain],
            (Some(NodeState::Draining), NodeTarget::Ready) => {
                vec![Remove, Rejoin, ModelsVerified, ReadinessPassed]
            }
            (Some(NodeState::Draining), NodeTarget::Gone) => vec![Remove],
            (Some(NodeState::Lost | NodeState::Removed), NodeTarget::Ready) => {
                vec![Rejoin, ModelsVerified, ReadinessPassed]
            }
            (
                Some(NodeState::Lost | NodeState::Removed),
                NodeTarget::NotReady | NodeTarget::Cordoned,
            ) => vec![Rejoin],
            (None | Some(NodeState::Lost | NodeState::Removed), NodeTarget::Gone)
            | (
                Some(NodeState::Admitted | NodeState::Loading | NodeState::Draining),
                NodeTarget::NotReady | NodeTarget::Cordoned,
            )
            | (Some(NodeState::Ready), NodeTarget::Ready)
            | (Some(NodeState::Suspect), NodeTarget::NotReady) => vec![],
        }
    }
}
