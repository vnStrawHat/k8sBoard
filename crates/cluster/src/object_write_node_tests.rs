//! Tests of the 0034 node maintenance operations over the fake transport: the eviction, the taint
//! patch, the label patch, and the 429 shapes of the eviction handler.

use serde_json::{Value, json};

use super::*;
use crate::fake_api::{FakeApi, RecordedRequest};

const EVICTION_PATH: &str = "/api/v1/namespaces/payments/pods/api-1/eviction";
const PDB_MESSAGE: &str = "Cannot evict pod as it would violate the pod's disruption budget.";

fn pod() -> ObjectRef {
    ObjectRef::new(
        ObjectKind::Pod,
        Some("payments".to_owned()),
        "api-1".to_owned(),
    )
    .expect("a pod has a namespace")
}

fn node() -> ObjectRef {
    ObjectRef::new(ObjectKind::Node, None, "wk-04".to_owned()).expect("a node has no namespace")
}

fn evict(grace: GracePeriod) -> WriteRequest {
    WriteRequest::new(
        pod(),
        WriteOperation::EvictPod {
            uid: "u-1".to_owned(),
            grace,
        },
    )
    .expect("a pod fits an eviction")
}

fn taint(key: &str, value: Option<&str>, effect: &str) -> NodeTaint {
    NodeTaint {
        key: key.to_owned(),
        value: value.map(str::to_owned),
        effect: effect.to_owned(),
        time_added: None,
    }
}

fn set_taints_from(previous: Vec<NodeTaint>, taints: Vec<NodeTaint>) -> WriteRequest {
    WriteRequest::new(
        node(),
        WriteOperation::SetNodeTaints {
            taints,
            resource_version: "42".to_owned(),
            previous,
        },
    )
    .expect("a node fits a taint edit")
}

fn set_taints(taints: Vec<NodeTaint>) -> WriteRequest {
    set_taints_from(Vec::new(), taints)
}

fn set_labels(changes: Vec<LabelChange>) -> WriteRequest {
    WriteRequest::new(node(), WriteOperation::SetNodeLabels { changes })
        .expect("a node fits a label edit")
}

fn body_of(request: &RecordedRequest) -> Value {
    serde_json::from_str(&request.body).expect("the body is JSON")
}

fn success_status() -> String {
    json!({"kind": "Status", "apiVersion": "v1", "status": "Success", "code": 201}).to_string()
}

fn answering(code: u16, body: String) -> (ClusterConnection, FakeApi) {
    FakeApi::connection(WritePolicy::Allowed, move |_| (code, body.clone()))
}

fn pdb_refusal(cause: &str, retry_after_seconds: Option<u32>) -> String {
    let mut details = json!({"causes": [{"reason": "DisruptionBudget", "message": cause}]});
    if let Some(seconds) = retry_after_seconds {
        details["retryAfterSeconds"] = json!(seconds);
    }
    json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure",
        "message": PDB_MESSAGE, "reason": "TooManyRequests",
        "details": details, "code": 429,
    })
    .to_string()
}

async fn evict_error(mode: WriteMode, code: u16, body: String) -> WriteError {
    let (connection, _api) = answering(code, body);
    connection
        .write(&evict(GracePeriod::Seconds(30)), mode)
        .await
        .expect_err("the server refused")
}

#[tokio::test]
async fn evict_sends_a_policy_v1_eviction_with_uid() {
    let (connection, api) = answering(201, success_status());
    connection
        .write(&evict(GracePeriod::Seconds(30)), WriteMode::Commit)
        .await
        .expect("the eviction is accepted");
    let requests = api.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, EVICTION_PATH);
    assert_eq!(
        requests[0].content_type.as_deref(),
        Some("application/json")
    );
    assert!(requests[0].has_query("fieldManager", "k8sboard"));
    assert_eq!(
        body_of(&requests[0]),
        json!({
            "apiVersion": "policy/v1",
            "kind": "Eviction",
            "metadata": {"name": "api-1", "namespace": "payments"},
            "deleteOptions": {"preconditions": {"uid": "u-1"}, "gracePeriodSeconds": 30},
        })
    );
}

