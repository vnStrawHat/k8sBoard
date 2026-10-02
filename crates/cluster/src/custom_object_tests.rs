use serde_json::json;

use super::*;
use crate::custom_resource_definition::{ColumnType, PrinterColumn};

/// Literals that appear nowhere else, so a leak is easy to find.
const SECRET_ONE: &str = "distinctive-secret-one";
const SECRET_TWO: &str = "distinctive-secret-two";
const SECRET_THREE: &str = "distinctive-secret-three";

fn object(value: serde_json::Value) -> DynamicObject {
    serde_json::from_value(value).expect("fixture decodes")
}

fn compile(columns: &[PrinterColumn]) -> Vec<CompiledColumn> {
    columns
        .iter()
        .map(|column| CompiledColumn {
            path: ColumnPath::parse(&column.json_path).ok(),
            column: column.clone(),
        })
        .collect()
}

fn summary_of(value: serde_json::Value, columns: &[PrinterColumn]) -> CustomObjectSummary {
    let object = object(value);
    let kind = object
        .types
        .as_ref()
        .map_or_else(String::new, |types| types.kind.clone());
    custom_object_summary(&object, &kind, &compile(columns))
}

fn column(name: &str, column_type: ColumnType, json_path: &str) -> PrinterColumn {
    PrinterColumn::new(name, column_type, json_path, false)
}

fn certificate() -> serde_json::Value {
    json!({
        "apiVersion": "cert-manager.io/v1",
        "kind": "Certificate",
        "metadata": {
            "name": "web-tls",
            "namespace": "shop",
            "labels": {"team": "web", "app": "shop"},
            "creationTimestamp": "2026-01-01T00:00:00Z",
        },
        "spec": {
            "secretName": "web-tls-secret",
            "issuerRef": {"name": "letsencrypt", "kind": "ClusterIssuer"},
            "dnsNames": ["a.example.com", "b.example.com"],
        },
        "status": {
            "notAfter": "2026-12-01T00:00:00Z",
            "conditions": [{
                "type": "Ready",
                "status": "True",
                "reason": "Ready",
                "message": "Certificate is up to date",
                "lastTransitionTime": "2026-01-01T00:01:00Z",
            }],
        },
    })
}

fn certificate_columns() -> Vec<PrinterColumn> {
    vec![
        column(
            "Ready",
            ColumnType::String,
            ".status.conditions[?(@.type==\"Ready\")].status",
        ),
        column("Issuer", ColumnType::String, ".spec.issuerRef.name"),
        column("Expires", ColumnType::Date, ".status.notAfter"),
    ]
}

fn flatten(value: serde_json::Value, kind: &str) -> CustomObjectFields {
    let mut value = value;
    value["kind"] = json!(kind);
    object_fields(&object(value), kind)
}

fn spec_entries(fields: &CustomObjectFields) -> Vec<(&str, &FieldValue)> {
    fields
        .spec
        .entries
        .iter()
        .map(|entry| (entry.path.as_str(), &entry.value))
        .collect()
}

#[test]
fn summary_keeps_metadata_labels_and_phase() {
    let mut value = certificate();
    value["status"]["phase"] = json!("Issued");
    let summary = summary_of(value, &[]);
    assert_eq!(summary.name, "web-tls");
    assert_eq!(summary.namespace.as_deref(), Some("shop"));
    assert_eq!(summary.labels, ["app=shop", "team=web"]);
    assert_eq!(
        summary.created_at,
        Some("2026-01-01T00:00:00Z".parse().expect("timestamp"))
    );
    assert_eq!(summary.phase.as_deref(), Some("Issued"));
    let without_phase = summary_of(certificate(), &[]);
    assert_eq!(without_phase.phase, None);
}

#[test]
fn phase_falls_back_to_a_top_level_status_status() {
    let mut value = certificate();
    value["status"]["status"] = json!("Completed");
    assert_eq!(
        summary_of(value.clone(), &[]).phase.as_deref(),
        Some("Completed")
    );
    // `status.phase` wins when both exist.
    value["status"]["phase"] = json!("Running");
    assert_eq!(summary_of(value, &[]).phase.as_deref(), Some("Running"));
    // A nested object under `status.status` is not a state word.
    let mut nested = certificate();
    nested["status"]["status"] = json!({"phase": "x"});
    assert_eq!(summary_of(nested, &[]).phase, None);
}

