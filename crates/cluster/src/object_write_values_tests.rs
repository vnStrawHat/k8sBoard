//! Tests of the 0047 values patch over the fake transport.

use k8s_openapi::api::core::v1::Secret;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::*;
use crate::config_values::{KeyChange, NewValue};
use crate::fake_api::{FakeApi, RecordedRequest};

const SECRET_VALUE: &str = "S3cr3t-0047-ZZ";
const SECRET_BASE64: &str = "UzNjcjN0LTAwNDctWlo=";
const PATH: &str = "/api/v1/namespaces/payments/secrets/api-db";

fn secret_ref() -> ObjectRef {
    ObjectRef::new(
        ObjectKind::Secret,
        Some("payments".to_owned()),
        "api-db".to_owned(),
    )
    .expect("a secret has a namespace")
}

fn config_map_ref() -> ObjectRef {
    ObjectRef::new(
        ObjectKind::ConfigMap,
        Some("payments".to_owned()),
        "api-config".to_owned(),
    )
    .expect("a config map has a namespace")
}

async fn base_of(target: &ObjectRef) -> crate::config_values::ValuesBase {
    let kind = target.kind_name();
    let body = json!({
        "apiVersion": "v1", "kind": kind,
        "metadata": {"name": target.name(), "namespace": "payments", "resourceVersion": "42"},
        "data": {"K": "azg=", "OLD": "b2xk"},
    })
    .to_string();
    let (connection, _api) =
        FakeApi::connection(WritePolicy::Blocked, move |_| (200, body.clone()));
    connection
        .values_base(target)
        .await
        .expect("an editable base")
}

fn change(key: &str, text: &str) -> KeyChange {
    KeyChange::Set {
        key: key.to_owned(),
        value: NewValue::new(Zeroizing::new(text.to_owned())),
    }
}

async fn request_for(target: ObjectRef, changes: Vec<KeyChange>) -> WriteRequest {
    let base = base_of(&target).await;
    let edit = base.edit(changes).expect("a valid edit");
    WriteRequest::new(target, WriteOperation::SetDataValues(Box::new(edit)))
        .expect("a secret or config map fits")
}

fn body_of(request: &RecordedRequest) -> Value {
    serde_json::from_str(&request.body).expect("the body is JSON")
}

fn answering(code: u16, body: String) -> (ClusterConnection, FakeApi) {
    FakeApi::connection(WritePolicy::Allowed, move |_| (code, body.clone()))
}

fn ok_secret() -> String {
    json!({"apiVersion": "v1", "kind": "Secret", "metadata": {"name": "api-db"},
        "data": {"K": SECRET_BASE64}})
    .to_string()
}

#[tokio::test]
async fn dry_run_request_shape() {
    let request = request_for(secret_ref(), vec![change("K", SECRET_VALUE)]).await;
    let (connection, api) = answering(200, ok_secret());
    let outcome = connection
        .write(&request, WriteMode::DryRun)
        .await
        .expect("accepted");
    assert_eq!(outcome.effect, WriteEffect::Patched);
    let requests = api.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "PATCH");
    assert_eq!(requests[0].path, PATH);
    assert!(
        requests[0].has_query("dryRun", "All"),
        "{}",
        requests[0].query
    );
    assert!(requests[0].has_query("fieldManager", "k8sboard"));
    assert_eq!(
        requests[0].content_type.as_deref(),
        Some("application/merge-patch+json")
    );
    assert_eq!(
        body_of(&requests[0]),
        json!({"metadata": {"resourceVersion": "42"}, "data": {"K": SECRET_BASE64}})
    );
}

#[tokio::test]
async fn commit_request_has_no_dry_run() {
    let request = request_for(config_map_ref(), vec![change("K", "plain")]).await;
    let (connection, api) = answering(200, ok_secret());
    connection
        .write(&request, WriteMode::Commit)
        .await
        .expect("accepted");
    let sent = &api.requests()[0];
    assert!(!sent.has_query_key("dryRun"), "{}", sent.query);
    assert!(sent.has_query("fieldManager", "k8sboard"));
    assert_eq!(
        sent.path,
        "/api/v1/namespaces/payments/configmaps/api-config"
    );
    assert_eq!(body_of(sent)["data"], json!({"K": "plain"}));
}

#[tokio::test]
async fn blocked_policy_sends_nothing() {
    let request = request_for(secret_ref(), vec![change("K", "v")]).await;
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, |_| (200, ok_secret()));
    let error = connection
        .write(&request, WriteMode::Commit)
        .await
        .expect_err("blocked");
    assert!(matches!(error, WriteError::WritesBlocked), "{error}");
    assert!(api.requests().is_empty());
}