#[tokio::test]
async fn evict_omits_grace_for_pod_default() {
    let (connection, api) = answering(201, success_status());
    connection
        .write(&evict(GracePeriod::PodDefault), WriteMode::Commit)
        .await
        .expect("the eviction is accepted");
    let body = body_of(&api.requests()[0]);
    assert!(body["deleteOptions"].get("gracePeriodSeconds").is_none());
}

#[tokio::test]
async fn evict_dry_run_sends_dry_run_all() {
    let (connection, api) = answering(201, success_status());
    connection
        .write(&evict(GracePeriod::Seconds(30)), WriteMode::DryRun)
        .await
        .expect("the dry-run is accepted");
    let request = &api.requests()[0];
    assert!(request.has_query("dryRun", "All"), "{}", request.query);
    assert!(request.has_query("fieldManager", "k8sboard"));
}

#[tokio::test]
async fn evict_success_reports_created() {
    let (connection, _api) = answering(201, success_status());
    let outcome = connection
        .write(&evict(GracePeriod::PodDefault), WriteMode::Commit)
        .await
        .expect("the eviction is accepted");
    assert_eq!(outcome.effect, WriteEffect::Created);
    assert_eq!(outcome.created_name, None);
    assert_eq!(outcome.mode, WriteMode::Commit);
}

#[tokio::test]
async fn evict_429_is_too_many_requests() {
    let cause = "The disruption budget api-pdb needs 2 healthy pods and has 2 currently";
    let error = evict_error(WriteMode::Commit, 429, pdb_refusal(cause, None)).await;
    match error {
        WriteError::TooManyRequests {
            message,
            retry_after,
        } => {
            assert_eq!(message, cause);
            assert_eq!(retry_after, None);
        }
        other => panic!("expected TooManyRequests, got {other:?}"),
    }
}

#[tokio::test]
async fn evict_429_still_processing_carries_ten_seconds() {
    let cause = "The disruption budget api-pdb is still being processed by the server.";
    let error = evict_error(WriteMode::Commit, 429, pdb_refusal(cause, Some(10))).await;
    match error {
        WriteError::TooManyRequests {
            message,
            retry_after,
        } => {
            assert_eq!(message, cause);
            assert_eq!(retry_after, Some(Duration::from_secs(10)));
        }
        other => panic!("expected TooManyRequests, got {other:?}"),
    }
}

#[tokio::test]
async fn a_429_without_causes_falls_back_to_the_status_message() {
    let body = json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure",
        "message": PDB_MESSAGE, "reason": "TooManyRequests", "code": 429,
    })
    .to_string();
    let error = evict_error(WriteMode::Commit, 429, body).await;
    assert!(
        matches!(&error, WriteError::TooManyRequests { message, retry_after: None } if message == PDB_MESSAGE),
        "{error:?}"
    );
}

#[tokio::test]
async fn evict_429_on_dry_run_is_too_many_requests() {
    let error = evict_error(WriteMode::DryRun, 429, pdb_refusal("needs 2 healthy", None)).await;
    assert!(
        matches!(error, WriteError::TooManyRequests { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn too_many_requests_message_is_the_first_cause_for_every_operation() {
    let (connection, _api) = answering(429, pdb_refusal("first cause", None));
    let request = WriteRequest::new(
        node(),
        WriteOperation::SetNodeSchedulable { schedulable: false },
    )
    .expect("a node fits");
    let error = connection
        .write(&request, WriteMode::Commit)
        .await
        .expect_err("refused");
    assert!(
        matches!(&error, WriteError::TooManyRequests { message, .. } if message == "first cause"),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_commit_429_is_never_outcome_unknown() {
    let error = evict_error(WriteMode::Commit, 429, pdb_refusal("needs 2 healthy", None)).await;
    assert!(!matches!(error, WriteError::OutcomeUnknown), "{error:?}");
}

#[tokio::test]
async fn evict_uid_conflict_is_conflict() {
    let body = json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure",
        "message": "Precondition failed: UID in precondition: u-1, UID in object meta: u-2",
        "reason": "Conflict", "code": 409,
    })
    .to_string();
    let error = evict_error(WriteMode::Commit, 409, body).await;
    assert!(matches!(error, WriteError::Conflict { .. }), "{error:?}");
}

#[tokio::test]
async fn evict_missing_pod_is_not_found() {
    let body = json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure",
        "message": "pods \"api-1\" not found", "reason": "NotFound", "code": 404,
    })
    .to_string();
    let error = evict_error(WriteMode::Commit, 404, body).await;
    assert!(matches!(error, WriteError::NotFound), "{error:?}");
}

#[tokio::test]
async fn evict_201_with_failure_status_is_an_error() {
    let message = "This pod has more than one PodDisruptionBudget, which the eviction subresource does not support.";
    let body = json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure",
        "message": message, "reason": "InternalError", "code": 500,
    })
    .to_string();
    let error = evict_error(WriteMode::Commit, 201, body).await;
    assert!(
        matches!(&error, WriteError::Cluster(cluster) if cluster.to_string().contains("more than one PodDisruptionBudget")),
        "{error:?}"
    );
}