#[test]
fn status_status_is_masked_like_any_text() {
    let mut value = certificate();
    value["status"]["status"] = json!(format!("redis://u:{SECRET_ONE}@host"));
    let summary = summary_of(value, &[]);
    assert!(!format!("{summary:?}").contains(SECRET_ONE));
    assert_eq!(summary.phase.as_deref(), Some("redis://<hidden>@host"));
}

#[test]
fn summary_has_one_value_per_column() {
    let summary = summary_of(certificate(), &certificate_columns());
    assert_eq!(
        summary.columns,
        [
            ColumnValue::Text("True".to_owned()),
            ColumnValue::Text("letsencrypt".to_owned()),
            ColumnValue::Date("2026-12-01T00:00:00Z".parse().expect("timestamp")),
        ]
    );
}

#[test]
fn metadata_columns_read_the_metadata() {
    let columns = [column(
        "Created",
        ColumnType::Date,
        ".metadata.creationTimestamp",
    )];
    let summary = summary_of(certificate(), &columns);
    assert_eq!(
        summary.columns,
        [ColumnValue::Date(
            "2026-01-01T00:00:00Z".parse().expect("timestamp")
        )]
    );
}

#[test]
fn unsupported_columns_are_absent() {
    let columns = [
        column("Issuer", ColumnType::String, ".spec.issuerRef.name"),
        column("Odd", ColumnType::String, ".spec.dnsNames[0:2]"),
        column("Missing", ColumnType::String, ".spec.nothing"),
    ];
    let summary = summary_of(certificate(), &columns);
    assert_eq!(summary.columns.len(), 3);
    assert_eq!(summary.columns[1], ColumnValue::Absent);
    assert_eq!(summary.columns[2], ColumnValue::Absent);
}

#[test]
fn conditions_capped_at_10_with_messages_cut_at_300() {
    let conditions: Vec<_> = (0..12)
        .map(|index| {
            json!({
                "type": format!("Condition{index}"),
                "status": if index == 1 { "False" } else if index == 2 { "Maybe" } else { "True" },
                "message": "m".repeat(400),
            })
        })
        .collect();
    let mut value = certificate();
    value["status"]["conditions"] = json!(conditions);
    let summary = summary_of(value, &[]);
    assert_eq!(summary.conditions.len(), 10);
    assert_eq!(summary.conditions[0].name, "Condition0");
    assert_eq!(summary.conditions[1].status, ConditionStatus::False);
    assert_eq!(summary.conditions[2].status, ConditionStatus::Unknown);
    let message = summary.conditions[0].message.as_deref().expect("message");
    assert_eq!(message.chars().count(), 300);
    assert!(message.ends_with('…'));
    assert_eq!(summary.conditions[0].reason, None);
}

#[test]
fn condition_without_a_type_is_skipped() {
    let mut value = certificate();
    value["status"]["conditions"] =
        json!([{"status": "True"}, {"type": "Ready", "status": "True"}]);
    let summary = summary_of(value, &[]);
    assert_eq!(summary.conditions.len(), 1);
    assert_eq!(summary.conditions[0].name, "Ready");
}

#[test]
fn fields_flatten_spec_and_status_to_depth_3() {
    let fields = flatten(
        json!({
            "metadata": {"name": "x"},
            "spec": {
                "replicas": 3,
                "enabled": true,
                "issuerRef": {"name": "letsencrypt", "group": {"a": "1", "b": "2"}},
                "deep": {"one": {"two": {"three": "x"}}},
            },
            "status": {"state": "ok"},
        }),
        "Widget",
    );
    assert_eq!(
        spec_entries(&fields),
        [
            ("deep.one.two", &FieldValue::Fields(1)),
            ("enabled", &FieldValue::Text("true".to_owned())),
            ("issuerRef.group.a", &FieldValue::Text("1".to_owned())),
            ("issuerRef.group.b", &FieldValue::Text("2".to_owned())),
            (
                "issuerRef.name",
                &FieldValue::Text("letsencrypt".to_owned())
            ),
            ("replicas", &FieldValue::Text("3".to_owned())),
        ]
    );
    assert_eq!(fields.status.entries.len(), 1);
    assert_eq!(fields.status.entries[0].path, "state");
}

