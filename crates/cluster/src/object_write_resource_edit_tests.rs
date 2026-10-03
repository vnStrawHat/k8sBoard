//! Spec 0032b: HPA min / max, PVC Expand, and Set default StorageClass.

use serde_json::{Value, json};

use super::*;
use crate::fake_api::{FakeApi, RecordedRequest};

const ANSWER: &str = r#"{"apiVersion":"v1","kind":"Any","metadata":{"name":"x"}}"#;
const GA_KEY: &str = "storageclass.kubernetes.io/is-default-class";
const BETA_KEY: &str = "storageclass.beta.kubernetes.io/is-default-class";

fn hpa() -> ObjectRef {
    ObjectRef::new(
        ObjectKind::HorizontalPodAutoscaler,
        Some("web".to_owned()),
        "frontend-hpa".to_owned(),
    )
    .expect("an HPA has a namespace")
}

fn claim() -> ObjectRef {
    ObjectRef::new(
        ObjectKind::PersistentVolumeClaim,
        Some("kafka".to_owned()),
        "data-kafka-0".to_owned(),
    )
    .expect("a claim has a namespace")
}

fn class(name: &str) -> ObjectRef {
    ObjectRef::new(ObjectKind::StorageClass, None, name.to_owned()).expect("a class has none")
}

fn hpa_range(min: u32, max: u32) -> Option<WriteRequest> {
    WriteRequest::new(hpa(), WriteOperation::SetHpaReplicaRange { min, max })
}

fn expand(storage: &str) -> Option<WriteRequest> {
    WriteRequest::new(
        claim(),
        WriteOperation::ExpandClaim {
            storage: storage.to_owned(),
        },
    )
}

fn set_default(name: &str, is_default: bool) -> WriteRequest {
    WriteRequest::new(
        class(name),
        WriteOperation::SetDefaultStorageClass { is_default },
    )
    .expect("a class fits the operation")
}

fn accepting() -> (ClusterConnection, FakeApi) {
    FakeApi::connection(WritePolicy::Allowed, |_| (200, ANSWER.to_owned()))
}

fn body_of(request: &RecordedRequest) -> Value {
    serde_json::from_str(&request.body).expect("the body is JSON")
}

async fn commit_once(request: &WriteRequest) -> RecordedRequest {
    let (connection, api) = accepting();
    connection
        .write(request, WriteMode::Commit)
        .await
        .expect("the patch is accepted");
    let mut requests = api.requests();
    assert_eq!(requests.len(), 1);
    requests.remove(0)
}

fn assert_merge_patch(request: &RecordedRequest, path: &str) {
    assert_eq!(request.method, "PATCH");
    assert_eq!(request.path, path);
    assert_eq!(
        request.content_type.as_deref(),
        Some("application/merge-patch+json")
    );
    assert!(request.has_query("fieldManager", "k8sboard"), "{request:?}");
    assert!(!request.has_query_key("dryRun"), "{request:?}");
}

#[tokio::test]
async fn hpa_range_patches_both_fields() {
    let request = commit_once(&hpa_range(3, 20).expect("a valid range")).await;
    assert_merge_patch(
        &request,
        "/apis/autoscaling/v2/namespaces/web/horizontalpodautoscalers/frontend-hpa",
    );
    assert_eq!(
        body_of(&request),
        json!({ "spec": { "minReplicas": 3, "maxReplicas": 20 } })
    );
}

#[test]
fn hpa_range_rejects_zero_inverted_and_oversized() {
    assert!(hpa_range(0, 5).is_none());
    assert!(hpa_range(5, 3).is_none());
    assert!(hpa_range(1, 1).is_some());
    assert!(hpa_range(1, MAX_REPLICAS).is_some());
    assert!(hpa_range(1, MAX_REPLICAS + 1).is_none());
    assert!(hpa_range(MAX_REPLICAS + 1, MAX_REPLICAS + 2).is_none());
}

#[tokio::test]
async fn expand_patches_the_storage_request() {
    let request = commit_once(&expand("150Gi").expect("a valid size")).await;
    assert_merge_patch(
        &request,
        "/api/v1/namespaces/kafka/persistentvolumeclaims/data-kafka-0",
    );
    assert_eq!(
        body_of(&request),
        json!({ "spec": { "resources": { "requests": { "storage": "150Gi" } } } })
    );
}

#[tokio::test]
async fn expand_trims_whitespace() {
    let trimmed = expand(" 150Gi ").expect("a valid size");
    assert_eq!(
        trimmed.operation(),
        &WriteOperation::ExpandClaim {
            storage: "150Gi".to_owned()
        }
    );
    let request = commit_once(&trimmed).await;
    assert_eq!(
        body_of(&request)["spec"]["resources"]["requests"]["storage"],
        "150Gi"
    );
    assert!(expand("  ").is_none());
}

