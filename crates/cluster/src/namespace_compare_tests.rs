use serde_json::json;

use super::*;
use crate::fake_api::FakeApi;
use crate::object_write::WritePolicy;

fn deployment(namespace: &str, replicas: u32, image: &str) -> Value {
    json!({
        "metadata": {
            "name": "web", "namespace": namespace, "uid": "u", "resourceVersion": "9",
            "annotations": {"deployment.kubernetes.io/revision": "3"},
        },
        "spec": {
            "replicas": replicas,
            "template": {"spec": {"containers": [{
                "name": "web", "image": image,
                "env": [{"name": "LOG_LEVEL", "value": "info"}],
                "resources": {"requests": {"cpu": "100m"}},
            }]}},
        },
        "status": {"readyReplicas": replicas},
    })
}

fn secret(namespace: &str, password: &str) -> Value {
    json!({
        "metadata": {"name": "web-secret", "namespace": namespace},
        "type": "Opaque",
        "data": {"password": password, "user": "dXNlcg=="},
    })
}

fn reduced(kind: ObjectKind, object: Value, env: EnvValues, salt: &RandomState) -> Value {
    reduce_object(kind, object, env, salt)
        .expect("a named object")
        .1
}

fn one(kind: ObjectKind, name: &str, object: Value) -> KindObjects {
    let salt = RandomState::new();
    let reduced = reduced(kind, object, EnvValues::Shown, &salt);
    KindObjects::from([(name.to_owned(), reduced)])
}

#[test]
fn the_same_object_in_two_namespaces_is_equal() {
    let salt = RandomState::new();
    let left = reduced(
        ObjectKind::Deployment,
        deployment("lab-shop", 1, "web:1"),
        EnvValues::Shown,
        &salt,
    );
    let right = reduced(
        ObjectKind::Deployment,
        deployment("lab-shop-stg", 1, "web:1"),
        EnvValues::Shown,
        &salt,
    );
    assert_eq!(left, right, "namespace, uid, status and revision are noise");
    assert!(left.pointer("/metadata/annotations").is_none());
    assert!(left.pointer("/status").is_none());
}

#[test]
fn a_changed_replica_count_and_image_are_field_changes() {
    let outcome = compare_kind(
        one(
            ObjectKind::Deployment,
            "web",
            deployment("lab-shop", 1, "web:1"),
        ),
        one(
            ObjectKind::Deployment,
            "web",
            deployment("lab-shop-stg", 2, "web:2"),
        ),
    );
    let KindOutcome::Compared { differs, same, .. } = outcome else {
        panic!("a compared kind");
    };
    assert_eq!(same, 0);
    let [difference] = differs.as_slice() else {
        panic!("one object differs");
    };
    let lines: Vec<String> = difference
        .changes
        .iter()
        .map(|change| format!("{} {:?} -> {:?}", change.path, change.old, change.new))
        .collect();
    assert_eq!(
        lines,
        [
            "spec.replicas Some(\"1\") -> Some(\"2\")",
            "spec.template.spec.containers[web].image Some(\"web:1\") -> Some(\"web:2\")",
        ]
    );
    assert!(difference.left_text.contains("image: web:1"));
    assert!(difference.right_text.contains("image: web:2"));
}

#[test]
fn objects_in_one_namespace_only_are_listed_by_name() {
    let mut left = one(
        ObjectKind::ConfigMap,
        "a",
        json!({"metadata": {"name": "a"}}),
    );
    left.extend(one(
        ObjectKind::ConfigMap,
        "both",
        json!({"metadata": {"name": "both"}}),
    ));
    let mut right = one(
        ObjectKind::ConfigMap,
        "both",
        json!({"metadata": {"name": "both"}}),
    );
    right.extend(one(
        ObjectKind::ConfigMap,
        "z",
        json!({"metadata": {"name": "z"}}),
    ));
    let KindOutcome::Compared {
        only_left,
        only_right,
        differs,
        same,
    } = compare_kind(left, right)
    else {
        panic!("a compared kind");
    };
    assert_eq!(only_left, ["a"]);
    assert_eq!(only_right, ["z"]);
    assert!(differs.is_empty());
    assert_eq!(same, 1);
}

#[test]
fn a_secret_difference_never_carries_a_value() {
    let salt = RandomState::new();
    let left = reduced(
        ObjectKind::Secret,
        secret("lab-shop", "c2VjcmV0LW9uZQ=="),
        EnvValues::Hidden,
        &salt,
    );
    let right = reduced(
        ObjectKind::Secret,
        secret("lab-shop-stg", "c2VjcmV0LXR3bw=="),
        EnvValues::Hidden,
        &salt,
    );
    let difference = difference("web-secret".to_owned(), &left, &right);
    let [change] = difference.changes.as_slice() else {
        panic!("only the password differs");
    };
    assert_eq!(change.path.to_string(), "data.password");
    for text in [
        change.old.as_deref().unwrap_or_default(),
        change.new.as_deref().unwrap_or_default(),
        &difference.left_text,
        &difference.right_text,
    ] {
        assert!(!text.contains("c2VjcmV0"), "a value leaked: {text}");
    }
    assert!(
        change
            .old
            .as_deref()
            .is_some_and(|old| old.starts_with("<hash:"))
    );
    assert_ne!(change.old, change.new);
}

#[test]
fn equal_secret_values_give_equal_tokens() {
    let salt = RandomState::new();
    let left = reduced(
        ObjectKind::Secret,
        secret("lab-shop", "c2FtZQ=="),
        EnvValues::Hidden,
        &salt,
    );
    let right = reduced(
        ObjectKind::Secret,
        secret("lab-shop-stg", "c2FtZQ=="),
        EnvValues::Hidden,
        &salt,
    );
    assert_eq!(left, right);
}

