use std::path::PathBuf;

use super::*;
use crate::cluster_registry::ClusterRef;

fn cluster() -> ClusterRef {
    ClusterRef {
        kubeconfig: PathBuf::from("/home/me/.kube/config"),
        context: "prod-ctx".to_owned(),
    }
}

fn scope(cluster: &ClusterRef) -> NodeScope<'_> {
    NodeScope {
        cluster,
        cluster_name: "prod-a",
    }
}

fn target(kind: ObjectKind) -> ObjectRef {
    ObjectRef::new(kind, Some("shop".to_owned()), "web".to_owned()).expect("a namespaced kind")
}

fn edit(labels: &[(&str, &str)], annotations: &[(&str, &str)]) -> ObjectMetadata {
    let map = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    };
    ObjectMetadata {
        labels: map(labels),
        annotations: map(annotations),
        hidden_annotations: Vec::new(),
    }
}

fn row(key: &str, value: &str) -> MetadataRow {
    MetadataRow {
        key: key.to_owned(),
        value: value.to_owned(),
    }
}

fn intent(
    kind: ObjectKind,
    edit: &ObjectMetadata,
    labels: &[MetadataRow],
    annotations: &[MetadataRow],
) -> Result<WriteIntent, SharedString> {
    let cluster = cluster();
    metadata_intent(
        &scope(&cluster),
        kind,
        &target(kind),
        edit,
        labels,
        annotations,
    )
}

