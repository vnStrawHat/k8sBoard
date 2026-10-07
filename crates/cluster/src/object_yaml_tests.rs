use serde_json::json;

use super::*;

const ALL_KINDS: [ObjectKind; 26] = [
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
    ObjectKind::PersistentVolumeClaim,
    ObjectKind::PersistentVolume,
    ObjectKind::StorageClass,
    ObjectKind::ServiceAccount,
    ObjectKind::Secret,
    ObjectKind::Role,
    ObjectKind::ClusterRole,
    ObjectKind::RoleBinding,
    ObjectKind::ClusterRoleBinding,
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
        (
            ObjectKind::PersistentVolumeClaim,
            "PersistentVolumeClaim",
            true,
        ),
        (ObjectKind::PersistentVolume, "PersistentVolume", false),
        (ObjectKind::StorageClass, "StorageClass", false),
        (ObjectKind::ServiceAccount, "ServiceAccount", true),
        (ObjectKind::Secret, "Secret", true),
        (ObjectKind::Role, "Role", true),
        (ObjectKind::ClusterRole, "ClusterRole", false),
        (ObjectKind::RoleBinding, "RoleBinding", true),
        (ObjectKind::ClusterRoleBinding, "ClusterRoleBinding", false),
    ];
    assert_eq!(table.len(), ALL_KINDS.len());
    for (kind, name, is_namespaced) in table {
        assert_eq!(kind.name(), name);
        assert_eq!(kind.is_namespaced(), is_namespaced, "{name}");
    }
}

