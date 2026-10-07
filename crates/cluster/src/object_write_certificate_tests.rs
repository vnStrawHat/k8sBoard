//! Tests of the 0018 step 6 Certificate renewal over the fake transport.

use serde_json::{Value, json};

use super::*;
use crate::fake_api::{FakeApi, RecordedRequest};

const PATH: &str = "/apis/cert-manager.io/v1/namespaces/shop/certificates/tls";

fn resource(group: &str, version: &str, scope: ResourceScope) -> CustomResourceType {
    CustomResourceType {
        group: group.to_owned(),
        version: version.to_owned(),
        kind: "Certificate".to_owned(),
        plural: "certificates".to_owned(),
        scope,
    }
}

fn certificate_ref() -> ObjectRef {
    ObjectRef::custom(
        resource("cert-manager.io", "v1", ResourceScope::Namespaced),
        Some("shop".to_owned()),
        "tls".to_owned(),
    )
    .expect("a namespaced resource has a namespace")
}

fn requested_at() -> jiff::Timestamp {
    "2026-10-04T08:30:15Z".parse().expect("a valid timestamp")
}

fn renew() -> WriteOperation {
    WriteOperation::RenewCertificate {
        requested_at: requested_at(),
    }
}

fn request() -> WriteRequest {
    WriteRequest::new(certificate_ref(), renew()).expect("a cert-manager v1 certificate fits")
}

fn certificate(conditions: Value) -> Value {
    json!({
        "apiVersion": "cert-manager.io/v1",
        "kind": "Certificate",
        "metadata": {
            "name": "tls", "namespace": "shop", "resourceVersion": "812", "generation": 2,
            "managedFields": [{"manager": "cert-manager"}],
        },
        "spec": {"secretName": "tls-secret"},
        "status": {"conditions": conditions, "notAfter": "2026-12-01T00:00:00Z", "revision": 1},
    })
}

fn ready() -> Value {
    certificate(json!([{"type": "Ready", "status": "True"}]))
}

/// GET answers `get`; every PUT answers 200 with the request body.
fn serving(get: (u16, Value)) -> (ClusterConnection, FakeApi) {
    FakeApi::connection(WritePolicy::Allowed, move |recorded| {
        if recorded.method == "GET" {
            (get.0, get.1.to_string())
        } else {
            (200, recorded.body.clone())
        }
    })
}

fn body_of(request: &RecordedRequest) -> Value {
    serde_json::from_str(&request.body).expect("the body is JSON")
}

fn puts(api: &FakeApi) -> Vec<RecordedRequest> {
    api.requests()
        .into_iter()
        .filter(|request| request.method == "PUT")
        .collect()
}

#[tokio::test]
async fn renew_dry_run_request_shape() {
    let (connection, api) = serving((200, ready()));
    let outcome = connection
        .write(&request(), WriteMode::DryRun)
        .await
        .expect("accepted");
    assert_eq!(outcome.effect, WriteEffect::Patched);
    let requests = api.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        (requests[0].method.as_str(), requests[0].path.as_str()),
        ("GET", PATH)
    );
    let put = &requests[1];
    assert_eq!(put.method, "PUT");
    assert_eq!(put.path, format!("{PATH}/status"));
    assert!(put.has_query("dryRun", "All"), "{}", put.query);
    assert!(put.has_query("fieldManager", "k8sboard"), "{}", put.query);
    assert_eq!(put.content_type.as_deref(), Some("application/json"));
    let body = body_of(put);
    assert_eq!(body["metadata"]["resourceVersion"], "812");
    assert!(body["metadata"].get("managedFields").is_none());
    assert_eq!(body["spec"], json!({"secretName": "tls-secret"}));
    assert_eq!(body["status"]["notAfter"], "2026-12-01T00:00:00Z");
    assert_eq!(body["status"]["revision"], 1);
    assert_eq!(
        body["status"]["conditions"],
        json!([
            {"type": "Ready", "status": "True"},
            {
                "type": "Issuing", "status": "True", "reason": "ManuallyTriggered",
                "message": "Certificate re-issuance manually triggered",
                "lastTransitionTime": "2026-10-04T08:30:15Z", "observedGeneration": 2,
            },
        ])
    );
}

#[tokio::test]
async fn renew_commit_has_no_dry_run() {
    let (connection, api) = serving((200, ready()));
    connection
        .write(&request(), WriteMode::Commit)
        .await
        .expect("accepted");
    let puts = puts(&api);
    assert_eq!(puts.len(), 1);
    assert!(!puts[0].has_query_key("dryRun"), "{}", puts[0].query);
    assert!(puts[0].has_query("fieldManager", "k8sboard"));
    assert_eq!(puts[0].path, format!("{PATH}/status"));
}

#[tokio::test]
async fn dry_run_and_commit_send_one_body() {
    let (connection, api) = serving((200, ready()));
    connection
        .write(&request(), WriteMode::DryRun)
        .await
        .expect("dry-run");
    connection
        .write(&request(), WriteMode::Commit)
        .await
        .expect("commit");
    let puts = puts(&api);
    assert_eq!(puts.len(), 2);
    assert_eq!(puts[0].body, puts[1].body);
}

