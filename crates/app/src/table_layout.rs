//! Column layout shared by the three table delegates: which columns are shown, how wide, and
//! the sortable header cell.

use std::collections::BTreeSet;

use gpui_kit::assets::IconName;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::table::Column;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex};
use gpui_kit::{
    AnyElement, App, ClickEvent, Div, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Pixels, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, TextAlign, WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};

use crate::app_shell::AppShell;
use crate::resource_kind::{Align, KindColumn};
use crate::table_sort::{SortDirection, TableSort};

/// Width the table cannot use: the empty trailing column and the vertical scrollbar.
const TABLE_GUTTER: Pixels = px(28.);

/// The checkbox column that opens every table.
const SELECT_WIDTH: Pixels = px(32.);

/// The logical columns of a table. A logical column is an index into this list, before any
/// column is hidden.
pub(crate) struct ColumnPlan {
    pub(crate) specs: Vec<KindColumn>,
    /// The logical column that can never be hidden: the table is always about something.
    pub(crate) flexible: usize,
}

/// A plan and the columns it currently lays out at the last known table width.
pub(crate) struct TableLayout {
    pub(crate) plan: ColumnPlan,
    pub(crate) columns: TableColumns,
    table_width: Pixels,
}

impl TableLayout {
    /// Laid out at the minimum width until the first `fit_width`.
    pub(crate) fn new(plan: ColumnPlan) -> Self {
        let columns = layout_plan(&plan, Pixels::ZERO, &BTreeSet::new());
        Self {
            plan,
            columns,
            table_width: Pixels::ZERO,
        }
    }

    /// Lays the columns out again for a table `table_width` wide. Returns whether they
    /// changed, so the caller refreshes the table only then.
    pub(crate) fn fit_width(&mut self, table_width: Pixels, hidden: &BTreeSet<usize>) -> bool {
        self.table_width = table_width;
        self.relayout(hidden)
    }

    /// Replaces the columns for another kind. The last table width is kept, so the flexible
    /// column fills the table at once.
    pub(crate) fn replace_plan(&mut self, plan: ColumnPlan, hidden: &BTreeSet<usize>) {
        self.columns = layout_plan(&plan, self.table_width, hidden);
        self.plan = plan;
    }

    /// Lays the columns out again at the last width, for example after a column was hidden.
    pub(crate) fn relayout(&mut self, hidden: &BTreeSet<usize>) -> bool {
        let next = layout_plan(&self.plan, self.table_width, hidden);
        let is_changed = !self.columns.is_same_layout(&next);
        self.columns = next;
        is_changed
    }
}

fn layout_plan(plan: &ColumnPlan, table_width: Pixels, hidden: &BTreeSet<usize>) -> TableColumns {
    layout_columns(&plan.specs, plan.flexible, table_width, hidden)
}

/// The visible columns, in order, and the logical column each one shows.
pub(crate) struct TableColumns {
    pub(crate) columns: Vec<Column>,
    /// `None` is the checkbox column.
    logical: Vec<Option<usize>>,
}

impl TableColumns {
    /// The logical column shown at table column `col_ix`; `None` for the checkbox column.
    pub(crate) fn logical(&self, col_ix: usize) -> Option<usize> {
        self.logical.get(col_ix).copied().flatten()
    }

    /// Whether table column `col_ix` is the checkbox column.
    pub(crate) fn is_select(&self, col_ix: usize) -> bool {
        matches!(self.logical.get(col_ix), Some(None))
    }

    /// Whether the table would draw the same columns, so it needs no refresh.
    fn is_same_layout(&self, other: &Self) -> bool {
        self.logical == other.logical
            && self
                .columns
                .iter()
                .map(|column| column.width)
                .eq(other.columns.iter().map(|column| column.width))
    }
}