#[test]
fn expand_rejects_bad_quantities() {
    for storage in [
        "",
        "abc",
        "0",
        "0Gi",
        "-5Gi",
        "5 Gi",
        "1.5.2Gi",
        "99999999999999999999Ei",
    ] {
        assert!(expand(storage).is_none(), "{storage:?}");
    }
    for storage in ["1", "100Gi", "1.5Gi", "2Ti", "129e6", "512Mi"] {
        assert!(expand(storage).is_some(), "{storage:?}");
    }
}

#[tokio::test]
async fn set_default_true_sets_the_ga_annotation() {
    let request = commit_once(&set_default("gp3", true)).await;
    assert_merge_patch(&request, "/apis/storage.k8s.io/v1/storageclasses/gp3");
    assert_eq!(
        body_of(&request),
        json!({ "metadata": { "annotations": { GA_KEY: "true" } } })
    );
}

#[tokio::test]
async fn set_default_false_clears_the_beta_annotation() {
    let request = commit_once(&set_default("io2", false)).await;
    assert_merge_patch(&request, "/apis/storage.k8s.io/v1/storageclasses/io2");
    assert_eq!(
        body_of(&request),
        json!({ "metadata": { "annotations": { GA_KEY: "false", BETA_KEY: null } } })
    );
    assert!(request.body.contains("null"));
}

#[tokio::test]
async fn default_class_patch_is_cluster_scoped() {
    let request = commit_once(&set_default("gp3", true)).await;
    assert!(!request.path.contains("namespaces"), "{request:?}");
    let namespaced = ObjectRef::new(
        ObjectKind::StorageClass,
        Some("web".to_owned()),
        "gp3".to_owned(),
    );
    assert!(namespaced.is_none());
}

#[tokio::test]
async fn dry_run_sends_dry_run_all() {
    let requests = [
        hpa_range(3, 20).expect("valid"),
        expand("150Gi").expect("valid"),
        set_default("gp3", true),
        set_default("io2", false),
    ];
    for request in requests {
        let (connection, api) = accepting();
        connection
            .write(&request, WriteMode::DryRun)
            .await
            .expect("the dry-run is accepted");
        let sent = api.requests();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].has_query("dryRun", "All"), "{request:?}");
        assert!(sent[0].has_query("fieldManager", "k8sboard"), "{request:?}");
        let outcome = connection
            .write(&request, WriteMode::Commit)
            .await
            .expect("the commit is accepted");
        assert_eq!(outcome.effect, WriteEffect::Patched);
    }
}

async fn refused_commit(
    request: &WriteRequest,
    code: u16,
    message: &str,
    field: Option<&str>,
) -> WriteError {
    let causes: Vec<Value> = field
        .iter()
        .map(|field| json!({ "reason": "FieldValueForbidden", "message": "x", "field": field }))
        .collect();
    let body = json!({
        "kind": "Status", "apiVersion": "v1", "status": "Failure", "message": message,
        "reason": "Reason", "details": { "causes": causes }, "code": code,
    })
    .to_string();
    let (connection, _api) =
        FakeApi::connection(WritePolicy::Allowed, move |_| (code, body.clone()));
    connection
        .write(request, WriteMode::Commit)
        .await
        .expect_err("the server refused")
}

#[tokio::test]
async fn expand_admission_refusal_is_invalid() {
    let message = "persistentvolumeclaims \"data-kafka-0\" is forbidden: only dynamically provisioned pvc can be resized and the storageclass that provisions the pvc must support resize";
    let error = refused_commit(&expand("150Gi").expect("valid"), 403, message, None).await;
    assert!(
        matches!(&error, WriteError::Invalid { message: shown, .. } if shown == message),
        "{error:?}"
    );
    assert!(!error.to_string().contains("not permitted"));
}

#[tokio::test]
async fn expand_shrink_is_invalid() {
    let error = refused_commit(
        &expand("50Gi").expect("valid"),
        422,
        "PersistentVolumeClaim is invalid",
        Some("spec.resources.requests.storage"),
    )
    .await;
    assert!(
        matches!(
            &error,
            WriteError::Invalid { fields, .. }
                if fields == &["spec.resources.requests.storage".to_owned()]
        ),
        "{error:?}"
    );
}

#[test]
fn request_rejects_a_kind_that_does_not_fit() {
    let mismatches = [
        (
            claim(),
            WriteOperation::SetHpaReplicaRange { min: 1, max: 2 },
        ),
        (
            hpa(),
            WriteOperation::ExpandClaim {
                storage: "1Gi".to_owned(),
            },
        ),
        (
            hpa(),
            WriteOperation::SetDefaultStorageClass { is_default: true },
        ),
        (
            class("gp3"),
            WriteOperation::SetHpaReplicaRange { min: 1, max: 2 },
        ),
    ];
    for (target, operation) in mismatches {
        assert!(WriteRequest::new(target, operation).is_none());
    }
}

