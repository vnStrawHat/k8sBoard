use serde_json::json;

use super::*;
use crate::edit_placeholders;
use crate::fake_api::FakeApi;
use crate::object_edit::EditBase;
use crate::object_write::WritePolicy;
use crate::object_yaml::{EnvValues, ObjectRef};

fn paths(before: &Value, after: &Value) -> Vec<String> {
    field_paths(before, after)
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn display_quotes_only_keys_that_need_it() {
    let path = FieldPath::new(vec![
        PathSegment::Key("spec".to_owned()),
        PathSegment::Key("template".to_owned()),
        PathSegment::Key("containers".to_owned()),
        PathSegment::Name("api".to_owned()),
        PathSegment::Key("env".to_owned()),
        PathSegment::Index(3),
        PathSegment::Key("value".to_owned()),
    ]);
    assert_eq!(
        path.to_string(),
        "spec.template.containers[api].env[3].value"
    );
    let labelled = FieldPath::new(vec![
        PathSegment::Key("metadata".to_owned()),
        PathSegment::Key("labels".to_owned()),
        PathSegment::Key("app.kubernetes.io/name".to_owned()),
    ]);
    assert_eq!(
        labelled.to_string(),
        r#"metadata.labels["app.kubernetes.io/name"]"#
    );
    assert_eq!(
        FieldPath::new(vec![PathSegment::Key("a b".to_owned())]).to_string(),
        r#"["a b"]"#
    );
}

#[test]
fn equal_trees_have_no_paths() {
    let value = json!({"a": [1, 2], "b": {"c": "d"}});
    assert!(field_paths(&value, &value).is_empty());
}

#[test]
fn scalars_and_added_or_removed_keys_are_one_path_each() {
    let before = json!({"spec": {"replicas": 3, "gone": {"deep": 1}, "same": true}});
    let after = json!({"spec": {"replicas": 5, "new": [1], "same": true}});
    assert_eq!(
        paths(&before, &after),
        ["spec.gone", "spec.new", "spec.replicas"]
    );
}

#[test]
fn a_type_change_is_one_path_at_its_root() {
    let before = json!({"a": {"x": 1}});
    let after = json!({"a": "text"});
    assert_eq!(paths(&before, &after), ["a"]);
}

#[test]
fn named_lists_are_compared_by_name() {
    let before = json!({"containers": [
        {"name": "api", "image": "a:1"}, {"name": "old", "image": "o:1"},
    ]});
    let after = json!({"containers": [
        {"name": "api", "image": "a:2"}, {"name": "new", "image": "n:1"},
    ]});
    assert_eq!(
        paths(&before, &after),
        [
            "containers[api].image",
            "containers[old]",
            "containers[new]"
        ]
    );
}

#[test]
fn reordering_a_named_list_is_a_change_of_the_list() {
    let before = json!({"containers": [{"name": "a"}, {"name": "b"}]});
    let after = json!({"containers": [{"name": "b"}, {"name": "a"}]});
    assert_eq!(paths(&before, &after), ["containers"]);
}

#[test]
fn index_lists_compare_by_position_and_length() {
    let before = json!({"args": ["a", "b", "c"], "ports": [{"port": 80}, {"port": 81}]});
    let same_length = json!({"args": ["a", "x", "c"], "ports": [{"port": 80}, {"port": 82}]});
    assert_eq!(paths(&before, &same_length), ["args[1]", "ports[1].port"]);
    let shorter = json!({"args": ["a"], "ports": [{"port": 80}, {"port": 81}]});
    assert_eq!(paths(&before, &shorter), ["args"]);
}

#[test]
fn duplicate_names_fall_back_to_the_index() {
    let before = json!({"env": [{"name": "X", "value": "1"}, {"name": "X", "value": "2"}]});
    let after = json!({"env": [{"name": "X", "value": "1"}, {"name": "X", "value": "3"}]});
    assert_eq!(paths(&before, &after), ["env[1].value"]);
}

#[test]
fn changes_hold_cut_text_and_container_markers() {
    let long = "x".repeat(200);
    let before = json!({"a": "short", "b": {"k": 1}, "c": [1]});
    let after = json!({"a": long, "b": 7, "d": true});
    let found = field_paths(&before, &after);
    let (changes, more) = field_changes(&before, &after, &found);
    assert_eq!(more, 0);
    let by_path = |path: &str| {
        changes
            .iter()
            .find(|change| change.path.to_string() == path)
            .expect("a change")
    };
    assert_eq!(by_path("a").old.as_deref(), Some("short"));
    assert_eq!(
        by_path("a").new.as_deref().map(|text| text.chars().count()),
        Some(81)
    );
    assert_eq!(by_path("b").old.as_deref(), Some("{\u{2026}}"));
    assert_eq!(by_path("b").new.as_deref(), Some("7"));
    assert_eq!(by_path("c").old.as_deref(), Some("[\u{2026}]"));
    assert_eq!(by_path("c").new, None);
    assert_eq!(by_path("d").old, None);
    assert_eq!(by_path("d").new.as_deref(), Some("true"));
}

#[tokio::test]
async fn changes_are_capped() {
    let before: Value = (0..250)
        .map(|index| (format!("k{index:03}"), json!(0)))
        .collect();
    let after: Value = (0..250)
        .map(|index| (format!("k{index:03}"), json!(1)))
        .collect();
    let found = field_paths(&before, &after);
    let (changes, more) = field_changes(&before, &after, &found);
    assert_eq!(found.len(), 250);
    assert_eq!(changes.len(), MAX_CHANGES);
    assert_eq!(more, 50);
}

fn deployment() -> Value {
    json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "name": "api", "namespace": "payments", "uid": "uid-1", "resourceVersion": "100",
            "annotations": {"kubectl.kubernetes.io/last-applied-configuration": "{}"},
        },
        "spec": {
            "replicas": 3,
            "strategy": {"type": "Recreate"},
            "template": {"spec": {"containers": [
                {"name": "api", "image": "api:1", "env": [{"name": "DB_PASS", "value": "s3cr3t-env"}]},
            ]}},
        },
        "status": {"readyReplicas": 3},
    })
}

