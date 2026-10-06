use serde_json::{Value, json};

use super::*;
use crate::fake_api::{Failure, FakeApi};

const NODE_JSON: &str = r#"{"apiVersion":"v1","kind":"Node","metadata":{"name":"wk-04"}}"#;
const RBAC_MESSAGE: &str = r#"nodes "wk-04" is forbidden: User "readonly" cannot patch resource "nodes" in API group "" at the cluster scope"#;

fn node(name: &str) -> ObjectRef {
    ObjectRef::new(ObjectKind::Node, None, name.to_owned()).expect("a node has no namespace")
}

fn set_schedulable(schedulable: bool) -> WriteRequest {
    WriteRequest::new(
        node("wk-04"),
        WriteOperation::SetNodeSchedulable { schedulable },
    )
    .expect("a node fits the operation")
}

fn accepting() -> (ClusterConnection, FakeApi) {
    FakeApi::connection(WritePolicy::Allowed, |_| (200, NODE_JSON.to_owned()))
}

/// A connection that answers every request with a `Status` failure.
fn refusing(code: u16, message: &str, causes: &[(&str, &str)]) -> (ClusterConnection, FakeApi) {
    let causes: Vec<Value> = causes
        .iter()
        .map(|(field, cause)| json!({ "reason": "FieldValueInvalid", "message": cause, "field": field }))
        .collect();
    let body = json!({
        "kind": "Status",
        "apiVersion": "v1",
        "status": "Failure",
        "message": message,
        "reason": "Reason",
        "details": { "causes": causes },
        "code": code,
    })
    .to_string();
    FakeApi::connection(WritePolicy::Allowed, move |_| (code, body.clone()))
}

fn body_of(request: &crate::fake_api::RecordedRequest) -> Value {
    serde_json::from_str(&request.body).expect("the body is JSON")
}

async fn commit_error(code: u16, message: &str) -> WriteError {
    let (connection, _api) = refusing(code, message, &[]);
    connection
        .write(&set_schedulable(false), WriteMode::Commit)
        .await
        .expect_err("the server refused")
}

#[tokio::test]
async fn allow_list_matches_the_operations() {
    let (connection, api) = accepting();
    connection
        .write(&set_schedulable(false), WriteMode::Commit)
        .await
        .expect("the patch is accepted");
    let requests = api.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "PATCH");
    assert_eq!(requests[0].path, "/api/v1/nodes/wk-04");
    assert_eq!(
        requests[0].content_type.as_deref(),
        Some("application/merge-patch+json")
    );
    assert_eq!(
        body_of(&requests[0]),
        json!({ "spec": { "unschedulable": true } })
    );
}

#[tokio::test]
async fn uncordon_sends_false() {
    let (connection, api) = accepting();
    connection
        .write(&set_schedulable(true), WriteMode::Commit)
        .await
        .expect("the patch is accepted");
    let requests = api.requests();
    assert_eq!(
        body_of(&requests[0]),
        json!({ "spec": { "unschedulable": false } })
    );
    assert!(!requests[0].body.contains("null"));
}

#[tokio::test]
async fn dry_run_sends_dry_run_all_and_field_manager() {
    let (connection, api) = accepting();
    connection
        .write(&set_schedulable(false), WriteMode::DryRun)
        .await
        .expect("the dry-run is accepted");
    let requests = api.requests();
    assert!(requests[0].has_query("dryRun", "All"), "{:?}", requests[0]);
    assert!(requests[0].has_query("fieldManager", "k8sboard"));
}

#[tokio::test]
async fn commit_sends_field_manager_without_dry_run() {
    let (connection, api) = accepting();
    connection
        .write(&set_schedulable(false), WriteMode::Commit)
        .await
        .expect("the patch is accepted");
    let requests = api.requests();
    assert!(requests[0].has_query("fieldManager", "k8sboard"));
    assert!(!requests[0].has_query_key("dryRun"), "{:?}", requests[0]);
}

#[tokio::test]
async fn outcome_reports_patched_effect() {
    let (connection, _api) = accepting();
    let outcome = connection
        .write(&set_schedulable(false), WriteMode::Commit)
        .await
        .expect("the patch is accepted");
    assert_eq!(outcome.mode, WriteMode::Commit);
    assert_eq!(outcome.effect, WriteEffect::Patched);
    assert_eq!(outcome.created_name, None);
    assert_eq!(outcome.uid, None);
}

