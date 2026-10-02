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

fn checked_rows(view: &TableView, items: &[Row]) -> Vec<&'static str> {
    view.rows()
        .iter()
        .filter(|&&index| view.is_checked(&items[index]))
        .map(|&index| items[index].name)
        .collect()
}

fn ticked_view(items: &[Row]) -> TableView {
    let mut view = TableView::default();
    view.rebuild(items, 2, now());
    view
}

#[test]
fn rebuild_prunes_checked_to_visible_rows() {
    let rows = items();
    let mut view = ticked_view(&rows);
    view.set_all_checked(&rows, true);
    assert_eq!(view.checked_count(), 4);
    // A filter hides pod-2 and pod-3: they are no longer ticked, and stay unticked on return.
    view.filter.text = "g=b".to_owned();
    view.rebuild(&rows, 2, now());
    assert_eq!(view.checked_count(), 2);
    view.filter.text.clear();
    view.rebuild(&rows, 2, now());
    assert_eq!(checked_rows(&view, &rows), ["pod-10", "pod-1"]);
}

#[test]
fn rebuild_prunes_checked_rows_that_were_deleted() {
    let rows = items();
    let mut view = ticked_view(&rows);
    view.set_all_checked(&rows, true);
    view.rebuild(&rows[..3], 2, now());
    assert_eq!(view.checked_count(), 3);
    view.rebuild(&rows[1..], 2, now());
    assert_eq!(view.checked_count(), 2);
    view.rebuild(&rows[2..], 2, now());
    assert_eq!(view.checked_count(), 1);
}

#[test]
fn set_all_checked_touches_visible_rows_only() {
    let rows = items();
    let mut view = ticked_view(&rows);
    view.toggle_checked(&rows[0]);
    view.filter.text = "g=a".to_owned();
    view.rebuild(&rows, 2, now());
    // pod-10 is hidden, so it was pruned: only the two shown rows are ticked now.
    assert!(!view.all_checked(&rows));
    view.set_all_checked(&rows, true);
    assert!(view.all_checked(&rows));
    assert_eq!(checked_rows(&view, &rows), ["pod-2", "pod-3"]);
}

#[test]
fn set_all_checked_false_unticks_the_shown_rows() {
    let rows = items();
    let mut view = ticked_view(&rows);
    view.set_all_checked(&rows, true);
    view.set_all_checked(&rows, false);
    assert_eq!(view.checked_count(), 0);
}

#[test]
fn nothing_shown_is_never_all_checked() {
    let rows = items();
    let mut view = ticked_view(&rows);
    view.filter.text = "no-such-row".to_owned();
    view.rebuild(&rows, 2, now());
    assert!(!view.all_checked(&rows));
}

#[test]
fn check_range_checks_visible_rows_from_the_anchor() {
    let rows = items();
    let mut view = ticked_view(&rows);
    view.toggle_checked(&rows[1]);
    view.check_range(&rows, 3);
    assert_eq!(checked_rows(&view, &rows), ["pod-2", "pod-1", "pod-3"]);
}

#[test]
fn check_range_also_extends_upwards() {
    let rows = items();
    let mut view = ticked_view(&rows);
    view.toggle_checked(&rows[2]);
    view.check_range(&rows, 0);
    assert_eq!(checked_rows(&view, &rows), ["pod-10", "pod-2", "pod-1"]);
}

#[test]
fn check_range_follows_the_view_order() {
    let rows = items();
    let mut view = sorted_by(0, SortDirection::Ascending);
    view.rebuild(&rows, 2, now());
    // Shown as pod-1, pod-2, pod-3, pod-10: the range follows that, not the source order.
    view.toggle_checked(&rows[2]);
    view.check_range(&rows, 2);
    assert_eq!(checked_rows(&view, &rows), ["pod-1", "pod-2", "pod-3"]);
}

#[test]
fn check_range_without_an_anchor_acts_as_toggle() {
    let rows = items();
    let mut view = ticked_view(&rows);
    view.check_range(&rows, 2);
    assert_eq!(checked_rows(&view, &rows), ["pod-1"]);
    // Out of range changes nothing.
    view.check_range(&rows, 9);
    assert_eq!(view.checked_count(), 1);
}

#[test]
fn clearing_or_resetting_the_filter_unticks_every_row() {
    let rows = items();
    let mut view = ticked_view(&rows);
    view.set_all_checked(&rows, true);
    view.clear_filter();
    assert_eq!(view.checked_count(), 0);
    view.set_all_checked(&rows, true);
    view.reset_filter();
    assert_eq!(view.checked_count(), 0);
}
