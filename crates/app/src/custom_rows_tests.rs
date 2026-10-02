use cluster::{ColumnType, CrdState, CrdSummary, CrdVersion, ResourceScope, SchemaOutline};

use super::*;
use crate::custom_kind::{CustomKindCache, custom_kinds};

fn column(name: &str, column_type: ColumnType, path: &str) -> PrinterColumn {
    PrinterColumn::new(name, column_type, path, false)
}

/// cert-manager Certificates: Ready (toned), Issuer, then the built-in Expires and Age.
fn certificate_kind() -> CustomKind {
    let crd = CrdSummary {
        name: "certificates.cert-manager.io".to_owned(),
        group: "cert-manager.io".to_owned(),
        kind: "Certificate".to_owned(),
        plural: "certificates".to_owned(),
        singular: "certificate".to_owned(),
        scope: ResourceScope::Namespaced,
        versions: vec![CrdVersion {
            name: "v1".to_owned(),
            is_served: true,
            is_storage: true,
            is_deprecated: false,
            deprecation_warning: None,
            printer_columns: vec![
                column(
                    "Ready",
                    ColumnType::String,
                    ".status.conditions[?(@.type==\"Ready\")].status",
                ),
                column("Issuer", ColumnType::String, ".spec.issuerRef.name"),
                column("Age", ColumnType::Date, ".metadata.creationTimestamp"),
            ],
            schema: SchemaOutline::default(),
        }],
        state: CrdState::Established,
        created_at: None,
    };
    custom_kinds(&[crd], &mut CustomKindCache::default())[0]
}

fn timestamp(second: i64) -> Timestamp {
    Timestamp::from_second(second).expect("valid timestamp")
}

fn condition(name: &str, status: ConditionStatus, reason: Option<&str>) -> ObjectCondition {
    ObjectCondition {
        name: name.to_owned(),
        status,
        reason: reason.map(str::to_owned),
        message: None,
        changed_at: None,
    }
}

fn summary(columns: Vec<ColumnValue>, conditions: Vec<ObjectCondition>) -> CustomObjectSummary {
    CustomObjectSummary {
        namespace: Some("ingress".to_owned()),
        name: "tls-shop-example".to_owned(),
        created_at: Some(timestamp(1_000_000)),
        labels: vec!["team=web".to_owned()],
        columns,
        conditions,
        phase: None,
    }
}

fn text(value: &str) -> ColumnValue {
    ColumnValue::Text(value.to_owned())
}

fn certificate_summary() -> CustomObjectSummary {
    summary(
        vec![
            text("True"),
            text("letsencrypt-prod"),
            ColumnValue::Date(timestamp(2_000_000)),
            ColumnValue::Date(timestamp(1_000_000)),
        ],
        vec![condition("Ready", ConditionStatus::True, None)],
    )
}

fn toned(text: &str, tone: StatusTone) -> KindCell {
    KindCell::Toned(StatusLabel {
        text: text.to_owned().into(),
        tone,
    })
}

#[test]
fn custom_row_cells_match_kind_columns() {
    let kind = certificate_kind();
    let row = custom_object_row(kind, &certificate_summary());
    // The Name column is not a cell.
    assert_eq!(row.cells.len(), kind.spec().columns.len());
    assert_eq!(row.cells.len(), kind.printer_columns().len());
    assert_eq!(row.name, "tls-shop-example");
    assert_eq!(row.namespace.as_deref(), Some("ingress"));
    assert_eq!(row.labels, ["team=web"]);
    assert_eq!(row.cells[1], KindCell::Text("letsencrypt-prod".into()));
    assert!(matches!(row.object, KindObject::Custom(_)));
}

#[test]
fn missing_values_read_absent() {
    let row = custom_object_row(certificate_kind(), &summary(Vec::new(), Vec::new()));
    assert!(row.cells.iter().all(|cell| *cell == KindCell::Absent));
}

#[test]
fn kinds_without_printer_columns_show_age_only() {
    let crd = CrdSummary {
        name: "widgets.x.io".to_owned(),
        group: "x.io".to_owned(),
        kind: "Widget".to_owned(),
        plural: "widgets".to_owned(),
        singular: "widget".to_owned(),
        scope: ResourceScope::Namespaced,
        versions: vec![CrdVersion {
            name: "v1".to_owned(),
            is_served: true,
            is_storage: true,
            is_deprecated: false,
            deprecation_warning: None,
            printer_columns: Vec::new(),
            schema: SchemaOutline::default(),
        }],
        state: CrdState::Established,
        created_at: None,
    };
    let kind = custom_kinds(&[crd], &mut CustomKindCache::default())[0];
    let row = custom_object_row(kind, &summary(Vec::new(), Vec::new()));
    assert_eq!(row.cells, [KindCell::age(Some(timestamp(1_000_000)))]);
}

#[test]
fn condition_status_cells_are_toned() {
    let kind = certificate_kind();
    let cell = |status: &str| {
        custom_object_row(kind, &summary(vec![text(status)], Vec::new())).cells[0].clone()
    };
    assert_eq!(cell("True"), toned("True", StatusTone::Ok));
    assert_eq!(cell("False"), toned("False", StatusTone::Bad));
    assert_eq!(cell("Unknown"), toned("Unknown", StatusTone::Warn));
    // Any other text is not a condition status.
    assert_eq!(cell("Maybe"), KindCell::Text("Maybe".into()));
}

#[test]
fn plain_text_is_not_toned_even_when_it_reads_true() {
    let kind = certificate_kind();
    let row = custom_object_row(kind, &summary(vec![text("x"), text("True")], Vec::new()));
    assert_eq!(row.cells[1], KindCell::Text("True".into()));
}

