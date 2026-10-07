use serde_json::json;

use super::*;
use crate::fake_api::FakeApi;
use crate::object_write::WritePolicy;

fn target(kind: ObjectKind, name: &str) -> ObjectRef {
    let namespace = kind.is_namespaced().then(|| "payments".to_owned());
    ObjectRef::new(kind, namespace, name.to_owned()).expect("the namespace fits the kind")
}

/// A Deployment as the server answers it: server fields, an env literal, an applied manifest.
fn deployment() -> Value {
    json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "name": "api",
            "namespace": "payments",
            "uid": "uid-1",
            "resourceVersion": "100",
            "generation": 4,
            "creationTimestamp": "2026-10-01T08:00:00Z",
            "managedFields": [{"manager": "kubectl-distinctive"}],
            "annotations": {
                "kubectl.kubernetes.io/last-applied-configuration": "{\"spec\":\"applied-distinctive\"}",
                "team": "payments",
            },
        },
        "spec": {
            "replicas": 3,
            "template": {"spec": {"containers": [
                {"name": "api", "image": "api:1", "env": [
                    {"name": "DB_PASS", "value": "s3cr3t-env"},
                    {"name": "MODE", "value": "fast"},
                ]},
                {"name": "sidecar", "image": "proxy:1"},
            ]}},
        },
        "status": {"readyReplicas": 3},
    })
}

fn secret() -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": {
            "name": "db", "namespace": "payments", "uid": "uid-2", "resourceVersion": "7",
            "labels": {"app": "db"},
        },
        "type": "Opaque",
        "data": {"password": "c2VjcmV0", "user": "YWRtaW4="},
    })
}

fn config_map() -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": {"name": "settings", "namespace": "payments", "uid": "uid-3", "resourceVersion": "9"},
        "data": {
            "yes": "yes", "float": "1.0", "mode": "0755", "nothing": "null", "tilde": "~",
            "exp": "1e3", "keep": "line one\nline two\n", "strip": "one\ntwo", "empty": "",
        },
    })
}

fn cron_job() -> Value {
    json!({
        "apiVersion": "batch/v1",
        "kind": "CronJob",
        "metadata": {"name": "nightly", "namespace": "payments", "uid": "uid-4", "resourceVersion": "5"},
        "spec": {
            "schedule": "0 3 * * *",
            "jobTemplate": {"spec": {"template": {"spec": {"containers": [
                {"name": "job", "image": "job:1", "env": [{"name": "TOKEN", "value": "s3cr3t-env"}]},
            ], "restartPolicy": "Never"}}}},
        },
        "status": {"lastScheduleTime": "2026-10-02T03:00:00Z"},
    })
}

fn base(kind: ObjectKind, name: &str, object: Value) -> EditBase {
    EditBase::from_object(target(kind, name), object, EnvValues::Hidden).expect("a valid object")
}

fn deployment_base() -> EditBase {
    base(ObjectKind::Deployment, "api", deployment())
}

fn edit(base: &EditBase, from: &str, to: &str) -> Result<ObjectEdit, EditError> {
    assert!(base.text().contains(from), "{from:?} not in the text");
    ObjectEdit::new(base, &base.text().replacen(from, to, 1))
}

#[test]
fn editor_text_round_trips_to_the_same_value() {
    let fixtures = [
        base(ObjectKind::Deployment, "api", deployment()),
        base(ObjectKind::ConfigMap, "settings", config_map()),
        base(ObjectKind::Secret, "db", secret()),
        base(ObjectKind::CronJob, "nightly", cron_job()),
    ];
    for fixture in fixtures {
        let parsed: Value = serde_saphyr::from_str(fixture.text()).expect("the text parses");
        assert_eq!(parsed, fixture.masked, "{fixture:?}");
    }
}

/// A ConfigMap that also has a `defaultMode`, to edit in the leading-zero tests.
fn mode_base() -> EditBase {
    let mut object = config_map();
    object["data"] = json!({"a": "1"});
    object["spec"] = json!({"defaultMode": 420});
    base(ObjectKind::ConfigMap, "settings", object)
}