/// The visible columns of a table `table_width` wide, after the checkbox column. Every column has
/// its `width`; the width left over is shared among the columns that have a weight (see
/// `distribute_spare_width`). The `flexible` column is shown even when hidden. The columns are
/// fixed pixel widths, so this runs again whenever the window size changes.
pub(crate) fn layout_columns(
    specs: &[KindColumn],
    flexible: usize,
    table_width: Pixels,
    hidden: &BTreeSet<usize>,
) -> TableColumns {
    let mut visible: Vec<(usize, &KindColumn)> = specs
        .iter()
        .enumerate()
        .filter(|(index, _)| *index == flexible || !hidden.contains(index))
        .collect();
    let available = f32::from(table_width - TABLE_GUTTER - SELECT_WIDTH);
    shed_columns(&mut visible, flexible, available);
    let shown: Vec<&KindColumn> = visible.iter().map(|(_, spec)| *spec).collect();
    let widths = distribute_spare_width(&shown, available);
    let select = Column::new("select", "")
        .width(SELECT_WIDTH)
        .resizable(false);
    let columns = visible
        .iter()
        .zip(widths)
        .map(|((index, spec), width)| {
            let column = Column::new(spec.name, spec.name);
            let column = match spec.align {
                Align::Left => column,
                Align::Right => column.text_right(),
            };
            let column = column.width(px(width));
            if *index == flexible {
                column.min_width(px(spec.width))
            } else {
                column
            }
        })
        .collect::<Vec<_>>();
    TableColumns {
        columns: std::iter::once(select).chain(columns).collect(),
        logical: std::iter::once(None)
            .chain(visible.iter().map(|(index, _)| Some(*index)))
            .collect(),
    }
}

/// Drops columns that have a `shed_order`, the lowest first, until the rest fit in `available`, so
/// a narrow window (1024 px) shows the table without a horizontal scroll. Before the first
/// measure `available` is not positive and every column stays.
fn shed_columns(visible: &mut Vec<(usize, &KindColumn)>, flexible: usize, available: f32) {
    if available <= 0. {
        return;
    }
    while visible.iter().map(|(_, spec)| spec.width).sum::<f32>() > available {
        let next = visible
            .iter()
            .enumerate()
            .filter(|(_, (index, spec))| *index != flexible && spec.shed_order > 0)
            .min_by_key(|(_, (_, spec))| spec.shed_order)
            .map(|(position, _)| position);
        let Some(position) = next else {
            return;
        };
        visible.remove(position);
    }
}

/// The width of each column when `available` pixels are to be filled: its own width, plus a share
/// of what the widths leave over in proportion to its weight. A column stops at its `max_width`
/// and the share it cannot take goes to the others; width nobody can take stays unused, so short
/// columns never pad out a wide table.
fn distribute_spare_width(columns: &[&KindColumn], available: f32) -> Vec<f32> {
    let mut widths: Vec<f32> = columns.iter().map(|column| column.width).collect();
    let mut spare = available - widths.iter().sum::<f32>();
    // Each round spends all the spare width or caps at least one more column.
    for _ in 0..columns.len() {
        let growing: Vec<usize> = (0..columns.len())
            .filter(|&index| {
                let column = columns[index];
                column.weight > 0 && (column.max_width == 0. || widths[index] < column.max_width)
            })
            .collect();
        if spare < 1. || growing.is_empty() {
            break;
        }
        let total_weight: f32 = growing
            .iter()
            .map(|&index| f32::from(columns[index].weight))
            .sum();
        let mut spent = 0.;
        for index in growing {
            let column = columns[index];
            let share = spare * f32::from(column.weight) / total_weight;
            let grown = if column.max_width == 0. {
                widths[index] + share
            } else {
                (widths[index] + share).min(column.max_width)
            };
            spent += grown - widths[index];
            widths[index] = grown;
        }
        spare -= spent;
    }
    widths
}

/// The header cell of table column `col_ix`: a click cycles the sort of its logical column.
/// The handler is a plain closure, not `cx.listener`: it runs outside the table update, so the
/// shell may update the table again.
pub(crate) fn header_cell(
    layout: &TableLayout,
    sort: Option<TableSort>,
    all_checked: bool,
    shell: &WeakEntity<AppShell>,
    col_ix: usize,
    cx: &App,
) -> AnyElement {
    if layout.columns.is_select(col_ix) {
        return select_all_cell(all_checked, shell);
    }
    let (Some(column), Some(logical)) = (
        layout.columns.columns.get(col_ix),
        layout.columns.logical(col_ix),
    ) else {
        return div().size_full().into_any_element();
    };
    let direction = sort
        .filter(|sort| sort.column == logical)
        .map(|sort| sort.direction);
    let shell = shell.clone();
    sortable_header(
        logical,
        column,
        direction,
        move |_, _, cx| {
            let _ = shell.update(cx, |shell, cx| shell.cycle_sort(logical, cx));
        },
        cx,
    )
}

