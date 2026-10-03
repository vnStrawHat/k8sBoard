use serde_json::{Value, json};

use super::*;
use crate::fake_api::FakeApi;
use crate::object_write::WritePolicy;

fn pod_json(name: &str, namespace: &str, instance: &str, phase: &str) -> Value {
    json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {
            "name": name, "namespace": namespace, "uid": format!("uid-{name}"),
            "creationTimestamp": "2026-10-03T08:00:00Z",
            "labels": {
                "app.kubernetes.io/managed-by": "k8sboard",
                "k8sboard.io/purpose": "node-shell",
                "k8sboard.io/instance": instance,
            },
        },
        "spec": {"nodeName": "wk-03", "containers": []},
        "status": {"phase": phase},
    })
}

fn list_of(pods: &[Value]) -> String {
    json!({"apiVersion": "v1", "kind": "PodList", "metadata": {}, "items": pods}).to_string()
}

fn connection(pods: Vec<Value>) -> (ClusterConnection, FakeApi) {
    FakeApi::connection(WritePolicy::Allowed, move |_| (200, list_of(&pods)))
}

#[tokio::test]
async fn the_selector_excludes_this_run_and_lists_only_node_shell_pods() {
    let (connection, api) = connection(Vec::new());
    connection
        .node_shell_leftovers(&NamespaceScope::All, "q4m7x2k9pa")
        .await
        .expect("the list goes through");
    let requests = api.requests();
    let [request] = requests.as_slice() else {
        panic!("expected one list, got {requests:?}");
    };
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/api/v1/pods");
    let selector = request
        .query
        .split('&')
        .find_map(|pair| pair.strip_prefix("labelSelector="))
        .expect("a label selector");
    let selector = selector
        .replace("%2C", ",")
        .replace("%21", "!")
        .replace("%3D", "=")
        .replace("%2F", "/");
    assert_eq!(
        selector,
        "app.kubernetes.io/managed-by=k8sboard,k8sboard.io/purpose=node-shell,k8sboard.io/instance!=q4m7x2k9pa"
    );
}

#[tokio::test]
async fn running_and_finished_pods_of_other_runs_are_listed_and_this_runs_are_not() {
    let (connection, _api) = connection(vec![
        pod_json(
            "k8sboard-node-shell-wk-03-aaaaa",
            "kube-system",
            "other-run",
            "Running",
        ),
        pod_json(
            "k8sboard-node-shell-wk-03-bbbbb",
            "kube-system",
            "other-run",
            "Succeeded",
        ),
        pod_json(
            "k8sboard-node-shell-wk-03-ccccc",
            "kube-system",
            "this-run",
            "Running",
        ),
    ]);
    let found = connection
        .node_shell_leftovers(&NamespaceScope::All, "this-run")
        .await
        .expect("the list goes through");
    let names: Vec<_> = found.iter().map(|pod| pod.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "k8sboard-node-shell-wk-03-aaaaa",
            "k8sboard-node-shell-wk-03-bbbbb"
        ]
    );
    assert_eq!(found[0].phase, LeftoverPhase::Running);
    assert!(!found[0].phase.is_finished());
    assert!(found[1].phase.is_finished());
    assert_eq!(found[0].uid, "uid-k8sboard-node-shell-wk-03-aaaaa");
    assert_eq!(found[0].node.as_deref(), Some("wk-03"));
    assert!(found[0].created_at.is_some());
}

#[tokio::test]
async fn nothing_but_a_k8sboard_node_shell_pod_is_ever_offered() {
    // A server that ignores the selector still cannot make the sweep offer another pod.
    let mut unlabeled = pod_json(
        "k8sboard-node-shell-wk-03-ddddd",
        "kube-system",
        "other",
        "Running",
    );
    unlabeled["metadata"]["labels"]["k8sboard.io/purpose"] = json!("something-else");
    let mut foreign_name = pod_json(
        "coredns-5d78c9869d-abcde",
        "kube-system",
        "other",
        "Running",
    );
    foreign_name["metadata"]["labels"]["k8sboard.io/purpose"] = json!("node-shell");
    let mut no_labels = pod_json(
        "k8sboard-node-shell-wk-03-eeeee",
        "kube-system",
        "other",
        "Running",
    );
    no_labels["metadata"]["labels"] = Value::Null;
    let mut no_uid = pod_json(
        "k8sboard-node-shell-wk-03-fffff",
        "kube-system",
        "other",
        "Running",
    );
    no_uid["metadata"]["uid"] = json!("");
    let (connection, _api) = connection(vec![unlabeled, foreign_name, no_labels, no_uid]);
    let found = connection
        .node_shell_leftovers(&NamespaceScope::All, "this-run")
        .await
        .expect("the list goes through");
    assert!(found.is_empty(), "{found:?}");
}

