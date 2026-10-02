//! The CRD row builder. A row holds the definition (names, versions, printer columns, a two-level
//! schema outline): no instance data, so nothing here can leak a credential. The Instances cell is
//! filled later by the counts join.

use cluster::{CrdState, CrdSummary, CrdVersion, ResourceScope, SchemaField};

use crate::kind_row::{DetailRow, DetailSection, KindCell, KindObject, KindRow};
use crate::status_tone::{StatusLabel, StatusTone};

pub(crate) fn crd_row(crd: &CrdSummary) -> KindRow {
    let preferred = crd.preferred_version();
    KindRow {
        namespace: None,
        name: crd.name.clone(),
        created_at: crd.created_at,
        status: crd_status(&crd.state),
        cells: vec![
            KindCell::mono_or_absent(&crd.group),
            preferred.map_or(KindCell::Absent, |version| {
                KindCell::mono_or_absent(&version.name)
            }),
            KindCell::Text(scope_text(crd.scope).into()),
            // The cluster-wide counts join fills it.
            KindCell::Absent,
            KindCell::age(crd.created_at),
        ],
        sections: vec![
            DetailSection {
                title: "Definition",
                rows: definition_rows(crd),
            },
            DetailSection {
                title: "Versions",
                rows: version_rows(crd, preferred),
            },
            DetailSection {
                title: "Printer columns",
                rows: printer_column_rows(preferred),
            },
            DetailSection {
                title: "Schema",
                rows: schema_rows(preferred),
            },
        ],
        event: None,
        related_pods: None,
        labels: Vec::new(),
        object: KindObject::Crd(crd.clone()),
    }
}

pub(crate) fn crd_status(state: &CrdState) -> StatusLabel {
    let (text, tone) = match state {
        CrdState::Established => ("Established", StatusTone::Ok),
        CrdState::NamesNotAccepted { .. } => ("Names not accepted", StatusTone::Bad),
        CrdState::NotEstablished { .. } => ("Not established", StatusTone::Warn),
        CrdState::Terminating => ("Terminating", StatusTone::Info),
    };
    StatusLabel {
        text: text.into(),
        tone,
    }
}

fn scope_text(scope: ResourceScope) -> &'static str {
    match scope {
        ResourceScope::Namespaced => "Namespaced",
        ResourceScope::Cluster => "Cluster",
    }
}

fn definition_rows(crd: &CrdSummary) -> Vec<DetailRow> {
    let mut rows = vec![
        DetailRow::field("Kind", KindCell::Mono(crd.kind.clone().into())),
        DetailRow::field(
            "Plural",
            KindCell::Mono(format!("{} / {}", crd.plural, crd.singular).into()),
        ),
        DetailRow::field("Scope", KindCell::Text(scope_text(crd.scope).into())),
    ];
    let note = match &crd.state {
        CrdState::NamesNotAccepted { message } => message.as_deref(),
        CrdState::NotEstablished { reason } => reason.as_deref(),
        CrdState::Established | CrdState::Terminating => None,
    };
    if let Some(note) = note {
        rows.push(DetailRow::Note(note.to_owned().into()));
    }
    rows
}

fn version_rows(crd: &CrdSummary, preferred: Option<&CrdVersion>) -> Vec<DetailRow> {
    let mut rows = Vec::new();
    for version in &crd.versions {
        let is_preferred = preferred.is_some_and(|preferred| preferred.name == version.name);
        let label = if is_preferred {
            format!("{} (preferred)", version.name)
        } else {
            version.name.clone()
        };
        rows.push(DetailRow::field(
            label,
            KindCell::Toned(version_status(version)),
        ));
        if version.is_deprecated
            && let Some(warning) = &version.deprecation_warning
        {
            rows.push(DetailRow::Note(warning.clone().into()));
        }
    }
    rows
}

fn version_status(version: &CrdVersion) -> StatusLabel {
    let (text, tone) = if !version.is_served {
        ("not served", StatusTone::Done)
    } else if version.is_deprecated {
        ("deprecated", StatusTone::Warn)
    } else if version.is_storage {
        ("served · storage", StatusTone::Ok)
    } else {
        ("served", StatusTone::Ok)
    };
    StatusLabel {
        text: text.into(),
        tone,
    }
}

fn printer_column_rows(preferred: Option<&CrdVersion>) -> Vec<DetailRow> {
    let Some(version) = preferred else {
        return vec![DetailRow::Note("No served version.".into())];
    };
    if version.printer_columns.is_empty() {
        return vec![DetailRow::Note(
            "No printer columns: the table shows Name and Age.".into(),
        )];
    }
    let names = version
        .printer_columns
        .iter()
        .map(|column| {
            if column.is_supported {
                column.name.clone().into()
            } else {
                format!("{} (unsupported)", column.name).into()
            }
        })
        .collect();
    vec![DetailRow::Chips(names)]
}

fn schema_rows(preferred: Option<&CrdVersion>) -> Vec<DetailRow> {
    let outline = preferred.map(|version| &version.schema);
    let Some(outline) = outline.filter(|outline| {
        !outline.spec.is_empty() || !outline.status.is_empty() || outline.omitted > 0
    }) else {
        return vec![DetailRow::Note("No schema.".into())];
    };
    let mut lines = Vec::new();
    for (title, fields) in [("spec", &outline.spec), ("status", &outline.status)] {
        if fields.is_empty() {
            continue;
        }
        lines.push(format!("{title}:"));
        lines.extend(fields.iter().map(field_line));
    }
    if outline.omitted > 0 {
        lines.push(format!("… {} more", outline.omitted));
    }
    vec![DetailRow::Code(lines.join("\n").into())]
}

fn field_line(field: &SchemaField) -> String {
    format!(
        "  {}: {}",
        field.name,
        field.field_type.as_deref().unwrap_or("any")
    )
}

#[cfg(test)]
#[path = "crd_rows_tests.rs"]
mod crd_rows_tests;
