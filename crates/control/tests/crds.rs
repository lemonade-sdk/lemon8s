//! Control's custom resource definitions, and the hand-written `rbac.json` beside this test that
//! lets control read exactly those kinds, the `EndpointSlices` in its namespace and the Nodes, and
//! write only the kinds' status.

use serde_json::{Value, json};

const RBAC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/rbac.json");
const NAMESPACE: &str = "lemonade";

#[test]
fn the_rbac_grants_control_exactly_what_its_watch_reads_and_writes() {
    let rbac: Value = serde_json::from_slice(&std::fs::read(RBAC).unwrap()).unwrap();
    let items = rbac["items"].as_array().unwrap();
    let rules = |kind: &str, name: String, namespace: Value| {
        let role = items
            .iter()
            .find(|i| i["kind"] == kind && i["metadata"]["namespace"] == namespace)
            .unwrap();
        assert_eq!(role["metadata"]["name"], name);
        let binding = items
            .iter()
            .find(|i| i["kind"] == format!("{kind}Binding") && i["metadata"] == role["metadata"])
            .unwrap();
        assert_eq!(
            binding["roleRef"],
            json!({"apiGroup": "rbac.authorization.k8s.io", "kind": kind, "name": name})
        );
        assert_eq!(
            binding["subjects"],
            json!([{"kind": "ServiceAccount", "name": "lemon8s-control", "namespace": NAMESPACE}])
        );
        role["rules"].clone()
    };
    let plurals: Vec<String> = control::crd::definitions()
        .into_iter()
        .map(|d| d.spec.names.plural)
        .collect();
    let statuses: Vec<String> = plurals.iter().map(|p| format!("{p}/status")).collect();
    assert_eq!(
        rules("Role", "lemon8s-control".to_owned(), json!(NAMESPACE)),
        json!([
            {"apiGroups": ["lemon8s.io"], "resources": plurals, "verbs": ["list", "watch"]},
            {"apiGroups": ["lemon8s.io"], "resources": statuses, "verbs": ["patch"]},
            {"apiGroups": ["discovery.k8s.io"], "resources": ["endpointslices"],
             "verbs": ["list", "watch"]},
        ])
    );
    assert_eq!(
        rules(
            "ClusterRole",
            format!("lemon8s-control-{NAMESPACE}"),
            Value::Null
        ),
        json!([{"apiGroups": [""], "resources": ["nodes"], "verbs": ["list", "watch"]}]),
        "named for the namespace, so two installs don't share it"
    );
    assert_eq!(
        items.len(),
        5,
        "the ServiceAccount, and these two roles with their bindings"
    );
}

#[test]
fn every_crd_is_namespaced_in_the_lemon8s_group() {
    let names: Vec<String> = control::crd::definitions()
        .iter()
        .map(|d| {
            assert_eq!(d.spec.group, "lemon8s.io");
            assert_eq!(d.spec.scope, "Namespaced");
            assert_eq!(d.spec.versions[0].name, "v1alpha1");
            d.metadata.name.clone().unwrap()
        })
        .collect();
    assert_eq!(
        names,
        [
            "users.lemon8s.io",
            "modeldeployments.lemon8s.io",
            "models.lemon8s.io",
        ]
    );
}
