//! The rows a table shows: which items pass the filter, in which order. The view holds item
//! indices only, so the table never owns a copy of the rows and a newer snapshot cannot race it.

use std::borrow::Cow;
use std::collections::{BTreeSet, HashSet};
use std::time::Instant;

use gpui_kit::App;
use gpui_kit::SharedString;
use gpui_kit::component::table::TableDelegate;

use crate::app_shell::Screen;
use crate::resource_kind::ResourceKind;
use crate::settings::{SavedSort, TablePrefs};
use crate::status_tone::StatusTone;
use crate::table_filter::{FilterChip, FilterPreset, TableFilter, matches};
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
    /// Whether the row passes the screen's own switch. A switch the screen does not have
    /// keeps every row.
    fn in_preset(&self, preset: &FilterPreset) -> bool;
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
    /// Ticks or unticks rows of the view, reading the session items for their identity.
    fn check_rows(&mut self, change: RowCheck, cx: &App);
}

/// Filter, sort, and hidden columns of one table, and the item indices they produce.
#[derive(Default)]
pub(crate) struct TableView {
    pub(crate) filter: TableFilter,
    pub(crate) sort: Option<TableSort>,
    /// Logical columns.
    pub(crate) hidden: BTreeSet<usize>,
    /// What a context switch restores, and what Clear filters does not.
    default_filter: TableFilter,
    /// The ticked rows by identity, so they survive a reorder. Only visible rows stay ticked.
    checked: HashSet<RowName>,
    /// The row a Shift click extends the range from.
    anchor: Option<RowName>,
    /// Item indices in display order.
    rows: Vec<usize>,
    total: usize,
}

/// A row's identity within one table: `(namespace, name)` is unique in the one open cluster. A
/// switch clears every tick, so a name never carries over to another cluster.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RowName {
    namespace: Option<String>,
    name: String,
}

impl RowName {
    fn of<T: TableRow>(row: &T) -> Self {
        Self {
            namespace: row.namespace().map(str::to_owned),
            name: row.name().to_owned(),
        }
    }
}

/// A change of the ticked rows, by table row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowCheck {
    Toggle(usize),
    Range(usize),
    All(bool),
}

/// The filter a screen starts with: ReplicaSets hide the inactive ones (decision 26), and
/// ClusterRoles and ClusterRoleBindings hide the `system:*` objects (0015 decision 16).
pub(crate) fn default_filter(screen: Screen) -> TableFilter {
    match screen {
        Screen::Kind(ResourceKind::ReplicaSets) => TableFilter {
            preset: Some(FilterPreset::HideInactive),
            ..TableFilter::default()
        },
        Screen::Kind(ResourceKind::ClusterRoles | ResourceKind::ClusterRoleBindings) => {
            TableFilter {
                preset: Some(FilterPreset::HideSystem),
                ..TableFilter::default()
            }
        }
        Screen::Overview
        | Screen::Pods
        | Screen::Nodes
        | Screen::Issues
        | Screen::Topology
        | Screen::PortForwarding
        | Screen::Kind(_) => TableFilter::default(),
    }
}

impl TableView {
    /// A view whose filter starts at `default_filter`.
    pub(crate) fn new(default_filter: TableFilter) -> Self {
        Self {
            filter: default_filter.clone(),
            default_filter,
            ..Self::default()
        }
    }

    /// Replaces the sort and the hidden columns with saved ones. A name the plan does not know
    /// is dropped, and the flexible column is never hidden.
    pub(crate) fn apply_prefs(&mut self, prefs: &TablePrefs, plan: &ColumnPlan) {
        let index_of = |name: &str| plan.specs.iter().position(|spec| spec.name == name);
        self.hidden = prefs
            .hidden
            .iter()
            .filter_map(|name| index_of(name))
            .filter(|&column| column != plan.flexible)
            .collect();
        self.sort = prefs.sort.as_ref().and_then(|saved| {
            Some(TableSort {
                column: index_of(&saved.column)?,
                direction: saved.direction,
            })
        });
    }