/// The content of a body cell, centred vertically. The kit lays a cell out as a plain block, so a
/// bare child sits at the top of the cell padding; a text line is taller than the padding box of
/// a 28 px row, which pushed the text towards the row border.
pub(crate) fn centered_cell(content: impl IntoElement) -> Div {
    div().size_full().flex().items_center().child(content)
}

/// The cell around a checkbox, centred in its cell.
fn select_cell_base() -> Div {
    div().size_full().flex().items_center()
}

/// The header checkbox: ticks every shown row, or unticks them when all are ticked.
fn select_all_cell(all_checked: bool, shell: &WeakEntity<AppShell>) -> AnyElement {
    let shell = shell.clone();
    select_cell_base()
        .child(
            Checkbox::new("select-all")
                .checked(all_checked)
                .on_click(move |_, _, cx| {
                    let _ = shell.update(cx, |shell, cx| shell.set_all_checked(!all_checked, cx));
                }),
        )
        .into_any_element()
}

/// A row's checkbox. The cell stops the mouse down and the click, so ticking a row never also
/// selects it and opens the drawer.
pub(crate) fn select_cell(
    row_ix: usize,
    is_checked: bool,
    shell: &WeakEntity<AppShell>,
) -> AnyElement {
    let shell = shell.clone();
    select_cell_base()
        .id(("select-cell", row_ix))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(
            Checkbox::new(("select", row_ix))
                .checked(is_checked)
                .on_click(move |_, _, cx| {
                    let _ = shell.update(cx, |shell, cx| shell.toggle_row_checked(row_ix, cx));
                }),
        )
        .into_any_element()
}

/// The row element of a table. Ctrl (Cmd on macOS) and Shift clicks tick rows; the kit adds its
/// own click after this one, so a modified click also selects the row.
pub(crate) fn clickable_row(row_ix: usize, shell: &WeakEntity<AppShell>) -> Stateful<Div> {
    let shell = shell.clone();
    div().id(("row", row_ix)).on_click(move |event, _, cx| {
        let modifiers = event.modifiers();
        if modifiers.secondary() {
            let _ = shell.update(cx, |shell, cx| shell.toggle_row_checked(row_ix, cx));
        } else if modifiers.shift {
            let _ = shell.update(cx, |shell, cx| shell.check_row_range(row_ix, cx));
        }
    })
}

/// The header label plus a sort arrow: solid on the sorted column, a hint while the pointer
/// is over any other. The whole cell is the click
/// target, and the label stays in the plain foreground colour: the kit's header colour is too
/// faint in the dark theme. It honours the column alignment, which the kit does not apply.
fn sortable_header(
    id: usize,
    column: &Column,
    sort: Option<SortDirection>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let group = SharedString::from(format!("sort-header-{id}"));
    let is_right_aligned = column.align == TextAlign::Right;
    let arrow = match sort {
        Some(direction) => {
            let icon = match direction {
                SortDirection::Ascending => IconName::SortAscending,
                SortDirection::Descending => IconName::SortDescending,
            };
            Icon::new(icon)
                .size_3()
                .text_color(theme.muted_foreground)
                .into_any_element()
        }
        // Out of the flow, so the hint never narrows the label; it sits on the side away from
        // the text.
        None => div()
            .absolute()
            .top_0()
            .h_full()
            .flex()
            .items_center()
            .when(is_right_aligned, |hint| hint.left_0())
            .when(!is_right_aligned, |hint| hint.right_0())
            .invisible()
            .group_hover(group.clone(), |style| style.visible())
            .child(
                Icon::new(IconName::SortAscending)
                    .size_3()
                    .text_color(theme.muted_foreground),
            )
            .into_any_element(),
    };
    let label = h_flex()
        .relative()
        .size_full()
        .gap_1()
        .items_center()
        .text_color(theme.foreground)
        .child(div().truncate().child(column.name.clone()))
        .child(arrow);
    let label = if is_right_aligned {
        label.justify_end()
    } else {
        label
    };
    label
        .group(group)
        .id(("sort-header", id))
        .cursor_pointer()
        .on_click(on_click)
        .into_any_element()
}

#[cfg(test)]
#[path = "table_layout_tests.rs"]
mod table_layout_tests;