#[test]
fn api_resources_match_kinds() {
    const RBAC: &str = "rbac.authorization.k8s.io";
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
        (
            ObjectKind::PersistentVolumeClaim,
            "",
            "v1",
            "persistentvolumeclaims",
        ),
        (ObjectKind::PersistentVolume, "", "v1", "persistentvolumes"),
        (
            ObjectKind::StorageClass,
            "storage.k8s.io",
            "v1",
            "storageclasses",
        ),
        (ObjectKind::ServiceAccount, "", "v1", "serviceaccounts"),
        (ObjectKind::Secret, "", "v1", "secrets"),
        (ObjectKind::Role, RBAC, "v1", "roles"),
        (ObjectKind::ClusterRole, RBAC, "v1", "clusterroles"),
        (ObjectKind::RoleBinding, RBAC, "v1", "rolebindings"),
        (
            ObjectKind::ClusterRoleBinding,
            RBAC,
            "v1",
            "clusterrolebindings",
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
    for kind in &ALL_KINDS[13..17] {
        assert!(kind.is_namespaced(), "{}", kind.name());
    }
    assert_eq!(
        ObjectKind::PodDisruptionBudget.name(),
        "PodDisruptionBudget"
    );
}

#[test]
fn storage_kinds_have_names_and_scope() {
    assert!(ObjectKind::PersistentVolumeClaim.is_namespaced());
    assert!(!ObjectKind::PersistentVolume.is_namespaced());
    assert!(!ObjectKind::StorageClass.is_namespaced());
    assert!(ObjectRef::new(ObjectKind::PersistentVolume, None, "pv".to_owned()).is_some());
    assert!(
        ObjectRef::new(
            ObjectKind::StorageClass,
            Some("shop".to_owned()),
            "fast".to_owned()
        )
        .is_none()
    );
}

#[test]
fn storage_class_parameters_are_masked_in_yaml() {
    let text = masked_text(json!({
        "kind": "StorageClass",
        "metadata": {"name": "gluster"},
        "parameters": {
            "restuserkey": "distinctive-key",
            "type": "gp3",
            "csi.storage.k8s.io/provisioner-secret-name": "creds",
        },
    }));
    assert!(
        text.starts_with("# k8sBoard hid 1 value as <hidden>.\n"),
        "{text}"
    );
    assert!(!text.contains("distinctive-key"));
    assert!(text.contains("restuserkey: <hidden>"));
    assert!(text.contains("type: gp3"));
    assert!(text.contains("provisioner-secret-name: creds"));
}

#[test]
fn only_storage_classes_have_parameters_masked() {
    let text = masked_text(json!({"kind": "Pod", "parameters": {"password": "visible"}}));
    assert!(text.contains("password: visible"));
}

#[test]
fn credential_mount_options_are_masked_in_yaml() {
    let class = masked(
        json!({
            "kind": "StorageClass",
            "metadata": {"name": "smb"},
            "mountOptions": ["vers=3.0", "password=distinctive-secret", "hard"],
        }),
        EnvValues::Hidden,
    );
    assert!(
        class
            .text
            .starts_with("# k8sBoard hid 1 value as <hidden>.\n"),
        "{}",
        class.text
    );
    assert!(!class.text.contains("distinctive-secret"));
    assert!(class.text.contains("- password=<hidden>"));
    assert!(class.text.contains("- vers=3.0"));
    let volume = masked_text(json!({
        "kind": "PersistentVolume",
        "metadata": {"name": "pv"},
        "spec": {"mountOptions": ["password=distinctive-secret"]},
    }));
    assert!(!volume.contains("distinctive-secret"));
    assert!(volume.contains("- password=<hidden>"));
}

#[test]
fn mount_options_of_other_kinds_are_not_masked() {
    let text = masked_text(json!({"kind": "Pod", "mountOptions": ["password=visible"]}));
    assert!(text.contains("password=visible"));
}

#[test]
fn access_kinds_have_names_and_scope() {
    for kind in &ALL_KINDS[20..25] {
        let is_cluster_wide = matches!(
            kind,
            ObjectKind::ClusterRole | ObjectKind::ClusterRoleBinding
        );
        assert_eq!(kind.is_namespaced(), !is_cluster_wide, "{}", kind.name());
    }
    assert!(ObjectRef::new(ObjectKind::ClusterRole, None, "view".to_owned()).is_some());
    assert!(
        ObjectRef::new(
            ObjectKind::ClusterRoleBinding,
            Some("shop".to_owned()),
            "x".to_owned()
        )
        .is_none()
    );
    assert!(
        ObjectRef::new(
            ObjectKind::ServiceAccount,
            Some("shop".to_owned()),
            "default".to_owned()
        )
        .is_some()
    );
}

#[test]
fn secret_kind_has_name_and_scope() {
    assert_eq!(ObjectKind::Secret.name(), "Secret");
    assert!(ObjectKind::Secret.is_namespaced());
    assert_eq!(api_resource(ObjectKind::Secret).plural, "secrets");
}

#[test]
fn secret_yaml_hides_data_values() {
    let secret = k8s_openapi::api::core::v1::Secret {
        data: Some(
            [(
                "password".to_owned(),
                k8s_openapi::ByteString(b"data-distinctive".to_vec()),
            )]
            .into(),
        ),
        ..Default::default()
    };
    let object = serde_json::to_value(&secret).expect("secret serializes");
    let text = masked_text(object);
    assert!(!text.contains("data-distinctive"));
    assert!(text.contains("password: <hidden>"));
}

// Custom resources

fn custom_text(object: Value) -> String {
    to_masked_custom_yaml(object, EnvValues::Hidden)
        .expect("fixture serializes")
        .text
}

fn custom_type(scope: ResourceScope) -> CustomResourceType {
    CustomResourceType {
        group: "example.io".to_owned(),
        version: "v1".to_owned(),
        kind: "Widget".to_owned(),
        plural: "widgets".to_owned(),
        scope,
    }
}

#[test]
fn crd_kind_has_name_and_cluster_scope() {
    let kind = ObjectKind::CustomResourceDefinition;
    assert_eq!(kind.name(), "CustomResourceDefinition");
    assert!(!kind.is_namespaced());
    let resource = api_resource(kind);
    assert_eq!(resource.group, "apiextensions.k8s.io");
    assert_eq!(resource.version, "v1");
    assert_eq!(resource.plural, "customresourcedefinitions");
    assert!(ObjectRef::new(kind, None, "x.example.io".to_owned()).is_some());
    assert!(ObjectRef::new(kind, Some("ns".to_owned()), "x.example.io".to_owned()).is_none());
}

#[test]
fn client_secret_and_api_key_are_hidden() {
    for key in [
        "clientSecret",
        "apiKey",
        "passphrase",
        "bearerToken",
        "access_key",
        "private-key",
        "dbPassword",
        "adminPasswd",
        "serviceCredentials",
        "restuserkey",
        "clientKey",
        "kubeconfig",
        "connectionString",
    ] {
        assert!(is_secret_key(key), "{key}");
    }
}

#[test]
fn secret_refs_stay_readable() {
    for key in [
        "secretRef",
        "passwordSecretRef",
        "tokenSecretRefs",
        "secretName",
        "csi.storage.k8s.io/provisioner-secret-name",
        "csi.storage.k8s.io/node-stage-secret-namespace",
        "imagePullSecretRefs",
        "clientKeySecretRef",
        "clientKeyRef",
        "kmsKeyId",
        "type",
        "fsType",
        "host",
    ] {
        assert!(!is_secret_key(key), "{key}");
    }
}

#[test]
fn custom_ref_requires_matching_scope() {
    let name = || "x".to_owned();
    let namespaced = custom_type(ResourceScope::Namespaced);
    let cluster = custom_type(ResourceScope::Cluster);
    assert!(ObjectRef::custom(namespaced.clone(), Some("ns".to_owned()), name()).is_some());
    assert!(ObjectRef::custom(namespaced, None, name()).is_none());
    assert!(ObjectRef::custom(cluster.clone(), None, name()).is_some());
    assert!(ObjectRef::custom(cluster, Some("ns".to_owned()), name()).is_none());
}

#[test]
fn custom_yaml_hides_secret_like_keys() {
    let text = custom_text(json!({
        "apiVersion": "example.io/v1",
        "kind": "Widget",
        "metadata": {"name": "w", "namespace": "shop"},
        "spec": {
            "clientSecret": "distinctive-1",
            "nested": {"apiKey": "distinctive-2", "port": 8080, "secretRef": {"name": "creds"}},
            "tokens": ["distinctive-3", "distinctive-4"],
            "pin": 1234,
            "adminPassword": 98765,
            "replicas": 3,
        },
    }));
    assert!(!text.contains("distinctive"), "{text}");
    assert!(!text.contains("98765"), "{text}");
    assert!(text.contains("name: creds"), "{text}");
    assert!(text.contains("port: 8080"), "{text}");
    assert!(text.contains("replicas: 3"), "{text}");
    assert!(text.contains("pin: 1234"), "{text}");
    assert!(
        text.starts_with("# k8sBoard hid 5 values as <hidden>."),
        "{text}"
    );
}

#[test]
fn secret_like_kinds_hide_all_scalars_but_status() {
    let text = custom_text(json!({
        "apiVersion": "example.io/v1",
        "kind": "ClusterSecret",
        "metadata": {"name": "shared", "labels": {"team": "ops"}},
        "data": {"password": "distinctive-1", "user": "distinctive-2", "count": 7},
        "spec": {"match": "distinctive-3", "enabled": true, "empty": {}},
        "status": {"phase": "Synced", "syncedAt": "2026-01-01T00:00:00Z", "apiKey": "distinctive-4"},
    }));
    assert!(!text.contains("distinctive"), "{text}");
    assert!(text.contains("name: shared"), "{text}");
    assert!(text.contains("team: ops"), "{text}");
    assert!(text.contains("kind: ClusterSecret"), "{text}");
    assert!(text.contains("enabled: true"), "{text}");
    assert!(text.contains("phase: Synced"), "{text}");
    assert!(text.contains("syncedAt: "), "{text}");
    // Keys stay, so the shape of the data is still visible.
    assert!(text.contains("password: <hidden>"), "{text}");
    assert!(text.contains("count: <hidden>"), "{text}");
}

#[test]
fn url_userinfo_is_hidden() {
    assert_eq!(
        mask_url_userinfo("postgres://u:p@db:5432/x").as_deref(),
        Some("postgres://<hidden>@db:5432/x")
    );
    assert_eq!(
        mask_url_userinfo("a https://u:p@one/x and amqp://v@two").as_deref(),
        Some("a https://<hidden>@one/x and amqp://<hidden>@two")
    );
    assert_eq!(
        mask_url_userinfo("https://a@b@host/path").as_deref(),
        Some("https://<hidden>@host/path")
    );
    let text = custom_text(json!({
        "apiVersion": "example.io/v1",
        "kind": "Widget",
        "metadata": {"name": "w"},
        "spec": {"dsn": "postgres://svc:distinctive-1@db:5432/x"},
    }));
    assert!(!text.contains("distinctive"), "{text}");
    assert!(text.contains("postgres://<hidden>@db:5432/x"), "{text}");
}

#[test]
fn urls_without_userinfo_are_unchanged() {
    for text in [
        "https://example.com/a@b",
        "https://example.com?mail=a@b",
        "https://example.com#frag@x",
        "postgres://db:5432/x",
        "no url here, a@b",
        "https://<hidden>@host/x",
    ] {
        assert_eq!(mask_url_userinfo(text), None, "{text}");
    }
}

#[test]
fn custom_masking_counts_toward_header() {
    let object = json!({
        "kind": "Widget",
        "metadata": {"name": "w", "annotations": {"kubectl.kubernetes.io/last-applied-configuration": "{}"}},
        "spec": {
            "password": "p",
            "dsn": "redis://u:p@h",
            "containers": [{"name": "c", "env": [{"name": "A", "value": "literal"}]}],
        },
    });
    // Annotation, env literal, password, and DSN: the env literal is not counted twice.
    let yaml = to_masked_custom_yaml(object, EnvValues::Hidden).expect("serializes");
    assert!(
        yaml.text
            .starts_with("# k8sBoard hid 4 values as <hidden>."),
        "{}",
        yaml.text
    );
    assert_eq!(yaml.hidden_env_values, 1);
}

#[test]
fn builtin_yaml_is_unchanged_by_custom_rules() {
    let object = json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": {"name": "settings"},
        "data": {"password": "shown-as-before", "url": "https://u:p@host/x"},
    });
    let text = masked_text(object);
    assert!(text.contains("password: shown-as-before"), "{text}");
    assert!(text.contains("https://u:p@host/x"), "{text}");
    assert!(!text.contains("<hidden>"), "{text}");
}

