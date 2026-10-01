//! Column layout shared by the three table delegates: which columns are shown, how wide, and
//! the sortable header cell.

use std::collections::BTreeSet;

use gpui_kit::assets::IconName;
use gpui_kit::component::table::Column;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex};
use gpui_kit::{
    AnyElement, App, ClickEvent, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    SharedString, StatefulInteractiveElement as _, Styled as _, TextAlign, WeakEntity, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::app_shell::AppShell;
use crate::resource_kind::{Align, KindColumn};
use crate::table_sort::{SortDirection, TableSort};

/// Width the table cannot use: the empty trailing column and the vertical scrollbar.
const TABLE_GUTTER: Pixels = px(28.);

/// The logical columns of a table. A logical column is an index into this list, before any
/// column is hidden.
pub(crate) struct ColumnPlan {
    pub(crate) specs: Vec<KindColumn>,
    /// The logical column that takes the spare width and can never be hidden.
    pub(crate) flexible: usize,
    pub(crate) flexible_min: Pixels,
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
    layout_columns(
        &plan.specs,
        plan.flexible,
        plan.flexible_min,
        table_width,
        hidden,
    )
}

/// The visible columns, in order, and the logical column each one shows.
pub(crate) struct TableColumns {
    pub(crate) columns: Vec<Column>,
    logical: Vec<usize>,
}

impl TableColumns {
    /// The logical column shown at table column `col_ix`.
    pub(crate) fn logical(&self, col_ix: usize) -> Option<usize> {
        self.logical.get(col_ix).copied()
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

/// The visible columns of a table `table_width` wide. The `flexible` column takes the width
/// the others leave over, never less than `flexible_min`, and is shown even when hidden. The
/// columns are fixed pixel widths, so this runs again whenever the window size changes.
pub(crate) fn layout_columns(
    specs: &[KindColumn],
    flexible: usize,
    flexible_min: Pixels,
    table_width: Pixels,
    hidden: &BTreeSet<usize>,
) -> TableColumns {
    let visible: Vec<(usize, &KindColumn)> = specs
        .iter()
        .enumerate()
        .filter(|(index, _)| *index == flexible || !hidden.contains(index))
        .collect();
    let fixed_width: f32 = visible
        .iter()
        .filter(|(index, _)| *index != flexible)
        .map(|(_, spec)| spec.width)
        .sum();
    let flexible_width = (table_width - TABLE_GUTTER - px(fixed_width)).max(flexible_min);
    let columns = visible
        .iter()
        .map(|(index, spec)| {
            let column = Column::new(spec.name, spec.name);
            let column = match spec.align {
                Align::Left => column,
                Align::Right => column.text_right(),
            };
            if *index == flexible {
                column.width(flexible_width).min_width(flexible_min)
            } else {
                column.width(px(spec.width))
            }
        })
        .collect();
    TableColumns {
        columns,
        logical: visible.iter().map(|(index, _)| *index).collect(),
    }
}

/// The header cell of table column `col_ix`: a click cycles the sort of its logical column.
/// The handler is a plain closure, not `cx.listener`: it runs outside the table update, so the
/// shell may update the table again.
pub(crate) fn header_cell(
    layout: &TableLayout,
    sort: Option<TableSort>,
    shell: &WeakEntity<AppShell>,
    col_ix: usize,
    cx: &App,
) -> AnyElement {
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
