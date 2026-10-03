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

/// The names after the checkbox column.
fn names(layout: &TableColumns) -> Vec<&str> {
    layout
        .columns
        .iter()
        .skip(1)
        .map(|column| column.name.as_ref())
        .collect()
}

#[test]
fn layout_columns_skips_hidden_and_maps_logical() {
    let layout = layout_columns(&specs(), 0, px(160.), None, px(1100.), &hidden(&[1, 3]));
    assert_eq!(names(&layout), ["Name", "Restarts"]);
    // The checkbox column comes first and has no logical column.
    assert!(layout.is_select(0));
    assert_eq!(layout.logical(0), None);
    assert_eq!(layout.logical(1), Some(0));
    assert_eq!(layout.logical(2), Some(2));
    assert_eq!(layout.logical(3), None);
    assert!(!layout.is_select(3));
}

#[test]
fn layout_columns_never_hides_the_flexible_column() {
    let layout = layout_columns(&specs(), 0, px(160.), None, px(1100.), &hidden(&[0, 1]));
    assert_eq!(names(&layout), ["Name", "Restarts", "Age"]);
}

#[test]
fn layout_columns_gives_spare_width_to_flexible() {
    let layout = layout_columns(&specs(), 0, px(160.), None, px(1100.), &BTreeSet::new());
    let widths: Vec<_> = layout.columns.iter().map(|column| column.width).collect();
    assert_eq!(
        widths,
        [
            px(32.),
            px(1100. - 28. - 32. - 320.),
            px(170.),
            px(80.),
            px(70.)
        ]
    );
    // A hidden column hands its width to the flexible one.
    let narrower = layout_columns(&specs(), 0, px(160.), None, px(1100.), &hidden(&[1]));
    assert_eq!(
        narrower.columns.get(1).map(|column| column.width),
        Some(px(1100. - 28. - 32. - 150.))
    );
}

#[test]
fn layout_columns_never_drops_below_the_minimum() {
    let layout = layout_columns(&specs(), 0, px(160.), None, px(300.), &BTreeSet::new());
    assert_eq!(
        layout.columns.get(1).map(|column| column.width),
        Some(px(160.))
    );
}

#[test]
fn table_layout_reports_only_real_changes() {
    let plan = ColumnPlan {
        specs: specs(),
        flexible: 0,
        flexible_min: px(160.),
        session_column: None,
    };
    let mut layout = TableLayout::new(plan);
    assert!(layout.fit_width(px(1100.), &BTreeSet::new()));
    assert!(!layout.fit_width(px(1100.), &BTreeSet::new()));
    assert!(layout.relayout(&hidden(&[1])));
    assert!(!layout.relayout(&hidden(&[1])));
}