#[test]
fn hidden_values_read_hidden() {
    let row = custom_object_row(
        certificate_kind(),
        &summary(vec![ColumnValue::Hidden], Vec::new()),
    );
    assert_eq!(row.cells[0], KindCell::Text("<hidden>".into()));
}

#[test]
fn negative_integers_are_text() {
    let column = column("Count", ColumnType::Integer, ".spec.count");
    let cell = |number| custom_cell(&column, ColumnRule::Plain, &ColumnValue::Integer(number));
    assert_eq!(
        cell(7),
        KindCell::Quantity {
            text: "7".into(),
            value: 7,
            tone: None
        }
    );
    assert_eq!(cell(-3), KindCell::Text("-3".into()));
}

#[test]
fn numbers_and_booleans_are_text() {
    let number = column("Ratio", ColumnType::Number, ".spec.ratio");
    assert_eq!(
        custom_cell(
            &number,
            ColumnRule::Plain,
            &ColumnValue::Number("1.5".to_owned())
        ),
        KindCell::Text("1.5".into())
    );
    let flag = column("Paused", ColumnType::Boolean, ".spec.paused");
    assert_eq!(
        custom_cell(&flag, ColumnRule::Plain, &ColumnValue::Boolean(false)),
        KindCell::Text("false".into())
    );
}

#[test]
fn expiry_dates_get_the_expiry_rule() {
    let kind = certificate_kind();
    let row = custom_object_row(kind, &certificate_summary());
    // Ready, Issuer, Expires (built-in), Age.
    assert_eq!(
        row.cells[2],
        KindCell::Date {
            at: timestamp(2_000_000),
            rule: DateRule::Expiry
        }
    );
    assert_eq!(row.cells[3], KindCell::age(Some(timestamp(1_000_000))));
    let until = column("Until", ColumnType::Date, ".spec.until");
    assert_eq!(
        custom_cell(&until, ColumnRule::Plain, &ColumnValue::Date(timestamp(5))),
        KindCell::Date {
            at: timestamp(5),
            rule: DateRule::Plain
        }
    );
}

#[test]
fn ready_condition_sets_status() {
    let ready = custom_status(&[condition("Ready", ConditionStatus::True, None)], None);
    assert_eq!((ready.text.as_ref(), ready.tone), ("Ready", StatusTone::Ok));
    let not_ready = custom_status(
        &[condition("Ready", ConditionStatus::False, Some("Pending"))],
        None,
    );
    assert_eq!(
        (not_ready.text.as_ref(), not_ready.tone),
        ("Pending", StatusTone::Bad)
    );
    let bare = custom_status(&[condition("Ready", ConditionStatus::False, None)], None);
    assert_eq!(bare.text.as_ref(), "Not ready");
    let unknown = custom_status(&[condition("Ready", ConditionStatus::Unknown, None)], None);
    assert_eq!(
        (unknown.text.as_ref(), unknown.tone),
        ("Unknown", StatusTone::Warn)
    );
}

#[test]
fn available_is_used_without_ready() {
    let available = custom_status(
        &[condition("Available", ConditionStatus::True, None)],
        Some("Running"),
    );
    assert_eq!(
        (available.text.as_ref(), available.tone),
        ("Available", StatusTone::Ok)
    );
    let down = custom_status(
        &[condition("Available", ConditionStatus::False, None)],
        None,
    );
    assert_eq!(
        (down.text.as_ref(), down.tone),
        ("Unavailable", StatusTone::Bad)
    );
}

#[test]
fn failing_condition_warns_even_when_ready() {
    let conditions = [
        condition("Ready", ConditionStatus::True, None),
        condition("Issuing", ConditionStatus::False, Some("Failed")),
    ];
    let status = custom_status(&conditions, None);
    assert_eq!(
        (status.text.as_ref(), status.tone),
        ("Issuing: Failed", StatusTone::Warn)
    );
    // A reason that names an error counts too, whatever its case.
    let errored = custom_status(
        &[condition(
            "Synced",
            ConditionStatus::False,
            Some("ReconcileERROR"),
        )],
        None,
    );
    assert_eq!(errored.text.as_ref(), "Synced: ReconcileERROR");
}

#[test]
fn phase_is_info_without_conditions() {
    let status = custom_status(&[], Some("Running"));
    assert_eq!(
        (status.text.as_ref(), status.tone),
        ("Running", StatusTone::Info)
    );
}

#[test]
fn no_status_without_conditions_or_phase() {
    let status = custom_status(&[], None);
    assert_eq!(
        (status.text.as_ref(), status.tone),
        ("No status", StatusTone::Info)
    );
    // A condition that is neither failing nor Ready/Available says nothing.
    let quiet = custom_status(&[condition("Synced", ConditionStatus::True, None)], None);
    assert_eq!(quiet.text.as_ref(), "No status");
}

#[test]
fn date_text_reads_past_and_future() {
    let now = timestamp(10 * 86_400);
    assert_eq!(date_text(timestamp(5 * 86_400), now), "5d");
    assert_eq!(date_text(timestamp(16 * 86_400), now), "in 6d");
    assert_eq!(date_text(now, now), "0s");
}

#[test]
fn expiry_date_tone_follows_tls_thresholds() {
    let now = timestamp(1_000 * 86_400);
    let days = |count: i64| timestamp(now.as_second() + count * 86_400);
    assert_eq!(date_tone(DateRule::Expiry, days(64), now), None);
    assert_eq!(
        date_tone(DateRule::Expiry, days(6), now),
        Some(StatusTone::Warn)
    );
    assert_eq!(
        date_tone(DateRule::Expiry, days(-2), now),
        Some(StatusTone::Bad)
    );
    assert_eq!(date_tone(DateRule::Plain, days(-2), now), None);
}