#[tokio::test]
async fn renew_get_not_found_is_not_found() {
    let status = json!({"kind": "Status", "apiVersion": "v1", "status": "Failure",
        "message": "certificates.cert-manager.io \"tls\" not found", "reason": "NotFound",
        "code": 404});
    let (connection, api) = serving((404, status));
    let error = connection
        .write(&request(), WriteMode::Commit)
        .await
        .expect_err("gone");
    assert!(matches!(error, WriteError::NotFound), "{error}");
    assert!(puts(&api).is_empty());
}

#[tokio::test]
async fn renew_already_issuing_sends_no_put() {
    let issuing = certificate(json!([{"type": "Issuing", "status": "True"}]));
    let (connection, api) = serving((200, issuing));
    let error = connection
        .write(&request(), WriteMode::Commit)
        .await
        .expect_err("refused");
    match error {
        WriteError::Invalid { fields, .. } => {
            assert_eq!(fields, ["status.conditions[Issuing]"]);
        }
        other => panic!("expected Invalid, got {other}"),
    }
    assert_eq!(api.requests().len(), 1);
    assert!(puts(&api).is_empty());
}

#[tokio::test]
async fn renew_deleting_sends_no_put() {
    let mut deleting = ready();
    deleting["metadata"]["deletionTimestamp"] = json!("2026-10-04T08:00:00Z");
    let (connection, api) = serving((200, deleting));
    let error = connection
        .write(&request(), WriteMode::DryRun)
        .await
        .expect_err("refused");
    assert!(matches!(error, WriteError::Invalid { .. }), "{error}");
    assert!(puts(&api).is_empty());
}

#[tokio::test]
async fn renew_conflict_is_a_conflict() {
    let conflict = json!({"kind": "Status", "apiVersion": "v1", "status": "Failure",
        "message": "the object has been modified", "reason": "Conflict", "code": 409});
    let get = ready().to_string();
    let (connection, _api) = FakeApi::connection(WritePolicy::Allowed, move |recorded| {
        if recorded.method == "GET" {
            (200, get.clone())
        } else {
            (409, conflict.to_string())
        }
    });
    let error = connection
        .write(&request(), WriteMode::Commit)
        .await
        .expect_err("conflict");
    assert!(matches!(error, WriteError::Conflict { .. }), "{error}");
}

#[tokio::test]
async fn blocked_policy_sends_nothing() {
    let (connection, api) =
        FakeApi::connection(WritePolicy::Blocked, |_| (200, ready().to_string()));
    let error = connection
        .write(&request(), WriteMode::Commit)
        .await
        .expect_err("blocked");
    assert!(matches!(error, WriteError::WritesBlocked), "{error}");
    assert!(api.requests().is_empty());
}

#[test]
fn renew_needs_cert_manager_v1_target() {
    let fits =
        |target: Option<ObjectRef>| target.and_then(|target| WriteRequest::new(target, renew()));
    let namespaced = |group: &str, version: &str| {
        ObjectRef::custom(
            resource(group, version, ResourceScope::Namespaced),
            Some("shop".to_owned()),
            "tls".to_owned(),
        )
    };
    assert_eq!(
        fits(namespaced("cert-manager.io", "v1")).map(|request| request.access_check()),
        Some(AccessCheck::UpdateCertificateStatus)
    );
    assert!(fits(namespaced("cert-manager.io", "v1alpha2")).is_none());
    assert!(fits(namespaced("example.org", "v1")).is_none());
    let cluster_scoped = ObjectRef::custom(
        resource("cert-manager.io", "v1", ResourceScope::Cluster),
        None,
        "tls".to_owned(),
    );
    assert!(fits(cluster_scoped).is_none());
    let mut other_kind = resource("cert-manager.io", "v1", ResourceScope::Namespaced);
    other_kind.plural = "issuers".to_owned();
    other_kind.kind = "Issuer".to_owned();
    assert!(
        fits(ObjectRef::custom(
            other_kind,
            Some("shop".to_owned()),
            "tls".to_owned()
        ))
        .is_none()
    );
    let builtin = ObjectRef::new(
        ObjectKind::Deployment,
        Some("shop".to_owned()),
        "tls".to_owned(),
    );
    assert!(fits(builtin).is_none());
}

#[test]
fn other_custom_targets_fit_no_operation() {
    let operations = [
        WriteOperation::DeleteObject {
            uid: "uid-1".to_owned(),
            propagation: DeletePropagation::Background,
        },
        WriteOperation::SetNodeSchedulable { schedulable: false },
        WriteOperation::ScaleWorkload {
            replicas: 1,
            previous: 2,
        },
        WriteOperation::RestartRollout {
            restarted_at: requested_at(),
        },
        WriteOperation::TriggerCronJob,
        WriteOperation::RerunJob,
        WriteOperation::SetDefaultStorageClass { is_default: true },
    ];
    for operation in operations {
        assert!(
            WriteRequest::new(certificate_ref(), operation.clone()).is_none(),
            "{operation:?} on the certificate"
        );
        let other = ObjectRef::custom(
            resource("argoproj.io", "v1alpha1", ResourceScope::Namespaced),
            Some("shop".to_owned()),
            "app".to_owned(),
        )
        .expect("a namespaced resource has a namespace");
        assert!(WriteRequest::new(other, operation).is_none());
    }
}

#[test]
fn renew_changed_field_is_the_issuing_condition() {
    let fields = request().changed_fields();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].path, "status.conditions[Issuing]");
    assert_eq!(fields[0].value.as_deref(), Some("True (ManuallyTriggered)"));
    assert!(request().supports_dry_run());
}