/// A real kit table with the checkbox column, to see how clicks reach the row.
mod checkbox_clicks {
    use gpui_kit::base::Root;
    use gpui_kit::component::table::{DataTable, TableDelegate, TableState};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Bounds, Context, Entity, IntoElement, Point, Render, TestAppContext,
        WindowBounds, WindowOptions, px, size,
    };

    use super::*;

    struct Rows {
        columns: TableColumns,
    }

    impl TableDelegate for Rows {
        fn columns_count(&self, _: &App) -> usize {
            self.columns.columns.len()
        }

        fn rows_count(&self, _: &App) -> usize {
            3
        }

        fn column(&self, col_ix: usize, _: &App) -> Column {
            self.columns.columns[col_ix].clone()
        }

        fn render_tr(
            &mut self,
            row_ix: usize,
            _: &mut Window,
            _: &mut Context<TableState<Self>>,
        ) -> Stateful<Div> {
            clickable_row(row_ix, &WeakEntity::new_invalid())
        }

        fn render_td(
            &mut self,
            row_ix: usize,
            col_ix: usize,
            _: &mut Window,
            _: &mut Context<TableState<Self>>,
        ) -> impl IntoElement {
            if self.columns.is_select(col_ix) {
                return select_cell(row_ix, false, &WeakEntity::new_invalid());
            }
            div().id(("plain", row_ix)).child("cell").into_any_element()
        }
    }

    struct Host {
        table: Entity<TableState<Rows>>,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(DataTable::new(&self.table))
        }
    }

    fn open(cx: &mut TestAppContext) -> (gpui_kit::WindowHandle<Root>, Entity<Host>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            let bounds = Bounds {
                origin: Point::default(),
                size: size(px(640.), px(320.)),
            };
            let (window, host) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let columns =
                        layout_columns(&specs(), 0, px(160.), None, px(640.), &BTreeSet::new());
                    cx.new(|cx| Host {
                        table: cx.new(|cx| {
                            TableState::new(Rows { columns }, window, cx).row_selectable(true)
                        }),
                    })
                },
            )
            .expect("open the test window");
            (window.downcast::<Root>().expect("a Root window"), host)
        })
    }

    fn selected_row_after(
        cx: &mut TestAppContext,
        click: impl FnOnce(&mut Window, &mut App),
    ) -> Option<usize> {
        let (window, host) = open(cx);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            click(window, cx);
            host.read(cx).table.read(cx).selected_row()
        })
        .expect("the window is open")
    }

    #[gpui_kit::test]
    fn a_click_on_the_row_selects_it(cx: &mut TestAppContext) {
        let selected = selected_row_after(cx, |window, cx| window.click(("row", 1usize), cx));
        assert_eq!(selected, Some(1));
    }

    #[gpui_kit::test]
    fn a_click_on_the_row_checkbox_does_not_select_the_row(cx: &mut TestAppContext) {
        let selected = selected_row_after(cx, |window, cx| window.click(("select", 1usize), cx));
        assert_eq!(selected, None);
    }
}

// ---- The session column (Cluster) stays in view ----

/// `specs()` plus a 170 px Cluster column at logical index 4.
fn specs_with_cluster() -> Vec<KindColumn> {
    let mut specs = specs();
    specs.push(column("Cluster", 170., Align::Left));
    specs
}

fn width_of(layout: &TableColumns, name: &str) -> Option<Pixels> {
    layout
        .columns
        .iter()
        .find(|column| column.name.as_ref() == name)
        .map(|column| column.width)
}

#[test]
fn session_column_keeps_its_width_in_a_wide_table() {
    let layout = layout_columns(
        &specs_with_cluster(),
        0,
        px(160.),
        Some(4),
        px(1100.),
        &BTreeSet::new(),
    );
    assert_eq!(width_of(&layout, "Cluster"), Some(px(170.)));
    assert_eq!(
        width_of(&layout, "Name"),
        Some(px(1100. - 28. - 32. - 490.))
    );
}

#[test]
fn narrow_table_shrinks_the_flexible_column_before_the_session_column() {
    // 260 px are left for Name and Cluster: the Name minimum (160) and 170 do not fit.
    let layout = layout_columns(
        &specs_with_cluster(),
        0,
        px(160.),
        Some(4),
        px(640.),
        &BTreeSet::new(),
    );
    assert_eq!(width_of(&layout, "Cluster"), Some(px(170.)));
    assert_eq!(width_of(&layout, "Name"), Some(px(90.)));
}

#[test]
fn narrower_table_shrinks_the_session_column_to_its_minimum() {
    let layout = layout_columns(
        &specs_with_cluster(),
        0,
        px(160.),
        Some(4),
        px(400.),
        &BTreeSet::new(),
    );
    assert_eq!(width_of(&layout, "Cluster"), Some(px(110.)));
    // The table scrolls from here on: the flexible column never goes below its floor.
    assert_eq!(width_of(&layout, "Name"), Some(px(40.)));
}

#[test]
fn a_hidden_session_column_leaves_the_layout_alone() {
    let layout = layout_columns(
        &specs_with_cluster(),
        0,
        px(160.),
        Some(4),
        px(640.),
        &hidden(&[4]),
    );
    assert_eq!(width_of(&layout, "Cluster"), None);
    assert_eq!(width_of(&layout, "Name"), Some(px(640. - 28. - 32. - 320.)));
}
