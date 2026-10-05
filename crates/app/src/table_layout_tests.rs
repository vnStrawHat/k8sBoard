use super::*;
use crate::resource_kind::column;

fn specs() -> Vec<KindColumn> {
    vec![
        column("Name", 160., Align::Left).grows(1),
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
    let layout = layout_columns(&specs(), 0, px(1100.), &hidden(&[1, 3]));
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
    let layout = layout_columns(&specs(), 0, px(1100.), &hidden(&[0, 1]));
    assert_eq!(names(&layout), ["Name", "Restarts", "Age"]);
}

#[test]
fn layout_columns_gives_spare_width_to_flexible() {
    let layout = layout_columns(&specs(), 0, px(1100.), &BTreeSet::new());
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
    let narrower = layout_columns(&specs(), 0, px(1100.), &hidden(&[1]));
    assert_eq!(
        narrower.columns.get(1).map(|column| column.width),
        Some(px(1100. - 28. - 32. - 150.))
    );
}

#[test]
fn layout_columns_never_drops_below_the_minimum() {
    let layout = layout_columns(&specs(), 0, px(300.), &BTreeSet::new());
    assert_eq!(
        layout.columns.get(1).map(|column| column.width),
        Some(px(160.))
    );
}

#[test]
fn spare_width_is_shared_by_weight() {
    let name = column("Name", 100., Align::Left).grows(3);
    let message = column("Message", 100., Align::Left).grows(1);
    let status = column("Status", 80., Align::Left);
    let widths = distribute_spare_width(&[&name, &status, &message], 580.);
    // 300 spare: 225 for Name, 75 for Message, none for the fixed Status.
    assert_eq!(widths, [325., 80., 175.]);
}

#[test]
fn a_capped_column_hands_its_share_to_the_others() {
    let name = column("Name", 100., Align::Left).grows(1).up_to(150.);
    let message = column("Message", 100., Align::Left).grows(1);
    let widths = distribute_spare_width(&[&name, &message], 500.);
    assert_eq!(widths, [150., 350.]);
}

#[test]
fn spare_width_nobody_can_take_stays_unused() {
    let name = column("Name", 100., Align::Left).grows(1).up_to(150.);
    let age = column("Age", 70., Align::Right);
    assert_eq!(distribute_spare_width(&[&name, &age], 1000.), [150., 70.]);
    let narrow = distribute_spare_width(&[&name, &age], 100.);
    assert_eq!(narrow, [100., 70.]);
}
#[test]
fn table_layout_reports_only_real_changes() {
    let plan = ColumnPlan {
        specs: specs(),
        flexible: 0,
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
    use gpui_kit::component::{Sizable as _, Size};
    use gpui_kit::test::{TestSupportExt as _, TestWindowExt as _};
    use gpui_kit::{
        AppContext as _, Bounds, Context, Entity, IntoElement, Point, Render, TestAppContext,
        WindowBounds, WindowOptions, px, size,
    };

    use super::*;

    /// What the first column shows. The kit draws only that column in a test window.
    #[derive(Clone, Copy)]
    enum Probe {
        Checkbox,
        Text,
    }

    struct Rows {
        columns: TableColumns,
        probe: Probe,
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
            if !self.columns.is_select(col_ix) {
                return div().into_any_element();
            }
            let content = match self.probe {
                Probe::Checkbox => select_cell(row_ix, false, &WeakEntity::new_invalid()),
                // Shorter than the cell padding box of a 28 px row, so a top-aligned cell is caught.
                Probe::Text => div()
                    .id(("plain", row_ix))
                    .h(px(14.))
                    .child("cell")
                    .test_support()
                    .into_any_element(),
            };
            centered_cell(content).into_any_element()
        }
    }

    struct Host {
        table: Entity<TableState<Rows>>,
        size: Size,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(DataTable::new(&self.table).with_size(self.size))
        }
    }

    fn open(
        cx: &mut TestAppContext,
        row_size: Size,
        probe: Probe,
    ) -> (gpui_kit::WindowHandle<Root>, Entity<Host>) {
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
                    let columns = layout_columns(&specs(), 0, px(640.), &BTreeSet::new());
                    cx.new(|cx| Host {
                        table: cx.new(|cx| {
                            TableState::new(Rows { columns, probe }, window, cx)
                                .row_selectable(true)
                        }),
                        size: row_size,
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
        let (window, host) = open(cx, Size::Medium, Probe::Checkbox);
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

    /// The vertical distance from the centre of `id` to the centre of the second body row.
    fn centre_offset(
        cx: &mut TestAppContext,
        row_size: Size,
        probe: Probe,
        id: (&'static str, usize),
    ) -> f32 {
        let (window, _host) = open(cx, row_size, probe);
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            let row = window.find(("row", 1usize)).bounds().center().y;
            let content = window.find(id).bounds().center().y;
            f32::from(content - row).abs()
        })
        .expect("the window is open")
    }

    #[gpui_kit::test]
    fn cell_content_is_centred_in_the_row_at_both_densities(cx: &mut TestAppContext) {
        for height in [28., 36.] {
            let row_size = Size::Size(px(height));
            // A pixel is the slack: the row border sits below the content box.
            let text = centre_offset(cx, row_size, Probe::Text, ("plain", 1));
            assert!(text <= 1., "text off centre by {text} px at {height}");
            let checkbox = centre_offset(cx, row_size, Probe::Checkbox, ("select", 1));
            assert!(
                checkbox <= 1.,
                "box off centre by {checkbox} px at {height}"
            );
        }
    }
}
