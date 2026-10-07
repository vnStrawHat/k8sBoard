//! Spec 0032b, UX round 3: a claim recreated with another class, and a volume's reclaim policy.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use super::*;
use crate::fake_api::{FakeApi, RecordedRequest};

const CLAIM_PATH: &str = "/api/v1/namespaces/lab-broken/persistentvolumeclaims/pvc-wrong-class";
const CLAIMS_PATH: &str = "/api/v1/namespaces/lab-broken/persistentvolumeclaims";
const NOT_FOUND: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"gone","reason":"NotFound","code":404}"#;
const QUOTA: &str = r#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"exceeded quota: compute","reason":"Forbidden","code":403}"#;

fn claim_json(phase: &str) -> String {
    json!({
        "apiVersion": "v1", "kind": "PersistentVolumeClaim",
        "metadata": { "name": "pvc-wrong-class", "namespace": "lab-broken", "uid": "claim-uid" },
        "spec": {
            "accessModes": ["ReadWriteOnce"],
            "resources": { "requests": { "storage": "100Mi" } },
            "storageClassName": "missing",
        },
        "status": { "phase": phase },
    })
    .to_string()
}

fn claim() -> ObjectRef {
    ObjectRef::new(
        ObjectKind::PersistentVolumeClaim,
        Some("lab-broken".to_owned()),
        "pvc-wrong-class".to_owned(),
    )
    .expect("a claim has a namespace")
}

fn recreate(uid: &str, class: &str) -> Option<WriteRequest> {
    WriteRequest::new(
        claim(),
        WriteOperation::RecreateClaim {
            uid: uid.to_owned(),
            storage_class: class.to_owned(),
            previous_class: Some("missing".to_owned()),
        },
    )
}

/// The claim until a real DELETE arrives, then nothing; `create` answers the POST.
fn recreating(phase: &'static str, create: (u16, &'static str)) -> (ClusterConnection, FakeApi) {
    let is_deleted = Arc::new(AtomicBool::new(false));
    FakeApi::connection(
        WritePolicy::Allowed,
        move |request: &RecordedRequest| match request.method.as_str() {
            "GET" if is_deleted.load(Ordering::SeqCst) => (404, NOT_FOUND.to_owned()),
            "GET" => (200, claim_json(phase)),
            "DELETE" => {
                if !request.body.contains("\"dryRun\"") {
                    is_deleted.store(true, Ordering::SeqCst);
                }
                (200, claim_json(phase))
            }
            "POST" => (create.0, create.1.to_owned()),
            _ => (404, NOT_FOUND.to_owned()),
        },
    )
}

fn methods(api: &FakeApi) -> Vec<String> {
    api.requests()
        .into_iter()
        .map(|request| format!("{} {}", request.method, request.path))
        .collect()
}

#[test]
fn a_recreate_needs_a_uid_and_a_class_name_the_api_accepts() {
    assert!(recreate("claim-uid", "standard").is_some());
    assert!(recreate("", "standard").is_none());
    assert!(recreate("claim-uid", "").is_none());
    assert!(recreate("claim-uid", "Not A Class").is_none());
    let request = recreate("claim-uid", "standard").expect("a request");
    assert_eq!(
        request.access_check(),
        AccessCheck::Delete(ObjectKind::PersistentVolumeClaim)
    );
    let fields: Vec<_> = request
        .changed_fields()
        .into_iter()
        .map(|field| (field.path.into_owned(), field.value, field.from))
        .collect();
    assert_eq!(
        fields,
        [(
            "spec.storageClassName".to_owned(),
            Some("standard".to_owned()),
            Some("missing".to_owned())
        )]
    );
}

#[tokio::test]
async fn a_dry_run_reads_the_claim_and_dry_runs_the_delete_only() {
    let (connection, api) = recreating("Pending", (201, "{}"));
    let request = recreate("claim-uid", "standard").expect("a request");
    connection
        .write(&request, WriteMode::DryRun)
        .await
        .expect("the check passes");
    assert_eq!(
        methods(&api),
        [format!("GET {CLAIM_PATH}"), format!("DELETE {CLAIM_PATH}")]
    );
    let delete = &api.requests()[1];
    let body: Value = serde_json::from_str(&delete.body).expect("a delete body");
    assert_eq!(body["dryRun"], json!(["All"]));
    assert_eq!(body["preconditions"]["uid"], "claim-uid");
}

