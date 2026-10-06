//! The namespace picker: a popover with a checkbox per namespace and Apply. Two triggers share
//! one state (the title bar and the filter bar chip), and only the clicked one opens.

use std::collections::BTreeSet;

use cluster::NamespaceScope;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, SharedString, StatefulInteractiveElement as _, Styled as _,
    WeakEntity, div, prelude::FluentBuilder as _, px,
};

use crate::app_shell::AppShell;
use crate::cluster_session::LiveList;

/// Bounds the watches (2N + 3) and the access reviews (16N + 3) of one scope.
pub(crate) const MAX_NAMESPACES: usize = 5;

const MAX_LIST_HEIGHT: Pixels = px(360.);
const PICKER_WIDTH: Pixels = px(280.);
/// Says what the two click targets of a row do, since nothing else on the row does.
const ROW_NOTE: &str = "Click a name to switch · tick boxes to combine";

/// Which trigger opened the picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PickerAnchor {
    TitleBar,
    FilterBar,
}

/// The open trigger, and the namespaces ticked but not applied yet.
#[derive(Default)]
pub(crate) struct NamespacePickerState {
    pub(crate) anchor: Option<PickerAnchor>,
    draft: BTreeSet<String>,
}

impl NamespacePickerState {
    /// Opening copies the current scope's namespaces into the draft.
    pub(crate) fn open(&mut self, anchor: PickerAnchor, scope: &NamespaceScope) {
        self.anchor = Some(anchor);
        self.draft = scope.namespaces().iter().cloned().collect();
    }

    /// Closes the picker, but only when `anchor` is the open one: the other trigger's popover
    /// reporting that it closed must not shut a picker opened elsewhere.
    pub(crate) fn close(&mut self, anchor: PickerAnchor) {
        if self.anchor == Some(anchor) {
            self.anchor = None;
        }
    }

    /// Closes the picker whatever the anchor (Apply, or a name was picked).
    pub(crate) fn dismiss(&mut self) {
        self.anchor = None;
    }

    /// Ticks or unticks `name`. Returns false, and changes nothing, when ticking would pass
    /// `MAX_NAMESPACES`.
    pub(crate) fn toggle(&mut self, name: &str) -> bool {
        if self.draft.remove(name) {
            return true;
        }
        if !self.can_add() {
            return false;
        }
        self.draft.insert(name.to_owned());
        true
    }

    pub(crate) fn clear(&mut self) {
        self.draft.clear();
    }

    pub(crate) fn can_add(&self) -> bool {
        self.draft.len() < MAX_NAMESPACES
    }

    pub(crate) fn is_ticked(&self, name: &str) -> bool {
        self.draft.contains(name)
    }

    pub(crate) fn ticked_count(&self) -> usize {
        self.draft.len()
    }

    /// The scope Apply would set; `None` (Apply disabled) when nothing is ticked or the draft
    /// is the current scope.
    pub(crate) fn applied_scope(&self, current: &NamespaceScope) -> Option<NamespaceScope> {
        if self.draft.is_empty() {
            return None;
        }
        let scope = NamespaceScope::of_namespaces(self.draft.iter().cloned());
        (scope != *current).then_some(scope)
    }
}

/// What the popover shows, copied out of the session so the content closure owns it.
struct PickerContent {
    scope: NamespaceScope,
    namespaces: NamespaceRows,
    draft: NamespacePickerState,
    filter: Entity<InputState>,
    filter_text: String,
    shell: WeakEntity<AppShell>,
}

enum NamespaceRows {
    Loading,
    /// The list failed: only the context namespace can be picked.
    Failed {
        message: String,
        default: String,
    },
    Ready(Vec<String>),
}

/// `trigger` wrapped in the picker popover. It is open only when `anchor` is the one that was
/// clicked, so the title bar and the filter bar chip never open together.
pub(crate) fn namespace_picker(
    anchor: PickerAnchor,
    trigger: Button,
    shell: &AppShell,
    cx: &Context<AppShell>,
) -> AnyElement {
    let state = shell.namespace_picker();
    let is_open = state.anchor == Some(anchor);
    let weak = cx.weak_entity();
    let on_open_change = {
        let weak = weak.clone();
        move |open: &bool, window: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            let _ = weak.update(cx, |shell, cx| {
                if *open {
                    shell.open_namespace_picker(anchor, window, cx);
                } else {
                    shell.close_namespace_picker(anchor, cx);
                }
            });
        }
    };
    let content = is_open
        .then(|| content_of(shell, state, weak, cx))
        .flatten();
    Popover::new(match anchor {
        PickerAnchor::TitleBar => "namespace-popover-title",
        PickerAnchor::FilterBar => "namespace-popover-filter",
    })
    .open(is_open)
    .on_open_change(on_open_change)
    // The kit focuses the filter while the popover opens.
    .track_focus(&shell.namespace_filter().read(cx).focus_handle(cx))
    .trigger(trigger)
    .when_some(content, |popover, content| {
        popover.content(move |_, _, cx| render_content(&content, cx))
    })
    .into_any_element()
}