#[test]
fn env_style_pairs_hide_secret_named_values_in_custom_yaml() {
    let text = custom_text(json!({
        "apiVersion": "example.io/v1",
        "kind": "Widget",
        "metadata": {"name": "w"},
        "spec": {
            "extraEnv": [
                {"name": "DB_PASSWORD", "value": "distinctive-1"},
                {"name": "LOG_LEVEL", "value": "debug"},
                {"name": "PIN_TOKEN", "value": 4242},
            ],
            "param": {"name": "clientSecret", "value": "distinctive-2"},
        },
    }));
    assert!(!text.contains("distinctive"), "{text}");
    assert!(!text.contains("4242"), "{text}");
    assert!(text.contains("value: debug"), "{text}");
    assert!(
        text.starts_with("# k8sBoard hid 3 values as <hidden>."),
        "{text}"
    );
}

/// Fixtures of the 0007 output. Each expected text was captured before `mask_to_yaml` was split
/// into `mask_object` and `to_yaml_text` (spec 0031 step 0), so a refactor cannot change a byte.
fn golden_fixtures() -> Vec<(&'static str, String)> {
    let deployment = json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "name": "api",
            "namespace": "shop",
            "managedFields": [{"manager": "kubectl"}],
            "annotations": {
                "kubectl.kubernetes.io/last-applied-configuration": "{\"spec\":1}",
                "team": "payments",
            },
        },
        "spec": {
            "replicas": 3,
            "template": {"spec": {"containers": [{
                "name": "api",
                "image": "api:1.2.3",
                "env": [
                    {"name": "DB_PASSWORD", "value": "s3cr3t-env"},
                    {"name": "NODE", "valueFrom": {"fieldRef": {"fieldPath": "spec.nodeName"}}},
                ],
            }]}},
        },
    });
    let secret = json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": {"name": "db", "namespace": "shop"},
        "type": "Opaque",
        "data": {"password": "c2VjcmV0", "user": "YWRtaW4="},
        "stringData": {"token": "plain-token"},
    });
    let storage_class = json!({
        "apiVersion": "storage.k8s.io/v1",
        "kind": "StorageClass",
        "metadata": {"name": "fast"},
        "provisioner": "example.com/fs",
        "parameters": {"adminPassword": "pw-1", "type": "ssd", "secretName": "creds"},
        "mountOptions": ["password=hunter2", "ro"],
    });
    let config_map = json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": {"name": "settings", "namespace": "shop", "labels": {"b": "2", "a": "1"}},
        "data": {
            "yes": "yes",
            "float": "1.0",
            "mode": "0755",
            "nothing": "null",
            "tilde": "~",
            "exp": "1e3",
            "script": "line one\nline two\n",
            "strip": "no newline",
        },
    });
    let custom = json!({
        "apiVersion": "example.com/v1",
        "kind": "Widget",
        "metadata": {"name": "w", "namespace": "shop"},
        "spec": {"apiToken": "tok-1", "url": "https://user:pw@host/path", "size": 3},
        "status": {"phase": "Ready"},
    });
    vec![
        (
            "deployment_hidden",
            masked(deployment.clone(), EnvValues::Hidden).text,
        ),
        (
            "deployment_shown",
            masked(deployment, EnvValues::Shown).text,
        ),
        ("secret", masked_text(secret)),
        ("storage_class", masked_text(storage_class)),
        ("config_map", masked_text(config_map)),
        (
            "custom",
            to_masked_custom_yaml(custom, EnvValues::Hidden)
                .expect("fixture serializes")
                .text,
        ),
    ]
}