#[tokio::test]
async fn a_scope_lists_each_namespace_and_the_result_is_sorted() {
    let (connection, api) = connection(vec![pod_json(
        "k8sboard-node-shell-wk-03-aaaaa",
        "kube-system",
        "other",
        "Pending",
    )]);
    let scope = NamespaceScope::of_namespaces(["kube-system".to_owned(), "debug".to_owned()]);
    let found = connection
        .node_shell_leftovers(&scope, "this-run")
        .await
        .expect("the list goes through");
    let paths: Vec<_> = api
        .requests()
        .into_iter()
        .map(|request| request.path)
        .collect();
    assert_eq!(
        paths,
        [
            "/api/v1/namespaces/debug/pods",
            "/api/v1/namespaces/kube-system/pods"
        ]
    );
    assert_eq!(
        found.len(),
        2,
        "the fake answers both lists with the same pod"
    );
    assert_eq!(found[0].phase, LeftoverPhase::Pending);
}

#[tokio::test]
async fn a_run_id_that_is_no_label_value_sends_nothing() {
    let (connection, api) = connection(Vec::new());
    let error = connection
        .node_shell_leftovers(&NamespaceScope::All, "a b,c")
        .await
        .expect_err("the id would break the selector");
    assert!(matches!(error, ClusterError::Rendered { .. }), "{error:?}");
    assert!(api.requests().is_empty());
}

#[tokio::test]
async fn a_forbidden_list_is_an_error_not_an_empty_answer() {
    let (connection, _api) = FakeApi::connection(WritePolicy::Allowed, |_| {
        (
            403,
            json!({"kind": "Status", "apiVersion": "v1", "status": "Failure", "code": 403,
                "reason": "Forbidden", "message": "pods is forbidden: User \"x\" cannot list"})
            .to_string(),
        )
    });
    let error = connection
        .node_shell_leftovers(&NamespaceScope::All, "this-run")
        .await
        .expect_err("a denied list");
    assert!(matches!(error, ClusterError::Forbidden { .. }), "{error:?}");
}

#[tokio::test]
async fn a_forbidden_namespace_is_skipped_and_the_others_are_still_listed() {
    let pod = pod_json(
        "k8sboard-node-shell-wk-03-aaaaa",
        "team-a",
        "other",
        "Running",
    );
    let (connection, _api) = FakeApi::connection(WritePolicy::Allowed, move |request| {
        if request.path.contains("/kube-system/") {
            let status = json!({"kind": "Status", "apiVersion": "v1", "status": "Failure",
                "code": 403, "reason": "Forbidden", "message": "pods is forbidden"});
            (403, status.to_string())
        } else {
            (200, list_of(std::slice::from_ref(&pod)))
        }
    });
    let scope = NamespaceScope::of_namespaces(["kube-system".to_owned(), "team-a".to_owned()]);
    let found = connection
        .node_shell_leftovers(&scope, "this-run")
        .await
        .expect("one namespace could be listed");
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].namespace, "team-a");
}

#[test]
fn a_phase_word_maps_to_its_state() {
    assert_eq!(LeftoverPhase::of(Some("Pending")), LeftoverPhase::Pending);
    assert_eq!(LeftoverPhase::of(Some("Failed")), LeftoverPhase::Failed);
    assert_eq!(LeftoverPhase::of(Some("odd")), LeftoverPhase::Unknown);
    assert_eq!(LeftoverPhase::of(None), LeftoverPhase::Unknown);
}
