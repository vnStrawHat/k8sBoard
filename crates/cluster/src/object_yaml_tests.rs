use serde_json::json;

use super::*;

const ALL_KINDS: [ObjectKind; 17] = [
    ObjectKind::Pod,
    ObjectKind::Node,
    ObjectKind::Namespace,
    ObjectKind::Event,
    ObjectKind::Deployment,
    ObjectKind::StatefulSet,
    ObjectKind::DaemonSet,
    ObjectKind::ReplicaSet,
    ObjectKind::Job,
    ObjectKind::CronJob,
    ObjectKind::Service,
    ObjectKind::Ingress,
    ObjectKind::ConfigMap,
    ObjectKind::NetworkPolicy,
    ObjectKind::HorizontalPodAutoscaler,
    ObjectKind::ResourceQuota,
    ObjectKind::PodDisruptionBudget,
];

fn masked(object: Value, env: EnvValues) -> ObjectYaml {
    to_masked_yaml(object, env).expect("fixture serializes")
}

fn masked_text(object: Value) -> String {
    masked(object, EnvValues::Hidden).text
}

/// A pod-shaped object with one env literal and a `valueFrom` env.
fn pod_with_env() -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {"name": "api-0", "namespace": "shop"},
        "spec": {"containers": [{
            "name": "api",
            "env": [
                {"name": "DB_PASSWORD", "value": "hunter2-literal"},
                {"name": "NODE", "valueFrom": {"fieldRef": {"fieldPath": "spec.nodeName"}}},
            ],
        }]},
    })
}

#[test]
fn object_kind_names_and_scopes() {
    let table = [
        (ObjectKind::Pod, "Pod", true),
        (ObjectKind::Node, "Node", false),
        (ObjectKind::Namespace, "Namespace", false),
        (ObjectKind::Event, "Event", true),
        (ObjectKind::Deployment, "Deployment", true),
        (ObjectKind::StatefulSet, "StatefulSet", true),
        (ObjectKind::DaemonSet, "DaemonSet", true),
        (ObjectKind::ReplicaSet, "ReplicaSet", true),
        (ObjectKind::Job, "Job", true),
        (ObjectKind::CronJob, "CronJob", true),
        (ObjectKind::Service, "Service", true),
        (ObjectKind::Ingress, "Ingress", true),
        (ObjectKind::ConfigMap, "ConfigMap", true),
        (ObjectKind::NetworkPolicy, "NetworkPolicy", true),
        (
            ObjectKind::HorizontalPodAutoscaler,
            "HorizontalPodAutoscaler",
            true,
        ),
        (ObjectKind::ResourceQuota, "ResourceQuota", true),
        (ObjectKind::PodDisruptionBudget, "PodDisruptionBudget", true),
    ];
    assert_eq!(table.len(), ALL_KINDS.len());
    for (kind, name, is_namespaced) in table {
        assert_eq!(kind.name(), name);
        assert_eq!(kind.is_namespaced(), is_namespaced, "{name}");
    }
}

#[test]
fn api_resources_match_kinds() {
    let table = [
        (ObjectKind::Pod, "", "v1", "pods"),
        (ObjectKind::Node, "", "v1", "nodes"),
        (ObjectKind::Namespace, "", "v1", "namespaces"),
        (ObjectKind::Event, "", "v1", "events"),
        (ObjectKind::Deployment, "apps", "v1", "deployments"),
        (ObjectKind::StatefulSet, "apps", "v1", "statefulsets"),
        (ObjectKind::DaemonSet, "apps", "v1", "daemonsets"),
        (ObjectKind::ReplicaSet, "apps", "v1", "replicasets"),
        (ObjectKind::Job, "batch", "v1", "jobs"),
        (ObjectKind::CronJob, "batch", "v1", "cronjobs"),
        (ObjectKind::Service, "", "v1", "services"),
        (ObjectKind::Ingress, "networking.k8s.io", "v1", "ingresses"),
        (ObjectKind::ConfigMap, "", "v1", "configmaps"),
        (
            ObjectKind::NetworkPolicy,
            "networking.k8s.io",
            "v1",
            "networkpolicies",
        ),
        (
            ObjectKind::HorizontalPodAutoscaler,
            "autoscaling",
            "v2",
            "horizontalpodautoscalers",
        ),
        (ObjectKind::ResourceQuota, "", "v1", "resourcequotas"),
        (
            ObjectKind::PodDisruptionBudget,
            "policy",
            "v1",
            "poddisruptionbudgets",
        ),
    ];
    assert_eq!(table.len(), ALL_KINDS.len());
    for (kind, group, version, plural) in table {
        let resource = api_resource(kind);
        assert_eq!(resource.kind, kind.name());
        assert_eq!(resource.group, group, "{}", kind.name());
        assert_eq!(resource.version, version, "{}", kind.name());
        assert_eq!(resource.plural, plural, "{}", kind.name());
    }
}

