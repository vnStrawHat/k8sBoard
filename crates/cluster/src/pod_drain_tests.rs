use serde_json::{Value, json};

use super::*;
use crate::fake_api::FakeApi;
use crate::object_write::WritePolicy;

fn pod(value: Value) -> Pod {
    serde_json::from_value(value).expect("a pod")
}

fn list_of(pods: &[Value], continue_token: &str) -> String {
    json!({
        "apiVersion": "v1", "kind": "PodList",
        "metadata": {"continue": continue_token},
        "items": pods,
    })
    .to_string()
}

#[test]
fn drain_pod_reads_identity_labels_and_controller() {
    let drain = drain_pod(&pod(json!({
        "metadata": {
            "name": "api-1", "namespace": "payments", "uid": "u-1",
            "labels": {"app": "api", "tier": "web"},
            "ownerReferences": [{
                "apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "api-5d",
                "uid": "o-1", "controller": true,
            }],
        },
        "status": {"phase": "Running"},
    })));
    assert_eq!(drain.namespace, "payments");
    assert_eq!(drain.name, "api-1");
    assert_eq!(drain.uid, "u-1");
    assert_eq!(drain.labels, ["app=api", "tier=web"]);
    let controller = drain.controller.expect("a controller");
    assert_eq!(
        (controller.kind.as_str(), controller.name.as_str()),
        ("ReplicaSet", "api-5d")
    );
    assert!(!drain.is_mirror && !drain.has_empty_dir);
    assert!(!drain.is_finished && !drain.is_pending && !drain.is_terminating);
}

#[test]
fn drain_pod_flags_mirror_empty_dir_and_terminating() {
    let drain = drain_pod(&pod(json!({
        "metadata": {
            "name": "etcd-wk-04", "namespace": "kube-system", "uid": "u-2",
            "annotations": {"kubernetes.io/config.mirror": "abc"},
            "deletionTimestamp": "2026-10-03T08:00:00Z",
        },
        "spec": {"containers": [], "volumes": [{"name": "scratch", "emptyDir": {}}]},
    })));
    assert!(drain.is_mirror);
    assert!(drain.has_empty_dir);
    assert!(drain.is_terminating);
}

#[test]
fn drain_pod_phase_flags() {
    let phase = |phase: &str| {
        drain_pod(&pod(json!({
            "metadata": {"name": "p", "namespace": "n", "uid": "u"},
            "status": {"phase": phase},
        })))
    };
    assert!(phase("Succeeded").is_finished);
    assert!(phase("Failed").is_finished);
    assert!(!phase("Pending").is_finished);
    assert!(phase("Pending").is_pending);
    assert!(!phase("Running").is_pending);
}

#[tokio::test]
async fn drain_pods_use_the_node_field_selector() {
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, |_| (200, list_of(&[], "")));
    connection
        .drain_pods("wk-04")
        .await
        .expect("the list goes through");
    let requests = api.requests();
    let [request] = requests.as_slice() else {
        panic!("expected one list, got {requests:?}");
    };
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/api/v1/pods");
    assert!(
        request.has_query("fieldSelector", "spec.nodeName%3Dwk-04"),
        "{}",
        request.query
    );
    assert!(request.has_query_key("limit"), "{}", request.query);
}

#[tokio::test]
async fn drain_pods_follow_continue_tokens_and_sort() {
    let first = json!({"metadata": {"name": "b", "namespace": "z", "uid": "u-b"}});
    let second = json!({"metadata": {"name": "a", "namespace": "a", "uid": "u-a"}});
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, move |request| {
        if request.has_query_key("continue") {
            (200, list_of(std::slice::from_ref(&second), ""))
        } else {
            (200, list_of(std::slice::from_ref(&first), "next"))
        }
    });
    let pods = connection.drain_pods("wk-04").await.expect("both pages");
    let names: Vec<_> = pods
        .iter()
        .map(|pod| (pod.namespace.as_str(), pod.name.as_str()))
        .collect();
    assert_eq!(names, [("a", "a"), ("z", "b")]);
    assert_eq!(api.requests().len(), 2);
}

#[tokio::test]
async fn drain_pods_refuse_a_node_name_that_could_change_the_selector() {
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, |_| (200, list_of(&[], "")));
    let error = connection
        .drain_pods("wk-04,spec.x=y")
        .await
        .expect_err("an unsafe name");
    assert!(
        matches!(error, ClusterError::UnexpectedResponse { .. }),
        "{error:?}"
    );
    assert!(api.requests().is_empty());
}

#[test]
fn drain_pod_lists_the_claims_it_mounts() {
    let drain = drain_pod(&pod(json!({
        "metadata": {"name": "db-0", "namespace": "shop", "uid": "u-3"},
        "spec": {"containers": [], "volumes": [
            {"name": "scratch", "emptyDir": {}},
            {"name": "data", "persistentVolumeClaim": {"claimName": "data-db-0"}},
        ]},
    })));
    assert_eq!(drain.claims, ["data-db-0"]);
    assert_eq!(drain.pinned_volume, None);
}