fn content_of(
    shell: &AppShell,
    state: &NamespacePickerState,
    weak: WeakEntity<AppShell>,
    cx: &Context<AppShell>,
) -> Option<PickerContent> {
    let live = shell.live(cx)?;
    let namespaces = match &live.namespaces {
        LiveList::Loading => NamespaceRows::Loading,
        LiveList::Failed { message } => NamespaceRows::Failed {
            message: message.clone(),
            default: live.default_namespace().to_owned(),
        },
        LiveList::Ready { items, .. } => NamespaceRows::Ready(
            items
                .iter()
                .map(|namespace| namespace.name.clone())
                .collect(),
        ),
    };
    Some(PickerContent {
        scope: live.scope.clone(),
        namespaces,
        draft: NamespacePickerState {
            anchor: state.anchor,
            draft: state.draft.clone(),
        },
        filter: shell.namespace_filter().clone(),
        filter_text: shell.namespace_filter().read(cx).value().to_string(),
        shell: weak,
    })
}

fn render_content(content: &PickerContent, cx: &gpui_kit::App) -> AnyElement {
    let shell = content.shell.clone();
    let all = Button::new("ns-all")
        .ghost()
        .small()
        .w_full()
        .label("All namespaces")
        .selected(content.scope == NamespaceScope::All)
        .on_click({
            let shell = shell.clone();
            move |_, _, cx| {
                let _ = shell.update(cx, |shell, cx| {
                    shell.apply_namespace_scope(NamespaceScope::All, cx)
                });
            }
        });
    v_flex()
        .w(PICKER_WIDTH)
        .gap_2()
        .child(Input::new(&content.filter).small().cleanable(true))
        .child(all)
        .child(separator(cx))
        .child(namespace_list(content))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(ROW_NOTE),
        )
        .child(separator(cx))
        .child(footer(content))
        .into_any_element()
}

fn separator(cx: &gpui_kit::App) -> impl IntoElement {
    div().h(px(1.)).w_full().bg(cx.theme().border)
}

fn namespace_list(content: &PickerContent) -> AnyElement {
    let list = v_flex()
        .id("ns-list")
        .max_h(MAX_LIST_HEIGHT)
        .overflow_y_scroll()
        .gap_0p5();
    match &content.namespaces {
        NamespaceRows::Loading => list
            .child(div().px_2().child("Loading namespaces…"))
            .into_any_element(),
        NamespaceRows::Failed { message, default } => list
            .child(div().px_2().child("Could not list namespaces"))
            .child(div().px_2().text_xs().child(message.clone()))
            .child(namespace_row(0, default, content))
            .into_any_element(),
        NamespaceRows::Ready(names) => {
            let shown = matching_namespaces(names, &content.filter_text);
            if shown.is_empty() {
                return list
                    .child(div().px_2().child("No namespace matches"))
                    .into_any_element();
            }
            list.children(
                shown
                    .into_iter()
                    .enumerate()
                    .map(|(index, name)| namespace_row(index, name, content)),
            )
            .into_any_element()
        }
    }
}

/// The names that contain `filter`, ignoring case; all of them for a blank filter.
fn matching_namespaces<'a>(names: &'a [String], filter: &str) -> Vec<&'a String> {
    let needle = filter.trim().to_lowercase();
    names
        .iter()
        .filter(|name| name.to_lowercase().contains(&needle))
        .collect()
}

/// A checkbox that toggles the draft, and the name as a button that picks only this namespace.
fn namespace_row(index: usize, name: &str, content: &PickerContent) -> AnyElement {
    let is_ticked = content.draft.is_ticked(name);
    let is_blocked = !is_ticked && !content.draft.can_add();
    let toggle = {
        let (shell, name) = (content.shell.clone(), name.to_owned());
        move |_: &bool, _: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            let _ = shell.update(cx, |shell, cx| shell.toggle_picker_namespace(&name, cx));
        }
    };
    let pick = {
        let (shell, name) = (content.shell.clone(), name.to_owned());
        move |_: &gpui_kit::ClickEvent, _: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            let scope = NamespaceScope::Named(name.clone());
            let _ = shell.update(cx, |shell, cx| shell.apply_namespace_scope(scope, cx));
        }
    };
    let checkbox = Checkbox::new(("ns-check", index))
        .checked(is_ticked)
        .disabled(is_blocked)
        .on_click(toggle);
    let checkbox = if is_blocked {
        checkbox.tooltip(format!(
            "At most {MAX_NAMESPACES} namespaces; pick All namespaces for more"
        ))
    } else {
        checkbox
    };
    h_flex()
        .items_center()
        .gap_1()
        .child(checkbox)
        .child(
            Button::new(("ns-name", index))
                .ghost()
                .small()
                .label(SharedString::from(name.to_owned()))
                .on_click(pick),
        )
        .into_any_element()
}

