//! What the cluster switcher lists, without any view code: the environment sections, the row
//! texts the filter reads, the `Ctrl 1…9` numbers, and the keyboard highlight.

use gpui_kit::SharedString;

use crate::cluster_form::{ClusterGroup, RowOrigin, file_name_text};
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
    /// For a row of a watched folder: what its file reads and runs (the tooltip of the row).
    pub(crate) note: Option<String>,
    /// The `Ctrl n` number: the first nine rows of the unfiltered list.
    pub(crate) shortcut: Option<u8>,
    /// The cluster is open: it has a session in the window.
    pub(crate) is_active: bool,
    /// Lowercased label, context name, environment badge, and file name without any whitespace,
    /// which the filter reads (`normalize_query`).
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
    pub(crate) title: SharedString,
    pub(crate) rows: Vec<SwitcherRow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HighlightStep {
    Next,
    Previous,
}

/// A cluster with a session, and the health that session reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ViewedCluster {
    pub(crate) cluster: ClusterRef,
    pub(crate) health: RowHealth,
}

/// The sections in display order. `viewed` are the clusters with a session, with the health it
/// reports; every other row reads the probe board.
pub(crate) fn switcher_sections(
    groups: &[ClusterGroup],
    health: &HealthBoard,
    viewed: &[ViewedCluster],
) -> Vec<SwitcherSection> {
    let mut next_shortcut = 1_u8;
    groups
        .iter()
        .map(|group| SwitcherSection {
            title: group.title.clone(),
            rows: group
                .rows
                .iter()
                .map(|row| {
                    let active_health = viewed
                        .iter()
                        .find(|viewed| viewed.cluster == row.cluster)
                        .map(|viewed| viewed.health);
                    let environment = row.profile.environment.clone();
                    // A row of a watched folder has no number: Ctrl n would start a cluster
                    // the user did not pick, and the numbers below must not shift for it.
                    let is_numbered = row.origin != RowOrigin::Folder;
                    let shortcut = (is_numbered && next_shortcut <= 9).then_some(next_shortcut);
                    if is_numbered {
                        next_shortcut = next_shortcut.saturating_add(1);
                    }
                    SwitcherRow {
                        failure: health
                            .failure_reason(&row.cluster)
                            .filter(|_| active_health.is_none())
                            .map(str::to_owned),
                        health: active_health.unwrap_or_else(|| health.row_health(&row.cluster)),
                        shortcut,
                        note: row.trust_note.clone(),
                        is_active: active_health.is_some(),
                        search_text: search_text(
                            &row.label,
                            &row.cluster.context,
                            &environment,
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

/// The text the filter reads. Whitespace is dropped inside each part, and the parts are joined by
/// a separator that is not whitespace, so a query never matches across two parts.
pub(crate) fn search_text(
    label: &str,
    context: &str,
    environment: &Environment,
    file: &str,
) -> String {
    [
        label,
        context,
        environment.badge().as_ref(),
        file_name_text(file),
    ]
    .map(normalize_query)
    .join("\u{1f}")
}

/// The filter text as the rows are searched: lowercase, with every whitespace removed, so
/// `prod eu` and `prodeu` both find `prod eu 1` (decision 9: Space never types in the filter).
pub(crate) fn normalize_query(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The rows that match `filter` (a case-insensitive substring) and `segment`; a section left
/// without rows is dropped.
pub(crate) fn visible_sections(
    sections: &[SwitcherSection],
    filter: &str,
    segment: SwitcherSegment,
) -> Vec<SwitcherSection> {
    let needle = normalize_query(filter);
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
                title: section.title.clone(),
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
