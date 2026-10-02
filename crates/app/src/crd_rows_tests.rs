use cluster::{ColumnType, PrinterColumn, SchemaOutline};

use super::*;
use crate::resource_kind::{ResourceKind, kind_columns};

fn version(name: &str) -> CrdVersion {
    CrdVersion {
        name: name.to_owned(),
        is_served: true,
        is_storage: false,
        is_deprecated: false,
        deprecation_warning: None,
        printer_columns: Vec::new(),
        schema: SchemaOutline::default(),
    }
}

fn crd(versions: Vec<CrdVersion>, state: CrdState) -> CrdSummary {
    CrdSummary {
        name: "certificates.cert-manager.io".to_owned(),
        group: "cert-manager.io".to_owned(),
        kind: "Certificate".to_owned(),
        plural: "certificates".to_owned(),
        singular: "certificate".to_owned(),
        scope: ResourceScope::Namespaced,
        versions,
        state,
        created_at: jiff::Timestamp::from_second(1_700_000_000).ok(),
    }
}

fn field(name: &str, field_type: Option<&str>) -> SchemaField {
    SchemaField {
        name: name.to_owned(),
        field_type: field_type.map(str::to_owned),
    }
}

fn section<'a>(row: &'a KindRow, title: &str) -> &'a [DetailRow] {
    &row.section(title).expect("section exists").rows
}

#[test]
fn crd_row_cells_match_column_count() {
    let row = crd_row(&crd(vec![version("v1")], CrdState::Established));
    // The Name column is not a cell.
    assert_eq!(row.cells.len(), kind_columns(ResourceKind::Crds).len() - 1);
    assert_eq!(row.name, "certificates.cert-manager.io");
    assert_eq!(row.namespace, None);
    assert_eq!(row.cells[0], KindCell::Mono("cert-manager.io".into()));
    assert_eq!(row.cells[1], KindCell::Mono("v1".into()));
    assert_eq!(row.cells[2], KindCell::Text("Namespaced".into()));
    assert_eq!(row.cells[3], KindCell::Absent);
}

#[test]
fn crd_without_a_served_version_has_no_version_cell() {
    let mut unserved = version("v1");
    unserved.is_served = false;
    let row = crd_row(&crd(vec![unserved], CrdState::Established));
    assert_eq!(row.cells[1], KindCell::Absent);
    assert_eq!(
        section(&row, "Printer columns"),
        [DetailRow::Note("No served version.".into())]
    );
}

#[test]
fn crd_status_follows_state() {
    let label = |state| crd_status(&state);
    let tone = |state| label(state).tone;
    assert_eq!(tone(CrdState::Established), StatusTone::Ok);
    assert_eq!(
        tone(CrdState::NamesNotAccepted { message: None }),
        StatusTone::Bad
    );
    assert_eq!(
        tone(CrdState::NotEstablished { reason: None }),
        StatusTone::Warn
    );
    assert_eq!(tone(CrdState::Terminating), StatusTone::Info);
    assert_eq!(label(CrdState::Established).text, "Established");
}

#[test]
fn not_established_definition_carries_the_reason() {
    let row = crd_row(&crd(
        vec![version("v1")],
        CrdState::NotEstablished {
            reason: Some("Installing".to_owned()),
        },
    ));
    let rows = section(&row, "Definition");
    assert_eq!(rows.last(), Some(&DetailRow::Note("Installing".into())));
    assert_eq!(
        rows[1],
        DetailRow::field(
            "Plural",
            KindCell::Mono("certificates / certificate".into())
        )
    );
}

#[test]
fn versions_section_marks_preferred_storage_and_deprecated() {
    let mut old = version("v1beta1");
    old.is_deprecated = true;
    old.deprecation_warning = Some("use v1".to_owned());
    let mut current = version("v1");
    current.is_storage = true;
    let mut unserved = version("v1alpha1");
    unserved.is_served = false;
    let row = crd_row(&crd(vec![old, current, unserved], CrdState::Established));
    let toned = |text: &str, tone| {
        KindCell::Toned(StatusLabel {
            text: text.into(),
            tone,
        })
    };
    assert_eq!(
        section(&row, "Versions"),
        [
            DetailRow::field("v1beta1", toned("deprecated", StatusTone::Warn)),
            DetailRow::Note("use v1".into()),
            DetailRow::field("v1 (preferred)", toned("served · storage", StatusTone::Ok)),
            DetailRow::field("v1alpha1", toned("not served", StatusTone::Done)),
        ]
    );
}

#[test]
fn printer_columns_section_marks_unsupported() {
    let mut preferred = version("v1");
    preferred.printer_columns = vec![
        PrinterColumn::new("Ready", ColumnType::String, ".status.phase", false),
        PrinterColumn::new("Odd", ColumnType::String, ".spec.ports[0:2]", false),
    ];
    let row = crd_row(&crd(vec![preferred], CrdState::Established));
    assert_eq!(
        section(&row, "Printer columns"),
        [DetailRow::Chips(vec![
            "Ready".into(),
            "Odd (unsupported)".into()
        ])]
    );
    let bare = crd_row(&crd(vec![version("v1")], CrdState::Established));
    assert_eq!(
        section(&bare, "Printer columns"),
        [DetailRow::Note(
            "No printer columns: the table shows Name and Age.".into()
        )]
    );
}

#[test]
fn schema_section_renders_outline_and_omitted() {
    let mut preferred = version("v1");
    preferred.schema = SchemaOutline {
        spec: vec![field("issuerRef", Some("object")), field("duration", None)],
        status: vec![field("notAfter", Some("string"))],
        omitted: 3,
    };
    let row = crd_row(&crd(vec![preferred], CrdState::Established));
    assert_eq!(
        section(&row, "Schema"),
        [DetailRow::Code(
            "spec:\n  issuerRef: object\n  duration: any\nstatus:\n  notAfter: string\n… 3 more"
                .into()
        )]
    );
}

#[test]
fn missing_schema_reads_no_schema() {
    let row = crd_row(&crd(vec![version("v1")], CrdState::Established));
    assert_eq!(
        section(&row, "Schema"),
        [DetailRow::Note("No schema.".into())]
    );
}