#[test]
fn object_ref_requires_namespace_exactly_for_namespaced_kinds() {
    let namespace = || Some("shop".to_owned());
    let name = || "x".to_owned();
    assert!(ObjectRef::new(ObjectKind::Pod, None, name()).is_none());
    assert!(ObjectRef::new(ObjectKind::Node, namespace(), name()).is_none());
    assert!(ObjectRef::new(ObjectKind::Namespace, namespace(), name()).is_none());
    assert!(ObjectRef::new(ObjectKind::Pod, namespace(), name()).is_some());
    assert!(ObjectRef::new(ObjectKind::Node, None, name()).is_some());
    assert!(ObjectRef::new(ObjectKind::Namespace, None, name()).is_some());
}

#[test]
fn managed_fields_are_removed() {
    let text = masked_text(json!({
        "kind": "Pod",
        "metadata": {
            "name": "api-0",
            "managedFields": [{"manager": "kubelet-distinctive", "operation": "Update"}],
        },
    }));
    assert!(!text.contains("managedFields"));
    assert!(!text.contains("kubelet-distinctive"));
    assert!(text.contains("name: api-0"));
}

#[test]
fn manifest_annotations_are_hidden() {
    let result = masked(
        json!({
            "kind": "Service",
            "metadata": {"annotations": {
                "kubectl.kubernetes.io/last-applied-configuration": "applied-distinctive",
                "kapp.k14s.io/original": "original-distinctive",
                "kapp.k14s.io/original-diff": "diff-distinctive",
                "owner": "team-a",
            }},
        }),
        EnvValues::Hidden,
    );
    for secret in [
        "applied-distinctive",
        "original-distinctive",
        "diff-distinctive",
    ] {
        assert!(!result.text.contains(secret), "{secret}");
    }
    assert!(result.text.contains("owner: team-a"));
    assert!(
        result
            .text
            .starts_with("# k8sBoard hid 3 values as <hidden>.\n")
    );
    assert_eq!(result.hidden_env_values, 0);
}

#[test]
fn manifest_annotations_are_hidden_in_templates() {
    let annotations =
        json!({"kubectl.kubernetes.io/last-applied-configuration": "tpl-distinctive"});
    let text = masked_text(json!({
        "kind": "CronJob",
        "metadata": {"name": "nightly"},
        "spec": {
            "jobTemplate": {"spec": {"template": {"metadata": {"annotations": annotations}}}},
        },
    }));
    assert!(!text.contains("tpl-distinctive"));
    assert!(text.starts_with("# k8sBoard hid 1 value as <hidden>.\n"));

    let text = masked_text(json!({
        "kind": "Deployment",
        "spec": {"template": {"metadata": {"annotations": annotations}}},
    }));
    assert!(!text.contains("tpl-distinctive"));
}

#[test]
fn secret_values_are_hidden_and_keys_kept() {
    let text = masked_text(json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": {"name": "db"},
        "data": {"password": "c2VjcmV0LWRpc3RpbmN0aXZl"},
        "stringData": {"token": "token-distinctive"},
    }));
    assert!(!text.contains("c2VjcmV0LWRpc3RpbmN0aXZl"));
    assert!(!text.contains("token-distinctive"));
    assert!(text.contains("password: <hidden>"));
    assert!(text.contains("token: <hidden>"));
    assert!(text.starts_with("# k8sBoard hid 2 values as <hidden>.\n"));
}

#[test]
fn secret_rule_ignores_api_version() {
    let text = masked_text(json!({
        "apiVersion": "example.com/v1",
        "kind": "Secret",
        "data": {"password": "other-group-distinctive"},
    }));
    assert!(!text.contains("other-group-distinctive"));
}

#[test]
fn config_map_data_is_shown() {
    let text = masked_text(json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "data": {"level": "info-distinctive"},
        "binaryData": {"blob": "YmluYXJ5"},
    }));
    assert!(text.contains("level: info-distinctive"));
    assert!(text.contains("blob: YmluYXJ5"));
    assert!(!text.contains("<hidden>"));
}

#[test]
fn env_values_are_hidden_in_pod_spec() {
    let container = |name: &str, literal: &str| {
        json!({"name": name, "env": [
            {"name": "A", "value": literal},
            {"name": "B", "valueFrom": {"secretKeyRef": {"name": "db", "key": "password"}}},
        ]})
    };
    let result = masked(
        json!({
            "kind": "Pod",
            "spec": {
                "containers": [container("main", "main-distinctive")],
                "initContainers": [container("init", "init-distinctive")],
                "ephemeralContainers": [container("debug", "debug-distinctive")],
            },
        }),
        EnvValues::Hidden,
    );
    for secret in ["main-distinctive", "init-distinctive", "debug-distinctive"] {
        assert!(!result.text.contains(secret), "{secret}");
    }
    assert_eq!(result.hidden_env_values, 3);
    assert_eq!(result.text.matches("secretKeyRef").count(), 3);
    assert!(
        result
            .text
            .starts_with("# k8sBoard hid 3 values as <hidden>.\n")
    );
}

