//! What the cluster switcher lists, without any view code: the environment sections, the row
//! texts the filter reads, the `Ctrl 1…9` numbers, and the keyboard highlight.

use crate::cluster_form::{ClusterGroup, file_name_text};
use crate::cluster_health::{HealthBoard, RowHealth};
use crate::cluster_registry::ClusterRef;
use crate::environment::Environment;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SwitcherSegment {
    All,
    Connected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SwitcherRow {
    pub(crate) cluster: ClusterRef,
    pub(crate) label: String,
    pub(crate) environment: Environment,
    pub(crate) health: RowHealth,
    /// Why the last probe failed; the tooltip of the Retry button.
    pub(crate) failure: Option<String>,
    /// The `Ctrl n` number: the first nine rows of the unfiltered list.
    pub(crate) shortcut: Option<u8>,
    pub(crate) is_active: bool,
    /// Lowercased label, context name, environment badge, and file name, which the filter reads.
    pub(crate) search_text: String,
}

impl SwitcherRow {
    /// Whether the last known health says the cluster answers.
    fn is_connected(&self) -> bool {
        matches!(self.health, RowHealth::Live(_) | RowHealth::Reachable(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SwitcherSection {
    pub(crate) title: &'static str,
    pub(crate) rows: Vec<SwitcherRow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HighlightStep {
    Next,
    Previous,
}

/// The sections in display order. `active` is the open cluster with the health its session
/// reports; every other row reads the probe board.
pub(crate) fn switcher_sections(
    groups: &[ClusterGroup],
    health: &HealthBoard,
    active: Option<(&ClusterRef, RowHealth)>,
) -> Vec<SwitcherSection> {
    let mut next_shortcut = 1_u8;
    groups
        .iter()
        .map(|group| SwitcherSection {
            title: group.title,
            rows: group
                .rows
                .iter()
                .map(|row| {
                    let active_health = active
                        .filter(|(cluster, _)| **cluster == row.cluster)
                        .map(|(_, health)| health);
                    let environment = row.profile.environment;
                    let shortcut = (next_shortcut <= 9).then_some(next_shortcut);
                    next_shortcut = next_shortcut.saturating_add(1);
                    SwitcherRow {
                        failure: health
                            .failure_reason(&row.cluster)
                            .filter(|_| active_health.is_none())
                            .map(str::to_owned),
                        health: active_health.unwrap_or_else(|| health.row_health(&row.cluster)),
                        shortcut,
                        is_active: active_health.is_some(),
                        search_text: search_text(
                            &row.label,
                            &row.cluster.context,
                            environment,
                            &row.cluster.kubeconfig.to_string_lossy(),
                        ),
                        label: row.label.clone(),
                        environment,
                        cluster: row.cluster.clone(),
                    }
                })
                .collect(),
        })
        .collect()
}

fn search_text(label: &str, context: &str, environment: Environment, file: &str) -> String {
    format!(
        "{label}\n{context}\n{}\n{}",
        environment.badge(),
        file_name_text(file)
    )
    .to_lowercase()
}

/// The rows that match `filter` (a case-insensitive substring) and `segment`; a section left
/// without rows is dropped.
pub(crate) fn visible_sections(
    sections: &[SwitcherSection],
    filter: &str,
    segment: SwitcherSegment,
) -> Vec<SwitcherSection> {
    let needle = filter.trim().to_lowercase();
    sections
        .iter()
        .filter_map(|section| {
            let rows: Vec<SwitcherRow> = section
                .rows
                .iter()
                .filter(|row| row.search_text.contains(&needle))
                .filter(|row| segment == SwitcherSegment::All || row.is_connected())
                .cloned()
                .collect();
            (!rows.is_empty()).then_some(SwitcherSection {
                title: section.title,
                rows,
            })
        })
        .collect()
}

pub(crate) fn row_count(sections: &[SwitcherSection]) -> usize {
    sections.iter().map(|section| section.rows.len()).sum()
}

pub(crate) fn connected_count(sections: &[SwitcherSection]) -> usize {
    sections
        .iter()
        .flat_map(|section| &section.rows)
        .filter(|row| row.is_connected())
        .count()
}

/// The cluster `Ctrl n` opens. It reads the unfiltered list: the numbers do not move while the
/// user types in the filter.
pub(crate) fn nth_cluster(sections: &[SwitcherSection], shortcut: u8) -> Option<&ClusterRef> {
    sections
        .iter()
        .flat_map(|section| &section.rows)
        .find(|row| row.shortcut == Some(shortcut))
        .map(|row| &row.cluster)
}

/// The row the highlight moves to, wrapping at both ends. A highlight that is not in `visible`
/// (the filter hid it) starts from the first row going down and from the last going up.
pub(crate) fn move_highlight(
    visible: &[SwitcherSection],
    current: Option<&ClusterRef>,
    step: HighlightStep,
) -> Option<ClusterRef> {
    let rows: Vec<&SwitcherRow> = visible.iter().flat_map(|section| &section.rows).collect();
    let last = rows.len().checked_sub(1)?;
    let position = current.and_then(|cluster| rows.iter().position(|row| row.cluster == *cluster));
    let next = match (position, step) {
        (None, HighlightStep::Next) => 0,
        (None, HighlightStep::Previous) => last,
        (Some(index), HighlightStep::Next) => (index + 1) % rows.len(),
        (Some(index), HighlightStep::Previous) => index.checked_sub(1).unwrap_or(last),
    };
    Some(rows[next].cluster.clone())
}

#[cfg(test)]
#[path = "cluster_switcher_rows_tests.rs"]
mod cluster_switcher_rows_tests;