fn config_map() -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": {"name": "settings", "namespace": "payments", "uid": "uid-3", "resourceVersion": "9"},
        "data": {"mode": "420"},
    })
}

/// The base the editor would open for `object`, read through the fake transport.
async fn base_of(kind: ObjectKind, name: &str, object: Value) -> EditBase {
    let target =
        ObjectRef::new(kind, Some("payments".to_owned()), name.to_owned()).expect("namespaced");
    let body = object.to_string();
    let (connection, _api) =
        FakeApi::connection(WritePolicy::Blocked, move |_| (200, body.clone()));
    connection
        .edit_base(&target, EnvValues::Hidden)
        .await
        .expect("a valid object")
}

fn edit_of(base: &EditBase, from: &str, to: &str) -> ObjectEdit {
    ObjectEdit::new(base, &base.text().replacen(from, to, 1)).expect("an applicable edit")
}

/// The preview of `edit` when the server accepts it unchanged: the response is the restored body.
fn preview_of(fresh: &Value, edit: &ObjectEdit) -> EditPreview {
    let mut body = edit.edited().clone();
    let restored = edit_placeholders::restore(&mut body, fresh).expect("restores");
    build_preview(edit, fresh.clone(), body, &restored).expect("a preview")
}

#[tokio::test]
async fn preview_masks_both_sides() {
    let fresh = deployment();
    let base = base_of(ObjectKind::Deployment, "api", fresh.clone()).await;
    let edit = edit_of(&base, "value: <hidden>", "value: replaced-literal");
    let preview = preview_of(&fresh, &edit);
    for text in [&preview.before, &preview.after] {
        assert!(!text.contains("s3cr3t-env"), "{text}");
        assert!(!text.contains("replaced-literal"), "{text}");
        assert!(!text.starts_with('#'), "{text}");
    }
    assert!(preview.after.contains("<hidden, changed>"));
    assert!(preview.before.contains("value: <hidden>"));
    assert!(!preview.after.contains("status"));
}