#[tokio::test]
async fn request_needs_matching_target_and_kind() {
    let base = base_of(&secret_ref()).await;
    let edit = base.edit(vec![change("K", "v")]).expect("valid");
    let operation = || WriteOperation::SetDataValues(Box::new(edit.clone()));
    assert!(WriteRequest::new(config_map_ref(), operation()).is_none());
    let other = ObjectRef::new(
        ObjectKind::Secret,
        Some("payments".to_owned()),
        "other".to_owned(),
    )
    .expect("a secret has a namespace");
    assert!(WriteRequest::new(other, operation()).is_none());
    let deployment = ObjectRef::new(
        ObjectKind::Deployment,
        Some("payments".to_owned()),
        "api-db".to_owned(),
    )
    .expect("a deployment has a namespace");
    assert!(WriteRequest::new(deployment, operation()).is_none());
    assert!(WriteRequest::new(secret_ref(), operation()).is_some());
}

#[tokio::test]
async fn access_check_is_patch_of_kind() {
    let secret = request_for(secret_ref(), vec![change("K", "v")]).await;
    assert_eq!(
        secret.access_check(),
        AccessCheck::Patch(ObjectKind::Secret)
    );
    let config_map = request_for(config_map_ref(), vec![change("K", "v")]).await;
    assert_eq!(
        config_map.access_check(),
        AccessCheck::Patch(ObjectKind::ConfigMap)
    );
    assert!(secret.supports_dry_run());
}

#[tokio::test]
async fn changed_fields_are_names_and_markers() {
    let request = request_for(
        secret_ref(),
        vec![
            change("K", SECRET_VALUE),
            KeyChange::Add {
                key: "NEW".to_owned(),
                value: NewValue::new(Zeroizing::new(SECRET_VALUE.to_owned())),
            },
            KeyChange::Remove {
                key: "OLD".to_owned(),
            },
        ],
    )
    .await;
    let fields = request.changed_fields();
    let paths: Vec<_> = fields.iter().map(|field| field.path.as_ref()).collect();
    assert_eq!(
        paths,
        [
            "data[K] value changed",
            "data[NEW] added",
            "data[OLD] removed"
        ]
    );
    assert!(fields.iter().all(|field| field.value.is_none()));
    let text = format!("{fields:?}");
    assert!(!text.contains(SECRET_VALUE) && !text.contains(SECRET_BASE64));
}

fn status(code: u16, reason: &str, message: &str, field: &str) -> String {
    json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure",
        "message": message, "reason": reason, "code": code,
        "details": {"causes": [{"reason": "FieldValueInvalid", "message": message, "field": field}]},
    })
    .to_string()
}

#[tokio::test]
async fn conflict_maps_to_conflict() {
    let request = request_for(secret_ref(), vec![change("K", "v")]).await;
    let (connection, _api) = answering(
        409,
        status(409, "Conflict", "the object has been modified", ""),
    );
    let error = connection
        .write(&request, WriteMode::Commit)
        .await
        .expect_err("a conflict");
    assert!(matches!(error, WriteError::Conflict { .. }), "{error}");
}

#[tokio::test]
async fn secret_invalid_message_is_redacted() {
    let request = request_for(secret_ref(), vec![change("K", SECRET_VALUE)]).await;
    let message =
        format!("Secret \"api-db\" is invalid: data[K]: Invalid value: \"{SECRET_VALUE}\"");
    let (connection, _api) = answering(422, status(422, "Invalid", &message, "data[K]"));
    let error = connection
        .write(&request, WriteMode::DryRun)
        .await
        .expect_err("invalid");
    let text = format!("{error} {error:?}");
    assert!(!text.contains(SECRET_VALUE), "{text}");
    assert!(text.contains("Invalid: data[K]"), "{text}");
}

#[tokio::test]
async fn config_map_invalid_message_is_shown_as_sent() {
    let request = request_for(config_map_ref(), vec![change("K", "v")]).await;
    let (connection, _api) = answering(
        422,
        status(422, "Invalid", "ConfigMap is invalid: data[K]", "data[K]"),
    );
    let error = connection
        .write(&request, WriteMode::DryRun)
        .await
        .expect_err("invalid");
    assert!(
        error.to_string().contains("ConfigMap is invalid"),
        "{error}"
    );
}

#[tokio::test]
async fn request_debug_holds_no_value() {
    let request = request_for(secret_ref(), vec![change("K", SECRET_VALUE)]).await;
    let text = format!("{request:?}");
    assert!(
        !text.contains(SECRET_VALUE) && !text.contains(SECRET_BASE64),
        "{text}"
    );
    assert!(text.contains("SetDataValues"), "{text}");
}

#[tokio::test]
async fn dry_run_answer_with_values_is_dropped() {
    let request = request_for(secret_ref(), vec![change("K", SECRET_VALUE)]).await;
    let answer: Secret = serde_json::from_str(&ok_secret()).expect("a secret");
    let (connection, _api) = answering(200, serde_json::to_string(&answer).expect("json"));
    let outcome = connection
        .write(&request, WriteMode::DryRun)
        .await
        .expect("accepted");
    let text = format!("{outcome:?}");
    assert!(
        !text.contains(SECRET_VALUE) && !text.contains(SECRET_BASE64),
        "{text}"
    );
}