#[tokio::test]
async fn evict_201_with_a_429_failure_status_is_too_many_requests() {
    let error = evict_error(
        WriteMode::Commit,
        201,
        pdb_refusal("needs 2 healthy", Some(10)),
    )
    .await;
    assert!(
        matches!(
            &error,
            WriteError::TooManyRequests {
                retry_after: Some(_),
                ..
            }
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn evict_201_with_a_codeless_failure_is_an_unknown_outcome_on_commit() {
    let body = json!({"kind": "Status", "apiVersion": "v1", "status": "Failure"}).to_string();
    // The server said 2xx but not whether it evicted: a repeat is safe (the uid precondition).
    let error = evict_error(WriteMode::Commit, 201, body.clone()).await;
    assert!(matches!(error, WriteError::OutcomeUnknown), "{error:?}");
    // A dry-run changes nothing, so it is an unusable answer.
    let error = evict_error(WriteMode::DryRun, 201, body).await;
    assert!(matches!(error, WriteError::Cluster(_)), "{error:?}");
}

#[tokio::test]
async fn evict_denied_is_denied() {
    let body = json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure",
        "message": "pods/eviction \"api-1\" is forbidden: User \"readonly\" cannot create resource \"pods/eviction\"",
        "reason": "Forbidden", "code": 403,
    })
    .to_string();
    let error = evict_error(WriteMode::Commit, 403, body).await;
    assert!(matches!(error, WriteError::Denied { .. }), "{error:?}");
}

#[tokio::test]
async fn blocked_policy_blocks_evictions() {
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, |_| (201, success_status()));
    let error = connection
        .write(&evict(GracePeriod::PodDefault), WriteMode::Commit)
        .await
        .expect_err("writes are blocked");
    assert!(matches!(error, WriteError::WritesBlocked), "{error:?}");
    assert!(api.requests().is_empty());
}

#[test]
fn an_eviction_needs_a_pod_and_a_uid() {
    let operation = |uid: &str| WriteOperation::EvictPod {
        uid: uid.to_owned(),
        grace: GracePeriod::PodDefault,
    };
    assert!(WriteRequest::new(node(), operation("u-1")).is_none());
    assert!(WriteRequest::new(pod(), operation("")).is_none());
    assert!(WriteRequest::new(pod(), operation("u/../1")).is_none());
    let request = WriteRequest::new(pod(), operation("u-1")).expect("fits");
    assert_eq!(request.access_check(), AccessCheck::CreatePodEviction);
}

#[tokio::test]
async fn taints_send_the_full_list_with_resource_version() {
    let (connection, api) = answering(
        200,
        r#"{"apiVersion":"v1","kind":"Node","metadata":{"name":"wk-04"}}"#.to_owned(),
    );
    let mut unreachable = taint("node.kubernetes.io/unreachable", None, "NoExecute");
    unreachable.time_added = Some("2026-10-02T08:00:00Z".parse().expect("a timestamp"));
    let request = set_taints(vec![
        taint("dedicated", Some("ingress"), "NoSchedule"),
        unreachable,
    ]);
    connection
        .write(&request, WriteMode::Commit)
        .await
        .expect("the patch is accepted");
    let sent = &api.requests()[0];
    assert_eq!(sent.method, "PATCH");
    assert_eq!(sent.path, "/api/v1/nodes/wk-04");
    assert_eq!(
        sent.content_type.as_deref(),
        Some("application/merge-patch+json")
    );
    assert!(sent.has_query("fieldManager", "k8sboard"));
    assert!(!sent.has_query_key("dryRun"));
    assert_eq!(
        body_of(sent),
        json!({
            "metadata": {"resourceVersion": "42"},
            "spec": {"taints": [
                {"key": "dedicated", "value": "ingress", "effect": "NoSchedule"},
                {
                    "key": "node.kubernetes.io/unreachable",
                    "effect": "NoExecute",
                    "timeAdded": "2026-10-02T08:00:00Z",
                },
            ]},
        })
    );
}