#[test]
fn masked_yaml_output_is_unchanged() {
    let expected = golden_texts();
    let actual = golden_fixtures();
    assert_eq!(actual.len(), expected.len());
    for ((name, text), (expected_name, expected_text)) in actual.iter().zip(expected) {
        assert_eq!(*name, expected_name);
        assert_eq!(text, expected_text, "{name}");
    }
}

fn golden_texts() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "deployment_hidden",
            r#"# k8sBoard hid 2 values as <hidden>.
apiVersion: apps/v1
kind: Deployment
metadata:
  annotations:
    kubectl.kubernetes.io/last-applied-configuration: <hidden>
    team: payments
  name: api
  namespace: shop
spec:
  replicas: 3
  template:
    spec:
      containers:
      - env:
        - name: DB_PASSWORD
          value: <hidden>
        - name: NODE
          valueFrom:
            fieldRef:
              fieldPath: spec.nodeName
        image: api:1.2.3
        name: api
"#,
        ),
        (
            "deployment_shown",
            r#"# k8sBoard hid 1 value as <hidden>.
apiVersion: apps/v1
kind: Deployment
metadata:
  annotations:
    kubectl.kubernetes.io/last-applied-configuration: <hidden>
    team: payments
  name: api
  namespace: shop
spec:
  replicas: 3
  template:
    spec:
      containers:
      - env:
        - name: DB_PASSWORD
          value: s3cr3t-env
        - name: NODE
          valueFrom:
            fieldRef:
              fieldPath: spec.nodeName
        image: api:1.2.3
        name: api
"#,
        ),
        (
            "secret",
            r#"# k8sBoard hid 3 values as <hidden>.
apiVersion: v1
data:
  password: <hidden>
  user: <hidden>
kind: Secret
metadata:
  name: db
  namespace: shop
stringData:
  token: <hidden>
type: Opaque
"#,
        ),
        (
            "storage_class",
            r#"# k8sBoard hid 2 values as <hidden>.
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: fast
mountOptions:
- password=<hidden>
- ro
parameters:
  adminPassword: <hidden>
  secretName: creds
  type: ssd
provisioner: example.com/fs
"#,
        ),
        (
            "config_map",
            r#"apiVersion: v1
data:
  exp: "1e3"
  float: "1.0"
  mode: "0755"
  nothing: "null"
  script: |
    line one
    line two
  strip: no newline
  tilde: "~"
  "yes": "yes"
kind: ConfigMap
metadata:
  labels:
    a: "1"
    b: "2"
  name: settings
  namespace: shop
"#,
        ),
        (
            "custom",
            r#"# k8sBoard hid 2 values as <hidden>.
apiVersion: example.com/v1
kind: Widget
metadata:
  name: w
  namespace: shop
spec:
  apiToken: <hidden>
  size: 3
  url: https://<hidden>@host/path
status:
  phase: Ready
"#,
        ),
    ]
}

