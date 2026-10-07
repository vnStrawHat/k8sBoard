use serde_json::json;

use super::*;

fn set(key: &str, value: &str) -> LabelChange {
    LabelChange {
        key: key.to_owned(),
        value: Some(value.to_owned()),
    }
}

fn remove(key: &str) -> LabelChange {
    LabelChange {
        key: key.to_owned(),
        value: None,
    }
}

#[test]
fn metadata_reads_labels_and_annotations() {
    let object = json!({"metadata": {
        "labels": {"app": "web"},
        "annotations": {"team": "shop", "note": "blue"},
    }});
    let metadata = metadata_of(&object);
    assert_eq!(metadata.labels["app"], "web");
    assert_eq!(metadata.annotations.len(), 2);
    assert!(metadata.hidden_annotations.is_empty());
}

#[test]
fn an_applied_manifest_annotation_is_listed_by_key_and_never_returned() {
    let object = json!({"metadata": {"annotations": {
        "kubectl.kubernetes.io/last-applied-configuration": "{\"data\":\"s3cret\"}",
        "team": "shop",
    }}});
    let metadata = metadata_of(&object);
    assert_eq!(
        metadata.hidden_annotations,
        ["kubectl.kubernetes.io/last-applied-configuration"]
    );
    assert_eq!(metadata.annotations.keys().collect::<Vec<_>>(), ["team"]);
    assert!(!format!("{metadata:?}").contains("s3cret"));
}

#[test]
fn an_object_without_metadata_maps_reads_empty() {
    let metadata = metadata_of(&json!({"metadata": {"name": "x"}}));
    assert!(metadata.labels.is_empty() && metadata.annotations.is_empty());
}

#[test]
fn the_patch_holds_only_the_lists_that_change() {
    assert_eq!(
        metadata_patch(&[], &[set("team", "shop"), remove("old")]),
        json!({"metadata": {"annotations": {"team": "shop", "old": null}}})
    );
    assert_eq!(
        metadata_patch(&[set("app", "web")], &[]),
        json!({"metadata": {"labels": {"app": "web"}}})
    );
}

#[test]
fn valid_changes_need_a_change_and_qualified_keys() {
    assert!(are_valid_metadata_changes(
        &[],
        &[set("team", "any text, spaces ok")]
    ));
    assert!(are_valid_metadata_changes(&[remove("app")], &[]));
    assert!(!are_valid_metadata_changes(&[], &[]));
    assert!(!are_valid_metadata_changes(&[set("bad key", "v")], &[]));
    assert!(!are_valid_metadata_changes(&[], &[set("bad key", "v")]));
}

#[test]
fn a_label_value_must_be_a_label_value_but_an_annotation_value_may_be_any_text() {
    assert!(!are_valid_metadata_changes(&[set("app", "has space")], &[]));
    assert!(are_valid_metadata_changes(&[], &[set("app", "has space")]));
}

#[test]
fn a_key_listed_twice_in_one_list_is_refused() {
    assert!(!are_valid_metadata_changes(
        &[set("a", "1"), remove("a")],
        &[]
    ));
    assert!(are_valid_metadata_changes(
        &[set("a", "1")],
        &[set("a", "1")]
    ));
}

#[test]
fn annotations_stay_under_the_size_limit_and_leave_the_applied_manifest_alone() {
    let big = "x".repeat(MAX_ANNOTATION_BYTES);
    assert!(!are_valid_metadata_changes(&[], &[set("big", &big)]));
    assert!(!are_valid_metadata_changes(
        &[],
        &[remove("kubectl.kubernetes.io/last-applied-configuration")]
    ));
}