#[tokio::test]
async fn debug_build_blocks_writes_without_opt_in() {
    let policy = WritePolicy::resolve(true, None);
    let (connection, api) = FakeApi::connection(policy, |_| (200, NODE_JSON.to_owned()));
    for mode in [WriteMode::DryRun, WriteMode::Commit] {
        let error = connection
            .write(&set_schedulable(false), mode)
            .await
            .expect_err("a debug build blocks writes");
        assert!(matches!(error, WriteError::WritesBlocked), "{error:?}");
    }
    assert!(api.requests().is_empty());
}

#[test]
fn write_policy_resolve_table() {
    assert_eq!(WritePolicy::resolve(false, None), WritePolicy::Allowed);
    assert_eq!(WritePolicy::resolve(false, Some("0")), WritePolicy::Allowed);
    for opt_in in [None, Some("0"), Some("true"), Some("")] {
        assert_eq!(WritePolicy::resolve(true, opt_in), WritePolicy::Blocked);
    }
    assert_eq!(WritePolicy::resolve(true, Some("1")), WritePolicy::Allowed);
}

#[test]
fn request_rejects_a_kind_that_does_not_fit() {
    let pod = ObjectRef::new(
        ObjectKind::Pod,
        Some("team-a".to_owned()),
        "api-0".to_owned(),
    )
    .expect("a pod has a namespace");
    let operation = WriteOperation::SetNodeSchedulable { schedulable: false };
    assert!(WriteRequest::new(pod, operation).is_none());
}

#[test]
fn access_check_matches_the_operation() {
    let check = set_schedulable(false).access_check();
    assert_eq!(check, AccessCheck::PatchNodes);
    assert_eq!(check.to_string(), "patch nodes");
}

#[tokio::test]
async fn forbidden_becomes_denied() {
    let error = commit_error(403, RBAC_MESSAGE).await;
    assert!(
        matches!(&error, WriteError::Denied { message } if message == RBAC_MESSAGE),
        "{error:?}"
    );
    assert!(error.to_string().starts_with("not permitted: "));
}

#[tokio::test]
async fn admission_403_is_invalid() {
    let error = commit_error(403, "admission webhook \"policy\" denied the request").await;
    assert!(matches!(error, WriteError::Invalid { .. }), "{error:?}");
    assert!(!error.to_string().contains("not permitted"));
}

#[tokio::test]
async fn missing_object_becomes_not_found() {
    let error = commit_error(404, "nodes \"wk-04\" not found").await;
    assert!(matches!(error, WriteError::NotFound), "{error:?}");
}