#[test]
fn fields_skip_status_conditions() {
    let fields = flatten(certificate(), "Certificate");
    let paths: Vec<_> = fields
        .status
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();
    assert_eq!(paths, ["notAfter"]);
    // Only the top level of status is skipped; a nested key of that name is a field.
    let nested = flatten(json!({"status": {"sub": {"conditions": "kept"}}}), "Widget");
    assert_eq!(nested.status.entries[0].path, "sub.conditions");
}

#[test]
fn scalar_arrays_join_five_then_count() {
    let fields = flatten(
        json!({"spec": {"few": ["a", "b"], "many": ["1", "2", "3", "4", "5", "6", "7"], "numbers": [1, 2]}}),
        "Widget",
    );
    let entries = spec_entries(&fields);
    assert_eq!(entries[0], ("few", &FieldValue::Text("a, b".to_owned())));
    assert_eq!(
        entries[1],
        ("many", &FieldValue::Text("1, 2, 3, 4, 5 +2".to_owned()))
    );
    assert_eq!(
        entries[2],
        ("numbers", &FieldValue::Text("1, 2".to_owned()))
    );
}

#[test]
fn object_arrays_report_item_count() {
    let fields = flatten(
        json!({"spec": {"rules": [{"host": "a"}, {"host": "b"}, {"host": "c"}], "mixed": ["x", {"y": 1}]}}),
        "Widget",
    );
    let entries = spec_entries(&fields);
    assert_eq!(entries[0], ("mixed", &FieldValue::Items(2)));
    assert_eq!(entries[1], ("rules", &FieldValue::Items(3)));
}

#[test]
fn deep_objects_report_field_count() {
    let fields = flatten(
        json!({"spec": {"a": {"b": {"c": {"d": 1, "e": 2}, "f": "x"}}}}),
        "Widget",
    );
    let entries = spec_entries(&fields);
    assert_eq!(entries[0], ("a.b.c", &FieldValue::Fields(2)));
    assert_eq!(entries[1], ("a.b.f", &FieldValue::Text("x".to_owned())));
}

#[test]
fn fields_cap_at_60_and_count_omitted() {
    let spec: serde_json::Map<String, serde_json::Value> = (0..65)
        .map(|index| (format!("field{index:02}"), json!(index)))
        .collect();
    let fields = flatten(json!({ "spec": spec }), "Widget");
    assert_eq!(fields.spec.entries.len(), 60);
    assert_eq!(fields.spec.omitted, 5);
    assert_eq!(fields.spec.entries[0].path, "field00");
    assert_eq!(fields.status, FieldList::default());
}

#[test]
fn null_and_empty_values_are_skipped() {
    let fields = flatten(
        json!({"spec": {"nothing": null, "emptyMap": {}, "emptyList": [], "kept": "x"}}),
        "Widget",
    );
    assert_eq!(fields.spec.entries.len(), 1);
    assert_eq!(fields.spec.entries[0].path, "kept");
}

#[test]
fn long_field_text_is_cut_at_200_chars() {
    let fields = flatten(json!({"spec": {"long": "z".repeat(300)}}), "Widget");
    let FieldValue::Text(text) = &fields.spec.entries[0].value else {
        panic!("expected text");
    };
    assert_eq!(text.chars().count(), 200);
    assert!(text.ends_with('…'));
}

#[test]
fn secret_fields_read_hidden() {
    let fields = flatten(
        json!({"spec": {
            "clientSecret": SECRET_ONE,
            "secretRef": {"name": "creds"},
            "dsn": format!("postgres://svc:{SECRET_TWO}@db/x"),
            "tokens": [SECRET_THREE],
        }}),
        "Widget",
    );
    let entries = spec_entries(&fields);
    assert_eq!(entries[0], ("clientSecret", &FieldValue::Hidden));
    assert_eq!(
        entries[1],
        (
            "dsn",
            &FieldValue::Text("postgres://<hidden>@db/x".to_owned())
        )
    );
    assert_eq!(
        entries[2],
        ("secretRef.name", &FieldValue::Text("creds".to_owned()))
    );
    assert_eq!(entries[3], ("tokens", &FieldValue::Hidden));
    let text = format!("{fields:?}");
    for secret in [SECRET_ONE, SECRET_TWO, SECRET_THREE] {
        assert!(!text.contains(secret), "{text}");
    }
}

