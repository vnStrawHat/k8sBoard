//! The Helm release row builder, and the pure model of the History section. A row holds the
//! summary (names, numbers, chart facts) and never a value: the values, manifest, and notes of a
//! release are read on demand by `helm_release_view.rs`. Nothing here logs or traces: a
//! description can quote a field value.

use cluster::{HelmChart, HelmReleaseSummary, HelmRevision, HelmStatus};

use crate::cluster_session::LiveList;
use crate::kind_row::{DetailRow, DetailSection, KindCell, KindObject, KindRow, LiveContent};
use crate::status_tone::{StatusLabel, StatusTone};

/// The title of the Overview section the values view fills. The drawer skips its heading.
pub(crate) const VALUES_CHANGE_TITLE: &str = "Values changed";
/// How many revisions the History section lists.
const MAX_HISTORY_ROWS: usize = 50;

pub(crate) fn helm_release_row(release: &HelmReleaseSummary) -> KindRow {
    let status = helm_status_label(&release.status);
    KindRow {
        namespace: Some(release.namespace.clone()),
        name: release.name.clone(),
        // A release row is stamped with its last deploy, which is not its creation.
        created_at: None,
        status: status.clone(),
        cells: vec![
            release.chart.as_ref().map_or(KindCell::Absent, |chart| {
                KindCell::Mono(chart_text(chart).into())
            }),
            KindCell::text_or_absent(
                release
                    .chart
                    .as_ref()
                    .and_then(|chart| chart.app_version.as_deref()),
            ),
            KindCell::Quantity {
                text: release.revision.to_string().into(),
                value: u64::from(release.revision),
                tone: None,
            },
            KindCell::Toned(status),
            KindCell::age(release.updated_at),
        ],
        sections: vec![
            DetailSection {
                title: "Release",
                rows: release_rows(release),
            },
            DetailSection {
                // The view draws its own heading, which names the revision.
                title: VALUES_CHANGE_TITLE,
                rows: vec![DetailRow::Live(LiveContent::HelmValuesChange)],
            },
            DetailSection {
                title: "History",
                rows: vec![DetailRow::Live(LiveContent::HelmHistory)],
            },
        ],
        event: None,
        related_pods: None,
        labels: Vec::new(),
        object: KindObject::HelmRelease(release.clone()),
    }
}

/// `api-1.2.3`, as `helm list` prints the chart column.
pub(crate) fn chart_text(chart: &HelmChart) -> String {
    format!("{}-{}", chart.name, chart.version)
}

pub(crate) fn helm_status_label(status: &HelmStatus) -> StatusLabel {
    let tone = match status {
        HelmStatus::Deployed => StatusTone::Ok,
        HelmStatus::Failed => StatusTone::Bad,
        HelmStatus::PendingInstall
        | HelmStatus::PendingUpgrade
        | HelmStatus::PendingRollback
        | HelmStatus::Uninstalling
        | HelmStatus::Unknown(_) => StatusTone::Warn,
        HelmStatus::Uninstalled | HelmStatus::Superseded => StatusTone::Done,
    };
    StatusLabel {
        text: status.label().to_owned().into(),
        tone,
    }
}

/// The facts of the Release section. Without a decodable payload the chart facts are missing, and
/// one note says so.
fn release_rows(release: &HelmReleaseSummary) -> Vec<DetailRow> {
    let mut rows = Vec::new();
    match &release.chart {
        Some(chart) => {
            rows.push(DetailRow::field(
                "Chart",
                KindCell::Mono(chart_text(chart).into()),
            ));
            rows.push(DetailRow::field(
                "App version",
                KindCell::text_or_absent(chart.app_version.as_deref()),
            ));
        }
        None => rows.push(DetailRow::Note(
            "The release payload could not be read, so chart facts are missing.".into(),
        )),
    }
    rows.push(DetailRow::field(
        "Revision",
        KindCell::Mono(release.revision.to_string().into()),
    ));
    rows.push(DetailRow::field(
        "Status",
        KindCell::Toned(helm_status_label(&release.status)),
    ));
    rows.push(DetailRow::field(
        "Updated",
        KindCell::age(release.updated_at),
    ));
    if let Some(deployed) = release.deployed_revision {
        rows.push(DetailRow::field(
            "Still deployed",
            KindCell::Mono(format!("rev {deployed}").into()),
        ));
    }
    if let Some(description) = &release.description {
        rows.push(DetailRow::stacked(
            "Description",
            KindCell::Text(description.clone().into()),
        ));
    }
    rows
}

/// One line of the History section, from labels and metadata only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HistoryRow {
    pub(crate) revision: u32,
    pub(crate) status: StatusLabel,
    pub(crate) updated_at: Option<jiff::Timestamp>,
    /// An older revision exists to compare with, listed or not.
    pub(crate) can_diff: bool,
}

/// What the History section shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HistoryModel {
    /// Loading, failed, or empty: one muted note.
    Note(String),
    Rows {
        /// Newest first, at most `MAX_HISTORY_ROWS`.
        rows: Vec<HistoryRow>,
        /// The older revisions that are not listed.
        omitted: usize,
    },
}

/// The model of the related history list; `None` is a list that was not started yet.
pub(crate) fn history_model(list: Option<&LiveList<HelmRevision>>) -> HistoryModel {
    let Some(list) = list else {
        return HistoryModel::Note("Loading…".to_owned());
    };
    match list {
        LiveList::Loading => HistoryModel::Note("Loading…".to_owned()),
        LiveList::Failed { message } => HistoryModel::Note(message.clone()),
        LiveList::Ready { items, .. } if items.is_empty() => {
            HistoryModel::Note("No revisions found.".to_owned())
        }
        LiveList::Ready { items, .. } => {
            let mut revisions: Vec<&HelmRevision> = items.iter().collect();
            revisions.sort_by_key(|revision| std::cmp::Reverse(revision.revision));
            let omitted = revisions.len().saturating_sub(MAX_HISTORY_ROWS);
            let listed = revisions.len().min(MAX_HISTORY_ROWS);
            let rows = revisions
                .into_iter()
                .take(MAX_HISTORY_ROWS)
                .enumerate()
                .map(|(ix, revision)| HistoryRow {
                    revision: revision.revision,
                    status: helm_status_label(&revision.status),
                    updated_at: revision.updated_at,
                    can_diff: ix + 1 < listed || omitted > 0,
                })
                .collect();
            HistoryModel::Rows { rows, omitted }
        }
    }
}

#[cfg(test)]
#[path = "helm_rows_tests.rs"]
mod helm_rows_tests;
