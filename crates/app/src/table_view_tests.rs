use std::borrow::Cow;

use super::*;
use crate::table_filter::{LabelQuery, LabelTest};
use crate::table_sort::SortDirection;

struct Row {
    name: &'static str,
    group: &'static str,
    tone: StatusTone,
}

fn row(name: &'static str, group: &'static str, tone: StatusTone) -> Row {
    Row { name, group, tone }
}

impl TableRow for Row {
    fn namespace(&self) -> Option<&str> {
        None
    }

    fn name(&self) -> &str {
        self.name
    }

    fn labels(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.group)
    }

    fn tone(&self) -> StatusTone {
        self.tone
    }

    fn value(&self, column: usize) -> CellValue<'_> {
        match column {
            0 => CellValue::Text(Cow::Borrowed(self.name)),
            1 => CellValue::Text(Cow::Borrowed(self.group)),
            _ => CellValue::Absent,
        }
    }

    fn in_preset(&self, _: &FilterPreset) -> bool {
        self.tone != StatusTone::Done
    }
}

fn items() -> Vec<Row> {
    vec![
        row("pod-10", "g=b", StatusTone::Ok),
        row("pod-2", "g=a", StatusTone::Bad),
        row("pod-1", "g=b", StatusTone::Warn),
        row("pod-3", "g=a", StatusTone::Ok),
    ]
}

fn now() -> jiff::Timestamp {
    jiff::Timestamp::UNIX_EPOCH
}

fn sorted_by(column: usize, direction: SortDirection) -> TableView {
    TableView {
        sort: Some(TableSort { column, direction }),
        ..Default::default()
    }
}

#[test]
fn rebuild_without_filter_or_sort_is_identity() {
    let mut view = TableView::default();
    view.rebuild(&items(), 2, now());
    assert_eq!(view.rows(), [0, 1, 2, 3]);
    assert_eq!(view.total(), 4);
    assert!(!view.is_filtering());
}

#[test]
fn rebuild_sorts_in_natural_order() {
    let mut view = sorted_by(0, SortDirection::Ascending);
    view.rebuild(&items(), 2, now());
    // pod-1, pod-2, pod-3, pod-10
    assert_eq!(view.rows(), [2, 1, 3, 0]);
}

#[test]
fn rebuild_filters_before_it_sorts() {
    let mut view = sorted_by(0, SortDirection::Ascending);
    view.filter.text = "g=a".to_owned();
    view.rebuild(&items(), 2, now());
    assert_eq!(view.rows(), [1, 3]);
    assert_eq!(view.total(), 4);
    assert!(view.is_filtering());
}

#[test]
fn rebuild_sort_is_stable_on_ties() {
    let mut view = sorted_by(1, SortDirection::Ascending);
    view.rebuild(&items(), 2, now());
    // g=a: items 1, 3; g=b: items 0, 2 — each group keeps the source order.
    assert_eq!(view.rows(), [1, 3, 0, 2]);
    let mut view = sorted_by(1, SortDirection::Descending);
    view.rebuild(&items(), 2, now());
    assert_eq!(view.rows(), [0, 2, 1, 3]);
}

#[test]
fn row_of_and_item_index_round_trip() {
    let mut view = sorted_by(0, SortDirection::Descending);
    view.rebuild(&items(), 2, now());
    for row in 0..view.rows().len() {
        let item = view.item_index(row).expect("a shown row");
        assert_eq!(view.row_of(item), Some(row));
    }
    assert_eq!(view.item_index(4), None);
    view.filter.text = "pod-2".to_owned();
    view.rebuild(&items(), 2, now());
    assert_eq!(view.row_of(0), None);
    assert_eq!(view.row_of(1), Some(0));
}

#[test]
fn clear_filter_keeps_sort_and_hidden_columns() {
    let mut view = sorted_by(0, SortDirection::Ascending);
    view.hidden.insert(1);
    view.filter.text = "pod".to_owned();
    view.filter.chips.push(FilterChip::Unhealthy);
    view.clear_filter();
    assert!(!view.is_filtering());
    assert!(view.sort.is_some());
    assert!(view.hidden.contains(&1));
}

#[test]
fn add_chips_dedupes() {
    let label = FilterChip::Label(LabelQuery {
        key: "g".to_owned(),
        test: LabelTest::Exists,
    });
    let mut view = TableView::default();
    view.add_chips(vec![FilterChip::Unhealthy, label.clone()]);
    view.add_chips(vec![label, FilterChip::Unhealthy]);
    assert_eq!(view.filter.chips.len(), 2);
    view.rebuild(&items(), 2, now());
    assert_eq!(view.rows(), [1, 2]);
}

#[test]
fn rebuild_keeps_sort_on_a_hidden_column() {
    let mut view = sorted_by(1, SortDirection::Descending);
    view.hidden.insert(1);
    view.rebuild(&items(), 2, now());
    assert_eq!(view.rows(), [0, 2, 1, 3]);
}

#[test]
fn replica_sets_start_with_hide_inactive() {
    use crate::app_shell::Screen;
    use crate::resource_kind::ResourceKind;

    let hide_inactive = default_filter(Screen::Kind(ResourceKind::ReplicaSets));
    assert_eq!(hide_inactive.preset, Some(FilterPreset::HideInactive));
    for screen in [
        Screen::Pods,
        Screen::Nodes,
        Screen::Kind(ResourceKind::Events),
        Screen::Kind(ResourceKind::Deployments),
    ] {
        assert_eq!(default_filter(screen), TableFilter::default(), "{screen:?}");
    }
    let view = TableView::new(hide_inactive);
    assert!(view.is_filtering());
}

#[test]
fn reset_filter_restores_the_default() {
    let default = TableFilter {
        preset: Some(FilterPreset::HideInactive),
        ..Default::default()
    };
    let mut view = TableView::new(default.clone());
    view.filter.text = "pod".to_owned();
    view.filter.preset = None;
    view.reset_filter();
    assert_eq!(view.filter, default);
}

#[test]
fn clear_filter_turns_the_default_preset_off() {
    let mut view = TableView::new(TableFilter {
        preset: Some(FilterPreset::HideInactive),
        ..Default::default()
    });
    assert!(view.is_filtering());
    view.clear_filter();
    assert!(!view.is_filtering());
}

#[test]
fn a_preset_hides_rows_in_the_rebuild() {
    let mut view = TableView::new(TableFilter {
        preset: Some(FilterPreset::HideInactive),
        ..Default::default()
    });
    let mut rows = items();
    rows.push(row("pod-done", "g=a", StatusTone::Done));
    view.rebuild(&rows, 2, now());
    // The test rows are inactive when their tone is Done.
    assert_eq!(view.rows(), [0, 1, 2, 3]);
    assert_eq!(view.total(), 5);
}