#[tokio::test]
async fn no_taints_send_an_empty_list() {
    let (connection, api) = answering(
        200,
        r#"{"apiVersion":"v1","kind":"Node","metadata":{"name":"wk-04"}}"#.to_owned(),
    );
    connection
        .write(&set_taints(Vec::new()), WriteMode::DryRun)
        .await
        .expect("the dry-run is accepted");
    let sent = &api.requests()[0];
    assert!(sent.has_query("dryRun", "All"));
    assert_eq!(body_of(sent)["spec"]["taints"], json!([]));
}

#[tokio::test]
async fn a_taint_edit_on_a_changed_node_is_a_conflict() {
    let body = json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure",
        "message": "Operation cannot be fulfilled on nodes \"wk-04\": the object has been modified",
        "reason": "Conflict", "code": 409,
    })
    .to_string();
    let (connection, _api) = answering(409, body);
    let error = connection
        .write(&set_taints(Vec::new()), WriteMode::Commit)
        .await
        .expect_err("the node changed");
    assert!(matches!(error, WriteError::Conflict { .. }), "{error:?}");
}

#[tokio::test]
async fn labels_set_and_remove_keys() {
    let (connection, api) = answering(
        200,
        r#"{"apiVersion":"v1","kind":"Node","metadata":{"name":"wk-04"}}"#.to_owned(),
    );
    let changes = vec![
        LabelChange {
            key: "team".to_owned(),
            value: Some("infra".to_owned()),
        },
        LabelChange {
            key: "old".to_owned(),
            value: None,
        },
    ];
    connection
        .write(&set_labels(changes), WriteMode::Commit)
        .await
        .expect("the patch is accepted");
    let sent = &api.requests()[0];
    assert_eq!(sent.method, "PATCH");
    assert_eq!(
        sent.content_type.as_deref(),
        Some("application/merge-patch+json")
    );
    assert_eq!(
        body_of(sent),
        json!({"metadata": {"labels": {"team": "infra", "old": null}}})
    );
}

#[test]
fn invalid_node_edits_are_not_requests() {
    let taints = |taints: Vec<NodeTaint>, version: &str| {
        WriteRequest::new(
            node(),
            WriteOperation::SetNodeTaints {
                taints,
                resource_version: version.to_owned(),
                previous: Vec::new(),
            },
        )
    };
    assert!(taints(vec![taint("a", None, "NoSchedule")], "1").is_some());
    assert!(taints(vec![taint("a", None, "NoSchedule")], "").is_none());
    assert!(taints(vec![taint("a b", None, "NoSchedule")], "1").is_none());
    assert!(taints(vec![taint("a", None, "Bogus")], "1").is_none());
    let label = |key: &str, value: &str| {
        WriteRequest::new(
            node(),
            WriteOperation::SetNodeLabels {
                changes: vec![LabelChange {
                    key: key.to_owned(),
                    value: Some(value.to_owned()),
                }],
            },
        )
    };
    assert!(label("team", "infra").is_some());
    assert!(label("te am", "infra").is_none());
    assert!(label("team", "in fra").is_none());
    assert!(
        WriteRequest::new(
            node(),
            WriteOperation::SetNodeLabels {
                changes: Vec::new()
            }
        )
        .is_none()
    );
    // A node edit does not fit a pod.
    assert!(
        WriteRequest::new(
            pod(),
            WriteOperation::SetNodeLabels {
                changes: Vec::new()
            }
        )
        .is_none()
    );
}