#[test]
fn unquoted_leading_zero_parses_as_decimal() {
    let edited = edit(&mode_base(), "defaultMode: 420", "defaultMode: 0755").expect("a change");
    assert_eq!(
        edited.edited().pointer("/spec/defaultMode"),
        Some(&json!(755))
    );
}

#[test]
fn unquoted_leading_zero_line_is_flagged() {
    let base = mode_base();
    let edited = edit(&base, "defaultMode: 420", "defaultMode: 0755").expect("a change");
    let line = base
        .text()
        .lines()
        .position(|line| line.contains("defaultMode"))
        .expect("the line")
        + 1;
    assert_eq!(edited.leading_zero_lines(), [line]);
}

#[test]
fn quoted_leading_zero_is_not_flagged() {
    let quoted = edit(&mode_base(), "defaultMode: 420", "defaultMode: \"0755\"").expect("a change");
    assert!(quoted.leading_zero_lines().is_empty());
}

#[test]
fn leading_zero_scan_reads_values_after_a_key_or_a_dash() {
    let text = "a: 0755\nb: \"0755\"\nc: 0\nd: 10\ne: 0755 # mode\n- 0644\n- - 0600\nf:\n  0777\n";
    assert_eq!(leading_zero_lines(text), [1, 5, 6, 7]);
}

#[test]
fn base_strips_server_fields() {
    let base = deployment_base();
    assert_eq!(base.resource_version(), "100");
    let text = base.text();
    for field in [
        "managedFields",
        "resourceVersion",
        "uid",
        "generation",
        "creationTimestamp",
        "status:",
        "readyReplicas",
        "kubectl-distinctive",
    ] {
        assert!(!text.contains(field), "{field}");
    }
    assert!(text.contains("name: api"));
}

#[test]
fn base_masks_like_the_yaml_tab() {
    let text = deployment_base().text().to_owned();
    assert!(!text.contains("s3cr3t-env"));
    assert!(!text.contains("applied-distinctive"));
    assert!(text.contains("value: <hidden>"));
    assert!(text.contains("last-applied-configuration: <hidden>"));
    assert!(text.contains("image: api:1"));
}

#[test]
fn edit_header_names_the_placeholder_rule() {
    let base = deployment_base();
    assert!(base.text().starts_with(EDIT_HEADER));
    assert!(!base.text().contains(SECRET_HEADER));
}

#[test]
fn secret_header_says_data_is_locked() {
    let base = base(ObjectKind::Secret, "db", secret());
    assert!(base.is_secret());
    assert!(base.text().contains(SECRET_HEADER));
    assert!(!base.text().contains("c2VjcmV0"));
}

#[test]
fn has_last_applied_follows_the_annotation() {
    assert!(deployment_base().has_last_applied());
    assert!(!base(ObjectKind::ConfigMap, "settings", config_map()).has_last_applied());
}

#[test]
fn syntax_error_has_line_and_column() {
    let base = deployment_base();
    let text = format!("{}\nbroken: [unclosed\n", base.text());
    let error = ObjectEdit::new(&base, &text).expect_err("not YAML");
    let EditError::Syntax { line, column, .. } = &error else {
        panic!("{error:?}");
    };
    assert!(*line > 1 && *column > 0, "{error}");
    assert!(error.to_string().starts_with("YAML error at line "));
}

#[test]
fn non_mapping_is_rejected() {
    let base = deployment_base();
    for text in ["- a\n- b\n", "just text", ""] {
        let error = ObjectEdit::new(&base, text).expect_err("not a mapping");
        assert!(
            matches!(error, EditError::NotAnObject),
            "{text:?}: {error:?}"
        );
    }
}

#[test]
fn identity_fields_cannot_change() {
    let base = deployment_base();
    let cases = [
        ("apiVersion: apps/v1", "apiVersion: apps/v2", "apiVersion"),
        ("kind: Deployment", "kind: StatefulSet", "kind"),
        (
            "  name: api\n  namespace",
            "  name: other\n  namespace",
            "metadata.name",
        ),
        (
            "namespace: payments",
            "namespace: other",
            "metadata.namespace",
        ),
    ];
    for (from, to, field) in cases {
        let error = edit(&base, from, to).expect_err("the identity changed");
        assert!(
            matches!(&error, EditError::IdentityChanged { field: changed } if *changed == field),
            "{field}: {error:?}"
        );
    }
}

