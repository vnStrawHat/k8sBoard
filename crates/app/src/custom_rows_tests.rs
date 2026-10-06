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
        ("Running", StatusTone::Ok)
    );
    // A word the table does not know stays Info.
    let other = custom_status(&[], Some("Reconciling soon"));
    assert_eq!(other.tone, StatusTone::Info);
    let completed = custom_status(&[], Some("Completed"));
    assert_eq!(completed.tone, StatusTone::Ok);
}

#[test]
fn no_status_without_conditions_or_phase() {
    let status = custom_status(&[], None);
    assert_eq!(
        (status.text.as_ref(), status.tone),
        ("No status", StatusTone::Done)
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

// Drawer sections

fn with_message(mut condition: ObjectCondition, message: &str) -> ObjectCondition {
    condition.message = Some(message.to_owned());
    condition
}

fn field(path: &str, value: FieldValue) -> FieldEntry {
    FieldEntry {
        path: path.to_owned(),
        value,
    }
}

fn mono(text: &str) -> KindCell {
    KindCell::Mono(text.to_owned().into())
}

fn ready_fields(fields: CustomObjectFields) -> LiveList<CustomObjectFields> {
    LiveList::Ready {
        items: vec![fields],
        interruption: None,
    }
}

#[test]
fn conditions_rows_tone_by_type_and_reason() {
    let rows = conditions_rows(&[
        condition("Ready", ConditionStatus::True, None),
        condition("Available", ConditionStatus::False, Some("Down")),
        condition("Synced", ConditionStatus::Unknown, None),
        with_message(
            condition("Issuing", ConditionStatus::False, Some("Failed")),
            "ACME challenge returned 404",
        ),
        condition("Progressing", ConditionStatus::True, Some("Updated")),
    ]);
    assert_eq!(
        rows,
        [
            DetailRow::field("Ready", toned("True", StatusTone::Ok)),
            DetailRow::field("Available", toned("False · Down", StatusTone::Bad)),
            // A type other than Ready and Available is neutral (Done) unless its reason names a failure.
            DetailRow::field("Synced", toned("Unknown", StatusTone::Done)),
            DetailRow::field("Issuing", toned("False · Failed", StatusTone::Bad)),
            DetailRow::Note("ACME challenge returned 404".into()),
            DetailRow::field("Progressing", toned("True · Updated", StatusTone::Done)),
        ]
    );
}

#[test]
fn ready_unknown_is_warn_in_the_conditions() {
    let rows = conditions_rows(&[condition("Ready", ConditionStatus::Unknown, None)]);
    assert_eq!(
        rows,
        [DetailRow::field(
            "Ready",
            toned("Unknown", StatusTone::Warn)
        )]
    );
}

#[test]
fn no_conditions_say_so() {
    assert_eq!(
        conditions_rows(&[]),
        [DetailRow::Note("No conditions reported.".into())]
    );
}

#[test]
fn status_fields_render_entries_and_omitted_note() {
    let fields = CustomObjectFields {
        spec: FieldList::default(),
        status: FieldList {
            entries: vec![
                field(
                    "notAfter",
                    FieldValue::Text("2026-12-01T00:00:00Z".to_owned()),
                ),
                field("revision", FieldValue::Text("5".to_owned())),
                field("token", FieldValue::Hidden),
                field("rules", FieldValue::Items(3)),
                field("sync", FieldValue::Fields(2)),
                field("long", FieldValue::Text("x".repeat(61))),
            ],
            omitted: 4,
        },
    };
    let rows = field_list_rows(
        Some(&ready_fields(fields)),
        FieldsSide::Status,
        Some("shop"),
    );
    assert_eq!(
        rows,
        [
            DetailRow::field("notAfter", mono("2026-12-01T00:00:00Z")),
            DetailRow::field("revision", mono("5")),
            DetailRow::field("token", KindCell::Text("<hidden>".into())),
            DetailRow::field("rules", KindCell::Text("3 items".into())),
            DetailRow::field("sync", KindCell::Text("2 fields".into())),
            DetailRow::stacked("long", mono(&"x".repeat(61))),
            DetailRow::Note("4 more fields in the YAML tab.".into()),
        ]
    );
}

#[test]
fn secret_name_fields_link_to_secrets() {
    let fields = CustomObjectFields {
        spec: FieldList {
            entries: vec![
                field("secretName", FieldValue::Text("web-tls".to_owned())),
                field("issuerRef.secretName", FieldValue::Text("ca".to_owned())),
                field(
                    "secretNameSuffix",
                    FieldValue::Text("not-a-link".to_owned()),
                ),
            ],
            omitted: 0,
        },
        status: FieldList::default(),
    };
    let list = ready_fields(fields);
    let rows = field_list_rows(Some(&list), FieldsSide::Spec, Some("shop"));
    let target = |name: &str| ResourceKey::Kind {
        kind: crate::resource_kind::ResourceKind::Secrets,
        namespace: Some("shop".to_owned()),
        name: name.to_owned(),
    };
    assert_eq!(
        rows[0],
        DetailRow::Link {
            label: "secretName".into(),
            text: "web-tls".into(),
            target: target("web-tls")
        }
    );
    assert!(matches!(&rows[1], DetailRow::Link { target: found, .. } if *found == target("ca")));
    assert_eq!(
        rows[2],
        DetailRow::field("secretNameSuffix", mono("not-a-link"))
    );
    // A cluster-scoped object has no namespace to look a Secret up in.
    let cluster = field_list_rows(Some(&list), FieldsSide::Spec, None);
    assert_eq!(cluster[0], DetailRow::field("secretName", mono("web-tls")));
}

#[test]
fn field_list_states_render_notes() {
    let note = |text: &str| vec![DetailRow::Note(text.to_owned().into())];
    assert_eq!(
        field_list_rows(None, FieldsSide::Spec, None),
        note("Loading…")
    );
    assert_eq!(
        field_list_rows(Some(&LiveList::Loading), FieldsSide::Spec, None),
        note("Loading…")
    );
    let failed = LiveList::<CustomObjectFields>::Failed {
        message: "the watch failed".to_owned(),
    };
    assert_eq!(
        field_list_rows(Some(&failed), FieldsSide::Status, None),
        note("the watch failed")
    );
    let gone = LiveList::<CustomObjectFields>::Ready {
        items: Vec::new(),
        interruption: None,
    };
    assert_eq!(
        field_list_rows(Some(&gone), FieldsSide::Spec, None),
        note("The object no longer exists.")
    );
    let empty = ready_fields(CustomObjectFields::default());
    assert_eq!(
        field_list_rows(Some(&empty), FieldsSide::Status, None),
        note("No status fields.")
    );
    assert_eq!(
        field_list_rows(Some(&empty), FieldsSide::Spec, None),
        note("No spec fields.")
    );
}

#[test]
fn custom_row_sections_are_live_placeholders() {
    let row = custom_object_row(certificate_kind(), &certificate_summary());
    let titles: Vec<_> = row.sections.iter().map(|section| section.title).collect();
    assert_eq!(titles, ["Conditions", "Status", "Spec"]);
    assert_eq!(
        row.sections[1].rows,
        [DetailRow::Live(LiveContent::CustomStatus)]
    );
}

#[test]
fn long_labels_stack_above_their_values() {
    let fields = CustomObjectFields {
        spec: FieldList {
            entries: vec![
                field("operationState.message", FieldValue::Text("ok".to_owned())),
                field("history", FieldValue::Items(2)),
                field("operationState.syncResult", FieldValue::Fields(3)),
            ],
            omitted: 0,
        },
        status: FieldList::default(),
    };
    let rows = field_list_rows(Some(&ready_fields(fields)), FieldsSide::Spec, None);
    assert_eq!(
        rows,
        [
            DetailRow::stacked("operationState.message", mono("ok")),
            DetailRow::field("history", KindCell::Text("2 items".into())),
            DetailRow::stacked(
                "operationState.syncResult",
                KindCell::Text("3 fields".into())
            ),
        ]
    );
}

#[test]
fn state_words_have_their_tone() {
    for word in [
        "Healthy",
        "synced",
        "True",
        "Ready",
        "Running",
        "Succeeded",
        "Completed",
        "Bound",
        "Available",
    ] {
        assert_eq!(value_tone(word), Some(StatusTone::Ok), "{word}");
    }
    for word in ["Degraded", "Failed", "ERROR", "False", "Missing"] {
        assert_eq!(value_tone(word), Some(StatusTone::Bad), "{word}");
    }
    for word in [
        "OutOfSync",
        "Progressing",
        "Pending",
        "Unknown",
        "Suspended",
        "Terminating",
    ] {
        assert_eq!(value_tone(word), Some(StatusTone::Warn), "{word}");
    }
    // Anything else is never painted as good or bad.
    for word in ["", "Reconciling", "default", "Healthy now", "1"] {
        assert_eq!(value_tone(word), None, "{word}");
    }
}

#[test]
fn status_named_string_columns_are_toned() {
    let toned_cell = |name: &str, value: &str| {
        custom_cell(
            &column(name, ColumnType::String, ".status.x"),
            ColumnRule::Plain,
            &text(value),
        )
    };
    assert_eq!(
        toned_cell("Sync Status", "OutOfSync"),
        toned("OutOfSync", StatusTone::Warn)
    );
    assert_eq!(
        toned_cell("Health Status", "Healthy"),
        toned("Healthy", StatusTone::Ok)
    );
    assert_eq!(
        toned_cell("Phase", "Failed"),
        toned("Failed", StatusTone::Bad)
    );
    assert_eq!(toned_cell("READY", "True"), toned("True", StatusTone::Ok));
    // A column that is not named like a state keeps plain text, whatever the value says.
    assert_eq!(
        toned_cell("Project", "Healthy"),
        KindCell::Text("Healthy".into())
    );
    // An unfamiliar value in a status column has no tone.
    assert_eq!(
        toned_cell("Health Status", "Weird"),
        KindCell::Text("Weird".into())
    );
    // Only string columns are read: a number column called Status is not.
    let number = column("Status", ColumnType::Number, ".status.x");
    assert_eq!(
        custom_cell(
            &number,
            ColumnRule::Plain,
            &ColumnValue::Number("1".to_owned())
        ),
        KindCell::Text("1".into())
    );
}

/// Argo CD Applications: no Ready condition and no phase, state only in printer columns.
fn application_kind() -> CustomKind {
    let crd = CrdSummary {
        name: "applications.argoproj.io".to_owned(),
        group: "argoproj.io".to_owned(),
        kind: "Application".to_owned(),
        plural: "applications".to_owned(),
        singular: "application".to_owned(),
        scope: ResourceScope::Namespaced,
        versions: vec![CrdVersion {
            name: "v1alpha1".to_owned(),
            is_served: true,
            is_storage: true,
            is_deprecated: false,
            deprecation_warning: None,
            printer_columns: vec![
                column("Sync Status", ColumnType::String, ".status.sync.status"),
                column("Health Status", ColumnType::String, ".status.health.status"),
                column("Revision", ColumnType::String, ".status.sync.revision"),
            ],
            schema: SchemaOutline::default(),
        }],
        state: CrdState::Established,
        created_at: None,
    };
    custom_kinds(&[crd], &mut CustomKindCache::default())[0]
}

#[test]
fn status_comes_from_the_status_columns_when_nothing_else_reports_one() {
    let kind = application_kind();
    let row = |sync: &str, health: &str| {
        custom_object_row(
            kind,
            &summary(vec![text(sync), text(health), text("a1b2c3")], Vec::new()),
        )
        .status
    };
    let synced = row("Synced", "Healthy");
    assert_eq!(
        (synced.text.as_ref(), synced.tone),
        ("Synced · Healthy", StatusTone::Ok)
    );
    let drifted = row("OutOfSync", "Healthy");
    assert_eq!(
        (drifted.text.as_ref(), drifted.tone),
        ("OutOfSync · Healthy", StatusTone::Warn)
    );
    let broken = row("Synced", "Degraded");
    assert_eq!(broken.tone, StatusTone::Bad);
    // A value the table does not know stays neutral.
    assert_eq!(row("Odd", "Strange").tone, StatusTone::Done);
}

#[test]
fn a_ready_condition_beats_the_status_columns() {
    let row = custom_object_row(
        application_kind(),
        &summary(
            vec![text("Synced"), text("Healthy"), text("a1b2c3")],
            vec![condition("Ready", ConditionStatus::True, None)],
        ),
    );
    assert_eq!(row.status.text.as_ref(), "Ready");
}

#[test]
fn no_status_columns_and_no_signal_stays_no_status() {
    let row = custom_object_row(application_kind(), &summary(vec![], Vec::new()));
    assert_eq!(
        (row.status.text.as_ref(), row.status.tone),
        ("No status", StatusTone::Done)
    );
}
