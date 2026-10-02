use serde_json::json;

use super::*;

fn path(text: &str) -> ColumnPath {
    ColumnPath::parse(text).expect("supported path")
}

fn column(column_type: ColumnType, json_path: &str) -> PrinterColumn {
    PrinterColumn::new("Column", column_type, json_path, false)
}

/// The cell of `json_path` over `data`, typed by `column_type`.
fn cell(column_type: ColumnType, json_path: &str, data: &Value) -> ColumnValue {
    let path = path(json_path);
    let found = path.first_match(&Value::Null, data);
    column_value(&column(column_type, json_path), &path, found)
}

fn certificate() -> Value {
    json!({
        "spec": {
            "issuerRef": {"name": "letsencrypt"},
            "dnsNames": ["a.example.com", "b.example.com"],
        },
        "status": {
            "notAfter": "2026-12-01T00:00:00Z",
            "conditions": [
                {"type": "Issuing", "status": "False", "message": "done"},
                {"type": "Ready", "status": "True", "message": "Certificate is up to date"},
            ],
        },
    })
}

#[test]
fn parses_dotted_fields() {
    let parsed = path(".spec.issuerRef.name");
    assert_eq!(
        parsed.steps,
        [
            Step::Field("spec".to_owned()),
            Step::Field("issuerRef".to_owned()),
            Step::Field("name".to_owned()),
        ]
    );
    assert_eq!(parsed.root, PathRoot::Data);
}