#[test]
fn all_lists_every_kind_in_declaration_order() {
    assert_eq!(ObjectKind::ALL.len(), ALL_KINDS.len() + 1);
    assert_eq!(&ObjectKind::ALL[..ALL_KINDS.len()], &ALL_KINDS);
    assert_eq!(
        ObjectKind::ALL.last(),
        Some(&ObjectKind::CustomResourceDefinition)
    );
}

#[test]
fn object_kind_resource_matches_the_api_resource() {
    for kind in ObjectKind::ALL {
        let (group, plural) = kind.resource();
        let resource = api_resource(kind);
        assert_eq!(resource.group, group, "{}", kind.name());
        assert_eq!(resource.plural, plural, "{}", kind.name());
    }
}

#[test]
fn only_the_editable_kinds_offer_edit_yaml() {
    let editable: Vec<&str> = ObjectKind::ALL
        .into_iter()
        .filter(|kind| kind.is_editable())
        .map(ObjectKind::name)
        .collect();
    assert_eq!(
        editable,
        [
            "Pod",
            "Deployment",
            "StatefulSet",
            "DaemonSet",
            "CronJob",
            "Service",
            "Ingress",
            "ConfigMap",
            "NetworkPolicy",
            "HorizontalPodAutoscaler",
            "ResourceQuota",
            "PodDisruptionBudget",
            "Secret",
            "Role",
            "ClusterRole",
            "RoleBinding",
            "ClusterRoleBinding",
        ]
    );
}

