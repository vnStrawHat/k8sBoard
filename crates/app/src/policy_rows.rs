//! Row builders for the policy kinds that read cluster limits: PodDisruptionBudgets.

use cluster::{BlockCause, DisruptionState, PodDisruptionBudgetSummary};

use crate::kind_row::{
    DetailRow, DetailSection, KindCell, KindObject, KindRow, LiveContent, chips,
};
use crate::status_tone::{StatusLabel, StatusTone};
use crate::workload_rows::condition_row;

const DEFAULT_EVICTION_POLICY: &str = "IfHealthyBudget (default)";

pub(crate) fn pod_disruption_budget_row(budget: &PodDisruptionBudgetSummary) -> KindRow {
    let allowed = allowed_label(budget);
    let mut budget_rows = Vec::new();
    if let Some(min_available) = &budget.min_available {
        budget_rows.push(DetailRow::field(
            "Min available",
            KindCell::Text(min_available.clone().into()),
        ));
    }
    if let Some(max_unavailable) = &budget.max_unavailable {
        budget_rows.push(DetailRow::field(
            "Max unavailable",
            KindCell::Text(max_unavailable.clone().into()),
        ));
    }
    budget_rows.push(DetailRow::field(
        "Healthy",
        KindCell::Text(
            format!(
                "{} of {} (needs {})",
                budget.current_healthy, budget.expected_pods, budget.desired_healthy
            )
            .into(),
        ),
    ));
    budget_rows.push(DetailRow::field(
        "Allowed disruptions",
        KindCell::Toned(allowed.clone()),
    ));
    budget_rows.push(DetailRow::field(
        "Unhealthy eviction",
        KindCell::Text(
            budget
                .unhealthy_pod_eviction_policy
                .clone()
                .unwrap_or_else(|| DEFAULT_EVICTION_POLICY.to_owned())
                .into(),
        ),
    ));
    if budget.is_status_stale {
        budget_rows.push(DetailRow::Note(
            "Status describes an older version of this budget; the controller has not caught up"
                .into(),
        ));
    }
    KindRow {
        namespace: Some(budget.namespace.clone()),
        name: budget.name.clone(),
        created_at: budget.created_at,
        status: budget_status(budget),
        cells: vec![
            KindCell::text_or_absent(budget.min_available.as_deref()),
            KindCell::text_or_absent(budget.max_unavailable.as_deref()),
            KindCell::Toned(allowed),
            KindCell::age(budget.created_at),
        ],
        sections: vec![
            DetailSection {
                title: "Budget",
                rows: budget_rows,
            },
            selector_section(budget),
            DetailSection {
                title: "Selected pods",
                rows: vec![DetailRow::Live(LiveContent::SelectedPods)],
            },
            DetailSection {
                title: "Conditions",
                rows: budget.conditions.iter().map(condition_row).collect(),
            },
        ],
        event: None,
        related_pods: None,
        labels: chips(&budget.labels),
        object: KindObject::PodDisruptionBudget(budget.clone()),
    }
}

/// The disruptions the budget allows right now, as a colored count: Bad when the budget refuses
/// every eviction, muted when it selects no pods.
fn allowed_label(budget: &PodDisruptionBudgetSummary) -> StatusLabel {
    let (text, tone) = match budget.disruption_state() {
        DisruptionState::Allowed(count) => (count.to_string(), StatusTone::Ok),
        DisruptionState::Blocked(_) => ("0".to_owned(), StatusTone::Bad),
        DisruptionState::NoPods => ("0".to_owned(), StatusTone::Done),
    };
    StatusLabel {
        text: text.into(),
        tone,
    }
}

fn budget_status(budget: &PodDisruptionBudgetSummary) -> StatusLabel {
    let (text, tone) = match budget.disruption_state() {
        DisruptionState::Allowed(1) => ("1 disruption allowed".to_owned(), StatusTone::Ok),
        DisruptionState::Allowed(count) => (format!("{count} disruptions allowed"), StatusTone::Ok),
        DisruptionState::Blocked(BlockCause::SyncFailed) => {
            ("Budget not computed".to_owned(), StatusTone::Bad)
        }
        DisruptionState::Blocked(BlockCause::UnhealthyPods | BlockCause::NoRoom) => {
            ("0 disruptions allowed".to_owned(), StatusTone::Bad)
        }
        DisruptionState::NoPods => ("Selects no pods".to_owned(), StatusTone::Done),
    };
    StatusLabel {
        text: text.into(),
        tone,
    }
}

fn selector_section(budget: &PodDisruptionBudgetSummary) -> DetailSection {
    let row = match &budget.selector {
        None => DetailRow::Note("No selector: selects no pods".into()),
        Some(selector) if selector.selects_everything() => {
            DetailRow::Note(format!("All pods in {}", budget.namespace).into())
        }
        Some(selector) => DetailRow::Chips(chips(&selector.terms())),
    };
    DetailSection {
        title: "Selector",
        rows: vec![row],
    }
}

#[cfg(test)]
#[path = "policy_rows_tests.rs"]
mod policy_rows_tests;
