use cluster::HelmReleaseSummary;

use super::*;
use crate::resource_kind::ResourceKind;

fn release(revision: u32, status: HelmStatus) -> HelmReleaseSummary {
    HelmReleaseSummary {
        namespace: "shop".to_owned(),
        name: "api".to_owned(),
        revision,
        status,
        chart: Some(HelmChart {
            name: "api".to_owned(),
            version: "1.2.3".to_owned(),
            app_version: Some("4.5.6".to_owned()),
        }),
        updated_at: jiff::Timestamp::from_second(1_700_000_000).ok(),
        description: None,
        deployed_revision: None,
    }
}

fn revision(number: u32, status: HelmStatus) -> HelmRevision {
    HelmRevision {
        revision: number,
        status,
        updated_at: None,
    }
}

fn ready(items: Vec<HelmRevision>) -> LiveList<HelmRevision> {
    LiveList::Ready {
        items,
        interruption: None,
    }
}

#[test]
fn release_row_cells_match_columns() {
    let row = helm_release_row(&release(7, HelmStatus::Deployed));
    assert_eq!(row.cells.len(), ResourceKind::HelmReleases.columns().len());
    assert_eq!(row.cells[0], KindCell::Mono("api-1.2.3".into()));
    assert_eq!(row.cells[1], KindCell::Text("4.5.6".into()));
    assert_eq!(
        row.cells[3],
        KindCell::Toned(helm_status_label(&HelmStatus::Deployed))
    );
    assert!(matches!(row.cells[4], KindCell::Age { at: Some(_), .. }));
    assert_eq!(row.namespace.as_deref(), Some("shop"));
    assert!(row.labels.is_empty());
}

#[test]
fn release_without_chart_has_absent_cells() {
    let mut summary = release(2, HelmStatus::Failed);
    summary.chart = None;
    let row = helm_release_row(&summary);
    assert_eq!(row.cells[0], KindCell::Absent);
    assert_eq!(row.cells[1], KindCell::Absent);
}