#[test]
fn secret_like_kinds_hide_spec_fields_but_not_status() {
    let fields = flatten(
        json!({
            "spec": {"match": SECRET_ONE, "enabled": true},
            "status": {"state": "Synced"},
        }),
        "ClusterSecret",
    );
    // Sorted by key: `enabled` (a boolean, kept) then `match` (hidden).
    assert_eq!(
        fields.spec.entries[0].value,
        FieldValue::Text("true".to_owned())
    );
    assert_eq!(fields.spec.entries[1].value, FieldValue::Hidden);
    assert_eq!(
        fields.status.entries[0].value,
        FieldValue::Text("Synced".to_owned())
    );
}

#[test]
fn custom_summary_debug_lacks_secret_values() {
    let columns = [
        column("Password", ColumnType::String, ".spec.adminPassword"),
        PrinterColumn::new("Note", ColumnType::String, ".spec.note", true),
        column("Dsn", ColumnType::String, ".spec.dsn"),
        column("Name", ColumnType::String, ".spec.name"),
    ];
    let value = json!({
        "apiVersion": "example.io/v1",
        "kind": "Widget",
        "metadata": {"name": "w", "namespace": "ns"},
        "spec": {
            "adminPassword": SECRET_ONE,
            "note": SECRET_TWO,
            "dsn": format!("redis://u:{SECRET_THREE}@host:6379"),
            "name": "shown",
        },
    });
    let summary = summary_of(value, &columns);
    assert_eq!(
        summary.columns,
        [
            ColumnValue::Hidden,
            ColumnValue::Hidden,
            ColumnValue::Text("redis://<hidden>@host:6379".to_owned()),
            ColumnValue::Text("shown".to_owned()),
        ]
    );
    let text = format!("{summary:?}");
    for secret in [SECRET_ONE, SECRET_TWO, SECRET_THREE] {
        assert!(!text.contains(secret), "{text}");
    }
}

#[test]
fn secret_like_kind_hides_columns_outside_status_and_metadata() {
    let columns = [
        column("Match", ColumnType::String, ".spec.match"),
        column("State", ColumnType::String, ".status.state"),
        column("Created", ColumnType::Date, ".metadata.creationTimestamp"),
        column("Expires", ColumnType::Date, ".spec.expires"),
        column("Missing", ColumnType::String, ".spec.missing"),
    ];
    let summary = summary_of(
        json!({
            "apiVersion": "example.io/v1",
            "kind": "ClusterSecret",
            "metadata": {"name": "s", "creationTimestamp": "2026-01-01T00:00:00Z"},
            "spec": {"match": SECRET_ONE, "expires": "2026-06-01T00:00:00Z"},
            "status": {"state": "Synced"},
        }),
        &columns,
    );
    assert_eq!(summary.columns[0], ColumnValue::Hidden);
    assert_eq!(summary.columns[1], ColumnValue::Text("Synced".to_owned()));
    assert!(matches!(summary.columns[2], ColumnValue::Date(_)));
    assert!(matches!(summary.columns[3], ColumnValue::Date(_)));
    assert_eq!(summary.columns[4], ColumnValue::Absent);
}

#[test]
fn objects_watch_config_pages_most_recent() {
    let config = objects_watch_config();
    assert_eq!(config.list_semantic, ListSemantic::MostRecent);
    assert_eq!(config.page_size, Some(50));
    assert_eq!(config.field_selector, None);
}

#[test]
fn fields_watch_selects_by_name() {
    let config = fields_watch_config("web-tls");
    assert_eq!(
        config.field_selector.as_deref(),
        Some("metadata.name=web-tls")
    );
    assert_eq!(config.page_size, Some(50));
}