#[test]
fn typed_server_fields_are_ignored() {
    let base = deployment_base();
    let text = format!(
        "{}status:\n  readyReplicas: 9\n",
        base.text()
            .replacen("replicas: 3", "replicas: 3\n  paused: true", 1)
    );
    let text = text.replacen(
        "  name: api\n",
        "  name: api\n  resourceVersion: \"999\"\n  uid: other\n",
        1,
    );
    let edited = ObjectEdit::new(&base, &text).expect("a change");
    assert!(edited.edited().get("status").is_none());
    assert!(
        edited
            .edited()
            .pointer("/metadata/resourceVersion")
            .is_none()
    );
    assert!(edited.edited().pointer("/metadata/uid").is_none());
    assert_eq!(edited.base_resource_version(), "100");
    assert_eq!(edited.base_uid(), "uid-1");
}

#[test]
fn secret_data_change_is_refused() {
    let base = base(ObjectKind::Secret, "db", secret());
    let changed = edit(&base, "password: <hidden>", "password: bmV3");
    let added = edit(&base, "data:\n", "data:\n  extra: ZXh0cmE=\n");
    let removed = edit(&base, "  user: <hidden>\n", "");
    let string_data = edit(
        &base,
        "kind: Secret",
        "kind: Secret\nstringData:\n  token: t",
    );
    for (name, result) in [
        ("changed", changed),
        ("added", added),
        ("removed", removed),
        ("stringData", string_data),
    ] {
        let error = result.expect_err(name);
        assert!(
            matches!(error, EditError::SecretValuesChanged),
            "{name}: {error:?}"
        );
    }
}

#[test]
fn secret_label_change_is_allowed() {
    let base = base(ObjectKind::Secret, "db", secret());
    let edited = edit(&base, "app: db", "app: db2").expect("a label change");
    assert_eq!(edited.changed_paths().len(), 1);
    assert_eq!(edited.changed_paths()[0].to_string(), "metadata.labels.app");
}

#[test]
fn unmatched_placeholder_names_its_path() {
    let base = deployment_base();
    let error = edit(&base, "name: DB_PASS", "name: DB_PASS2").expect_err("no counterpart");
    let EditError::UnmatchedPlaceholder { path } = &error else {
        panic!("{error:?}");
    };
    assert_eq!(
        path,
        "spec.template.spec.containers[api].env[DB_PASS2].value"
    );
    assert!(error.to_string().contains("<hidden>"));
}

#[test]
fn placeholder_survives_reordering() {
    let mut object = deployment();
    object["spec"]["template"]["spec"]["containers"] = json!([
        {"name": "api", "env": [
            {"name": "A", "value": "1"}, {"name": "B", "value": "2"},
        ]},
        {"name": "sidecar", "image": "proxy:1"},
    ]);
    let base = base(ObjectKind::Deployment, "api", object);
    let mut text: Value = serde_saphyr::from_str(base.text()).expect("parses");
    let containers = text["spec"]["template"]["spec"]["containers"]
        .as_array_mut()
        .expect("a list");
    containers.reverse();
    containers[1]["env"]
        .as_array_mut()
        .expect("a list")
        .reverse();
    let reordered = serde_saphyr::to_string(&text).expect("serializes");
    let edited = ObjectEdit::new(&base, &reordered).expect("placeholders still match");
    assert!(!edited.changed_paths().is_empty());
}

#[test]
fn duplicate_env_names_match_by_index() {
    let mut object = deployment();
    object["spec"]["template"]["spec"]["containers"] = json!([{"name": "api", "env": [
        {"name": "X", "value": "one"}, {"name": "X", "value": "two"},
    ]}]);
    let base = base(ObjectKind::Deployment, "api", object);
    let edited = edit(&base, "replicas: 3", "replicas: 4").expect("a change");
    assert_eq!(edited.changed_paths().len(), 1);
    // A third entry has no counterpart, and its path reads by index.
    let error = ObjectEdit::new(
        &base,
        &base.text().replacen(
            "        - name: X\n          value: <hidden>\n",
            "        - name: X\n          value: <hidden>\n        - name: X\n          value: <hidden>\n        - name: X\n          value: <hidden>\n",
            1,
        ),
    )
    .expect_err("an extra placeholder");
    let EditError::UnmatchedPlaceholder { path } = &error else {
        panic!("{error:?}");
    };
    assert!(path.contains("env[2].value"), "{path}");
}

