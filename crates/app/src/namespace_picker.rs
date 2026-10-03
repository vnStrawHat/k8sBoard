//! The namespace picker: a popover with a checkbox per namespace and Apply. Two triggers share
//! one state (the title bar and the filter bar chip), and only the clicked one opens.

use std::collections::BTreeSet;

use cluster::NamespaceScope;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    SharedString, StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _, px,
};

use crate::app_shell::AppShell;
use crate::cluster_session::{LiveList, SessionPhase};

/// Bounds the watches (2N + 3) and the access reviews (16N + 3) of one scope.
pub(crate) const MAX_NAMESPACES: usize = 5;

const MAX_LIST_HEIGHT: Pixels = px(360.);
const PICKER_WIDTH: Pixels = px(280.);

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
    /// One muted line for each viewed cluster whose list is not there (several clusters only).
    notes: Vec<String>,
    draft: NamespacePickerState,
    shell: WeakEntity<AppShell>,
}

/// What one viewed cluster knows of its namespaces.
pub(crate) enum SlotNamespaces {
    Ready(Vec<String>),
    Loading,
    /// The list failed or is not permitted.
    Denied,
    /// The connect failed, so there is no list to wait for.
    Unreachable,
}

/// The rows for several viewed clusters: the union of the namespace names of the clusters whose
/// list is ready, sorted, and a muted line for each of the others. `slots` are `(label, state)`.
pub(crate) fn union_rows(slots: &[(&str, SlotNamespaces)]) -> (Vec<String>, Vec<String>) {
    let mut names = BTreeSet::new();
    let mut notes = Vec::new();
    for (label, state) in slots {
        match state {
            SlotNamespaces::Ready(ready) => names.extend(ready.iter().cloned()),
            SlotNamespaces::Loading => notes.push(format!("Loading namespaces of {label}…")),
            SlotNamespaces::Denied => notes.push(format!("Not permitted in {label}")),
            SlotNamespaces::Unreachable => notes.push(format!("Cannot connect to {label}")),
        }
    }
    (names.into_iter().collect(), notes)
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
        move |open: &bool, _: &mut gpui_kit::Window, cx: &mut gpui_kit::App| {
            let _ = weak.update(cx, |shell, cx| {
                if *open {
                    shell.open_namespace_picker(anchor, cx);
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
    let live = shell.scope_live(cx)?;
    if shell.view().is_multi() {
        return Some(union_content(shell, live.scope.clone(), state, weak, cx));
    }
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
        notes: Vec::new(),
        draft: NamespacePickerState {
            anchor: state.anchor,
            draft: state.draft.clone(),
        },
        shell: weak,
    })
}

/// The picker of several viewed clusters: one list of every namespace any of them has.
fn union_content(
    shell: &AppShell,
    scope: NamespaceScope,
    state: &NamespacePickerState,
    weak: WeakEntity<AppShell>,
    cx: &Context<AppShell>,
) -> PickerContent {
    let slots: Vec<(&str, SlotNamespaces)> = shell
        .view()
        .slots()
        .iter()
        .map(|slot| {
            let session = slot.session.read(cx);
            if matches!(session.phase(), SessionPhase::Failed { .. }) {
                return (slot.label.as_str(), SlotNamespaces::Unreachable);
            }
            let namespaces = match session.live().map(|live| &live.namespaces) {
                Some(LiveList::Ready { items, .. }) => SlotNamespaces::Ready(
                    items
                        .iter()
                        .map(|namespace| namespace.name.clone())
                        .collect(),
                ),
                Some(LiveList::Failed { .. }) => SlotNamespaces::Denied,
                Some(LiveList::Loading) | None => SlotNamespaces::Loading,
            };
            (slot.label.as_str(), namespaces)
        })
        .collect();
    let (names, notes) = union_rows(&slots);
    PickerContent {
        scope,
        namespaces: NamespaceRows::Ready(names),
        notes,
        draft: NamespacePickerState {
            anchor: state.anchor,
            draft: state.draft.clone(),
        },
        shell: weak,
    }
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
        .child(all)
        .child(separator(cx))
        .child(namespace_list(content))
        .children(content.notes.iter().map(|note| {
            div()
                .px_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(note.clone())
        }))
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
        NamespaceRows::Ready(names) => list
            .children(
                names
                    .iter()
                    .enumerate()
                    .map(|(index, name)| namespace_row(index, name, content)),
            )
            .into_any_element(),
    }
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
    fn picker_names_a_cluster_that_failed_to_connect() {
        let (names, notes) = union_rows(&[
            ("prod-eu", SlotNamespaces::Unreachable),
            ("stg-b", SlotNamespaces::Ready(vec!["web".to_owned()])),
        ]);
        // A failed connect is not "Loading" for ever.
        assert_eq!(names, ["web"]);
        assert_eq!(notes, ["Cannot connect to prod-eu"]);
    }

    #[test]
    fn picker_lists_union_with_loading_lines() {
        let ready = |names: &[&str]| {
            SlotNamespaces::Ready(names.iter().map(|name| (*name).to_owned()).collect())
        };
        let (names, notes) = union_rows(&[
            ("prod-eu", ready(&["web", "payments"])),
            ("stg-b", ready(&["web", "kube-system"])),
            ("dev-c", SlotNamespaces::Loading),
            ("uat", SlotNamespaces::Denied),
        ]);
        assert_eq!(names, ["kube-system", "payments", "web"]);
        assert_eq!(
            notes,
            ["Loading namespaces of dev-c…", "Not permitted in uat"]
        );
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