#[test]
fn mask_object_counts_what_it_hides_and_sorts_keys() {
    let mut object = pod_with_env();
    let count = mask_object(&mut object, EnvValues::Hidden, |_| 0);
    assert_eq!((count.hidden, count.hidden_env_values), (1, 1));
    let mut shown = pod_with_env();
    let count = mask_object(&mut shown, EnvValues::Shown, |_| 0);
    assert_eq!((count.hidden, count.hidden_env_values), (0, 0));
    let keys: Vec<&String> = object.as_object().expect("an object").keys().collect();
    assert_eq!(keys, ["apiVersion", "kind", "metadata", "spec"]);
}

#[test]
fn to_yaml_text_adds_only_the_given_header() {
    let value = json!({"a": 1});
    assert_eq!(to_yaml_text(&value, None).expect("serializes"), "a: 1\n");
    assert_eq!(
        to_yaml_text(&value, Some("# header")).expect("serializes"),
        "# header\na: 1\n"
    );
}

// ---- Pod template of a ReplicaSet (spec 0039) ----

/// A ReplicaSet whose template has a hash label, a null creation time, an env literal, and the
/// given annotations on the template.
fn replica_set_json(template_annotations: Value) -> Value {
    json!({
        "apiVersion": "apps/v1",
        "kind": "ReplicaSet",
        "metadata": {
            "name": "api-7d9f8c",
            "namespace": "shop",
            "annotations": {
                "kubectl.kubernetes.io/last-applied-configuration": "{\"secret\":\"top\"}"
            },
        },
        "spec": {
            "replicas": 2,
            "template": {
                "metadata": {
                    "creationTimestamp": null,
                    "labels": {"app": "api", "pod-template-hash": "7d9f8c"},
                    "annotations": template_annotations,
                },
                "spec": {"containers": [{
                    "name": "api",
                    "image": "api:2",
                    "env": [{"name": "DB_PASSWORD", "value": "hunter2-literal"}],
                }]},
            },
        },
    })
}

fn template_of(object: Value, env: EnvValues) -> ObjectYaml {
    pod_template_text(object, env).expect("the fixture has a template")
}

#[test]
fn pod_template_text_drops_the_hash_label() {
    let yaml = template_of(replica_set_json(json!({})), EnvValues::Hidden);
    assert!(yaml.text.contains("app: api"), "{}", yaml.text);
    assert!(!yaml.text.contains("pod-template-hash"), "{}", yaml.text);
    assert!(!yaml.text.contains("creationTimestamp"), "{}", yaml.text);
    // Only the template is kept: nothing of the ReplicaSet around it.
    assert!(!yaml.text.contains("replicas"), "{}", yaml.text);
    assert!(!yaml.text.contains("kind: ReplicaSet"), "{}", yaml.text);
}

#[test]
fn pod_template_text_masks_env_by_default() {
    let yaml = template_of(replica_set_json(json!({})), EnvValues::Hidden);
    assert!(!yaml.text.contains("hunter2-literal"), "{}", yaml.text);
    assert!(yaml.text.contains(HIDDEN), "{}", yaml.text);
    assert_eq!(yaml.hidden_env_values, 1);
    // No hidden-count header: the diff dialog shows the toggle instead.
    assert!(!yaml.text.starts_with('#'), "{}", yaml.text);
}

