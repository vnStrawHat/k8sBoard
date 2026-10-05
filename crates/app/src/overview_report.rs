//! The Overview report: the four W3 sections as Markdown tables. The builder is pure; the save flow
//! lives on `AppShell` and writes only after the save dialog returned a path (C9).
//!
//! The report holds no kubeconfig data: no server URL, user, token, or certificate. Only the
//! context name appears, as the header shows it. Event messages are arbitrary text, so nothing here
//! logs anything.

use jiff::Timestamp;

use crate::cluster_capacity::CapacityRow;
use crate::cluster_session::LiveCluster;
use crate::event_rows::message_line;
use crate::issue::Issue;
use crate::issue_board::IssueBoard;
use crate::node_heatmap::{HeatCell, heat_cells};
use crate::overview::{
    CHANGES_FOOTNOTE, ChangeFeed, capacity_model, change_feed, headline_text, object_line,
    stats_text,
};
use crate::recent_changes::{ChangeEntry, ChangeInputs, ChangeWindow, recent_changes};
use cluster::NodeReadiness;

/// What the report reads.
pub(crate) struct ReportInputs<'a> {
    pub(crate) headline: &'a str,
    pub(crate) stats: &'a str,
    /// The issue board has run, so an empty list means no issues rather than not looked at yet.
    pub(crate) checked: bool,
    pub(crate) issues: &'a [Issue],
    /// The coverage note, when coverage is partial.
    pub(crate) coverage_note: Option<String>,
    pub(crate) capacity: &'a [CapacityRow],
    pub(crate) cells: &'a [HeatCell],
    pub(crate) changes: &'a [ChangeEntry],
    /// Why there is no table of changes: a feed is loading, failed, or denied.
    pub(crate) changes_note: Option<String>,
    pub(crate) window: ChangeWindow,
    pub(crate) now: Timestamp,
}

/// The report of the live snapshots at `now`: every issue and every change in the window, not
/// only the rows the panels show.
pub(crate) fn live_report(
    live: &LiveCluster,
    board: &IssueBoard,
    context: &str,
    window: ChangeWindow,
    now: Timestamp,
) -> String {
    let headline = headline_text(context, &live.server_version, live.nodes.ready_items());
    let stats = stats_text(live);
    let node_feed = &live.metrics.nodes;
    let usage = crate::overview::is_polling(&node_feed.status).then_some(&node_feed.history);
    let cells = heat_cells(live.nodes.items(), usage);
    let (rollouts, rescales, changes_note) = match change_feed(live) {
        ChangeFeed::Ready { rollouts, rescales } => (Some(rollouts), Some(rescales), None),
        ChangeFeed::Loading => (
            None,
            None,
            Some("Changes were still loading when the report was made.".to_owned()),
        ),
        ChangeFeed::Unavailable(text) => (None, None, Some(text)),
    };
    let changes = recent_changes(&ChangeInputs {
        rollouts,
        rescales,
        nodes: live.nodes.ready_items(),
        namespaces: live.namespaces.ready_items(),
        deployments: live.issue_feeds.deployments(),
        window,
        now,
    });
    overview_report(&ReportInputs {
        headline: &headline,
        stats: &stats,
        checked: board.summary().is_some(),
        issues: board.issues(),
        coverage_note: board
            .coverage()
            .is_partial()
            .then(|| board.coverage().note())
            .flatten(),
        capacity: &capacity_model(live),
        cells: &cells,
        changes: &changes,
        changes_note,
        window,
        now,
    })
}

/// Markdown: the title and generated time (RFC 3339), then the four W3 sections as tables.
pub(crate) fn overview_report(inputs: &ReportInputs) -> String {
    let mut text = format!(
        "# Overview — {}\n\nGenerated {}\n\n{}\n\n",
        inputs.headline, inputs.now, inputs.stats
    );
    attention_section(&mut text, inputs);
    capacity_section(&mut text, inputs.capacity);
    nodes_section(&mut text, inputs.cells);
    changes_section(&mut text, inputs);
    text
}

