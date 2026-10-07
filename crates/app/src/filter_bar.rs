//! The row under the screen header: active filter chips, Filter, the quick filter input,
//! and Columns. It also draws the empty state of a filter that hides every row.

use std::collections::BTreeSet;

use cluster::NamespaceScope;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Styled as _, WeakEntity, div, px,
};

use crate::app_shell::{AppShell, Screen};
use crate::namespace_picker::{PickerAnchor, namespace_picker};
use crate::node_summary::{NodeCounts, NodeGroup};
use crate::pod_table::STATUS as POD_STATUS_COLUMN;
use crate::resource_kind::ResourceKind;
use crate::status_tone::{StatusTone, tone_color};
use crate::table_filter::{FilterChip, FilterPreset};
use crate::table_view::{FilteredTable, TableView};

/// The Status column of the Jobs table: the Name column is 0.
const JOBS_STATUS_COLUMN: usize = 1;

/// The Status column and text that mean finished OK: pods read `Completed`, jobs `Complete`.
fn completed_status(screen: Screen) -> Option<(usize, &'static str)> {
    match screen {
        Screen::Pods => Some((POD_STATUS_COLUMN, "Completed")),
        Screen::Kind(ResourceKind::Jobs) => Some((JOBS_STATUS_COLUMN, "Complete")),
        _ => None,
    }
}

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
    /// How many rows are ticked.
    pub(crate) checked: usize,
    pub(crate) preset: Option<FilterPreset>,
    /// Counts of all nodes, for the summary chips; Nodes only.
    pub(crate) node_counts: Option<NodeCounts>,
    /// The namespace scope, on the screens it applies to.
    pub(crate) scope: Option<NamespaceScope>,
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
            checked: view.checked_count(),
            preset: view.filter.preset.clone(),
            node_counts: None,
            scope: None,
        })
    }
}

/// `Status: unhealthy` (rows toned Warn, Bad, or Info; finished pods and jobs are not matched), an
/// `Equals` chip such as `Status: Completed`, or the label query.
fn chip_text(chip: &FilterChip) -> String {
    match chip {
        FilterChip::Unhealthy => "Status: unhealthy".to_owned(),
        FilterChip::Equals { title, value, .. } => format!("{title}: {value}"),
        FilterChip::Label(query) => query.text(),
    }
}

/// The filter bar of the visible table. Nodes and the kinds show no namespace chips yet.
pub(crate) fn filter_bar(
    state: &ToolkitState,
    shell: &AppShell,
    quick_filter: &Entity<InputState>,
    cx: &Context<AppShell>,
) -> AnyElement {
    let chips = state.chips.iter().enumerate().map(|(index, chip)| {
        Button::new(("filter-chip", index))
            .small()
            .outline()
            .label(chip_text(chip))
            .child(Icon::new(IconName::X).size_3())
            .tooltip("Remove filter")
            .on_click(cx.listener(move |shell, _, _, cx| shell.remove_chip(index, cx)))
    });
    // Wraps: with several namespace chips a narrow window would clip a chip mid-way.
    h_flex()
        .flex_shrink_0()
        .flex_wrap()
        .gap_2()
        .items_center()
        .px_4()
        .py_1()
        .border_b_1()
        .border_color(cx.theme().border)
        .children(namespace_chips(state, shell, cx))
        .children(node_summary_chips(state, cx))
        .children(hidden_preset_chip(state, cx))
        .children(chips)
        .children(add_filter_button(state, cx))
        .child(
            h_flex()
                .ml_auto()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .key_context("QuickFilter")
                        .w(QUICK_FILTER_WIDTH)
                        .child(Input::new(quick_filter).small().cleanable(true)),
                )
                .child(columns_button(state, cx)),
        )
        .into_any_element()
}

/// `Hide system ×` or `Hide inactive ×` while that default filter is on, so a screen that hides
/// rows says so beside the Filter button. A click turns it off. The Nodes presets have their own
/// summary chips.
fn hidden_preset_chip(state: &ToolkitState, cx: &Context<AppShell>) -> Option<AnyElement> {
    let label = match state.preset {
        Some(FilterPreset::HideSystem) => "Hide system",
        Some(FilterPreset::HideInactive) => "Hide inactive",
        Some(FilterPreset::Changes) => "Changes",
        Some(FilterPreset::Nodes(_)) | None => return None,
    };
    Some(
        Button::new("preset-chip")
            .small()
            .outline()
            .label(label)
            .child(Icon::new(IconName::X).size_3())
            .tooltip("Remove filter")
            .on_click(cx.listener(|shell, _, _, cx| shell.set_preset(None, cx)))
            .into_any_element(),
    )
}