#[test]
fn env_values_are_hidden_in_nested_templates() {
    let containers =
        |literal: &str| json!([{"name": "c", "env": [{"name": "A", "value": literal}]}]);
    let deployment = masked(
        json!({"kind": "Deployment", "spec": {"template": {"spec": {
            "containers": containers("deployment-distinctive"),
        }}}}),
        EnvValues::Hidden,
    );
    assert!(!deployment.text.contains("deployment-distinctive"));
    assert_eq!(deployment.hidden_env_values, 1);

    let cron_job = masked(
        json!({"kind": "CronJob", "spec": {"jobTemplate": {"spec": {"template": {"spec": {
            "containers": containers("cron-distinctive"),
        }}}}}}),
        EnvValues::Hidden,
    );
    assert!(!cron_job.text.contains("cron-distinctive"));
    assert_eq!(cron_job.hidden_env_values, 1);
}

#[test]
fn env_values_are_shown_on_request() {
    let result = masked(pod_with_env(), EnvValues::Shown);
    assert!(result.text.contains("hunter2-literal"));
    assert_eq!(result.hidden_env_values, 0);
    assert!(!result.text.starts_with('#'));
}

#[test]
fn shown_env_values_keep_secret_data_and_annotations_hidden() {
    let mut object = pod_with_env();
    object["kind"] = json!("Secret");
    object["data"] = json!({"password": "secret-data-distinctive"});
    object["metadata"]["annotations"] =
        json!({"kubectl.kubernetes.io/last-applied-configuration": "annotation-distinctive"});
    let result = masked(object, EnvValues::Shown);
    assert!(result.text.contains("hunter2-literal"));
    assert!(!result.text.contains("secret-data-distinctive"));
    assert!(!result.text.contains("annotation-distinctive"));
    assert!(result.text.starts_with(
        "# k8sBoard hid 2 values as <hidden>.
"
    ));
    assert_eq!(result.hidden_env_values, 0);
}

#[test]
fn status_is_kept() {
    let text = masked_text(json!({"kind": "Pod", "status": {"phase": "Running"}}));
    assert!(text.contains("status:\n  phase: Running"));
}

#[test]
fn keys_are_sorted_like_kubectl() {
    let text = masked_text(json!({
        "status": {"phase": "Running"},
        "spec": {"nodeName": "n1", "containers": [{"name": "c", "image": "i"}]},
        "metadata": {"namespace": "shop", "name": "api-0"},
        "kind": "Pod",
        "apiVersion": "v1",
    }));
    let expected = "apiVersion: v1\n\
        kind: Pod\n\
        metadata:\n  name: api-0\n  namespace: shop\n\
        spec:\n  containers:\n  - image: i\n    name: c\n  nodeName: n1\n\
        status:\n  phase: Running\n";
    assert_eq!(text, expected);
}

#[test]
fn multi_line_strings_use_literal_blocks() {
    let text = masked_text(json!({"kind": "ConfigMap", "data": {"app.ini": "a=1\nb=2\n"}}));
    assert!(text.contains("app.ini: |\n"), "{text}");
    assert!(text.contains("    a=1\n    b=2\n"), "{text}");
}

#[test]
fn long_single_line_strings_stay_on_one_line() {
    let sentence = "word ".repeat(40).trim_end().to_owned();
    assert_eq!(sentence.len(), 199);
    let text = masked_text(json!({"kind": "ConfigMap", "data": {"note": sentence}}));
    assert!(text.contains(&format!("note: {sentence}\n")), "{text}");
    assert!(!text.contains('>'));
}

#[test]
fn header_counts_hidden_values() {
    let object = |literals: &[&str]| {
        let env: Vec<Value> = literals
            .iter()
            .map(|literal| json!({"name": "A", "value": literal}))
            .collect();
        json!({"kind": "Pod", "spec": {"containers": [{"name": "c", "env": env}]}})
    };
    assert!(
        masked_text(object(&["a", "b", "c"])).starts_with("# k8sBoard hid 3 values as <hidden>.\n")
    );
    assert!(masked_text(object(&["a"])).starts_with("# k8sBoard hid 1 value as <hidden>.\n"));
}

#[test]
fn no_header_without_hidden_values() {
    let text = masked_text(json!({"kind": "Pod", "metadata": {"name": "api-0"}}));
    assert!(!text.contains("k8sBoard hid"));
    assert!(text.starts_with("kind: Pod\n"));
}

#[test]
fn policy_kinds_have_names_and_scope() {
    for kind in &ALL_KINDS[13..] {
        assert!(kind.is_namespaced(), "{}", kind.name());
    }
    assert_eq!(
        ObjectKind::PodDisruptionBudget.name(),
        "PodDisruptionBudget"
    );
}
