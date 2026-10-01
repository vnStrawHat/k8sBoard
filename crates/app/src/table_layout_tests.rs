use super::*;
use crate::resource_kind::column;

fn specs() -> Vec<KindColumn> {
    vec![
        column("Name", 160., Align::Left),
        column("Status", 170., Align::Left),
        column("Restarts", 80., Align::Right),
        column("Age", 70., Align::Right),
    ]
}

fn hidden(columns: &[usize]) -> BTreeSet<usize> {
    columns.iter().copied().collect()
}

fn names(layout: &TableColumns) -> Vec<&str> {
    layout
        .columns
        .iter()
        .map(|column| column.name.as_ref())
        .collect()
}

#[test]
fn layout_columns_skips_hidden_and_maps_logical() {
    let layout = layout_columns(&specs(), 0, px(160.), px(1100.), &hidden(&[1, 3]));
    assert_eq!(names(&layout), ["Name", "Restarts"]);
    assert_eq!(layout.logical(0), Some(0));
    assert_eq!(layout.logical(1), Some(2));
    assert_eq!(layout.logical(2), None);
}

#[test]
fn layout_columns_never_hides_the_flexible_column() {
    let layout = layout_columns(&specs(), 0, px(160.), px(1100.), &hidden(&[0, 1]));
    assert_eq!(names(&layout), ["Name", "Restarts", "Age"]);
}

#[test]
fn layout_columns_gives_spare_width_to_flexible() {
    let layout = layout_columns(&specs(), 0, px(160.), px(1100.), &BTreeSet::new());
    let widths: Vec<_> = layout.columns.iter().map(|column| column.width).collect();
    assert_eq!(widths, [px(1100. - 28. - 320.), px(170.), px(80.), px(70.)]);
    // A hidden column hands its width to the flexible one.
    let narrower = layout_columns(&specs(), 0, px(160.), px(1100.), &hidden(&[1]));
    assert_eq!(
        narrower.columns.first().map(|column| column.width),
        Some(px(1100. - 28. - 150.))
    );
}

#[test]
fn layout_columns_never_drops_below_the_minimum() {
    let layout = layout_columns(&specs(), 0, px(160.), px(300.), &BTreeSet::new());
    assert_eq!(
        layout.columns.first().map(|column| column.width),
        Some(px(160.))
    );
}

#[test]
fn table_layout_reports_only_real_changes() {
    let plan = ColumnPlan {
        specs: specs(),
        flexible: 0,
        flexible_min: px(160.),
    };
    let mut layout = TableLayout::new(plan);
    assert!(layout.fit_width(px(1100.), &BTreeSet::new()));
    assert!(!layout.fit_width(px(1100.), &BTreeSet::new()));
    assert!(layout.relayout(&hidden(&[1])));
    assert!(!layout.relayout(&hidden(&[1])));
}