#[test]
fn unchanged_text_is_no_changes() {
    let base = deployment_base();
    let error = ObjectEdit::new(&base, base.text()).expect_err("nothing changed");
    assert!(matches!(error, EditError::NoChanges), "{error:?}");
}

#[test]
fn changed_paths_use_names_and_quoted_keys() {
    let base = deployment_base();
    let edited = edit(&base, "image: api:1", "image: api:2").expect("a change");
    let text = base
        .text()
        .replacen("image: api:1", "image: api:2", 1)
        .replacen(
            "    team: payments",
            "    team: payments\n    app.kubernetes.io/name: api",
            1,
        );
    let edited_twice = ObjectEdit::new(&base, &text).expect("two changes");
    assert_eq!(
        edited.changed_paths()[0].to_string(),
        "spec.template.spec.containers[api].image"
    );
    let paths: Vec<String> = edited_twice
        .changed_paths()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(
        paths.contains(&"metadata.annotations[\"app.kubernetes.io/name\"]".to_owned()),
        "{paths:?}"
    );
}

#[test]
fn debug_shows_names_and_counts_only() {
    let base = deployment_base();
    assert_eq!(
        format!("{base:?}"),
        r#"EditBase { kind: Deployment, name: "api", resource_version: "100" }"#
    );
    let edited = edit(&base, "replicas: 3", "replicas: 4").expect("a change");
    assert_eq!(
        format!("{edited:?}"),
        r#"ObjectEdit { kind: Deployment, name: "api", changes: 1 }"#
    );
}

#[tokio::test]
async fn edit_base_reads_one_object() {
    let body = deployment().to_string();
    let (connection, api) = FakeApi::connection(WritePolicy::Blocked, move |_| (200, body.clone()));
    let base = connection
        .edit_base(&target(ObjectKind::Deployment, "api"), EnvValues::Hidden)
        .await
        .expect("the object is read");
    let requests = api.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].path,
        "/apis/apps/v1/namespaces/payments/deployments/api"
    );
    assert_eq!(base.resource_version(), "100");
}

#[tokio::test]
async fn an_object_without_a_version_cannot_be_edited() {
    let mut object = deployment();
    object["metadata"]
        .as_object_mut()
        .expect("metadata")
        .remove("resourceVersion");
    let body = object.to_string();
    let (connection, _api) =
        FakeApi::connection(WritePolicy::Blocked, move |_| (200, body.clone()));
    let error = connection
        .edit_base(&target(ObjectKind::Deployment, "api"), EnvValues::Hidden)
        .await
        .expect_err("no resourceVersion");
    assert!(
        matches!(error, ClusterError::UnexpectedResponse { .. }),
        "{error:?}"
    );
}

#[test]
fn whole_floats_become_integers_and_fractions_stay() {
    let base = base(ObjectKind::ConfigMap, "settings", config_map());
    let edited = edit(
        &base,
        "data:\n",
        "data:\n  a: 1e3\n  b: 2.5\n  c: 3.0\n  d: 0755\n",
    )
    .expect("a change");
    let data = &edited.edited()["data"];
    assert_eq!(data["a"], json!(1000));
    assert_eq!(data["b"], json!(2.5));
    assert_eq!(data["c"], json!(3));
    assert_eq!(data["d"], json!(755));
    assert!(data["a"].is_i64() && data["c"].is_i64() && data["d"].is_i64());
}

#[test]
fn a_typed_changed_marker_is_refused() {
    let base = deployment_base();
    let error = edit(&base, "value: <hidden>", "value: <hidden, changed>").expect_err("a marker");
    assert!(
        matches!(&error, EditError::MarkerText { path } if path.ends_with("env[DB_PASS].value")),
        "{error:?}"
    );
    assert!(error.to_string().contains("diff marker text"));
}

#[test]
fn a_typed_moved_marker_is_refused() {
    let base = deployment_base();
    let error = edit(&base, "value: <hidden>", "value: <hidden, moved>").expect_err("a marker");
    assert!(matches!(&error, EditError::MarkerText { .. }), "{error:?}");
}