#[test]
fn release_section_notes_missing_payload() {
    let mut summary = release(2, HelmStatus::Failed);
    summary.chart = None;
    let row = helm_release_row(&summary);
    let section = row.section("Release").expect("release section");
    assert_eq!(
        section.rows[0],
        DetailRow::Note(
            "The release payload could not be read, so chart facts are missing.".into()
        )
    );
    let full = helm_release_row(&release(2, HelmStatus::Deployed));
    let labels: Vec<String> = full
        .section("Release")
        .expect("release section")
        .rows
        .iter()
        .filter_map(|row| match row {
            DetailRow::Field { label, .. } => Some(label.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(
        labels,
        ["Chart", "App version", "Revision", "Status", "Updated"]
    );
}

#[test]
fn release_section_names_the_deployed_revision_and_description() {
    let mut summary = release(5, HelmStatus::Failed);
    summary.deployed_revision = Some(4);
    summary.description = Some("Upgrade \"api\" failed".to_owned());
    let row = helm_release_row(&summary);
    let rows = &row.section("Release").expect("release section").rows;
    assert!(rows.iter().any(|row| matches!(row,
        DetailRow::Field { label, value: KindCell::Mono(text) }
            if label.as_ref() == "Still deployed" && text.as_ref() == "rev 4")));
    assert!(rows.iter().any(|row| matches!(row,
        DetailRow::Stacked { label, .. } if label.as_ref() == "Description")));
}

#[test]
fn helm_status_tones() {
    let tone = |status: HelmStatus| helm_status_label(&status).tone;
    assert_eq!(tone(HelmStatus::Deployed), StatusTone::Ok);
    assert_eq!(tone(HelmStatus::Failed), StatusTone::Bad);
    for warn in [
        HelmStatus::PendingInstall,
        HelmStatus::PendingUpgrade,
        HelmStatus::PendingRollback,
        HelmStatus::Uninstalling,
        HelmStatus::Unknown("odd".to_owned()),
    ] {
        assert_eq!(tone(warn), StatusTone::Warn);
    }
    assert_eq!(tone(HelmStatus::Uninstalled), StatusTone::Done);
    assert_eq!(tone(HelmStatus::Superseded), StatusTone::Done);
    assert_eq!(
        helm_status_label(&HelmStatus::PendingUpgrade).text.as_ref(),
        "pending-upgrade"
    );
}

#[test]
fn revision_cell_sorts_numerically() {
    let two = helm_release_row(&release(2, HelmStatus::Deployed));
    let ten = helm_release_row(&release(10, HelmStatus::Deployed));
    let value = |row: &KindRow| match &row.cells[2] {
        KindCell::Quantity { value, .. } => *value,
        other => panic!("expected a quantity, got {other:?}"),
    };
    assert!(value(&two) < value(&ten));
}

#[test]
fn helm_history_rows_newest_first() {
    let list = ready(vec![
        revision(2, HelmStatus::Superseded),
        revision(10, HelmStatus::Deployed),
        revision(9, HelmStatus::Superseded),
    ]);
    let HistoryModel::Rows { rows, omitted } = history_model(Some(&list)) else {
        panic!("expected rows");
    };
    let numbers: Vec<u32> = rows.iter().map(|row| row.revision).collect();
    assert_eq!(numbers, [10, 9, 2]);
    assert_eq!(omitted, 0);
}

#[test]
fn helm_history_caps_at_fifty() {
    let list = ready(
        (1..=60)
            .map(|number| revision(number, HelmStatus::Superseded))
            .collect(),
    );
    let HistoryModel::Rows { rows, omitted } = history_model(Some(&list)) else {
        panic!("expected rows");
    };
    assert_eq!(rows.len(), MAX_HISTORY_ROWS);
    assert_eq!(rows[0].revision, 60);
    assert_eq!(omitted, 10);
}

#[test]
fn helm_history_loading_failed_empty_notes() {
    let note = |list: Option<&LiveList<HelmRevision>>| match history_model(list) {
        HistoryModel::Note(text) => text,
        HistoryModel::Rows { .. } => panic!("expected a note"),
    };
    assert_eq!(note(None), "Loading…");
    assert_eq!(note(Some(&LiveList::Loading)), "Loading…");
    assert_eq!(
        note(Some(&LiveList::Failed {
            message: "Not permitted: list secrets".to_owned()
        })),
        "Not permitted: list secrets"
    );
    assert_eq!(note(Some(&ready(Vec::new()))), "No revisions found.");
}

#[test]
fn oldest_history_row_has_no_diff() {
    let list = ready(vec![
        revision(3, HelmStatus::Deployed),
        revision(2, HelmStatus::Superseded),
        revision(1, HelmStatus::Superseded),
    ]);
    let HistoryModel::Rows { rows, .. } = history_model(Some(&list)) else {
        panic!("expected rows");
    };
    let can_diff: Vec<bool> = rows.iter().map(|row| row.can_diff).collect();
    assert_eq!(can_diff, [true, true, false]);
    // A history cut at the cap still has older revisions to compare with.
    let long = ready(
        (1..=60)
            .map(|number| revision(number, HelmStatus::Superseded))
            .collect(),
    );
    let HistoryModel::Rows { rows, .. } = history_model(Some(&long)) else {
        panic!("expected rows");
    };
    assert!(rows.last().is_some_and(|row| row.can_diff));
}

#[test]
fn release_row_has_values_changed_section_between_release_and_history() {
    let row = helm_release_row(&release(7, HelmStatus::Deployed));
    let titles: Vec<&str> = row.sections.iter().map(|section| section.title).collect();
    assert_eq!(titles, ["Release", VALUES_CHANGE_TITLE, "History"]);
    assert_eq!(
        row.sections[1].rows,
        [DetailRow::Live(LiveContent::HelmValuesChange)]
    );
}