#[test]
fn env_literals_are_hidden_until_shown() {
    let salt = RandomState::new();
    let mut staged = deployment("lab-shop-stg", 1, "web:1");
    staged["spec"]["template"]["spec"]["containers"][0]["env"][0]["value"] = json!("debug");
    let hidden = |object: Value| {
        reduce_object(ObjectKind::Deployment, object, EnvValues::Hidden, &salt)
            .expect("a named object")
    };
    let (_, left, hidden_left) = hidden(deployment("lab-shop", 1, "web:1"));
    let (_, right, hidden_right) = hidden(staged.clone());
    assert_eq!(left, right, "a hidden literal is not compared");
    assert_eq!(hidden_left + hidden_right, 2);
    let shown = |object: Value| reduced(ObjectKind::Deployment, object, EnvValues::Shown, &salt);
    let difference = difference(
        "web".to_owned(),
        &shown(deployment("lab-shop", 1, "web:1")),
        &shown(staged),
    );
    let [change] = difference.changes.as_slice() else {
        panic!("only the literal differs");
    };
    assert_eq!(
        change.path.to_string(),
        "spec.template.spec.containers[web].env[LOG_LEVEL].value"
    );
}

#[test]
fn a_service_is_not_different_for_its_cluster_ip() {
    let salt = RandomState::new();
    let service = |namespace: &str, ip: &str| {
        reduced(
            ObjectKind::Service,
            json!({
                "metadata": {"name": "web", "namespace": namespace},
                "spec": {"clusterIP": ip, "clusterIPs": [ip], "ports": [{"port": 80}]},
            }),
            EnvValues::Hidden,
            &salt,
        )
    };
    assert_eq!(service("a", "10.0.0.1"), service("b", "10.0.0.2"));
}

#[test]
fn a_helm_release_record_is_no_object_of_the_namespace() {
    let record = json!({
        "metadata": {"name": "sh.helm.release.v1.web.v1"},
        "type": "helm.sh/release.v1",
        "data": {"release": "abc"},
    });
    assert!(
        reduce_object(
            ObjectKind::Secret,
            record,
            EnvValues::Hidden,
            &RandomState::new()
        )
        .is_none()
    );
}

#[test]
fn counts_add_up_over_the_readable_kinds() {
    let comparison = NamespaceComparison {
        left: "a".to_owned(),
        right: "b".to_owned(),
        hidden_env_values: 0,
        kinds: vec![
            KindComparison {
                kind: ObjectKind::Secret,
                outcome: KindOutcome::Unreadable("forbidden".to_owned()),
            },
            KindComparison {
                kind: ObjectKind::ConfigMap,
                outcome: KindOutcome::Compared {
                    only_left: vec!["x".to_owned()],
                    only_right: vec!["y".to_owned(), "z".to_owned()],
                    differs: Vec::new(),
                    same: 4,
                },
            },
        ],
    };
    assert_eq!(
        comparison.counts(),
        CompareCounts {
            differ: 0,
            only_left: 1,
            only_right: 2,
            same: 4
        }
    );
}

fn list_body(items: &[Value]) -> String {
    json!({"apiVersion": "v1", "kind": "List", "metadata": {}, "items": items}).to_string()
}

#[tokio::test]
async fn compare_namespaces_lists_every_kind_of_both_sides_and_only_reads() {
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, |request| {
        let items: Vec<Value> = match request.path.as_str() {
            "/apis/apps/v1/namespaces/lab-shop/deployments" => {
                vec![deployment("lab-shop", 1, "web:1")]
            }
            "/apis/apps/v1/namespaces/lab-shop-stg/deployments" => {
                vec![deployment("lab-shop-stg", 2, "web:1")]
            }
            _ => Vec::new(),
        };
        (200, list_body(&items))
    });
    let comparison = connection
        .compare_namespaces("lab-shop", "lab-shop-stg", EnvValues::Hidden)
        .await;
    assert_eq!(comparison.kinds.len(), COMPARE_KINDS.len());
    assert_eq!(
        comparison.counts(),
        CompareCounts {
            differ: 1,
            ..CompareCounts::default()
        }
    );
    let requests = api.requests();
    assert_eq!(requests.len(), COMPARE_KINDS.len() * 2, "{requests:?}");
    assert!(requests.iter().all(|request| request.method == "GET"));
}

#[tokio::test]
async fn a_forbidden_kind_is_unreadable_and_the_others_still_compare() {
    let (connection, _) = FakeApi::connection(WritePolicy::Blocked, |request| {
        if request.path.ends_with("/secrets") {
            let status = json!({
                "apiVersion": "v1", "kind": "Status", "status": "Failure",
                "message": "secrets is forbidden", "reason": "Forbidden", "code": 403,
            });
            return (403, status.to_string());
        }
        (200, list_body(&[]))
    });
    let comparison = connection
        .compare_namespaces("a", "b", EnvValues::Hidden)
        .await;
    let secrets = comparison
        .kinds
        .iter()
        .find(|kind| kind.kind == ObjectKind::Secret)
        .expect("Secret is compared");
    assert!(matches!(secrets.outcome, KindOutcome::Unreadable(_)));
    let readable = comparison
        .kinds
        .iter()
        .filter(|kind| matches!(kind.outcome, KindOutcome::Compared { .. }))
        .count();
    assert_eq!(readable, COMPARE_KINDS.len() - 1);
}
