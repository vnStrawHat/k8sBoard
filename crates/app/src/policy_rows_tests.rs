use cluster::{Selector, WorkloadCondition};

use super::*;
use crate::resource_kind::ResourceKind;

fn budget(expected: u32, healthy: u32, allowed: u32) -> PodDisruptionBudgetSummary {
    PodDisruptionBudgetSummary {
        namespace: "team-a".to_owned(),
        name: "web".to_owned(),
        created_at: None,
        labels: Vec::new(),
        min_available: Some("2".to_owned()),
        max_unavailable: None,
        selector: Selector::of_labels(&["app=web".to_owned()]),
        current_healthy: healthy,
        desired_healthy: 2,
        expected_pods: expected,
        disruptions_allowed: allowed,
        unhealthy_pod_eviction_policy: None,
        conditions: Vec::new(),
        is_status_stale: false,
    }
}

fn toned(text: &str, tone: StatusTone) -> KindCell {
    KindCell::Toned(StatusLabel {
        text: text.to_owned().into(),
        tone,
    })
}

#[test]
fn pdb_row_cells_match_column_count() {
    let row = pod_disruption_budget_row(&budget(3, 3, 1));
    assert_eq!(
        row.cells.len(),
        ResourceKind::PodDisruptionBudgets.columns().len()
    );
    assert_eq!(row.cells[0], KindCell::Text("2".into()));
    assert_eq!(row.cells[1], KindCell::Absent);
}

#[test]
fn pdb_allowed_cell_tone_by_state() {
    let cell =
        |budget: &PodDisruptionBudgetSummary| pod_disruption_budget_row(budget).cells[2].clone();
    assert_eq!(cell(&budget(3, 3, 1)), toned("1", StatusTone::Ok));
    assert_eq!(cell(&budget(3, 2, 0)), toned("0", StatusTone::Bad));
    assert_eq!(cell(&budget(0, 0, 0)), toned("0", StatusTone::Done));
}

#[test]
fn pdb_status_singular_and_plural() {
    let status = |allowed| pod_disruption_budget_row(&budget(5, 5, allowed)).status;
    assert_eq!(status(1).text.as_ref(), "1 disruption allowed");
    assert_eq!(status(2).text.as_ref(), "2 disruptions allowed");
    assert_eq!(status(2).tone, StatusTone::Ok);
}

#[test]
fn pdb_status_by_state() {
    let status = |budget: &PodDisruptionBudgetSummary| pod_disruption_budget_row(budget).status;
    let blocked = status(&budget(3, 3, 0));
    assert_eq!(blocked.text.as_ref(), "0 disruptions allowed");
    assert_eq!(blocked.tone, StatusTone::Bad);
    let none = status(&budget(0, 0, 0));
    assert_eq!(none.text.as_ref(), "Selects no pods");
    assert_eq!(none.tone, StatusTone::Done);
}

#[test]
fn pdb_sync_failed_status() {
    let mut failed = budget(0, 0, 0);
    failed.conditions = vec![WorkloadCondition {
        name: "DisruptionAllowed".to_owned(),
        is_true: false,
        reason: Some("SyncFailed".to_owned()),
        message: Some("found no controller ref".to_owned()),
    }];
    let status = pod_disruption_budget_row(&failed).status;
    assert_eq!(status.text.as_ref(), "Budget not computed");
    assert_eq!(status.tone, StatusTone::Bad);
}

#[test]
fn pdb_stale_status_note() {
    let note = DetailRow::Note(
        "Status describes an older version of this budget; the controller has not caught up".into(),
    );
    let fresh = pod_disruption_budget_row(&budget(3, 3, 1));
    assert!(
        !fresh
            .section("Budget")
            .expect("section")
            .rows
            .contains(&note)
    );
    let mut stale = budget(3, 3, 1);
    stale.is_status_stale = true;
    let row = pod_disruption_budget_row(&stale);
    assert_eq!(
        row.section("Budget").expect("section").rows.last(),
        Some(&note)
    );
}

#[test]
fn pdb_sections_in_order() {
    let row = pod_disruption_budget_row(&budget(3, 3, 1));
    let titles: Vec<&str> = row.sections.iter().map(|section| section.title).collect();
    assert_eq!(
        titles,
        ["Budget", "Selector", "Selected pods", "Conditions"]
    );
    let budget_rows = &row.section("Budget").expect("section").rows;
    assert_eq!(
        budget_rows[1],
        DetailRow::field("Healthy", KindCell::Text("3 of 3 (needs 2)".into()))
    );
    assert_eq!(
        budget_rows.last(),
        Some(&DetailRow::field(
            "Unhealthy eviction",
            KindCell::Text("IfHealthyBudget (default)".into())
        ))
    );
}

#[test]
fn pdb_selector_section_covers_none_everything_and_terms() {
    let selector_rows = |selector: Option<Selector>| {
        let mut with = budget(3, 3, 1);
        with.selector = selector;
        pod_disruption_budget_row(&with)
            .section("Selector")
            .expect("section")
            .rows
            .clone()
    };
    assert_eq!(
        selector_rows(None),
        [DetailRow::Note("No selector: selects no pods".into())]
    );
    assert_eq!(
        selector_rows(Some(Selector::everything())),
        [DetailRow::Note("All pods in team-a".into())]
    );
    assert_eq!(
        selector_rows(Selector::of_labels(&["app=web".to_owned()])),
        [DetailRow::Chips(vec!["app=web".into()])]
    );
}