fn footer(content: &PickerContent) -> AnyElement {
    let applied = content.draft.applied_scope(&content.scope);
    let (clear_shell, apply_shell) = (content.shell.clone(), content.shell.clone());
    h_flex()
        .items_center()
        .gap_2()
        .child(div().flex_1().text_sm().child(format!(
            "{} selected (max {MAX_NAMESPACES})",
            content.draft.ticked_count()
        )))
        .child(
            Button::new("ns-clear")
                .ghost()
                .small()
                .label("Clear")
                .on_click(move |_, _, cx| {
                    let _ = clear_shell.update(cx, |shell, cx| shell.clear_picker_draft(cx));
                }),
        )
        .child(
            Button::new("ns-apply")
                .primary()
                .small()
                .label("Apply")
                .disabled(applied.is_none())
                .on_click(move |_, _, cx| {
                    if let Some(scope) = applied.clone() {
                        let _ = apply_shell
                            .update(cx, |shell, cx| shell.apply_namespace_scope(scope, cx));
                    }
                }),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn several(names: &[&str]) -> NamespaceScope {
        NamespaceScope::of_namespaces(names.iter().map(|name| (*name).to_owned()))
    }

    #[test]
    fn the_filter_keeps_names_containing_it_ignoring_case() {
        let names: Vec<String> = ["kube-system", "Payments", "web"].map(str::to_owned).into();
        assert_eq!(matching_namespaces(&names, "  PAY "), [&names[1]]);
        assert_eq!(matching_namespaces(&names, "").len(), 3);
        assert!(matching_namespaces(&names, "zzz").is_empty());
    }

    #[test]
    fn open_copies_scope_into_draft() {
        let mut state = NamespacePickerState::default();
        state.open(PickerAnchor::TitleBar, &several(&["web", "payments"]));
        assert!(state.is_ticked("payments") && state.is_ticked("web"));
        assert_eq!(state.ticked_count(), 2);
    }

    #[test]
    fn reopening_starts_from_the_scope_not_the_old_draft() {
        let mut state = NamespacePickerState::default();
        state.open(PickerAnchor::TitleBar, &NamespaceScope::All);
        state.toggle("kube-system");
        state.open(PickerAnchor::TitleBar, &NamespaceScope::All);
        assert_eq!(state.ticked_count(), 0);
    }

    #[test]
    fn open_records_the_anchor() {
        let mut state = NamespacePickerState::default();
        assert_eq!(state.anchor, None);
        state.open(PickerAnchor::FilterBar, &NamespaceScope::All);
        assert_eq!(state.anchor, Some(PickerAnchor::FilterBar));
    }

    #[test]
    fn close_clears_the_state_only_for_the_open_anchor() {
        let mut state = NamespacePickerState::default();
        state.open(PickerAnchor::FilterBar, &NamespaceScope::All);
        // The title bar popover reporting a close must not shut the filter bar's picker.
        state.close(PickerAnchor::TitleBar);
        assert_eq!(state.anchor, Some(PickerAnchor::FilterBar));
        state.close(PickerAnchor::FilterBar);
        assert_eq!(state.anchor, None);
    }

    #[test]
    fn dismiss_closes_whichever_anchor_is_open() {
        let mut state = NamespacePickerState::default();
        state.open(PickerAnchor::TitleBar, &NamespaceScope::All);
        state.dismiss();
        assert_eq!(state.anchor, None);
    }

    #[test]
    fn toggle_refuses_past_max() {
        let mut state = NamespacePickerState::default();
        for index in 0..MAX_NAMESPACES {
            assert!(state.toggle(&format!("ns-{index}")));
        }
        assert!(!state.can_add());
        assert!(!state.toggle("one-too-many"));
        assert_eq!(state.ticked_count(), MAX_NAMESPACES);
        // Unticking always works, and frees a slot.
        assert!(state.toggle("ns-0"));
        assert!(state.can_add());
    }

    #[test]
    fn applied_scope_is_none_when_empty_or_unchanged() {
        let mut state = NamespacePickerState::default();
        let current = several(&["a", "b"]);
        state.open(PickerAnchor::TitleBar, &current);
        assert_eq!(state.applied_scope(&current), None);
        state.toggle("c");
        assert_eq!(
            state.applied_scope(&current),
            Some(several(&["a", "b", "c"]))
        );
        state.clear();
        assert_eq!(state.applied_scope(&current), None);
    }

    #[test]
    fn applied_scope_of_one_name_is_named() {
        let mut state = NamespacePickerState::default();
        state.open(PickerAnchor::TitleBar, &NamespaceScope::All);
        state.toggle("web");
        assert_eq!(
            state.applied_scope(&NamespaceScope::All),
            Some(NamespaceScope::Named("web".to_owned()))
        );
    }
}