#[tokio::test]
async fn a_helm_release_record_cannot_be_edited() {
    let mut object = secret();
    object["type"] = json!("helm.sh/release.v1");
    let body = object.to_string();
    let (connection, _api) =
        FakeApi::connection(WritePolicy::Blocked, move |_| (200, body.clone()));
    let error = connection
        .edit_base(&target(ObjectKind::Secret, "db"), EnvValues::Hidden)
        .await
        .expect_err("a release record");
    assert!(
        matches!(error, ClusterError::UnexpectedResponse { .. }),
        "{error:?}"
    );
    assert!(format!("{error:?}").contains("Helm release"), "{error:?}");
}

#[test]
fn an_ordinary_secret_type_can_be_edited() {
    assert!(!is_helm_release(&secret()));
    assert!(is_helm_release(&json!({"type": "helm.sh/release.v1"})));
}

#[test]
fn the_target_decides_what_is_masked_when_the_object_names_no_kind() {
    let mut object = secret();
    object.as_object_mut().expect("an object").remove("kind");
    let base = base(ObjectKind::Secret, "db", object);
    assert!(!base.text().contains("c2VjcmV0"), "{}", base.text());
    assert!(base.text().contains("kind: Secret"));
}

/// The Deployment as the server holds it after somebody else's change: `f` edits the raw object.
fn deployment_base_after(f: impl FnOnce(&mut Value)) -> EditBase {
    let mut object = deployment();
    object["metadata"]["resourceVersion"] = json!("101");
    f(&mut object);
    base(ObjectKind::Deployment, "api", object)
}

fn rebased(old: &EditBase, text: &str, new: &EditBase) -> (Value, Rebased) {
    let result = rebase(old, text, new).expect("the text parses");
    let value: Value = serde_saphyr::from_str(&result.text).expect("the rebased text parses");
    (value, result)
}

#[test]
fn rebase_keeps_user_changes_on_the_new_base() {
    let old = deployment_base();
    let text = old.text().replacen("replicas: 3", "replicas: 5", 1);
    let new = deployment_base_after(|object| {
        object["metadata"]["labels"] = json!({"tier": "backend"});
    });
    let (value, result) = rebased(&old, &text, &new);
    assert_eq!(value["spec"]["replicas"], json!(5));
    assert_eq!(value["metadata"]["labels"], json!({"tier": "backend"}));
    assert!(result.unreachable.is_empty());
    let changed: Vec<String> = result
        .server_changed
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(changed, ["metadata.labels"]);
}

#[test]
fn rebase_keeps_a_concurrent_list_item_change() {
    let old = deployment_base();
    let text = old.text().replacen("image: api:1", "image: api:2", 1);
    let new = deployment_base_after(|object| {
        object["spec"]["template"]["spec"]["containers"][1]["image"] = json!("proxy:2");
    });
    let (value, result) = rebased(&old, &text, &new);
    let containers = &value["spec"]["template"]["spec"]["containers"];
    assert_eq!(containers[0]["image"], json!("api:2"));
    assert_eq!(containers[1]["image"], json!("proxy:2"));
    assert!(result.unreachable.is_empty());
}

#[test]
fn rebase_user_value_wins_on_both_changed() {
    let old = deployment_base();
    let text = old.text().replacen("replicas: 3", "replicas: 5", 1);
    let new = deployment_base_after(|object| object["spec"]["replicas"] = json!(7));
    let (value, result) = rebased(&old, &text, &new);
    assert_eq!(value["spec"]["replicas"], json!(5));
    assert_eq!(result.server_changed[0].to_string(), "spec.replicas");
}

#[test]
fn rebase_removed_key_stays_removed() {
    let old = deployment_base();
    let text = old.text().replacen("    team: payments\n", "", 1);
    let new = deployment_base_after(|object| object["spec"]["replicas"] = json!(4));
    let (value, result) = rebased(&old, &text, &new);
    assert!(value["metadata"]["annotations"].get("team").is_none());
    assert_eq!(value["spec"]["replicas"], json!(4));
    assert!(result.unreachable.is_empty());
}