#[test]
fn changed_fields_describe_the_edits() {
    let value_of = |request: &WriteRequest| {
        let fields = request.changed_fields();
        let [field] = fields.as_slice() else {
            panic!("one field expected, got {fields:?}");
        };
        (
            field.path.to_string(),
            field.value.clone().unwrap_or_default(),
        )
    };
    assert_eq!(
        value_of(&evict(GracePeriod::Seconds(30))),
        ("pods/eviction".to_owned(), "grace 30s".to_owned())
    );
    assert_eq!(
        value_of(&evict(GracePeriod::PodDefault)),
        ("pods/eviction".to_owned(), "grace pod default".to_owned())
    );
    let changes = vec![
        LabelChange {
            key: "team".to_owned(),
            value: Some("infra".to_owned()),
        },
        LabelChange {
            key: "old-key".to_owned(),
            value: None,
        },
    ];
    assert_eq!(
        value_of(&set_labels(changes)),
        (
            "metadata.labels".to_owned(),
            "team=infra; -old-key".to_owned()
        )
    );
}

#[test]
fn every_new_operation_supports_dry_run_and_needs_its_check() {
    assert!(evict(GracePeriod::PodDefault).supports_dry_run());
    assert!(set_taints(Vec::new()).supports_dry_run());
    assert_eq!(
        set_taints(Vec::new()).access_check(),
        AccessCheck::PatchNodes
    );
    assert_eq!(
        set_labels(vec![LabelChange {
            key: "a".to_owned(),
            value: None
        }])
        .access_check(),
        AccessCheck::PatchNodes
    );
}

#[test]
fn debug_shows_names_only() {
    let request = set_taints(vec![taint(
        "dedicated",
        Some("secret-ingress"),
        "NoSchedule",
    )]);
    let text = format!("{request:?}");
    assert!(text.contains("SetNodeTaints"), "{text}");
    assert!(
        !text.contains("secret-ingress") && !text.contains("dedicated"),
        "{text}"
    );
    let text = format!("{:?}", evict(GracePeriod::Seconds(77)));
    assert!(!text.contains("u-1") && !text.contains("77"), "{text}");
}

fn taint_lines(previous: Vec<NodeTaint>, taints: Vec<NodeTaint>) -> Vec<(String, Option<String>)> {
    set_taints_from(previous, taints)
        .changed_fields()
        .into_iter()
        .map(|field| (field.path.into_owned(), field.value))
        .collect()
}

#[test]
fn a_taint_edit_lists_each_taint_that_changed() {
    let previous = vec![
        taint("conflict", Some("4"), "NoSchedule"),
        taint("workload", Some("data"), "NoSchedule"),
        taint("kept", None, "NoExecute"),
    ];
    let taints = vec![
        taint("conflict", Some("3"), "NoSchedule"),
        taint("kept", None, "NoExecute"),
        taint("maintenance", Some("true"), "NoSchedule"),
    ];
    assert_eq!(
        taint_lines(previous, taints),
        [
            ("conflict 4 → 3".to_owned(), None),
            ("− workload=data:NoSchedule".to_owned(), None),
            ("+ maintenance=true:NoSchedule".to_owned(), None),
        ]
    );
}

#[test]
fn a_taint_without_a_value_reads_as_no_value() {
    let lines = taint_lines(
        vec![taint("spot", None, "PreferNoSchedule")],
        vec![taint("spot", Some("x"), "PreferNoSchedule")],
    );
    assert_eq!(lines, [("spot (no value) → x".to_owned(), None)]);
}

#[test]
fn the_same_key_with_another_effect_is_another_taint() {
    let lines = taint_lines(
        vec![taint("gpu", None, "NoSchedule")],
        vec![taint("gpu", None, "NoExecute")],
    );
    assert_eq!(
        lines,
        [
            ("− gpu:NoSchedule".to_owned(), None),
            ("+ gpu:NoExecute".to_owned(), None),
        ]
    );
}

#[test]
fn an_edit_that_changes_no_taint_says_so() {
    let same = vec![taint("gpu", None, "NoSchedule")];
    assert_eq!(
        taint_lines(same.clone(), same),
        [("spec.taints".to_owned(), Some("unchanged".to_owned()))]
    );
}
