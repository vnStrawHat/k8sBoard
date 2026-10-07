//! Export table… (UX round 3, P9): the visible columns of the filtered rows of the screen's table
//! as a CSV file, in the order and the sort the table shows them. The text is built from what the
//! table already reads (`TableRow::export_text`), and the save flow is the one of every exporter:
//! nothing is written before the save dialog returned a path (C9), and no path is traced.

use std::borrow::Cow;

use gpui_kit::{App, Context};

use super::{AppShell, Screen};
use crate::file_export::{ExportState, export_file_name, start_export};
use crate::table_layout::ColumnPlan;
use crate::table_view::{FilteredTable, TableRow, TableView};

/// The header line, then one line per row the view shows, over the columns it does not hide.
pub(crate) fn table_csv<T: TableRow>(
    view: &TableView,
    plan: &ColumnPlan,
    items: &[T],
    now: jiff::Timestamp,
) -> String {
    let columns: Vec<usize> = (0..plan.specs.len())
        .filter(|column| !view.hidden.contains(column))
        .collect();
    let mut text = csv_line(
        columns
            .iter()
            .map(|&column| plan.specs[column].name.to_owned()),
    );
    for item in view.rows().iter().filter_map(|&index| items.get(index)) {
        text.push_str(&csv_line(
            columns.iter().map(|&column| item.export_text(column, now)),
        ));
    }
    text
}

fn csv_line(fields: impl Iterator<Item = String>) -> String {
    let mut line = fields
        .map(|field| csv_field(&field).into_owned())
        .collect::<Vec<_>>()
        .join(",");
    line.push('\n');
    line
}

/// One CSV field: quoted when it holds a comma, a quote, or a line break (RFC 4180). A spreadsheet
/// reads a text that starts with `=`, `+`, `-`, or `@` as a formula, so such a text that is no
/// number gets a leading `'`: a label or an event message must not run as one.
fn csv_field(text: &str) -> Cow<'_, str> {
    let is_formula = text.starts_with(['=', '+', '-', '@']) && text.parse::<f64>().is_err();
    let text: Cow<'_, str> = if is_formula {
        Cow::Owned(format!("'{text}"))
    } else {
        Cow::Borrowed(text)
    };
    if !text.contains([',', '"', '\n', '\r']) {
        return text;
    }
    Cow::Owned(format!("\"{}\"", text.replace('"', "\"\"")))
}

impl AppShell {
    /// The table of the visible screen as CSV; `None` for a screen without one.
    fn visible_table_csv(&self, now: jiff::Timestamp, cx: &App) -> Option<String> {
        match self.screen {
            Screen::Pods => self.pod_table.read(cx).delegate().export_csv(now, cx),
            Screen::Nodes => self.node_table.read(cx).delegate().export_csv(now, cx),
            Screen::Issues => self.issue_table.read(cx).delegate().export_csv(now, cx),
            Screen::Kind(_) => self.kind_table.read(cx).delegate().export_csv(now, cx),
            Screen::Overview | Screen::Topology | Screen::PortForwarding => None,
        }
    }

    /// Whether the visible screen has a table to export.
    pub(crate) fn has_table_to_export(&self, cx: &App) -> bool {
        self.screen != Screen::Overview
            && self.screen != Screen::Topology
            && self.screen != Screen::PortForwarding
            && self.live(cx).is_some()
    }

    /// `Export table…`: opens the save dialog, then writes the table as it is then (the filter, the
    /// sort, and the columns of that moment) to the chosen path.
    pub(crate) fn export_table(&mut self, cx: &mut Context<Self>) {
        if self.table_export.is_busy() || !self.has_table_to_export(cx) {
            return;
        }
        let Some(session) = self.session() else {
            return;
        };
        // The table must be the one the file name says, so a switch of screen or cluster while the
        // dialog is open cancels it.
        let (screen, session_id) = (self.screen, session.entity_id());
        let label = format!("{}-{}", self.screen_plural(), session.read(cx).context());
        let name = export_file_name(&label, "csv", jiff::Timestamp::now());
        self.table_export = ExportState::Choosing;
        self._table_export = Some(start_export(
            name,
            "table",
            move |shell: &mut Self, cx| {
                let is_same = shell.screen == screen
                    && shell.session().map(|session| session.entity_id()) == Some(session_id);
                if !is_same {
                    return Err("the screen changed while the dialog was open".to_owned());
                }
                shell
                    .visible_table_csv(jiff::Timestamp::now(), cx)
                    .map(|csv| (csv, ()))
                    .ok_or_else(|| "Could not save the table: nothing is shown".to_owned())
            },
            Self::set_table_export,
            |_, ()| {},
            cx,
        ));
        cx.notify();
    }

    fn set_table_export(&mut self, state: ExportState, cx: &mut Context<Self>) {
        self.table_export = state;
        cx.notify();
    }

    /// The plural the file of the visible screen is named after: `pods`, `secrets`.
    fn screen_plural(&self) -> &'static str {
        match self.screen {
            Screen::Pods => "pods",
            Screen::Nodes => "nodes",
            Screen::Issues => "issues",
            Screen::Kind(kind) => kind.plural(),
            Screen::Overview => "overview",
            Screen::Topology => "topology",
            Screen::PortForwarding => "port-forwards",
        }
    }
}

#[cfg(test)]
#[path = "table_export_tests.rs"]
mod table_export_tests;