#[test]
fn parses_quoted_keys_with_dots() {
    let parsed = path(".metadata.annotations['a.b/c']");
    assert_eq!(parsed.root, PathRoot::Metadata);
    assert_eq!(
        parsed.steps,
        [
            Step::Field("annotations".to_owned()),
            Step::Field("a.b/c".to_owned()),
        ]
    );
    let escaped = path(r#".spec["it\"s"]"#);
    assert_eq!(escaped.steps[1], Step::Field("it\"s".to_owned()));
}

#[test]
fn parses_index_and_wildcard() {
    assert_eq!(
        path(".status.ingress[0].ip").steps,
        [
            Step::Field("status".to_owned()),
            Step::Field("ingress".to_owned()),
            Step::Index(0),
            Step::Field("ip".to_owned()),
        ]
    );
    assert_eq!(path(".spec.hosts[*]").steps[2], Step::Wildcard);
}

#[test]
fn parses_filter_with_spaces_and_both_quotes() {
    for text in [
        r#".status.conditions[?(@.type == "Ready")].status"#,
        ".status.conditions[?(@.type=='Ready')].status",
        r#".status.conditions[ ?( @.type  ==  "Ready" ) ].status"#,
    ] {
        let parsed = path(text);
        assert_eq!(
            parsed.steps[2],
            Step::Filter {
                path: vec!["type".to_owned()],
                literal: "Ready".to_owned(),
            },
            "{text}"
        );
    }
}

#[test]
fn rejects_unsupported_syntax() {
    for text in [
        "{.spec.x}",
        "$.spec.x",
        "spec.x",
        ".*",
        ".items..name",
        "..",
        ".spec.ports[0:2]",
        ".spec.ports[0,1]",
        ".spec.ports[-1]",
        r#".s[?(@.a != "x")]"#,
        r#".s[?(@.a < "x")]"#,
        r#".s[?(@.a == "x" && @.b == "y")]"#,
        r#".s[?(@.a =~ "x")]"#,
        ".s[?(@.a == 1)]",
        ".s[?(@.a == true)]",
        r#".s[?(@ == "x")]"#,
        ".s[range .items]",
        ".",
        "",
        ".spec.x.",
        ".spec[",
        ".spec['open]",
    ] {
        assert_eq!(ColumnPath::parse(text), Err(UnsupportedPath), "{text}");
    }
}

#[test]
fn filter_selects_first_matching_item() {
    let data = certificate();
    let found = path(r#".status.conditions[?(@.type == "Ready")].message"#)
        .first_match(&Value::Null, &data);
    assert_eq!(
        found.map(|found| found.value),
        Some(&json!("Certificate is up to date"))
    );
}

#[test]
fn missing_filter_field_never_matches() {
    let data = json!({"items": [{"name": "a"}, {"other": "Ready"}]});
    let found = path(r#".items[?(@.type == "Ready")]"#).first_match(&Value::Null, &data);
    assert!(found.is_none());
}

#[test]
fn wildcard_returns_first_item() {
    let data = json!({"spec": {"containers": [{"image": "a:1"}, {"image": "b:2"}]}});
    let found = path(".spec.containers[*].image").first_match(&Value::Null, &data);
    assert_eq!(found.map(|found| found.value), Some(&json!("a:1")));
}

#[test]
fn metadata_paths_read_the_metadata_root() {
    let metadata = json!({"creationTimestamp": "2026-01-01T00:00:00Z"});
    let data = json!({"creationTimestamp": "wrong"});
    let parsed = path(".metadata.creationTimestamp");
    assert!(parsed.reads_metadata());
    assert_eq!(
        parsed
            .first_match(&metadata, &data)
            .map(|found| found.value),
        Some(&json!("2026-01-01T00:00:00Z"))
    );
    assert!(!path(".spec.x").reads_metadata());
}

#[test]
fn condition_status_path_is_detected() {
    assert!(path(r#".status.conditions[?(@.type == "Ready")].status"#).is_condition_status());
    assert!(!path(r#".status.conditions[?(@.type == "Ready")].message"#).is_condition_status());
    assert!(!path(".status.phase").is_condition_status());
}

#[test]
fn string_column_stringifies_scalars() {
    let data = json!({"a": "text", "b": 7, "c": true, "d": 1.5});
    assert_eq!(
        cell(ColumnType::String, ".a", &data),
        ColumnValue::Text("text".to_owned())
    );
    assert_eq!(
        cell(ColumnType::String, ".b", &data),
        ColumnValue::Text("7".to_owned())
    );
    assert_eq!(
        cell(ColumnType::String, ".c", &data),
        ColumnValue::Text("true".to_owned())
    );
    assert_eq!(
        cell(ColumnType::String, ".d", &data),
        ColumnValue::Text("1.5".to_owned())
    );
}

#[test]
fn object_values_are_absent_in_string_columns() {
    let data = json!({"map": {"a": 1}, "list": [1], "nothing": null});
    for json_path in [".map", ".list", ".nothing", ".missing"] {
        assert_eq!(
            cell(ColumnType::String, json_path, &data),
            ColumnValue::Absent,
            "{json_path}"
        );
    }
}

#[test]
fn integer_column_rejects_strings_and_floats() {
    let data = json!({"n": 3, "negative": -4, "text": "3", "float": 1.5});
    assert_eq!(
        cell(ColumnType::Integer, ".n", &data),
        ColumnValue::Integer(3)
    );
    assert_eq!(
        cell(ColumnType::Integer, ".negative", &data),
        ColumnValue::Integer(-4)
    );
    assert_eq!(
        cell(ColumnType::Integer, ".text", &data),
        ColumnValue::Absent
    );
    assert_eq!(
        cell(ColumnType::Integer, ".float", &data),
        ColumnValue::Absent
    );
}

#[test]
fn number_column_keeps_json_text() {
    let data = json!({"f": 1.5, "i": 2, "text": "1.5"});
    assert_eq!(
        cell(ColumnType::Number, ".f", &data),
        ColumnValue::Number("1.5".to_owned())
    );
    assert_eq!(
        cell(ColumnType::Number, ".i", &data),
        ColumnValue::Number("2".to_owned())
    );
    assert_eq!(
        cell(ColumnType::Number, ".text", &data),
        ColumnValue::Absent
    );
    let data = json!({"yes": true, "text": "true"});
    assert_eq!(
        cell(ColumnType::Boolean, ".yes", &data),
        ColumnValue::Boolean(true)
    );
    assert_eq!(
        cell(ColumnType::Boolean, ".text", &data),
        ColumnValue::Absent
    );
}

#[test]
fn date_column_parses_rfc3339_only() {
    let data = certificate();
    let expected: jiff::Timestamp = "2026-12-01T00:00:00Z".parse().expect("timestamp");
    assert_eq!(
        cell(ColumnType::Date, ".status.notAfter", &data),
        ColumnValue::Date(expected)
    );
    for text in ["2026-12-01", "next week", ""] {
        let data = json!({"at": text});
        assert_eq!(
            cell(ColumnType::Date, ".at", &data),
            ColumnValue::Absent,
            "{text}"
        );
    }
}

#[test]
fn secret_like_last_field_is_hidden() {
    let data = json!({"spec": {"adminPassword": "distinctive-1", "tokenSecretRef": "plain"}});
    assert_eq!(
        cell(ColumnType::String, ".spec.adminPassword", &data),
        ColumnValue::Hidden
    );
    assert_eq!(
        cell(ColumnType::String, ".spec.tokenSecretRef", &data),
        ColumnValue::Text("plain".to_owned())
    );
    let absent = json!({"spec": {}});
    assert_eq!(
        cell(ColumnType::String, ".spec.adminPassword", &absent),
        ColumnValue::Absent
    );
}

#[test]
fn password_format_columns_are_hidden() {
    let data = json!({"spec": {"note": "distinctive-2"}});
    let path = path(".spec.note");
    let found = path.first_match(&Value::Null, &data);
    let password = PrinterColumn::new("Note", ColumnType::String, ".spec.note", true);
    assert_eq!(column_value(&password, &path, found), ColumnValue::Hidden);
    assert_eq!(column_value(&password, &path, None), ColumnValue::Absent);
}

#[test]
fn long_text_is_cut_at_200_chars() {
    let data = json!({"long": "x".repeat(500), "exact": "y".repeat(200)});
    let ColumnValue::Text(text) = cell(ColumnType::String, ".long", &data) else {
        panic!("expected text");
    };
    assert_eq!(text.chars().count(), 200);
    assert!(text.ends_with('…'));
    let ColumnValue::Text(exact) = cell(ColumnType::String, ".exact", &data) else {
        panic!("expected text");
    };
    assert_eq!(exact.chars().count(), 200);
    assert!(!exact.ends_with('…'));
}

#[test]
fn url_userinfo_is_hidden_in_column_text() {
    let data = json!({"dsn": "postgres://user:distinctive-3@db:5432/app"});
    assert_eq!(
        cell(ColumnType::String, ".dsn", &data),
        ColumnValue::Text("postgres://<hidden>@db:5432/app".to_owned())
    );
}