#[tokio::test]
async fn conflict_keeps_message_and_managers() {
    let (connection, _api) = refusing(
        409,
        "the object has been modified",
        &[("spec.unschedulable", "conflict")],
    );
    let error = connection
        .write(&set_schedulable(false), WriteMode::Commit)
        .await
        .expect_err("the server refused");
    assert!(
        matches!(
            &error,
            WriteError::Conflict { message, managers }
                if message == "the object has been modified" && managers.is_empty()
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn invalid_lists_field_paths() {
    let (connection, _api) = refusing(
        422,
        "Node is invalid",
        &[
            ("spec.unschedulable", "wrong type"),
            ("spec.taints[0]", "bad"),
        ],
    );
    let error = connection
        .write(&set_schedulable(false), WriteMode::DryRun)
        .await
        .expect_err("the server refused");
    assert!(
        matches!(
            &error,
            WriteError::Invalid { fields, .. }
                if fields == &["spec.unschedulable".to_owned(), "spec.taints[0]".to_owned()]
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn too_many_requests_maps_429() {
    for mode in [WriteMode::DryRun, WriteMode::Commit] {
        let (connection, _api) = refusing(429, "slow down", &[]);
        let error = connection
            .write(&set_schedulable(false), mode)
            .await
            .expect_err("the server refused");
        assert!(
            matches!(&error, WriteError::TooManyRequests { message, .. } if message == "slow down"),
            "{error:?}"
        );
    }
}

#[tokio::test]
async fn webhook_dry_run_rejection_blocks() {
    let message = "admission webhook \"x.example.com\" does not support dry run";
    let (connection, _api) = refusing(400, message, &[]);
    let error = connection
        .write(&set_schedulable(false), WriteMode::DryRun)
        .await
        .expect_err("the webhook rejected the dry-run");
    assert!(
        matches!(&error, WriteError::DryRunRejected { reason } if reason == message),
        "{error:?}"
    );
}

#[tokio::test]
async fn dry_run_message_on_a_commit_is_no_webhook_rejection() {
    let (connection, _api) = refusing(400, "webhook does not support dry run", &[]);
    let error = connection
        .write(&set_schedulable(false), WriteMode::Commit)
        .await
        .expect_err("the server refused");
    assert!(matches!(error, WriteError::Cluster(_)), "{error:?}");
}

#[tokio::test(start_paused = true)]
async fn commit_timeout_is_outcome_unknown() {
    let (connection, api) = FakeApi::failing(WritePolicy::Allowed, Failure::Hang);
    let error = connection
        .write(&set_schedulable(false), WriteMode::Commit)
        .await
        .expect_err("nothing answers");
    assert!(matches!(error, WriteError::OutcomeUnknown), "{error:?}");
    assert_eq!(api.requests().len(), 1);
}

#[tokio::test]
async fn commit_transport_error_is_outcome_unknown() {
    let (connection, _api) = FakeApi::failing(WritePolicy::Allowed, Failure::Error);
    let error = connection
        .write(&set_schedulable(false), WriteMode::Commit)
        .await
        .expect_err("the transport failed");
    assert!(matches!(error, WriteError::OutcomeUnknown), "{error:?}");
}

#[tokio::test(start_paused = true)]
async fn dry_run_errors_are_never_outcome_unknown() {
    for failure in [Failure::Hang, Failure::Error] {
        let (connection, _api) = FakeApi::failing(WritePolicy::Allowed, failure);
        let error = connection
            .write(&set_schedulable(false), WriteMode::DryRun)
            .await
            .expect_err("the dry-run failed");
        assert!(
            matches!(error, WriteError::Cluster(_)),
            "{failure:?}: {error:?}"
        );
    }
}

#[test]
fn redaction_applies_to_secret_kinds_only() {
    let fields = ["data.password".to_owned()];
    let message = r#"Secret "db" is invalid: data.password: Invalid value: "hunter2""#;
    let redacted = redact_message("Secret", message, "Invalid", &fields);
    assert_eq!(redacted, "Invalid: data.password");
    assert!(!redacted.contains("hunter2"));
    assert_eq!(redact_message("Secret", message, "", &[]), "Failure");
    assert_eq!(redact_message("Node", message, "Invalid", &fields), message);
}

#[test]
fn changed_fields_name_paths_and_values() {
    let cordon = set_schedulable(false).changed_fields();
    assert_eq!(
        cordon,
        [ChangedField {
            path: "spec.unschedulable".into(),
            value: Some("true".to_owned()),
        }]
    );
    let uncordon = set_schedulable(true).changed_fields();
    assert_eq!(uncordon[0].value.as_deref(), Some("false"));
}

#[test]
fn every_operation_supports_dry_run() {
    assert!(set_schedulable(false).supports_dry_run());
}

#[test]
fn manual_debug_shows_names_only() {
    let request = set_schedulable(false);
    assert_eq!(
        format!("{request:?}"),
        r#"WriteRequest { operation: SetNodeSchedulable, kind: Node, namespace: None, name: "wk-04" }"#
    );
    assert_eq!(format!("{:?}", request.operation()), "SetNodeSchedulable");
    assert!(!format!("{request:?}").contains("unschedulable"));
}

fn status_of(code: u16, message: &str, reason: &str, fields: &[&str]) -> Status {
    let causes: Vec<Value> = fields
        .iter()
        .map(|field| json!({ "reason": "FieldValueInvalid", "message": "Invalid value: \"hunter2\"", "field": field }))
        .collect();
    serde_json::from_value(json!({
        "message": message,
        "reason": reason,
        "code": code,
        "details": { "causes": causes },
    }))
    .expect("a status")
}

#[test]
fn a_secret_rbac_403_is_denied_and_holds_only_the_reason_and_fields() {
    let status = status_of(403, RBAC_MESSAGE, "Forbidden", &["data.password"]);
    let error = error_from_status("ctx", WriteMode::Commit, "Secret", status);
    assert!(
        matches!(&error, WriteError::Denied { message } if message == "Forbidden: data.password"),
        "{error:?}"
    );
    assert!(!error.to_string().contains("readonly"));
}

#[test]
fn a_secret_error_loses_its_server_text_in_every_variant() {
    let message = r#"Secret "db" is invalid: "hunter2""#;
    for (code, mode) in [
        (409, WriteMode::Commit),
        (422, WriteMode::DryRun),
        (429, WriteMode::Commit),
        (500, WriteMode::Commit),
        (400, WriteMode::DryRun),
    ] {
        let text = if code == 400 {
            format!("{message} does not support dry run")
        } else {
            message.to_owned()
        };
        let status = status_of(code, &text, "Reason", &["data.password"]);
        let error = error_from_status("ctx", mode, "Secret", status);
        let shown = format!("{error} {error:?}");
        assert!(!shown.contains("hunter2"), "{code}: {shown}");
        assert!(shown.contains("data.password"), "{code}: {shown}");
    }
}

#[test]
fn a_node_error_keeps_its_server_text() {
    let status = status_of(403, RBAC_MESSAGE, "Forbidden", &[]);
    let error = error_from_status("ctx", WriteMode::Commit, "Node", status);
    assert!(
        matches!(&error, WriteError::Denied { message } if message == RBAC_MESSAGE),
        "{error:?}"
    );
}

#[test]
fn request_rejects_a_name_that_changes_the_path() {
    let operation = || WriteOperation::SetNodeSchedulable { schedulable: false };
    for name in ["wk-04/status", "a?b", "a#b", "..", "A", "a b", ""] {
        let target = ObjectRef::new(ObjectKind::Node, None, name.to_owned()).expect("a node");
        assert!(WriteRequest::new(target, operation()).is_none(), "{name:?}");
    }
    let fine = ObjectRef::new(
        ObjectKind::Node,
        None,
        "ip-10-0-1-23.ec2.internal".to_owned(),
    );
    assert!(WriteRequest::new(fine.expect("a node"), operation()).is_some());
}

#[tokio::test]
async fn a_non_utf8_answer_to_a_commit_is_outcome_unknown() {
    let (connection, api) =
        FakeApi::answering_bytes(WritePolicy::Allowed, 200, vec![0xff, 0xfe, 0xfd]);
    let error = connection
        .write(&set_schedulable(false), WriteMode::Commit)
        .await
        .expect_err("the answer cannot be read");
    assert!(matches!(error, WriteError::OutcomeUnknown), "{error:?}");
    assert_eq!(api.requests().len(), 1);
}

#[tokio::test]
async fn a_non_utf8_answer_to_a_dry_run_is_no_unknown_outcome() {
    let (connection, _api) =
        FakeApi::answering_bytes(WritePolicy::Allowed, 200, vec![0xff, 0xfe, 0xfd]);
    let error = connection
        .write(&set_schedulable(false), WriteMode::DryRun)
        .await
        .expect_err("the answer cannot be read");
    assert!(matches!(error, WriteError::Cluster(_)), "{error:?}");
}

#[test]
fn a_build_that_blocks_writes_ignores_the_opt_in() {
    for is_debug_build in [true, false] {
        for opt_in in [None, Some("1")] {
            assert_eq!(
                WritePolicy::of_build(true, is_debug_build, opt_in),
                WritePolicy::Blocked,
                "{is_debug_build} {opt_in:?}"
            );
        }
    }
    assert_eq!(
        WritePolicy::of_build(false, false, None),
        WritePolicy::Allowed
    );
    assert_eq!(
        WritePolicy::of_build(false, true, None),
        WritePolicy::Blocked
    );
    assert_eq!(
        WritePolicy::of_build(false, true, Some("1")),
        WritePolicy::Allowed
    );
}

#[test]
fn a_lab_build_writes_only_on_a_kind_context_with_the_opt_in() {
    assert_eq!(
        WritePolicy::of_lab_build(true, Some("1"), "kind-x"),
        WritePolicy::Allowed
    );
    assert_eq!(
        WritePolicy::of_lab_build(true, None, "kind-x"),
        WritePolicy::Blocked
    );
    for context in ["readonly@Monitor", "prod-kind-x", ""] {
        assert_eq!(
            WritePolicy::of_lab_build(true, Some("1"), context),
            WritePolicy::Blocked,
            "{context}"
        );
    }
}
