//! Back and forward (spec 0056 history): the shell reads the place it shows, records it when a
//! link is followed, and restores one through the same setters a reveal uses. It also follows a
//! drawer link and builds the drawer header's navigation controls.

use std::rc::Rc;

use gpui_kit::component::table::{TableDelegate, TableState};
use gpui_kit::{App, Context, Entity};

use super::keyboard_navigation::RowStep;
use super::{AppShell, Screen, remapped_screen};
use crate::custom_kind::CustomKind;
use crate::drawer::{BackTarget, ClickHandler, DrawerNavigation, RowControls};

use crate::navigation_history::{Place, row_position};
use crate::table_filter::TableFilter;
use crate::table_selection::ClusterObject;

/// Which way `step_history` walks.
#[derive(Clone, Copy)]
enum HistoryStep {
    Back,
    Forward,
}

/// Whether `place` can still be shown. A custom kind that the CRD list no longer serves is not;
/// `None` (the list is unknown, or not loaded yet) proves nothing, so the place stays.
pub(super) fn is_place_served(place: &Place, kinds: Option<&[CustomKind]>) -> bool {
    kinds.is_none_or(|kinds| remapped_screen(place.screen, kinds).is_none())
}

impl AppShell {
    /// What the shell shows now.
    fn current_place(&self, cx: &App) -> Place {
        let filter = self.toolkit_state(cx).map(|state| TableFilter {
            text: state.text,
            chips: state.chips,
            preset: state.preset,
        });
        Place {
            screen: self.screen,
            selection: self.selected.clone(),
            is_drawer_open: self.drawer.is_open,
            tab: self.drawer.tab,
            container: self.drawer.selected_container.clone(),
            container_tab: self.drawer.container_tab,
            filter,
        }
    }

    /// Remembers the place a reveal of `target` leaves. Revealing what is already shown records
    /// nothing, or Back would lead to the same place.
    pub(super) fn record_place_before_reveal(&mut self, target: &ClusterObject, cx: &App) {
        let is_shown = self.screen == target.key.screen() && self.selected.as_ref() == Some(target);
        if is_shown {
            return;
        }
        let from = self.current_place(cx);
        self.navigation.record(from);
    }

    /// Alt+Left: back to where the last link was followed from.
    pub(super) fn go_back(&mut self, cx: &mut Context<Self>) {
        self.step_history(HistoryStep::Back, cx);
    }

    /// Alt+Right: the place Back left.
    pub(super) fn go_forward(&mut self, cx: &mut Context<Self>) {
        self.step_history(HistoryStep::Forward, cx);
    }

    fn step_history(&mut self, step: HistoryStep, cx: &mut Context<Self>) {
        // Leaving the editor asks first, as a reveal does.
        if self.has_unsaved_edit(cx) {
            self.ask_discard(move |shell, cx| shell.step_history(step, cx), cx);
            return;
        }
        let current = self.current_place(cx);
        let kinds = self.served_custom_kinds(cx);
        let is_served = |place: &Place| is_place_served(place, kinds.as_deref());
        let target = match step {
            HistoryStep::Back => self.navigation.back(current, is_served),
            HistoryStep::Forward => self.navigation.forward(current, is_served),
        };
        if let Some(place) = target {
            self.restore_place(place, cx);
        }
        cx.notify();
    }

    /// The drawer header's Back button and Previous / Next controls.
    pub(crate) fn drawer_navigation(&self, cx: &Context<Self>) -> DrawerNavigation {
        let back = self.navigation.previous().map(|place| BackTarget {
            label: place.back_label().into(),
            tooltip: place.back_tooltip().into(),
            on_click: Rc::new(cx.listener(|shell, _, _, cx| shell.go_back(cx))),
        });
        DrawerNavigation {
            back,
            rows: self.row_controls(cx),
        }
    }

    /// Previous / Next for the screens with a table cursor. The cursor is kept under the Edit YAML
    /// view, so the controls are hidden there; a reveal that is still loading has no visible row
    /// yet, so they are disabled.
    fn row_controls(&self, cx: &Context<Self>) -> Option<RowControls> {
        if self.is_editing() {
            return None;
        }
        let position = match self.screen {
            Screen::Pods => cursor_position(&self.pod_table, cx),
            Screen::Nodes => cursor_position(&self.node_table, cx),
            Screen::Kind(_) => cursor_position(&self.kind_table, cx),
            Screen::Overview | Screen::Issues | Screen::Topology | Screen::PortForwarding => {
                return None;
            }
        };
        let step = |step: RowStep| -> ClickHandler {
            Rc::new(cx.listener(move |shell, _, window, cx| shell.step_cursor(step, window, cx)))
        };
        Some(RowControls {
            position: position.filter(|_| self.pending_reveal.is_none()),
            on_previous: step(RowStep::Previous),
            on_next: step(RowStep::Next),
        })
    }

    /// The custom kinds the open cluster serves, once its CRD list has loaded.
    fn served_custom_kinds(&self, cx: &App) -> Option<Vec<CustomKind>> {
        let crds = self.live(cx)?.crds.as_ref()?;
        crds.list.ready_count()?;
        Some(crds.kinds.clone())
    }

    /// Shows `place` again: the screen, its filter, and then, deferred like `reveal_then` (the
    /// `ClearSelection` events of `show_screen` would erase an earlier selection), the object, the
    /// drawer, its tabs, and the container. A place without a selection restores screen and filter
    /// only. A row that is gone is dropped by the pending reveal, so no drawer opens.
    fn restore_place(&mut self, place: Place, cx: &mut Context<Self>) {
        self.close_edit(cx);
        self.show_screen(place.screen, cx);
        if let Some(filter) = place.filter {
            self.rebuild_visible_view(cx, move |view| view.filter = filter);
            // The quick filter input follows the filter text of its screen.
            self.quick_filter_screen = None;
        }
        let Some(object) = place.selection else {
            return;
        };
        let (screen, is_drawer_open, tab) = (place.screen, place.is_drawer_open, place.tab);
        let (container, container_tab) = (place.container, place.container_tab);
        let shell = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = shell.update(cx, |shell, cx| {
                // The graph shows no table: its drawer appears once the Topology feeds deliver
                // the row, so there is no list to wait for.
                if screen != Screen::Topology {
                    shell.pending_reveal = Some(object.clone());
                }
                shell.change_selection(Some(object), cx);
                shell.set_drawer_open(is_drawer_open, cx);
                // Never assign `drawer.tab`: the setter drops revealed Secret values.
                shell.set_drawer_tab(tab, cx);
                if let Some(name) = container {
                    shell.select_container(name, cx);
                }
                shell.set_container_tab(container_tab, cx);
                shell.apply_pending_reveal(cx);
                shell.sync_selection(cx);
            });
        });
    }
}

/// The 1-based cursor row of `table` and the number of visible rows.
fn cursor_position<D: TableDelegate>(
    table: &Entity<TableState<D>>,
    cx: &App,
) -> Option<(usize, usize)> {
    let table = table.read(cx);
    row_position(table.selected_row(), table.delegate().rows_count(cx))
}