#[test]
fn pod_template_text_shows_env_when_asked() {
    let yaml = template_of(replica_set_json(json!({})), EnvValues::Shown);
    assert!(yaml.text.contains("hunter2-literal"), "{}", yaml.text);
    assert_eq!(yaml.hidden_env_values, 0);
}

#[test]
fn pod_template_text_masks_secret_annotations() {
    let yaml = template_of(
        replica_set_json(json!({"kapp.k14s.io/original": "{\"password\":\"p4ss\"}"})),
        EnvValues::Shown,
    );
    assert!(!yaml.text.contains("p4ss"), "{}", yaml.text);
    assert!(yaml.text.contains("kapp.k14s.io/original"), "{}", yaml.text);
    assert!(!yaml.text.contains("secret"), "{}", yaml.text);
}

#[test]
fn pod_template_text_masks_secret_annotations_under_template_metadata() {
    let yaml = template_of(
        replica_set_json(json!({
            "kubectl.kubernetes.io/last-applied-configuration": "{\"token\":\"t0k3n\"}",
            "team": "shop",
        })),
        EnvValues::Hidden,
    );
    assert!(!yaml.text.contains("t0k3n"), "{}", yaml.text);
    assert!(yaml.text.contains("team: shop"), "{}", yaml.text);
}

#[test]
fn pod_template_text_without_template_is_err() {
    let object = json!({"kind": "ReplicaSet", "metadata": {"name": "x"}, "spec": {"replicas": 1}});
    assert!(pod_template_text(object, EnvValues::Hidden).is_err());
}

#[tokio::test]
async fn pod_template_yaml_gets_the_replica_set() {
    let body = replica_set_json(json!({})).to_string();
    let (connection, api) = crate::fake_api::FakeApi::connection(
        crate::object_write::WritePolicy::Blocked,
        move |_| (200, body.clone()),
    );
    let replica_set = ObjectRef::new(
        ObjectKind::ReplicaSet,
        Some("shop".to_owned()),
        "api-7d9f8c".to_owned(),
    )
    .expect("a namespaced kind");
    let yaml = connection
        .pod_template_yaml(&replica_set, EnvValues::Hidden)
        .await
        .expect("the fake answers");
    assert!(yaml.text.contains("app: api"), "{}", yaml.text);
    let requests = api.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].path,
        "/apis/apps/v1/namespaces/shop/replicasets/api-7d9f8c"
    );
}

#[tokio::test]
async fn one_get_makes_the_text_of_a_new_job() {
    use crate::fake_api::FakeApi;
    use crate::object_write::WritePolicy;

    let body = serde_json::json!({
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {
            "name": "report-failed-29",
            "namespace": "lab-batch",
            "labels": { "app": "report", "controller-uid": "c" },
            "ownerReferences": [{ "kind": "CronJob", "name": "report-failed", "uid": "u" }],
            "uid": "job-uid", "resourceVersion": "9",
        },
        "spec": {
            "selector": { "matchLabels": { "controller-uid": "c" } },
            "template": {
                "metadata": { "labels": { "app": "report", "controller-uid": "c" } },
                "spec": {
                    "containers": [{ "name": "report", "image": "busybox", "env": [{ "name": "MODE", "value": "visible-literal" }] }],
                    "restartPolicy": "Never",
                },
            },
        },
        "status": { "failed": 1 },
    })
    .to_string();
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, move |_| (200, body.clone()));
    let now = jiff::Timestamp::from_second(1_790_000_000).expect("a timestamp");
    let text = connection
        .job_draft_text("lab-batch", "report-failed-29", now)
        .await
        .expect("a draft");
    assert!(
        text.contains("name: report-failed-manual-1790000000"),
        "{text}"
    );
    // The text is created as written, so a literal stays readable and server fields are gone.
    assert!(text.contains("visible-literal"), "{text}");
    for server_field in [
        "uid: job-uid",
        "resourceVersion",
        "status:",
        "ownerReferences",
        "selector",
    ] {
        assert!(!text.contains(server_field), "{server_field} in {text}");
    }
    assert!(!text.contains("controller-uid"), "{text}");
    let requests = api.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].path,
        "/apis/batch/v1/namespaces/lab-batch/jobs/report-failed-29"
    );
}