fn lines(intent: &WriteIntent) -> Vec<String> {
    intent
        .change_lines
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn an_added_annotation_reads_none_to_the_new_value() {
    let current = edit(&[], &[]);
    let sent = intent(
        ObjectKind::Deployment,
        &current,
        &[],
        &[row("note", "blue-green")],
    )
    .expect("a change");
    assert_eq!(
        lines(&sent),
        ["metadata.annotations.note: (none) → blue-green"]
    );
    assert_eq!(sent.button, "Apply changes");
    assert_eq!(sent.label, "Edit labels / annotations of deployment web");
}

#[test]
fn a_changed_and_a_removed_label_show_old_and_new() {
    let current = edit(&[("tier", "web"), ("old", "x")], &[]);
    let sent =
        intent(ObjectKind::Deployment, &current, &[row("tier", "api")], &[]).expect("a change");
    assert_eq!(
        lines(&sent),
        [
            "metadata.labels.old: x → (removed)",
            "metadata.labels.tier: web → api"
        ]
    );
}

#[test]
fn a_key_with_a_dot_or_a_slash_is_bracketed() {
    let current = edit(&[], &[]);
    let sent = intent(
        ObjectKind::Pod,
        &current,
        &[],
        &[row("example.com/owner", "infra")],
    )
    .expect("a change");
    assert_eq!(
        lines(&sent),
        ["metadata.annotations[example.com/owner]: (none) → infra"]
    );
}

#[test]
fn unchanged_rows_are_not_sent_and_no_change_keeps_review_off() {
    let current = edit(&[("app", "web")], &[("note", "a b")]);
    let same = intent(
        ObjectKind::Deployment,
        &current,
        &[row("app", "web")],
        &[row("note", "a b")],
    );
    assert_eq!(same.err().as_deref(), Some("No changes"));
}

#[test]
fn only_changed_keys_go_into_the_patch() {
    let current = edit(&[("app", "web")], &[("note", "a")]);
    let sent = intent(
        ObjectKind::Deployment,
        &current,
        &[row("app", "web")],
        &[row("note", "b"), row("owner", "me")],
    )
    .expect("a change");
    let cluster::WriteOperation::SetObjectMetadata {
        labels,
        annotations,
    } = sent.request.operation()
    else {
        panic!("a metadata edit");
    };
    assert!(labels.is_empty());
    let keys: Vec<&str> = annotations
        .iter()
        .map(|change| change.key.as_str())
        .collect();
    assert_eq!(keys, ["note", "owner"]);
}

#[test]
fn a_label_value_is_trimmed_and_an_annotation_is_kept_as_typed() {
    let current = edit(&[], &[]);
    let sent = intent(
        ObjectKind::Deployment,
        &current,
        &[row(" app ", " web ")],
        &[row("note", " two  spaces ")],
    )
    .expect("a change");
    let cluster::WriteOperation::SetObjectMetadata {
        labels,
        annotations,
    } = sent.request.operation()
    else {
        panic!("a metadata edit");
    };
    assert_eq!(labels[0].key, "app");
    assert_eq!(labels[0].value.as_deref(), Some("web"));
    assert_eq!(annotations[0].value.as_deref(), Some(" two  spaces "));
}

#[test]
fn a_credential_looking_annotation_never_shows_its_value() {
    let current = edit(&[], &[("deploy-token", "old-secret")]);
    let sent = intent(
        ObjectKind::Deployment,
        &current,
        &[],
        &[row("deploy-token", "new-secret")],
    )
    .expect("a change");
    let text = lines(&sent).join("\n");
    assert_eq!(
        text,
        "metadata.annotations.deploy-token: (hidden) → (hidden)"
    );
    assert!(!text.contains("secret"));
}

#[test]
fn a_long_value_is_cut_in_the_change_line() {
    let current = edit(&[], &[]);
    let long = "x".repeat(100);
    let sent =
        intent(ObjectKind::Deployment, &current, &[], &[row("note", &long)]).expect("a change");
    let line = &lines(&sent)[0];
    assert!(line.ends_with('…'));
    assert!(!line.contains(&"x".repeat(61)));
}

#[test]
fn problems_name_the_row() {
    let current = edit(&[], &[]);
    let empty = intent(ObjectKind::Pod, &current, &[row("  ", "v")], &[]);
    assert_eq!(empty.err().as_deref(), Some("Enter a key for every label"));
    let empty = intent(ObjectKind::Pod, &current, &[], &[row("", "v")]);
    assert_eq!(
        empty.err().as_deref(),
        Some("Enter a key for every annotation")
    );
    let twice = intent(
        ObjectKind::Pod,
        &current,
        &[row("a", "1"), row("a", "2")],
        &[],
    );
    assert_eq!(twice.err().as_deref(), Some("a is listed twice"));
    let bad_key = intent(ObjectKind::Pod, &current, &[], &[row("bad key", "v")]);
    assert!(
        bad_key
            .err()
            .is_some_and(|text| text.starts_with("Row 1: key"))
    );
    let bad_value = intent(ObjectKind::Pod, &current, &[row("app", "has space")], &[]);
    assert!(
        bad_value
            .err()
            .is_some_and(|text| text.starts_with("Row 1: value"))
    );
}

#[test]
fn an_annotation_value_may_be_any_text() {
    let current = edit(&[], &[]);
    assert!(
        intent(
            ObjectKind::Pod,
            &current,
            &[],
            &[row("note", "any text: with, punctuation & spaces")]
        )
        .is_ok()
    );
    assert!(metadata_row_problem(MetadataList::Annotations, &[row("note", "a b c")]).is_none());
}

#[test]
fn annotations_over_the_size_limit_are_refused() {
    let current = edit(&[], &[]);
    let big = "x".repeat(300 * 1024);
    let refused = intent(ObjectKind::Pod, &current, &[], &[row("big", &big)]);
    assert!(refused.err().is_some_and(|text| text.contains("256 KiB")));
}

#[test]
fn a_pod_label_change_warns_that_controllers_match_by_label() {
    let current = edit(&[("app", "web")], &[]);
    let pod = intent(ObjectKind::Pod, &current, &[row("app", "api")], &[]).expect("a change");
    assert_eq!(pod.warnings.len(), 1);
    // An annotation never detaches a pod, and a workload's labels do not select pods.
    let annotation = intent(
        ObjectKind::Pod,
        &current,
        &[row("app", "web")],
        &[row("a", "b")],
    )
    .expect("a change");
    assert!(annotation.warnings.is_empty());
    let workload =
        intent(ObjectKind::Deployment, &current, &[row("app", "api")], &[]).expect("a change");
    assert!(workload.warnings.is_empty());
}

#[test]
fn the_intent_is_a_change_tier_edit_of_the_object_in_its_own_cluster() {
    let current = edit(&[], &[]);
    let sent = intent(ObjectKind::StatefulSet, &current, &[row("a", "b")], &[]).expect("a change");
    assert_eq!(
        sent.action,
        ResourceAction::EditMetadata(ObjectKind::StatefulSet)
    );
    assert_eq!(sent.cluster, cluster());
    assert_eq!(sent.cluster_name, "prod-a");
    assert_eq!(sent.risk, crate::write_guard::ActionRisk::Change);
    assert_eq!(sent.request.target().name(), "web");
}

#[test]
fn rows_follow_the_key_order() {
    let rows = metadata_rows(&edit(&[("b", "2"), ("a", "1")], &[]).labels);
    assert_eq!(rows, [row("a", "1"), row("b", "2")]);
}
