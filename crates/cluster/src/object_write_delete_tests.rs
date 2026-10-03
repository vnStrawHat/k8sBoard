//! Tests of the 0033 `DeleteObject` operation and `object_identity` over the fake transport.

use serde_json::{Value, json};

use super::*;
use crate::fake_api::{FakeApi, RecordedRequest};

const POD_PATH: &str = "/api/v1/namespaces/payments/pods/api-x";
const SECRET_DATA: &str = "c2VjcmV0";

fn target(kind: ObjectKind, name: &str) -> ObjectRef {
    let namespace = kind.is_namespaced().then(|| "payments".to_owned());
    ObjectRef::new(kind, namespace, name.to_owned()).expect("the namespace fits the kind")
}

fn delete_of(target: ObjectRef, uid: &str, propagation: DeletePropagation) -> Option<WriteRequest> {
    WriteRequest::new(
        target,
        WriteOperation::DeleteObject {
            uid: uid.to_owned(),
            propagation,
        },
    )
}

fn delete_pod(propagation: DeletePropagation) -> WriteRequest {
    delete_of(target(ObjectKind::Pod, "api-x"), "u-1", propagation).expect("a pod fits a delete")
}

fn body_of(request: &RecordedRequest) -> Value {
    serde_json::from_str(&request.body).expect("the body is JSON")
}

fn status_ok() -> String {
    json!({"kind": "Status", "apiVersion": "v1", "status": "Success", "code": 200}).to_string()
}

fn answering(body: String) -> (ClusterConnection, FakeApi) {
    FakeApi::connection(WritePolicy::Allowed, move |_| (200, body.clone()))
}

fn refusing(code: u16, reason: &str, message: &str) -> (ClusterConnection, FakeApi) {
    let body = json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure",
        "message": message, "reason": reason, "code": code,
    })
    .to_string();
    FakeApi::connection(WritePolicy::Allowed, move |_| (code, body.clone()))
}

fn pod_answer(deleting: bool, finalizers: &[&str]) -> String {
    let mut metadata = json!({"name": "api-x", "namespace": "payments", "uid": "u-1"});
    if deleting {
        metadata["deletionTimestamp"] = json!("2026-10-03T08:00:00Z");
    }
    if !finalizers.is_empty() {
        metadata["finalizers"] = json!(finalizers);
    }
    json!({"apiVersion": "v1", "kind": "Pod", "metadata": metadata}).to_string()
}

#[tokio::test]
async fn delete_dry_run_shape() {
    let (connection, api) = answering(status_ok());
    connection
        .write(
            &delete_pod(DeletePropagation::Background),
            WriteMode::DryRun,
        )
        .await
        .expect("the dry-run is accepted");
    let requests = api.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "DELETE");
    assert_eq!(requests[0].path, POD_PATH);
    assert_eq!(requests[0].query, "");
    assert_eq!(
        requests[0].content_type.as_deref(),
        Some("application/json")
    );
    assert_eq!(
        body_of(&requests[0]),
        json!({
            "dryRun": ["All"],
            "propagationPolicy": "Background",
            "preconditions": {"uid": "u-1"},
        })
    );
}

#[tokio::test]
async fn delete_commit_has_no_dry_run() {
    let (connection, api) = answering(status_ok());
    connection
        .write(
            &delete_pod(DeletePropagation::Background),
            WriteMode::Commit,
        )
        .await
        .expect("the delete is accepted");
    let requests = api.requests();
    let body = body_of(&requests[0]);
    assert!(body.get("dryRun").is_none(), "{body}");
    assert!(!requests[0].has_query_key("fieldManager"));
    assert!(!requests[0].has_query_key("dryRun"));
    assert!(
        body["preconditions"].get("resourceVersion").is_none(),
        "{body}"
    );
    assert_eq!(body["preconditions"]["uid"], "u-1");
}