#[test]
fn certificate_fixture_matches_worked_example() {
    let summary = summary_of(certificate(), &certificate_columns());
    assert_eq!(summary.conditions.len(), 1);
    assert_eq!(summary.conditions[0].status, ConditionStatus::True);
    assert_eq!(summary.conditions[0].reason.as_deref(), Some("Ready"));
    assert!(summary.conditions[0].changed_at.is_some());
    assert_eq!(summary.columns[0], ColumnValue::Text("True".to_owned()));
    let fields = flatten(certificate(), "Certificate");
    let entries = spec_entries(&fields);
    assert_eq!(
        entries,
        [
            (
                "dnsNames",
                &FieldValue::Text("a.example.com, b.example.com".to_owned())
            ),
            (
                "issuerRef.kind",
                &FieldValue::Text("ClusterIssuer".to_owned())
            ),
            (
                "issuerRef.name",
                &FieldValue::Text("letsencrypt".to_owned())
            ),
            ("secretName", &FieldValue::Text("web-tls-secret".to_owned())),
        ]
    );
}

#[test]
fn condition_messages_hide_url_userinfo_before_the_cut() {
    let mut value = certificate();
    // The credential sits across the 300-character mark, so a cut first would leave a prefix.
    let message = format!("{}postgres://svc:{SECRET_ONE}@db:5432/app", "m".repeat(290));
    value["status"]["conditions"] =
        json!([{"type": "Ready", "status": "False", "message": message}]);
    let summary = summary_of(value, &[]);
    let shown = summary.conditions[0].message.as_deref().expect("message");
    assert!(!shown.contains("distinctive"), "{shown}");
    assert!(!shown.contains("svc"), "{shown}");
    assert_eq!(shown.chars().count(), 300);
}

#[test]
fn env_style_pairs_hide_secret_named_values_in_fields() {
    let fields = flatten(
        json!({"spec": {
            "param": {"name": "DB_PASSWORD", "value": SECRET_ONE},
            "other": {"name": "LOG_LEVEL", "value": "debug"},
        }}),
        "Widget",
    );
    let entries = spec_entries(&fields);
    assert_eq!(
        entries,
        [
            ("other.name", &FieldValue::Text("LOG_LEVEL".to_owned())),
            ("other.value", &FieldValue::Text("debug".to_owned())),
            ("param.name", &FieldValue::Text("DB_PASSWORD".to_owned())),
            ("param.value", &FieldValue::Hidden),
        ]
    );
    assert!(!format!("{fields:?}").contains(SECRET_ONE));
}

#[test]
fn env_style_pair_columns_hide_secret_named_values() {
    let columns = [
        column("First", ColumnType::String, ".spec.x.env[0].value"),
        column("Second", ColumnType::String, ".spec.x.env[1].value"),
    ];
    let summary = summary_of(
        json!({
            "apiVersion": "example.io/v1",
            "kind": "Widget",
            "metadata": {"name": "w"},
            "spec": {"x": {"env": [
                {"name": "API_TOKEN", "value": SECRET_ONE},
                {"name": "LOG_LEVEL", "value": "debug"},
            ]}},
        }),
        &columns,
    );
    assert_eq!(
        summary.columns,
        [ColumnValue::Hidden, ColumnValue::Text("debug".to_owned())]
    );
    assert!(!format!("{summary:?}").contains(SECRET_ONE));
}

#[test]
fn metadata_columns_hide_manifest_annotations() {
    let columns = [column(
        "Applied",
        ColumnType::String,
        ".metadata.annotations['kubectl.kubernetes.io/last-applied-configuration']",
    )];
    let summary = summary_of(
        json!({
            "apiVersion": "example.io/v1",
            "kind": "Widget",
            "metadata": {"name": "w", "annotations": {
                "kubectl.kubernetes.io/last-applied-configuration": SECRET_ONE,
            }},
        }),
        &columns,
    );
    // The annotation is masked to `<hidden>` before the path reads it.
    assert_eq!(summary.columns, [ColumnValue::Text("<hidden>".to_owned())]);
    assert!(!format!("{summary:?}").contains(SECRET_ONE));
}