#[tokio::test]
async fn preview_lists_field_changes() {
    let fresh = deployment();
    let base = base_of(ObjectKind::Deployment, "api", fresh.clone()).await;
    let preview = preview_of(&fresh, &edit_of(&base, "replicas: 3", "replicas: 5"));
    assert_eq!(preview.more_changes, 0);
    assert_eq!(preview.changes.len(), 1);
    assert_eq!(preview.changes[0].path.to_string(), "spec.replicas");
    assert_eq!(preview.changes[0].old.as_deref(), Some("3"));
    assert_eq!(preview.changes[0].new.as_deref(), Some("5"));
}

#[tokio::test]
async fn preview_checks() {
    let fresh = deployment();
    let base = base_of(ObjectKind::Deployment, "api", fresh.clone()).await;
    let preview = preview_of(&fresh, &edit_of(&base, "image: api:1", "image: api:2"));
    assert_eq!(
        preview.checks,
        [
            EditCheck::Rollout {
                strategy: "Recreate".to_owned()
            },
            EditCheck::StaleLastApplied,
        ]
    );
}

#[tokio::test]
async fn a_plain_config_map_has_no_checks() {
    let fresh = config_map();
    let base = base_of(ObjectKind::ConfigMap, "settings", fresh.clone()).await;
    let preview = preview_of(&fresh, &edit_of(&base, "mode: \"420\"", "mode: \"493\""));
    assert!(preview.checks.is_empty(), "{:?}", preview.checks);
}

#[tokio::test]
async fn rollout_defaults_to_rolling_update_and_ignores_other_changes() {
    let mut fresh = deployment();
    fresh["spec"]
        .as_object_mut()
        .expect("spec")
        .remove("strategy");
    let base = base_of(ObjectKind::Deployment, "api", fresh.clone()).await;
    let template = preview_of(&fresh, &edit_of(&base, "image: api:1", "image: api:2"));
    assert_eq!(
        template.checks[0],
        EditCheck::Rollout {
            strategy: "RollingUpdate".to_owned()
        }
    );
    let scale = preview_of(&fresh, &edit_of(&base, "replicas: 3", "replicas: 4"));
    assert!(
        !scale
            .checks
            .iter()
            .any(|check| matches!(check, EditCheck::Rollout { .. }))
    );
}

#[tokio::test]
async fn moved_placeholders_are_marked_and_checked() {
    let mut fresh = deployment();
    fresh["spec"]["template"]["spec"]["containers"][0]["env"] = json!([
        {"name": "X", "value": "raw-1", "tag": "one"},
        {"name": "X", "value": "raw-2", "tag": "two"},
    ]);
    let base = base_of(ObjectKind::Deployment, "api", fresh.clone()).await;
    let mut text: Value = serde_saphyr::from_str(base.text()).expect("parses");
    text["spec"]["template"]["spec"]["containers"][0]["env"]
        .as_array_mut()
        .expect("a list")
        .reverse();
    let edit = ObjectEdit::new(&base, &serde_saphyr::to_string(&text).expect("serializes"))
        .expect("an applicable edit");
    let preview = preview_of(&fresh, &edit);
    assert!(
        preview.after.contains("<hidden, moved>"),
        "{}",
        preview.after
    );
    let moved = preview
        .checks
        .iter()
        .filter(|check| matches!(check, EditCheck::Moved { .. }))
        .count();
    assert_eq!(moved, 2);
}

#[tokio::test]
async fn leading_zero_lines_become_checks() {
    let fresh = config_map();
    let base = base_of(ObjectKind::ConfigMap, "settings", fresh.clone()).await;
    let preview = preview_of(&fresh, &edit_of(&base, "data:\n", "data:\n  octal: 0755\n"));
    assert!(
        preview
            .checks
            .iter()
            .any(|check| matches!(check, EditCheck::LeadingZero { .. })),
        "{:?}",
        preview.checks
    );
}

#[test]
fn debug_shows_counts_only() {
    let preview = EditPreview {
        before: "before-distinctive".to_owned(),
        after: "after-distinctive".to_owned(),
        changes: Vec::new(),
        more_changes: 2,
        checks: vec![EditCheck::StaleLastApplied],
    };
    assert_eq!(
        format!("{preview:?}"),
        "EditPreview { changes: 0, more_changes: 2, checks: 1 }"
    );
}