/// `Namespace: all ▾`, which opens the picker, or one removable chip per picked namespace.
fn namespace_chips(
    state: &ToolkitState,
    shell: &AppShell,
    cx: &Context<AppShell>,
) -> Vec<AnyElement> {
    let Some(scope) = &state.scope else {
        return Vec::new();
    };
    if *scope == NamespaceScope::All {
        let trigger = Button::new("namespace-chip")
            .small()
            .outline()
            .icon(Icon::new(IconName::Folder))
            .label("Namespace: all")
            .dropdown_caret(true);
        return vec![namespace_picker(
            PickerAnchor::FilterBar,
            trigger,
            shell,
            cx,
        )];
    }
    let names = scope.namespaces();
    names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            // Removing one keeps the others; none left is All.
            let rest =
                NamespaceScope::of_namespaces(names.iter().filter(|other| *other != name).cloned());
            Button::new(("namespace-chip", index))
                .small()
                .outline()
                .icon(Icon::new(IconName::Folder))
                .label(format!("Namespace: {name}"))
                .child(Icon::new(IconName::X).size_3())
                .tooltip("Remove namespace")
                .on_click(cx.listener(move |shell, _, _, cx| shell.set_namespace(rest.clone(), cx)))
                .into_any_element()
        })
        .collect()
}

/// The Nodes summary: `All`, then one chip per group that has nodes. A click picks the group
/// as the preset, and a click on the picked one clears it. Counts read every node, so the text
/// filter never changes them.
fn node_summary_chips(state: &ToolkitState, cx: &Context<AppShell>) -> Vec<AnyElement> {
    let Some(counts) = &state.node_counts else {
        return Vec::new();
    };
    // (group, text, tone); `None` is All.
    let mut chips: Vec<(Option<NodeGroup>, String, Option<StatusTone>)> =
        vec![(None, format!("All {}", counts.total), None)];
    for (group, name, count, tone) in [
        (NodeGroup::Ready, "Ready", counts.ready, StatusTone::Ok),
        (
            NodeGroup::NotReady,
            "NotReady",
            counts.not_ready,
            StatusTone::Bad,
        ),
        (
            NodeGroup::Cordoned,
            "Cordoned",
            counts.cordoned,
            StatusTone::Warn,
        ),
    ] {
        if count > 0 {
            chips.push((Some(group), format!("{name} {count}"), Some(tone)));
        }
    }
    for (version, count) in &counts.versions {
        // Any version but the common one is skew.
        let is_skewed = counts.common_version.as_ref() != Some(version);
        chips.push((
            Some(NodeGroup::Version(version.clone())),
            format!("{version} × {count}"),
            is_skewed.then_some(StatusTone::Warn),
        ));
    }
    chips
        .into_iter()
        .map(|(group, text, tone)| {
            let id = group_id(group.as_ref());
            let target = group.map(FilterPreset::Nodes);
            let is_selected = state.preset == target;
            // Picking the active chip, or All, clears the preset.
            let next = if is_selected { None } else { target };
            // The picked chip is a filled pill; its text keeps the button's own colour.
            let label = div().child(text);
            let label = match tone {
                Some(tone) if !is_selected => label.text_color(tone_color(tone, cx)),
                _ => label,
            };
            let button = Button::new(id).small();
            let button = if is_selected {
                button.primary()
            } else {
                button.ghost()
            };
            button
                .selected(is_selected)
                .child(label)
                .on_click(cx.listener(move |shell, _, _, cx| shell.set_preset(next.clone(), cx)))
                .into_any_element()
        })
        .collect()
}