    /// The sort and the hidden columns by column name, in logical order.
    pub(crate) fn prefs(&self, plan: &ColumnPlan) -> TablePrefs {
        let name_of = |column: usize| plan.specs.get(column).map(|spec| spec.name.to_owned());
        TablePrefs {
            sort: self.sort.and_then(|sort| {
                Some(SavedSort {
                    column: name_of(sort.column)?,
                    direction: sort.direction,
                })
            }),
            hidden: self
                .hidden
                .iter()
                .filter_map(|&column| name_of(column))
                .collect(),
        }
    }

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
        self.prune_checked(items);
        tracing::trace!(
            rows = self.total,
            kept = self.rows.len(),
            elapsed_us = started.elapsed().as_micros() as u64,
            "rebuilt a table view"
        );
    }

    /// A filter, a deleted object, or a scope change unticks: bulk actions never reach a row that
    /// is not shown.
    fn prune_checked<T: TableRow>(&mut self, items: &[T]) {
        if self.checked.is_empty() && self.anchor.is_none() {
            return;
        }
        // Borrowed keys: nothing is allocated per row.
        let visible: HashSet<(Option<&str>, &str)> = self
            .rows
            .iter()
            .map(|&index| {
                let row = &items[index];
                (row.namespace(), row.name())
            })
            .collect();
        let is_visible =
            |name: &RowName| visible.contains(&(name.namespace.as_deref(), name.name.as_str()));
        self.checked.retain(is_visible);
        if self
            .anchor
            .as_ref()
            .is_some_and(|anchor| !is_visible(anchor))
        {
            self.anchor = None;
        }
    }

    pub(crate) fn is_checked<T: TableRow>(&self, row: &T) -> bool {
        self.checked.contains(&RowName::of(row))
    }

    pub(crate) fn checked_count(&self) -> usize {
        self.checked.len()
    }

    /// Ticks or unticks one row, and makes it the anchor of the next range.
    fn toggle_checked<T: TableRow>(&mut self, row: &T) {
        let name = RowName::of(row);
        if !self.checked.remove(&name) {
            self.checked.insert(name.clone());
        }
        self.anchor = Some(name);
    }

    /// Ticks every shown row between the anchor and table row `row`, both included, in view
    /// order. Without an anchor it acts like `toggle_checked`.
    fn check_range<T: TableRow>(&mut self, items: &[T], row: usize) {
        let Some(item) = self.item_index(row) else {
            return;
        };
        let anchor_row = self.anchor.as_ref().and_then(|anchor| {
            self.rows
                .iter()
                .position(|&index| RowName::of(&items[index]) == *anchor)
        });
        let Some(anchor_row) = anchor_row else {
            self.toggle_checked(&items[item]);
            return;
        };
        let (from, to) = (anchor_row.min(row), anchor_row.max(row));
        for &index in &self.rows[from..=to] {
            self.checked.insert(RowName::of(&items[index]));
        }
    }

    /// Ticks or unticks every shown row; rows the filter hides are not touched.
    fn set_all_checked<T: TableRow>(&mut self, items: &[T], checked: bool) {
        for &index in &self.rows {
            let name = RowName::of(&items[index]);
            if checked {
                self.checked.insert(name);
            } else {
                self.checked.remove(&name);
            }
        }
        self.anchor = None;
    }

    /// Whether every shown row is ticked; false for an empty table.
    pub(crate) fn all_checked<T: TableRow>(&self, items: &[T]) -> bool {
        !self.rows.is_empty()
            && self
                .rows
                .iter()
                .all(|&index| self.is_checked(&items[index]))
    }

    /// Applies one change of the ticked rows.
    pub(crate) fn apply_check<T: TableRow>(&mut self, items: &[T], change: RowCheck) {
        match change {
            RowCheck::Toggle(row) => {
                if let Some(item) = self.item_index(row) {
                    self.toggle_checked(&items[item]);
                }
            }
            RowCheck::Range(row) => self.check_range(items, row),
            RowCheck::All(checked) => self.set_all_checked(items, checked),
        }
    }

    /// Unticks every row. Ticked rows also go with the filter that selected them.
    pub(crate) fn clear_checked(&mut self) {
        self.checked.clear();
        self.anchor = None;
    }

    /// The items that are shown and ticked, in display order: what a bulk action changes.
    pub(crate) fn checked_rows<T: TableRow>(&self, items: &[T]) -> Vec<usize> {
        self.rows
            .iter()
            .copied()
            .filter(|&index| self.is_checked(&items[index]))
            .collect()
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

    /// Removes the text, the chips, and the preset; the sort and the hidden columns stay.
    pub(crate) fn clear_filter(&mut self) {
        self.filter = TableFilter::default();
        self.clear_checked();
    }

    /// Makes `item` visible for a reveal: when the filter hides it, removes the text, the chips,
    /// and the preset (the sort and the hidden columns stay). A visible item leaves the filter
    /// alone. Rebuild afterwards.
    pub(crate) fn reveal(&mut self, item: usize) {
        if self.row_of(item).is_none() {
            self.clear_filter();
        }
    }

    /// Back to the default filter (a context switch).
    pub(crate) fn reset_filter(&mut self) {
        self.filter = self.default_filter.clone();
        self.clear_checked();
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