#[test]
fn rebase_reports_unreachable_paths() {
    let old = deployment_base();
    let text = old.text().replacen("image: proxy:1", "image: proxy:9", 1);
    let new = deployment_base_after(|object| {
        let containers = object["spec"]["template"]["spec"]["containers"]
            .as_array_mut()
            .expect("containers");
        containers.retain(|container| container["name"] != "sidecar");
    });
    let (value, result) = rebased(&old, &text, &new);
    let unreachable: Vec<String> = result.unreachable.iter().map(ToString::to_string).collect();
    assert_eq!(
        unreachable,
        ["spec.template.spec.containers[sidecar].image"]
    );
    let containers = value["spec"]["template"]["spec"]["containers"]
        .as_array()
        .expect("containers");
    assert_eq!(containers.len(), 1);
}

#[test]
fn rebase_adds_a_new_named_item() {
    let old = deployment_base();
    let text = old.text().replacen(
        "      - image: proxy:1\n        name: sidecar\n",
        "      - image: proxy:1\n        name: sidecar\n      - image: extra:1\n        name: extra\n",
        1,
    );
    assert_ne!(text, old.text(), "the fixture text changed");
    let new = deployment_base_after(|object| object["spec"]["replicas"] = json!(4));
    let (value, _) = rebased(&old, &text, &new);
    let containers = value["spec"]["template"]["spec"]["containers"]
        .as_array()
        .expect("containers");
    assert_eq!(containers.len(), 3);
    assert_eq!(containers[2]["name"], json!("extra"));
}

#[test]
fn rebase_keeps_a_placeholder_meaning_the_new_server_value() {
    let old = deployment_base();
    let text = old.text().replacen("replicas: 3", "replicas: 5", 1);
    let new = deployment_base_after(|object| {
        object["spec"]["template"]["spec"]["containers"][0]["env"][0]["value"] =
            json!("n3w-s3cr3t");
    });
    let result = rebase(&old, &text, &new).expect("the text parses");
    assert!(!result.text.contains("s3cr3t"));
    assert!(result.text.contains("<hidden>"));
    assert!(result.text.starts_with(EDIT_HEADER));
    let applied = ObjectEdit::new(&new, &result.text).expect("the rebased text applies");
    assert_eq!(applied.changed_paths()[0].to_string(), "spec.replicas");
}

#[test]
fn rebase_rejects_text_that_is_not_a_mapping() {
    let old = deployment_base();
    let new = deployment_base_after(|_| {});
    assert!(matches!(
        rebase(&old, "- a", &new),
        Err(EditError::NotAnObject)
    ));
    assert!(matches!(
        rebase(&old, "a: [", &new),
        Err(EditError::Syntax { .. })
    ));
}

#[test]
fn rebased_debug_shows_counts_only() {
    let old = deployment_base();
    let new = deployment_base_after(|object| object["spec"]["replicas"] = json!(4));
    let result = rebase(&old, old.text(), &new).expect("the text parses");
    let debug = format!("{result:?}");
    assert!(debug.contains("server_changed: 1"), "{debug}");
    assert!(!debug.contains("replicas"), "{debug}");
}

#[test]
fn format_sorts_keys_and_keeps_the_header() {
    let base = deployment_base();
    let shuffled = "# a comment line\nspec:\n  replicas: 3\nmetadata:\n  namespace: payments\n  name: api\nkind: Deployment\napiVersion: apps/v1\n";
    let formatted = format_yaml(shuffled).expect("formats");
    assert!(formatted.starts_with("# a comment line\n"));
    let keys: Vec<&str> = formatted
        .lines()
        .filter(|line| !line.starts_with(' ') && !line.starts_with('#'))
        .collect();
    assert_eq!(
        keys,
        [
            "apiVersion: apps/v1",
            "kind: Deployment",
            "metadata:",
            "spec:"
        ]
    );
    let again = format_yaml(base.text()).expect("formats");
    assert_eq!(again, base.text(), "the editor text is already formatted");
}

#[test]
fn format_reports_syntax_errors_and_non_mappings() {
    assert!(matches!(format_yaml("a: ["), Err(EditError::Syntax { .. })));
    assert!(matches!(format_yaml("- a"), Err(EditError::NotAnObject)));
}