/// A stable element id for a chip: its group, never its position.
fn group_id(group: Option<&NodeGroup>) -> gpui_kit::SharedString {
    match group {
        None => "node-group-all".into(),
        Some(NodeGroup::Ready) => "node-group-ready".into(),
        Some(NodeGroup::NotReady) => "node-group-not-ready".into(),
        Some(NodeGroup::Cordoned) => "node-group-cordoned".into(),
        Some(NodeGroup::Version(version)) => format!("node-group-version-{version}").into(),
    }
}

/// `Filter`: the status toggle (Pods and the kinds) and `Label…`. Events have none: Warnings
/// only covers their status.
fn add_filter_button(state: &ToolkitState, cx: &Context<AppShell>) -> Option<AnyElement> {
    if state.screen == Screen::Kind(ResourceKind::Events) {
        return None;
    }
    let shell = cx.weak_entity();
    let screen = state.screen;
    let has_unhealthy = state.chips.contains(&FilterChip::Unhealthy);
    let has_completed = completed_status(screen).is_some_and(|(column, value)| {
        state.chips.iter().any(|chip| {
            matches!(chip, FilterChip::Equals { column: other, value: shown, .. }
                if *other == column && shown.as_ref() == value)
        })
    });
    let hides_inactive = state.preset == Some(FilterPreset::HideInactive);
    Some(
        Button::new("add-filter")
            .ghost()
            .small()
            .icon(Icon::new(IconName::ListFilterPlus))
            .label("Filter")
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                let menu = if screen == Screen::Nodes {
                    menu
                } else {
                    let shell = shell.clone();
                    menu.item(
                        PopupMenuItem::new(chip_text(&FilterChip::Unhealthy))
                            .checked(has_unhealthy)
                            .on_click(move |_, _, cx| {
                                let _ = shell.update(cx, |shell, cx| shell.toggle_unhealthy(cx));
                            }),
                    )
                };
                let menu = match completed_status(screen) {
                    Some((column, value)) => {
                        let shell = shell.clone();
                        let chip = FilterChip::Equals {
                            column,
                            title: "Status",
                            value: value.into(),
                        };
                        menu.item(
                            PopupMenuItem::new(chip_text(&chip))
                                .checked(has_completed)
                                .on_click(move |_, _, cx| {
                                    let _ = shell.update(cx, |shell, cx| {
                                        shell.toggle_equals(column, "Status", value, cx);
                                    });
                                }),
                        )
                    }
                    None => menu,
                };
                // The chip removes the filter; this is the way back once it is gone.
                let menu = if screen == Screen::Kind(ResourceKind::ReplicaSets) {
                    let shell = shell.clone();
                    menu.item(
                        PopupMenuItem::new("Hide inactive")
                            .checked(hides_inactive)
                            .on_click(move |_, _, cx| {
                                let next = (!hides_inactive).then_some(FilterPreset::HideInactive);
                                let _ = shell.update(cx, |shell, cx| shell.set_preset(next, cx));
                            }),
                    )
                } else {
                    menu
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
        .icon(Icon::new(IconName::Columns3))
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
            .child(div().text_center().child(empty))
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
                .icon(Icon::new(IconName::X))
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
    fn chip_text_names_what_the_chip_matches() {
        // The same text on every screen: it hides finished pods too, so it must not say Running.
        assert_eq!(chip_text(&FilterChip::Unhealthy), "Status: unhealthy");
        let completed = FilterChip::Equals {
            column: POD_STATUS_COLUMN,
            title: "Status",
            value: "Completed".into(),
        };
        assert_eq!(chip_text(&completed), "Status: Completed");
        let label = FilterChip::Label(LabelQuery {
            key: "app".to_owned(),
            test: LabelTest::Equals("api".to_owned()),
        });
        assert_eq!(chip_text(&label), "label:app=api");
    }

    #[test]
    fn completed_chip_is_offered_for_pods_and_jobs_only() {
        assert_eq!(
            completed_status(Screen::Pods),
            Some((POD_STATUS_COLUMN, "Completed"))
        );
        assert_eq!(
            completed_status(Screen::Kind(ResourceKind::Jobs)),
            Some((JOBS_STATUS_COLUMN, "Complete"))
        );
        assert_eq!(completed_status(Screen::Nodes), None);
        assert_eq!(
            completed_status(Screen::Kind(ResourceKind::Deployments)),
            None
        );
    }
}