#[tokio::test]
async fn a_commit_deletes_waits_until_the_claim_is_gone_and_creates_it_with_the_class() {
    let created = r#"{"apiVersion":"v1","kind":"PersistentVolumeClaim","metadata":{"name":"pvc-wrong-class","namespace":"lab-broken","uid":"new-uid"}}"#;
    let (connection, api) = recreating("Pending", (201, created));
    let request = recreate("claim-uid", "standard").expect("a request");
    let outcome = connection
        .write(&request, WriteMode::Commit)
        .await
        .expect("the claim is recreated");
    assert_eq!(outcome.effect, WriteEffect::Created);
    assert_eq!(outcome.created_name, None);
    assert_eq!(outcome.uid.as_deref(), Some("new-uid"));
    assert_eq!(
        methods(&api),
        [
            format!("GET {CLAIM_PATH}"),
            format!("DELETE {CLAIM_PATH}"),
            format!("GET {CLAIM_PATH}"),
            format!("POST {CLAIMS_PATH}"),
        ]
    );
    let post: Value = serde_json::from_str(&api.requests()[3].body).expect("a body");
    assert_eq!(post["spec"]["storageClassName"], "standard");
    assert_eq!(post["spec"]["resources"]["requests"]["storage"], "100Mi");
    assert_eq!(post["spec"]["accessModes"], json!(["ReadWriteOnce"]));
    assert!(post["metadata"].get("uid").is_none());
}

#[tokio::test]
async fn a_bound_claim_is_refused_before_anything_is_deleted() {
    let (connection, api) = recreating("Bound", (201, "{}"));
    let request = recreate("claim-uid", "standard").expect("a request");
    let error = connection
        .write(&request, WriteMode::Commit)
        .await
        .expect_err("a bound claim is refused");
    assert!(
        matches!(&error, WriteError::Invalid { message, .. } if message.contains("no volume is bound")),
        "{error}"
    );
    assert_eq!(methods(&api), [format!("GET {CLAIM_PATH}")]);
}

#[tokio::test]
async fn a_replaced_claim_is_refused() {
    let (connection, api) = recreating("Pending", (201, "{}"));
    let request = recreate("another-uid", "standard").expect("a request");
    let error = connection
        .write(&request, WriteMode::Commit)
        .await
        .expect_err("the uid differs");
    assert!(
        matches!(&error, WriteError::Invalid { message, .. } if message.contains("replaced")),
        "{error}"
    );
    assert_eq!(api.requests().len(), 1);
}

#[tokio::test]
async fn a_create_that_fails_after_the_delete_says_the_claim_is_gone() {
    let (connection, api) = recreating("Pending", (403, QUOTA));
    let request = recreate("claim-uid", "standard").expect("a request");
    let error = connection
        .write(&request, WriteMode::Commit)
        .await
        .expect_err("the create is refused");
    let text = error.to_string();
    assert!(
        text.contains("claim pvc-wrong-class was deleted, but creating it again failed"),
        "{text}"
    );
    assert_eq!(api.requests().len(), 4);
}

fn volume() -> ObjectRef {
    ObjectRef::new(
        ObjectKind::PersistentVolume,
        None,
        "pvc-e7e63176".to_owned(),
    )
    .expect("a volume has no namespace")
}

#[tokio::test]
async fn a_reclaim_policy_is_one_merge_patch_of_the_volume() {
    let request = WriteRequest::new(
        volume(),
        WriteOperation::SetReclaimPolicy {
            policy: ReclaimPolicy::Retain,
            previous: ReclaimPolicy::Delete,
        },
    )
    .expect("a volume fits the operation");
    assert_eq!(
        request.access_check(),
        AccessCheck::Patch(ObjectKind::PersistentVolume)
    );
    let fields = request.changed_fields();
    assert_eq!(fields[0].path, "spec.persistentVolumeReclaimPolicy");
    assert_eq!(fields[0].value.as_deref(), Some("Retain"));
    assert_eq!(fields[0].from.as_deref(), Some("Delete"));
    let (connection, api) = FakeApi::connection(WritePolicy::Allowed, |_| {
        (
            200,
            r#"{"apiVersion":"v1","kind":"PersistentVolume","metadata":{"name":"x"}}"#.to_owned(),
        )
    });
    connection
        .write(&request, WriteMode::Commit)
        .await
        .expect("the patch is accepted");
    let sent = &api.requests()[0];
    assert_eq!(sent.method, "PATCH");
    assert_eq!(sent.path, "/api/v1/persistentvolumes/pvc-e7e63176");
    assert_eq!(
        sent.content_type.as_deref(),
        Some("application/merge-patch+json")
    );
    assert_eq!(
        serde_json::from_str::<Value>(&sent.body).expect("a body"),
        json!({ "spec": { "persistentVolumeReclaimPolicy": "Retain" } })
    );
}

#[test]
fn a_reclaim_policy_fits_a_volume_only() {
    let operation = WriteOperation::SetReclaimPolicy {
        policy: ReclaimPolicy::Retain,
        previous: ReclaimPolicy::Delete,
    };
    assert!(WriteRequest::new(claim(), operation).is_none());
}
