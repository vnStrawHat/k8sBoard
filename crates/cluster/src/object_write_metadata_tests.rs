//! Tests of the labels and annotations patch (0032b) over the fake transport.

use serde_json::{Value, json};

use super::*;
use crate::fake_api::FakeApi;

fn target(kind: ObjectKind) -> ObjectRef {
    ObjectRef::new(kind, Some("shop".to_owned()), "web".to_owned()).expect("a namespaced kind")
}

fn change(key: &str, value: Option<&str>) -> LabelChange {
    LabelChange {
        key: key.to_owned(),
        value: value.map(str::to_owned),
    }
}

fn request(
    kind: ObjectKind,
    labels: Vec<LabelChange>,
    annotations: Vec<LabelChange>,
) -> Option<WriteRequest> {
    WriteRequest::new(
        target(kind),
        WriteOperation::SetObjectMetadata {
            labels,
            annotations,
        },
    )
}

#[tokio::test]
async fn labels_and_annotations_go_in_one_merge_patch() {
    let (connection, api) = FakeApi::connection(WritePolicy::Allowed, |_| {
        (
            200,
            r#"{"apiVersion":"apps/v1","kind":"Deployment","metadata":{"name":"web"}}"#.to_owned(),
        )
    });
    let sent = request(
        ObjectKind::Deployment,
        vec![change("tier", Some("web"))],
        vec![
            change("team", Some("storefront, on call")),
            change("old", None),
        ],
    )
    .expect("a deployment fits a metadata edit");
    connection
        .write(&sent, WriteMode::Commit)
        .await
        .expect("the patch is accepted");
    let requests = api.requests();
    assert_eq!(requests[0].method, "PATCH");
    assert_eq!(
        requests[0].path,
        "/apis/apps/v1/namespaces/shop/deployments/web"
    );
    assert_eq!(
        requests[0].content_type.as_deref(),
        Some("application/merge-patch+json")
    );
    let body: Value = serde_json::from_str(&requests[0].body).expect("the body is JSON");
    assert_eq!(
        body,
        json!({"metadata": {
            "labels": {"tier": "web"},
            "annotations": {"team": "storefront, on call", "old": null},
        }})
    );
}

#[test]
fn only_pods_and_workloads_take_a_metadata_edit() {
    let labels = || vec![change("tier", Some("web"))];
    for kind in [
        ObjectKind::Pod,
        ObjectKind::Deployment,
        ObjectKind::StatefulSet,
        ObjectKind::DaemonSet,
        ObjectKind::ReplicaSet,
        ObjectKind::Job,
        ObjectKind::CronJob,
    ] {
        assert!(request(kind, labels(), Vec::new()).is_some(), "{kind:?}");
    }
    for kind in [
        ObjectKind::Secret,
        ObjectKind::ConfigMap,
        ObjectKind::Service,
    ] {
        assert!(request(kind, labels(), Vec::new()).is_none(), "{kind:?}");
    }
}

#[test]
fn an_invalid_metadata_edit_is_not_a_request() {
    let kind = ObjectKind::Pod;
    assert!(request(kind, Vec::new(), Vec::new()).is_none());
    assert!(request(kind, vec![change("bad key", Some("v"))], Vec::new()).is_none());
    assert!(request(kind, vec![change("app", Some("has space"))], Vec::new()).is_none());
    assert!(request(kind, Vec::new(), vec![change("note", Some("has space"))]).is_some());
}

#[test]
fn the_audit_names_annotations_and_never_quotes_one() {
    let sent = request(
        ObjectKind::Pod,
        vec![change("tier", Some("web")), change("old", None)],
        vec![change("note", Some("hunter2"))],
    )
    .expect("a pod fits");
    let fields: Vec<(String, Option<String>)> = sent
        .changed_fields()
        .into_iter()
        .map(|field| (field.path.into_owned(), field.value))
        .collect();
    assert_eq!(
        fields,
        [
            ("metadata.labels[tier]".to_owned(), Some("web".to_owned())),
            ("metadata.labels[old]".to_owned(), None),
            ("metadata.annotations[note]".to_owned(), None),
        ]
    );
}