#[test]
fn rebase_lists_server_changes() {
    let old = deployment_base();
    let new = deployment_base_after(|object| {
        object["spec"]["replicas"] = json!(4);
        object["spec"]["template"]["spec"]["containers"][1]["image"] = json!("proxy:2");
    });
    let result = rebase(&old, old.text(), &new).expect("the text parses");
    let changed: Vec<String> = result
        .server_changed
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        changed,
        [
            "spec.replicas",
            "spec.template.spec.containers[sidecar].image"
        ]
    );
    // An untouched text takes the new object as it is.
    let value: Value = serde_saphyr::from_str(&result.text).expect("the text parses");
    assert_eq!(value["spec"]["replicas"], json!(4));
}

/// The Deployment of `deployment_base_after`, with a new uid: the object was deleted and created
/// again under the same name.
fn recreated_base() -> EditBase {
    deployment_base_after(|object| {
        object["metadata"]["uid"] = json!("uid-2");
    })
}

#[test]
fn rebase_onto_a_recreated_object_is_refused() {
    let old = deployment_base();
    let text = old.text().replacen("replicas: 3", "replicas: 5", 1);
    let error = rebase(&old, &text, &recreated_base()).expect_err("another object");
    assert!(matches!(error, EditError::Recreated), "{error:?}");
    assert_eq!(
        error.to_string(),
        "the object was deleted and created again"
    );
}

#[test]
fn rebase_lists_the_paths_the_user_overwrites() {
    let old = deployment_base();
    let text = old
        .text()
        .replacen("replicas: 3", "replicas: 5", 1)
        .replacen("image: api:1", "image: api:2", 1);
    // The server moved the replicas too, and changed another container.
    let new = deployment_base_after(|object| {
        object["spec"]["replicas"] = json!(7);
        object["spec"]["template"]["spec"]["containers"][1]["image"] = json!("proxy:2");
    });
    let result = rebase(&old, &text, &new).expect("the text parses");
    let overwritten: Vec<String> = result.overwritten.iter().map(ToString::to_string).collect();
    assert_eq!(overwritten, ["spec.replicas"]);
}

#[test]
fn a_list_that_became_one_path_overlaps_the_users_item_edit() {
    let with_args = |args: Value| {
        let mut object = deployment();
        object["spec"]["template"]["spec"]["containers"][0]["args"] = args;
        object
    };
    let old = base(
        ObjectKind::Deployment,
        "api",
        with_args(json!(["--a", "--b"])),
    );
    let text = old.text().replacen("- --a", "- --changed", 1);
    assert_ne!(text, old.text());
    let mut newer = with_args(json!(["--a", "--b", "--c"]));
    newer["metadata"]["resourceVersion"] = json!("101");
    let new = base(ObjectKind::Deployment, "api", newer);
    let result = rebase(&old, &text, &new).expect("the text parses");
    // The server's list is one path; the user's edit of item 0 lies inside it, and replaces it.
    assert_eq!(result.overwritten.len(), 1, "{:?}", result.overwritten);
    assert!(
        result.overwritten[0]
            .to_string()
            .ends_with("containers[api].args[0]"),
        "{}",
        result.overwritten[0]
    );
}

#[test]
fn changes_apart_from_the_servers_overwrite_nothing() {
    let old = deployment_base();
    let text = old.text().replacen("replicas: 3", "replicas: 5", 1);
    let new = deployment_base_after(|object| object["metadata"]["labels"] = json!({"tier": "x"}));
    let result = rebase(&old, &text, &new).expect("the text parses");
    assert!(result.overwritten.is_empty());
}

#[test]
fn format_refuses_a_leading_zero_and_names_the_line() {
    let text = "apiVersion: v1\nkind: ConfigMap\nspec:\n  defaultMode: 0444\n";
    let error = format_yaml(text).expect_err("a leading zero");
    assert!(
        matches!(error, EditError::LeadingZero { line: 4 }),
        "{error:?}"
    );
    assert!(error.to_string().starts_with("line 4: "), "{error}");
    // The same number quoted is a string, and formats.
    assert!(format_yaml("spec:\n  defaultMode: \"0444\"\n").is_ok());
}

