//! The row under the screen header: active filter chips, + Filter, the quick filter input,
//! and Columns. It also draws the empty state of a filter that hides every row.

use std::collections::BTreeSet;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, IntoElement, ParentElement as _, Styled as _, WeakEntity,
    div, px,
};

use crate::app_shell::{AppShell, Screen};
use crate::resource_kind::ResourceKind;
use crate::table_filter::FilterChip;
use crate::table_view::{FilteredTable, TableView};

const QUICK_FILTER_WIDTH: gpui_kit::Pixels = px(220.);

/// What the filter bar and the screen header read from the visible table's view.
pub(crate) struct ToolkitState {
    pub(crate) screen: Screen,
    pub(crate) text: String,
    pub(crate) chips: Vec<FilterChip>,
    /// Logical columns.
    pub(crate) hidden: BTreeSet<usize>,
    /// The columns the Columns menu lists: every logical column except the flexible one.
    pub(crate) columns: Vec<(usize, &'static str)>,
    pub(crate) shown: usize,
    pub(crate) total: usize,
    pub(crate) is_filtering: bool,
}

impl ToolkitState {
    /// `None` for a table without a view (a kind table without a kind).
    pub(crate) fn of<D: FilteredTable>(delegate: &D, screen: Screen) -> Option<Self> {
        let view = delegate.view()?;
        let plan = delegate.column_plan()?;
        Some(Self {
            screen,
            text: view.filter.text.clone(),
            chips: view.filter.chips.clone(),
            hidden: view.hidden.clone(),
            columns: plan
                .specs
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != plan.flexible)
                .map(|(index, spec)| (index, spec.name))
                .collect(),
            shown: view.rows().len(),
            total: view.total(),
            is_filtering: view.is_filtering(),
        })
    }
}

/// `Status: not Running` on Pods, `Status: unhealthy` elsewhere, or the label query.
fn chip_text(chip: &FilterChip, screen: Screen) -> String {
    match chip {
        FilterChip::Unhealthy if screen == Screen::Pods => "Status: not Running".to_owned(),
        FilterChip::Unhealthy => "Status: unhealthy".to_owned(),
        FilterChip::Label(query) => query.text(),
    }
}

/// The filter bar of the visible table. Nodes and the kinds show no namespace chips yet.
pub(crate) fn filter_bar(
    state: &ToolkitState,
    quick_filter: &Entity<InputState>,
    cx: &Context<AppShell>,
) -> AnyElement {
    let chips = state.chips.iter().enumerate().map(|(index, chip)| {
        Button::new(("filter-chip", index))
            .small()
            .outline()
            .label(chip_text(chip, state.screen))
            .child(Icon::new(IconName::X).size_3())
            .tooltip("Remove filter")
            .on_click(cx.listener(move |shell, _, _, cx| shell.remove_chip(index, cx)))
    });
    h_flex()
        .flex_shrink_0()
        .gap_2()
        .items_center()
        .px_4()
        .py_1()
        .border_b_1()
        .border_color(cx.theme().border)
        .children(chips)
        .children(add_filter_button(state, cx))
        .child(
            h_flex()
                .ml_auto()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .w(QUICK_FILTER_WIDTH)
                        .child(Input::new(quick_filter).small().cleanable(true)),
                )
                .child(columns_button(state, cx)),
        )
        .into_any_element()
}

/// `+ Filter`: the status toggle (Pods and the kinds) and `Label…`. Events have none: Warnings
/// only covers their status.
fn add_filter_button(state: &ToolkitState, cx: &Context<AppShell>) -> Option<AnyElement> {
    if state.screen == Screen::Kind(ResourceKind::Events) {
        return None;
    }
    let shell = cx.weak_entity();
    let screen = state.screen;
    let has_unhealthy = state.chips.contains(&FilterChip::Unhealthy);
    Some(
        Button::new("add-filter")
            .ghost()
            .small()
            .label("+ Filter")
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                let menu = if screen == Screen::Nodes {
                    menu
                } else {
                    let shell = shell.clone();
                    menu.item(
                        PopupMenuItem::new(chip_text(&FilterChip::Unhealthy, screen))
                            .checked(has_unhealthy)
                            .on_click(move |_, _, cx| {
                                let _ = shell.update(cx, |shell, cx| shell.toggle_unhealthy(cx));
                            }),
                    )
                };
                let shell = shell.clone();
                menu.item(PopupMenuItem::new("Label…").on_click(move |_, window, cx| {
                    let _ = shell.update(cx, |shell, cx| shell.begin_label_filter(window, cx));
                }))
            })
            .into_any_element(),
    )
}

/// `Columns ▾`: one checkable item per column except the flexible one.
fn columns_button(state: &ToolkitState, cx: &Context<AppShell>) -> AnyElement {
    let shell = cx.weak_entity();
    let columns = state.columns.clone();
    let hidden = state.hidden.clone();
    Button::new("columns")
        .ghost()
        .small()
        .label("Columns")
        .dropdown_caret(true)
        .dropdown_menu(move |menu, _, _| {
            columns.iter().fold(menu, |menu, (column, name)| {
                let (column, shell) = (*column, shell.clone());
                menu.item(
                    PopupMenuItem::new(*name)
                        .checked(!hidden.contains(&column))
                        .on_click(move |_, _, cx| {
                            let _ = shell.update(cx, |shell, cx| shell.toggle_column(column, cx));
                        }),
                )
            })
        })
        .into_any_element()
}

/// The text of a table with no rows: `empty` when the list itself is empty, otherwise a note
/// that the filters hide everything, with a button that clears them.
pub(crate) fn filtered_empty_state(
    view: &TableView,
    empty: String,
    plural: &str,
    shell: &WeakEntity<AppShell>,
    cx: &App,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    if view.total() == 0 {
        return h_flex()
            .size_full()
            .justify_center()
            .items_center()
            .text_color(muted)
            .child(empty)
            .into_any_element();
    }
    let shell = shell.clone();
    v_flex()
        .size_full()
        .justify_center()
        .items_center()
        .gap_2()
        .text_color(muted)
        .child(format!("No {plural} match the filters"))
        .child(
            Button::new("clear-filters")
                .small()
                .outline()
                .label("Clear filters")
                .on_click(move |_, window, cx| {
                    let _ = shell.update(cx, |shell, cx| shell.clear_filters(window, cx));
                }),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use crate::table_filter::{LabelQuery, LabelTest};

    use super::*;

    #[test]
    fn chip_text_reads_by_screen() {
        assert_eq!(
            chip_text(&FilterChip::Unhealthy, Screen::Pods),
            "Status: not Running"
        );
        assert_eq!(
            chip_text(&FilterChip::Unhealthy, Screen::Kind(ResourceKind::Jobs)),
            "Status: unhealthy"
        );
        let label = FilterChip::Label(LabelQuery {
            key: "app".to_owned(),
            test: LabelTest::Equals("api".to_owned()),
        });
        assert_eq!(chip_text(&label, Screen::Pods), "label:app=api");
    }
}
