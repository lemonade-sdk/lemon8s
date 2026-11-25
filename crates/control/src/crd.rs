//! The operator's configuration as Kubernetes custom resources (`lemon8s.io/v1alpha1`), applied
//! with Helm or kubectl and watched by control. The API server enforces their shape and
//! transition rules (`OpenAPI` patterns and CEL), so control never sees an object that breaks them.

use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Condition;
use kube::core::CustomResourceExt;
use kube::{CustomResource, KubeSchema};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Name;

/// The schema of a resource's list of tags: at most 64, each a `Name`. The fields themselves
/// hold strings, so an object always reads, and control rejects one it can't enforce.
#[derive(JsonSchema)]
pub struct Tags(#[schemars(length(max = 64))] pub Vec<Name>);

/// A user, named by `metadata.name`.
#[derive(CustomResource, KubeSchema, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[kube(
    group = "lemon8s.io",
    version = "v1alpha1",
    kind = "User",
    namespaced,
    status = "ResourceStatus",
    shortname = "l8suser",
    printcolumn = r#"{"name":"Clearance","type":"string","jsonPath":".spec.clearance"}"#,
    printcolumn = r#"{"name":"Revoked","type":"boolean","jsonPath":".spec.revoked"}"#
)]
#[x_kube(validation = Rule::new("!oldSelf.revoked || self.revoked")
    .message("revocation is permanent: a revoked user cannot be reinstated"))]
#[serde(rename_all = "camelCase")]
pub struct UserSpec {
    /// Tags the user is cleared for (empty: public only).
    #[schemars(with = "Tags")]
    pub clearance: Vec<String>,
    /// Once true, stays true; deleting the User revokes it too.
    #[serde(default)]
    pub revoked: bool,
}

/// An engine serving one model on one node, named by `metadata.name`.
#[derive(CustomResource, KubeSchema, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[kube(
    group = "lemon8s.io",
    version = "v1alpha1",
    kind = "ModelDeployment",
    namespaced,
    status = "ResourceStatus",
    shortname = "mdep",
    printcolumn = r#"{"name":"Node","type":"string","jsonPath":".spec.node"}"#,
    printcolumn = r#"{"name":"Model","type":"string","jsonPath":".spec.model"}"#,
    printcolumn = r#"{"name":"Trusted","type":"string","jsonPath":".spec.trustedFor"}"#
)]
#[x_kube(validation = Rule::new("self.node == oldSelf.node && self.model == oldSelf.model")
    .message("node and model are immutable: create a new ModelDeployment"))]
#[serde(rename_all = "camelCase")]
pub struct ModelDeploymentSpec {
    /// The Node of this cluster the engine runs on.
    #[schemars(length(min = 1, max = 253))]
    pub node: String,
    /// The `metadata.name` of the Model it serves.
    #[schemars(length(min = 1, max = 253))]
    pub model: String,
    /// Tags the operator trusts this engine with; the effective trust is this intersected with
    /// the Model's `approvedFor`.
    #[schemars(with = "Tags")]
    pub trusted_for: Vec<String>,
    /// The Service in front of the engine pod, in this namespace and labeled
    /// `app.kubernetes.io/name=engine`: the engine is ready when the `EndpointSlices` the
    /// `EndpointSlice` controller keeps for it list a ready endpoint on `node`. Without one,
    /// control has no readiness signal and never places work on the engine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_service: Option<ServiceRef>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, KubeSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceRef {
    #[schemars(length(min = 1, max = 63))]
    pub name: String,
}

/// An approved model file, named by `metadata.name`.
#[derive(CustomResource, KubeSchema, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[kube(
    group = "lemon8s.io",
    version = "v1alpha1",
    kind = "Model",
    namespaced,
    status = "ResourceStatus",
    shortname = "l8smodel",
    printcolumn = r#"{"name":"Served as","type":"string","jsonPath":".spec.name"}"#,
    printcolumn = r#"{"name":"Approved","type":"string","jsonPath":".spec.approvedFor"}"#
)]
#[x_kube(validation = Rule::new("self.source == oldSelf.source && self.sha256 == oldSelf.sha256")
    .message("source and sha256 are immutable: create a new Model"))]
#[serde(rename_all = "camelCase")]
pub struct ModelSpec {
    /// The model name clients request.
    #[schemars(with = "Name")]
    pub name: String,
    pub source: ModelSource,
    /// The file's SHA-256, in lowercase hex.
    #[schemars(pattern(r"^[0-9a-f]{64}$"))]
    pub sha256: String,
    /// Tags whose data this model is approved to receive.
    #[schemars(with = "Tags")]
    pub approved_for: Vec<String>,
}

/// Where a model's file comes from.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, KubeSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelSource {
    /// The Hugging Face repository, as `<owner>/<name>`.
    #[schemars(
        length(max = 200),
        pattern(r"^[A-Za-z0-9_-][A-Za-z0-9._-]*/[A-Za-z0-9_-][A-Za-z0-9._-]*$")
    )]
    pub repository: String,
    /// The file's path in the repository; no part of it starts with a dot.
    #[schemars(
        length(max = 512),
        pattern(r"^[A-Za-z0-9_-][A-Za-z0-9._-]*(/[A-Za-z0-9_-][A-Za-z0-9._-]*)*$")
    )]
    pub file: String,
}

/// What control made of an object, in the Kubernetes condition convention: `Accepted` (applied
/// to the policy, or why not) on every kind, and `Ready` (the engine can take work) on a
/// `ModelDeployment`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default, schemars::JsonSchema)]
pub struct ResourceStatus {
    #[serde(default)]
    pub conditions: Vec<Condition>,
}

/// Every lemon8s custom resource definition.
pub fn definitions() -> Vec<CustomResourceDefinition> {
    vec![User::crd(), ModelDeployment::crd(), Model::crd()]
}