#[tokio::test]
async fn propagation_as_str_matches_the_body() {
    for propagation in [
        DeletePropagation::Background,
        DeletePropagation::Foreground,
        DeletePropagation::Orphan,
    ] {
        let (connection, api) = answering(status_ok());
        connection
            .write(&delete_pod(propagation), WriteMode::Commit)
            .await
            .expect("the delete is accepted");
        let body = body_of(&api.requests()[0]);
        assert_eq!(body["propagationPolicy"], propagation.as_str());
    }
}

#[tokio::test]
async fn uid_precondition_conflict() {
    let (connection, _api) = refusing(409, "Conflict", "Precondition failed: UID");
    let error = connection
        .write(
            &delete_pod(DeletePropagation::Background),
            WriteMode::Commit,
        )
        .await
        .expect_err("the uid differs");
    assert!(matches!(error, WriteError::Conflict { .. }), "{error:?}");
}

#[tokio::test]
async fn missing_object_is_not_found() {
    let (connection, _api) = refusing(404, "NotFound", "pods \"api-x\" not found");
    let error = connection
        .write(
            &delete_pod(DeletePropagation::Background),
            WriteMode::DryRun,
        )
        .await
        .expect_err("the object is gone");
    assert!(matches!(error, WriteError::NotFound), "{error:?}");
}

#[tokio::test]
async fn object_with_deletion_timestamp_is_pending() {
    let (connection, _api) = answering(pod_answer(true, &["a", "b"]));
    let outcome = connection
        .write(
            &delete_pod(DeletePropagation::Background),
            WriteMode::Commit,
        )
        .await
        .expect("the delete is accepted");
    assert_eq!(
        outcome.effect,
        WriteEffect::DeletionPending {
            finalizers: vec!["a".to_owned(), "b".to_owned()]
        }
    );
}

#[tokio::test]
async fn pending_without_finalizers_has_an_empty_list() {
    let (connection, _api) = answering(pod_answer(true, &[]));
    let outcome = connection
        .write(
            &delete_pod(DeletePropagation::Background),
            WriteMode::Commit,
        )
        .await
        .expect("the delete is accepted");
    assert_eq!(
        outcome.effect,
        WriteEffect::DeletionPending { finalizers: vec![] }
    );
}

#[tokio::test]
async fn status_response_is_deleted() {
    let (connection, _api) = answering(status_ok());
    let outcome = connection
        .write(
            &delete_pod(DeletePropagation::Background),
            WriteMode::Commit,
        )
        .await
        .expect("the delete is accepted");
    assert_eq!(outcome.effect, WriteEffect::Deleted);
    assert_eq!(outcome.uid, None);
}

#[tokio::test]
async fn object_without_deletion_timestamp_is_deleted() {
    let (connection, _api) = answering(pod_answer(false, &[]));
    let outcome = connection
        .write(
            &delete_pod(DeletePropagation::Background),
            WriteMode::Commit,
        )
        .await
        .expect("the delete is accepted");
    assert_eq!(outcome.effect, WriteEffect::Deleted);
}

#[tokio::test]
async fn secret_delete_keeps_no_data() {
    let body = json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {
            "name": "db", "namespace": "payments",
            "deletionTimestamp": "2026-10-03T08:00:00Z", "finalizers": ["f"],
        },
        "data": {"password": SECRET_DATA},
    })
    .to_string();
    let (connection, _api) = answering(body);
    let request = delete_of(
        target(ObjectKind::Secret, "db"),
        "u-1",
        DeletePropagation::Background,
    )
    .expect("a secret fits a delete");
    let outcome = connection
        .write(&request, WriteMode::Commit)
        .await
        .expect("the delete is accepted");
    assert!(!format!("{outcome:?}").contains(SECRET_DATA));
}

#[tokio::test]
async fn debug_policy_blocks_delete() {
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, |_| (200, status_ok()));
    for mode in [WriteMode::DryRun, WriteMode::Commit] {
        let error = connection
            .write(&delete_pod(DeletePropagation::Background), mode)
            .await
            .expect_err("a blocked policy sends nothing");
        assert!(matches!(error, WriteError::WritesBlocked), "{error:?}");
    }
    assert!(api.requests().is_empty());
}