#[test]
fn access_check_matches_each_operation() {
    let hpa = hpa_range(3, 20).expect("valid");
    assert_eq!(
        hpa.access_check(),
        AccessCheck::PatchHorizontalPodAutoscalers
    );
    assert_eq!(
        hpa.access_check().to_string(),
        "patch horizontalpodautoscalers"
    );
    let claim = expand("150Gi").expect("valid");
    assert_eq!(
        claim.access_check(),
        AccessCheck::PatchPersistentVolumeClaims
    );
    assert_eq!(
        claim.access_check().to_string(),
        "patch persistentvolumeclaims"
    );
    let class = set_default("gp3", true);
    assert_eq!(class.access_check(), AccessCheck::PatchStorageClasses);
    assert_eq!(class.access_check().to_string(), "patch storageclasses");
}

fn field(path: &'static str, value: Option<&str>) -> ChangedField {
    ChangedField {
        path: path.into(),
        value: value.map(str::to_owned),
    }
}

#[test]
fn changed_fields_values() {
    assert_eq!(
        hpa_range(3, 20).expect("valid").changed_fields(),
        [
            field("spec.minReplicas", Some("3")),
            field("spec.maxReplicas", Some("20")),
        ]
    );
    assert_eq!(
        expand(" 150Gi ").expect("valid").changed_fields(),
        [field("spec.resources.requests.storage", Some("150Gi"))]
    );
    assert_eq!(
        set_default("gp3", true).changed_fields(),
        [field(
            "metadata.annotations[storageclass.kubernetes.io/is-default-class]",
            Some("true")
        )]
    );
    assert_eq!(
        set_default("io2", false).changed_fields(),
        [
            field(
                "metadata.annotations[storageclass.kubernetes.io/is-default-class]",
                Some("false")
            ),
            field(
                "metadata.annotations[storageclass.beta.kubernetes.io/is-default-class]",
                None
            ),
        ]
    );
}

#[test]
fn manual_debug_shows_names_only() {
    let hpa = hpa_range(3, 20).expect("valid");
    assert_eq!(
        format!("{hpa:?}"),
        r#"WriteRequest { operation: SetHpaReplicaRange, kind: HorizontalPodAutoscaler, namespace: Some("web"), name: "frontend-hpa" }"#
    );
    let claim = expand("150Gi").expect("valid");
    assert!(!format!("{claim:?}").contains("150Gi"));
    assert_eq!(format!("{:?}", claim.operation()), "ExpandClaim");
    assert_eq!(
        format!("{:?}", set_default("gp3", true).operation()),
        "SetDefaultStorageClass"
    );
}

#[test]
fn every_new_operation_supports_dry_run() {
    assert!(hpa_range(3, 20).expect("valid").supports_dry_run());
    assert!(expand("150Gi").expect("valid").supports_dry_run());
    assert!(set_default("gp3", true).supports_dry_run());
}

#[tokio::test]
async fn debug_build_blocks_the_new_operations() {
    let (connection, api) = FakeApi::connection(WritePolicy::resolve(true, None), |_| {
        (200, ANSWER.to_owned())
    });
    let requests = [
        hpa_range(3, 20).expect("valid"),
        expand("150Gi").expect("valid"),
        set_default("gp3", true),
    ];
    for request in &requests {
        for mode in [WriteMode::DryRun, WriteMode::Commit] {
            let error = connection
                .write(request, mode)
                .await
                .expect_err("a debug build blocks writes");
            assert!(matches!(error, WriteError::WritesBlocked), "{error:?}");
        }
    }
    assert!(api.requests().is_empty());
}

#[test]
fn operations_without_a_value_rule_pass_the_check_unchanged() {
    let operations = [
        WriteOperation::SetNodeSchedulable { schedulable: true },
        WriteOperation::ScaleWorkload { replicas: 3 },
        WriteOperation::SetRolloutPaused { paused: true },
        WriteOperation::SetCronJobSuspended { suspended: false },
        WriteOperation::TriggerCronJob,
        WriteOperation::RerunJob,
        WriteOperation::DeleteNodeShellPod {
            uid: "u".to_owned(),
        },
        WriteOperation::SetHpaReplicaRange { min: 1, max: 4 },
        WriteOperation::SetDefaultStorageClass { is_default: true },
    ];
    for operation in operations {
        assert_eq!(
            checked_operation(operation.clone()),
            Some(operation.clone()),
            "{operation:?}"
        );
    }
    // The ones with a value rule still refuse a bad value and trim a good one.
    assert_eq!(
        checked_operation(WriteOperation::SetHpaReplicaRange { min: 0, max: 4 }),
        None
    );
    assert_eq!(
        checked_operation(WriteOperation::ExpandClaim {
            storage: " 1Gi ".to_owned()
        }),
        Some(WriteOperation::ExpandClaim {
            storage: "1Gi".to_owned()
        })
    );
}