fn attention_section(text: &mut String, inputs: &ReportInputs) {
    if !inputs.checked {
        text.push_str("## Needs attention (—)\n\nNot checked yet.\n\n");
        return;
    }
    text.push_str(&format!("## Needs attention ({})\n\n", inputs.issues.len()));
    if inputs.issues.is_empty() {
        text.push_str("No issues found.\n\n");
    } else {
        push_header(text, &["Severity", "Reason", "Object", "Cause", "Since"]);
        for issue in inputs.issues {
            push_row(
                text,
                &[
                    issue.severity.label(),
                    &issue.reason,
                    &object_line(issue),
                    &message_line(&issue.cause),
                    &issue
                        .onset
                        .map_or_else(|| "—".to_owned(), |onset| onset.to_string()),
                ],
            );
        }
        text.push('\n');
    }
    if let Some(note) = &inputs.coverage_note {
        text.push_str(&format!("Partial coverage: {note}\n\n"));
    }
}

fn capacity_section(text: &mut String, rows: &[CapacityRow]) {
    text.push_str("## Capacity\n\n");
    push_header(text, &["Name", "Figures", "Note", "Ceiling"]);
    for row in rows {
        let label: String = row
            .label_parts()
            .into_iter()
            .map(|part| part.text)
            .collect();
        push_row(
            text,
            &[
                row.name(),
                &label,
                row.note().as_deref().unwrap_or(""),
                row.ceiling().unwrap_or(""),
            ],
        );
    }
    text.push('\n');
}

fn nodes_section(text: &mut String, cells: &[HeatCell]) {
    text.push_str("## Nodes\n\n");
    push_header(text, &["Node", "Status", "CPU %", "Memory %"]);
    let share = |ratio: Option<f64>| {
        ratio.map_or_else(|| "—".to_owned(), crate::usage_format::format_percent)
    };
    for cell in cells {
        let mut status = match cell.readiness {
            NodeReadiness::Ready => "Ready",
            NodeReadiness::NotReady => "NotReady",
            NodeReadiness::Unknown => "Unknown",
        }
        .to_owned();
        if cell.is_cordoned {
            status.push_str(", SchedulingDisabled");
        }
        push_row(
            text,
            &[
                &cell.node,
                &status,
                &share(cell.usage.cpu),
                &share(cell.usage.memory),
            ],
        );
    }
    text.push('\n');
}

fn changes_section(text: &mut String, inputs: &ReportInputs) {
    text.push_str(&format!(
        "## Recent changes ({})\n\n",
        inputs.window.label()
    ));
    if let Some(note) = &inputs.changes_note {
        text.push_str(&format!("{note}\n"));
    } else {
        push_changes_table(text, inputs.changes);
    }
    text.push_str(&format!("\n{CHANGES_FOOTNOTE}\n"));
}

fn push_changes_table(text: &mut String, changes: &[ChangeEntry]) {
    push_header(text, &["Time", "Change", "Who"]);
    for entry in changes {
        let count = if entry.count > 1 {
            format!(" ×{}", entry.count)
        } else {
            String::new()
        };
        let change = format!(
            "{} {} {}{count}",
            entry.kind.label(),
            entry.object,
            message_line(&entry.text)
        );
        push_row(
            text,
            &[
                &entry.at.to_string(),
                &change,
                entry.actor.as_deref().unwrap_or(""),
            ],
        );
    }
}

fn push_header(text: &mut String, columns: &[&str]) {
    push_row(text, columns);
    let rule: Vec<&str> = columns.iter().map(|_| "---").collect();
    push_row(text, &rule);
}

fn push_row(text: &mut String, cells: &[&str]) {
    let escaped: Vec<String> = cells.iter().map(|text| cell(text)).collect();
    text.push_str(&format!("| {} |\n", escaped.join(" | ")));
}

/// A table cell: `|` is escaped and line breaks become spaces.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
        .replace("\r\n", " ")
        .replace(['\n', '\r'], " ")
}

#[cfg(test)]
#[path = "overview_report_tests.rs"]
mod overview_report_tests;