#[test]
fn request_rejects_an_empty_uid() {
    let request = delete_of(
        target(ObjectKind::Pod, "api-x"),
        "",
        DeletePropagation::Background,
    );
    assert!(request.is_none());
}

#[test]
fn request_rejects_an_unsafe_name() {
    let request = delete_of(
        target(ObjectKind::Pod, "../x"),
        "u-1",
        DeletePropagation::Background,
    );
    assert!(request.is_none());
}

#[test]
fn manual_debug_hides_the_uid() {
    let request = delete_pod(DeletePropagation::Orphan);
    let text = format!("{request:?}");
    assert!(text.contains("DeleteObject"), "{text}");
    assert!(!text.contains("u-1"), "{text}");
    assert!(!text.contains("Orphan"), "{text}");
}

#[test]
fn delete_changed_fields_record_propagation() {
    let fields = delete_pod(DeletePropagation::Foreground).changed_fields();
    assert_eq!(
        fields,
        vec![ChangedField {
            path: Cow::Borrowed("deleteOptions.propagationPolicy"),
            value: Some("Foreground".to_owned()),
        }]
    );
}

#[test]
fn delete_needs_the_delete_permission_and_dry_runs() {
    let request = delete_pod(DeletePropagation::Background);
    assert_eq!(request.access_check(), AccessCheck::Delete(ObjectKind::Pod));
    assert!(request.supports_dry_run());
}

#[tokio::test]
async fn identity_reads_metadata_only() {
    let body = json!({
        "apiVersion": "meta.k8s.io/v1", "kind": "PartialObjectMetadata",
        "metadata": {
            "name": "api-x", "namespace": "payments", "uid": "u-9",
            "finalizers": ["f1"], "deletionTimestamp": "2026-10-03T08:00:00Z",
            "labels": {"app": "x"}, "annotations": {"note": "y"},
        },
    })
    .to_string();
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, move |_| (200, body.clone()));
    let identity = connection
        .object_identity(&target(ObjectKind::Pod, "api-x"))
        .await
        .expect("the metadata is readable");
    assert_eq!(identity.uid, "u-9");
    assert_eq!(identity.finalizers, vec!["f1".to_owned()]);
    assert!(identity.deletion_started.is_some());
    let requests = api.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, POD_PATH);
    let accept = requests[0].accept.as_deref().unwrap_or_default();
    assert!(accept.contains("PartialObjectMetadata"), "{accept}");
}

#[tokio::test]
async fn identity_without_uid_is_unexpected() {
    let body = json!({
        "apiVersion": "meta.k8s.io/v1", "kind": "PartialObjectMetadata",
        "metadata": {"name": "api-x"},
    })
    .to_string();
    let (connection, _api) =
        FakeApi::connection(WritePolicy::Blocked, move |_| (200, body.clone()));
    let error = connection
        .object_identity(&target(ObjectKind::Pod, "api-x"))
        .await
        .expect_err("no uid to pin");
    assert!(
        matches!(error, ClusterError::UnexpectedResponse { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn identity_of_a_missing_object_is_a_404() {
    let (connection, _api) = refusing(404, "NotFound", "not found");
    let error = connection
        .object_identity(&target(ObjectKind::Pod, "api-x"))
        .await
        .expect_err("the object is gone");
    assert!(
        matches!(error, ClusterError::Api { code: 404, .. }),
        "{error:?}"
    );
}

#[test]
fn owns_dependents_table() {
    for kind in ObjectKind::ALL {
        let expected = matches!(
            kind,
            ObjectKind::Deployment
                | ObjectKind::StatefulSet
                | ObjectKind::DaemonSet
                | ObjectKind::ReplicaSet
                | ObjectKind::Job
                | ObjectKind::CronJob
        );
        assert_eq!(kind.owns_dependents(), expected, "{kind:?}");
    }
}
