//! The rows a table shows: which items pass the filter, in which order. The view holds item
//! indices only, so the table never owns a copy of the rows and a newer snapshot cannot race it.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::time::Instant;

use gpui_kit::App;
use gpui_kit::SharedString;
use gpui_kit::component::table::TableDelegate;

use crate::status_tone::StatusTone;
use crate::table_filter::{FilterChip, TableFilter, matches};
use crate::table_layout::ColumnPlan;
use crate::table_sort::{TableSort, compare_values};

/// What the toolkit reads from a row. Implemented next to each delegate. `column` is a logical
/// column: an index into the table's full column list, before any column is hidden.
pub(crate) trait TableRow {
    fn namespace(&self) -> Option<&str>;
    fn name(&self) -> &str;
    /// `key=value` terms.
    fn labels(&self) -> impl Iterator<Item = &str>;
    fn tone(&self) -> StatusTone;
    fn value(&self, column: usize) -> CellValue<'_>;
}

/// One cell as the filter and the sort read it.
pub(crate) enum CellValue<'a> {
    /// Natural order.
    Text(Cow<'a, str>),
    /// Ordered by the prefix, then the text.
    Qualified {
        prefix: Option<&'a str>,
        text: &'a str,
    },
    Number(i64),
    Status {
        tone: StatusTone,
        text: SharedString,
    },
    /// Ascending is youngest first.
    Age(Option<jiff::Timestamp>),
    /// A running item (`finished: None`) lasts until now.
    Span {
        started: Option<jiff::Timestamp>,
        finished: Option<jiff::Timestamp>,
    },
    Absent,
}

/// What `AppShell` needs from any toolkit table.
pub(crate) trait FilteredTable: TableDelegate {
    /// `None` for a kind table without a kind.
    fn view(&self) -> Option<&TableView>;
    fn view_mut(&mut self) -> Option<&mut TableView>;
    /// The columns the Columns menu lists.
    fn column_plan(&self) -> Option<&ColumnPlan>;
    /// Reads the session items, rebuilds the view, and lays the columns out again at the last
    /// width. Returns whether the columns changed; the caller then refreshes the table.
    fn rebuild_view(&mut self, cx: &App) -> bool;
}

/// Filter, sort, and hidden columns of one table, and the item indices they produce.
#[derive(Default)]
pub(crate) struct TableView {
    pub(crate) filter: TableFilter,
    pub(crate) sort: Option<TableSort>,
    /// Logical columns.
    pub(crate) hidden: BTreeSet<usize>,
    /// Item indices in display order.
    rows: Vec<usize>,
    total: usize,
}

impl TableView {
    /// Keeps the items that pass the filter, then orders them. A sort computes each kept row's
    /// key once and is stable, so ties keep the source order.
    pub(crate) fn rebuild<T: TableRow>(
        &mut self,
        items: &[T],
        column_count: usize,
        now: jiff::Timestamp,
    ) {
        let started = Instant::now();
        self.total = items.len();
        let mut kept: Vec<usize> = (0..items.len())
            .filter(|&index| matches(&items[index], &self.filter, column_count))
            .collect();
        if let Some(sort) = self.sort {
            let keys: Vec<CellValue> = kept
                .iter()
                .map(|&index| items[index].value(sort.column))
                .collect();
            let mut order: Vec<usize> = (0..kept.len()).collect();
            order.sort_by(|&a, &b| compare_values(&keys[a], &keys[b], sort.direction, now));
            kept = order.into_iter().map(|position| kept[position]).collect();
        }
        self.rows = kept;
        tracing::trace!(
            rows = self.total,
            kept = self.rows.len(),
            elapsed_us = started.elapsed().as_micros() as u64,
            "rebuilt a table view"
        );
    }

    /// The item indices to show, in order.
    pub(crate) fn rows(&self) -> &[usize] {
        &self.rows
    }

    /// The item shown at table row `row`.
    pub(crate) fn item_index(&self, row: usize) -> Option<usize> {
        self.rows.get(row).copied()
    }

    /// The table row that shows `item`; `None` when the filter hides it. Linear.
    pub(crate) fn row_of(&self, item: usize) -> Option<usize> {
        self.rows.iter().position(|&index| index == item)
    }

    /// All items, shown or not.
    pub(crate) fn total(&self) -> usize {
        self.total
    }

    pub(crate) fn is_filtering(&self) -> bool {
        self.filter.is_active()
    }

    /// Removes the text and the chips; the sort and the hidden columns stay.
    pub(crate) fn clear_filter(&mut self) {
        self.filter = TableFilter::default();
    }

    /// Adds the chips that are not already present.
    pub(crate) fn add_chips(&mut self, chips: Vec<FilterChip>) {
        for chip in chips {
            if !self.filter.chips.contains(&chip) {
                self.filter.chips.push(chip);
            }
        }
    }
}

#[cfg(test)]
#[path = "table_view_tests.rs"]
mod table_view_tests;