#[test]
fn rebase_refuses_a_leading_zero_too() {
    let old = deployment_base();
    let text = old.text().replacen("replicas: 3", "replicas: 0444", 1);
    let error = rebase(&old, &text, &deployment_base_after(|_| {})).expect_err("a leading zero");
    assert!(matches!(error, EditError::LeadingZero { .. }), "{error:?}");
}

#[test]
fn a_text_over_two_mebibytes_is_refused_before_parsing() {
    let base = deployment_base();
    let big = format!("{}# {}\n", base.text(), "x".repeat(2 * 1024 * 1024));
    assert!(matches!(
        ObjectEdit::new(&base, &big),
        Err(EditError::TooLarge)
    ));
    assert!(matches!(format_yaml(&big), Err(EditError::TooLarge)));
    assert!(matches!(
        rebase(&base, &big, &deployment_base_after(|_| {})),
        Err(EditError::TooLarge)
    ));
}

#[test]
fn the_managed_by_label_marks_a_helm_object() {
    assert!(!deployment_base().is_helm_managed());
    let mut object = deployment();
    object["metadata"]["labels"] = json!({"app.kubernetes.io/managed-by": "Helm"});
    assert!(base(ObjectKind::Deployment, "api", object).is_helm_managed());
    let mut other = deployment();
    other["metadata"]["labels"] = json!({"app.kubernetes.io/managed-by": "Tiller"});
    assert!(!base(ObjectKind::Deployment, "api", other).is_helm_managed());
}

const SHOWN_SECRET: &str = "# k8sBoard hid 2 values as <hidden>.\napiVersion: v1\ndata:\n  password: <hidden>\n  username: <hidden>\nkind: Secret\nmetadata:\n  creationTimestamp: 2026-10-01T08:00:00Z\n  labels:\n    app: db\n  managedFields:\n    - manager: kubectl\n  name: db-credentials\n  namespace: payments\n  resourceVersion: \"100\"\n  uid: uid-1\ntype: Opaque\n";

#[test]
fn clean_yaml_drops_the_status_and_the_servers_metadata() {
    let shown = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  creationTimestamp: 2026-10-01T08:00:00Z\n  generation: 4\n  labels:\n    app: api\n  managedFields:\n    - manager: kubectl\n  name: api\n  namespace: payments\n  resourceVersion: \"100\"\n  selfLink: /apis/apps/v1/namespaces/payments/deployments/api\n  uid: uid-1\nspec:\n  replicas: 3\nstatus:\n  readyReplicas: 3\n";
    let clean = clean_yaml(shown).expect("a manifest");
    assert_eq!(
        clean.text,
        "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  labels:\n    app: api\n  name: api\n  namespace: payments\nspec:\n  replicas: 3\n"
    );
    assert_eq!(clean.hidden_dropped, 0);
}

#[test]
fn clean_yaml_drops_hidden_values_with_their_keys_and_counts_them() {
    let clean = clean_yaml(SHOWN_SECRET).expect("a manifest");
    assert_eq!(
        clean.text,
        "apiVersion: v1\ndata: {}\nkind: Secret\nmetadata:\n  labels:\n    app: db\n  name: db-credentials\n  namespace: payments\ntype: Opaque\n"
    );
    assert_eq!(clean.hidden_dropped, 2);
    assert!(!clean.text.contains("hidden"));
}

#[test]
fn clean_yaml_drops_a_hidden_env_value_wherever_it_sits() {
    let shown = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: api\nspec:\n  template:\n    spec:\n      containers:\n        - env:\n            - name: DB_PASS\n              value: <hidden>\n            - name: MODE\n              value: fast\n          name: api\n";
    let clean = clean_yaml(shown).expect("a manifest");
    assert_eq!(clean.hidden_dropped, 1);
    assert!(clean.text.contains("- name: DB_PASS\n"), "{}", clean.text);
    assert!(clean.text.contains("value: fast"), "{}", clean.text);
    assert!(!clean.text.contains("<hidden>"), "{}", clean.text);
}

#[test]
fn clean_yaml_refuses_text_that_is_no_mapping() {
    assert!(matches!(clean_yaml("a: ["), Err(EditError::Syntax { .. })));
    assert!(matches!(clean_yaml("- a"), Err(EditError::NotAnObject)));
}
