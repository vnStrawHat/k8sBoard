//! The shell side of the key map: moving the row cursor, opening and dismissing the drawer,
//! switching containers, and the single-letter row actions. The bindings are in `keymap`. This
//! module is a child of `app_shell`, like `workspace`, because the handlers change the shell's
//! private state.

use gpui_kit::base::TextSelection;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::table::{TableDelegate, TableState};
use gpui_kit::{
    Action, App, ClipboardItem, Context, Div, Entity, Focusable as _, InteractiveElement as _,
    Window,
};

use super::node_editor::NodeEditKind;
use super::resource_edit_flow::RowCheck;
use super::{AppShell, Screen, focus_table};
use crate::dock::{DockMode, TabStep};
use crate::drawer::DrawerTab;
use crate::keymap::{
    CloseDockTab, CopyName, Cordon, Delete, Dismiss, Drain, EditHpaRange, EditLabels, EditTaints,
    EditYaml, ExpandClaim, LeaveInput, NextContainer, NextDockTab, OpenDrawer, OpenShell,
    PauseRollout, PortForward, PreviousContainer, PreviousDockTab, RerunJob, RestartRollout,
    RollBack, Scale, SelectFirstRow, SelectLastRow, SelectNextPage, SelectNextRow,
    SelectPreviousPage, SelectPreviousRow, SetDefaultStorageClass, SuspendCronJob, ToggleDock,
    ToggleDockZoom, ToggleReadOnly, TriggerCronJob, ViewLogs, ViewYaml,
};
use crate::live_sections::loaded_replica_sets;
use crate::pod_drawer::{container_display_order, selected_container_index};
use crate::resource_actions::{
    DebugPod, KeyAvailability, ResourceAction, RowAction, action_label, key_availability,
    subject_action, unavailable_text,
};
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::workload_actions::{row_block, state_label};

/// How a row-move key changes the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowStep {
    Next,
    Previous,
    First,
    Last,
    NextPage,
    PreviousPage,
}

/// The row the cursor moves to. `Next` and `Previous` wrap, like the kit table's own arrow keys
/// (its `loop_selection` is on); pages, `First`, and `Last` clamp. Without a cursor every step
/// starts at the top, except `Last`. A cursor past the end (rows left since) counts as the last row.
/// `None` for an empty table.
pub(crate) fn step_row(
    current: Option<usize>,
    row_count: usize,
    step: RowStep,
    page: usize,
) -> Option<usize> {
    if row_count == 0 {
        return None;
    }
    let last = row_count - 1;
    let Some(current) = current else {
        return Some(if step == RowStep::Last { last } else { 0 });
    };
    let current = current.min(last);
    Some(match step {
        RowStep::Next if current == last => 0,
        RowStep::Next => current + 1,
        RowStep::Previous if current == 0 => last,
        RowStep::Previous => current - 1,
        RowStep::First => 0,
        RowStep::Last => last,
        RowStep::NextPage => (current + page).min(last),
        RowStep::PreviousPage => current.saturating_sub(page),
    })
}

/// What Esc can undo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DismissState {
    pub(crate) is_dock_zoomed: bool,
    pub(crate) is_drawer_open: bool,
    pub(crate) has_selection: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DismissStep {
    UnzoomDock,
    CloseDrawer,
    ClearSelection,
    /// Nothing to undo: the key goes on to the next binding.
    Propagate,
}

/// The first step of the Esc ladder that applies: a zoomed dock, then an open drawer (the row is
/// kept), then the cursor.
pub(crate) fn dismiss_step(state: DismissState) -> DismissStep {
    if state.is_dock_zoomed {
        DismissStep::UnzoomDock
    } else if state.is_drawer_open {
        DismissStep::CloseDrawer
    } else if state.has_selection {
        DismissStep::ClearSelection
    } else {
        DismissStep::Propagate
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContainerStep {
    Next,
    Previous,
}

/// The container `[` or `]` selects. `order` is the display order (`container_display_order`) and
/// `current` an index into the pod's containers. No wrap; without a current container (or one
/// outside `order`) the first of `order` is chosen. `None` for a pod with no containers.
pub(crate) fn step_container(
    order: &[usize],
    current: Option<usize>,
    step: ContainerStep,
) -> Option<usize> {
    let first = *order.first()?;
    let position = current.and_then(|current| order.iter().position(|&index| index == current));
    let Some(position) = position else {
        return Some(first);
    };
    let target = match step {
        ContainerStep::Next => (position + 1).min(order.len() - 1),
        ContainerStep::Previous => position.saturating_sub(1),
    };
    Some(order[target])
}

/// Marks the notice of an unavailable row key. One id, so repeated presses replace it instead of
/// stacking.
struct RowKeyNotice;

/// Registers the handlers of the row, drawer, container, and row-action keys on the shell root.
/// The handlers of the chords are registered in `AppShell::render`.
pub(super) fn register_key_handlers(root: Div, cx: &Context<AppShell>) -> Div {
    let root = on_step::<SelectNextRow>(root, RowStep::Next, cx);
    let root = on_step::<SelectPreviousRow>(root, RowStep::Previous, cx);
    let root = on_step::<SelectFirstRow>(root, RowStep::First, cx);
    let root = on_step::<SelectLastRow>(root, RowStep::Last, cx);
    let root = on_step::<SelectNextPage>(root, RowStep::NextPage, cx);
    let root = on_step::<SelectPreviousPage>(root, RowStep::PreviousPage, cx);
    let root = root
        .on_action(cx.listener(|shell, _: &OpenDrawer, window, cx| {
            shell.open_drawer_at_cursor(window, cx);
        }))
        .on_action(cx.listener(|shell, _: &Dismiss, _, cx| shell.dismiss(cx)))
        .on_action(cx.listener(|shell, _: &LeaveInput, window, cx| {
            shell.focus_visible_table(window, cx);
        }))
        .on_action(cx.listener(|shell, _: &NextContainer, _, cx| {
            shell.step_container_at_cursor(ContainerStep::Next, cx);
        }))
        .on_action(cx.listener(|shell, _: &PreviousContainer, _, cx| {
            shell.step_container_at_cursor(ContainerStep::Previous, cx);
        }))
        .on_action(
            cx.listener(|shell, _: &CopyName, window, cx| shell.copy_cursor_name(window, cx)),
        )
        .on_action(cx.listener(|shell, _: &ToggleReadOnly, window, cx| {
            shell.toggle_lock_of_target(window, cx);
        }))
        .on_action(cx.listener(|shell, _: &ToggleDock, _, cx| {
            shell.dock.update(cx, |dock, cx| dock.toggle_visibility(cx));
        }))
        .on_action(cx.listener(|shell, _: &ToggleDockZoom, _, cx| {
            shell.dock.update(cx, |dock, cx| dock.toggle_zoom(cx));
        }))
        .on_action(cx.listener(|shell, _: &NextDockTab, _, cx| {
            shell
                .dock
                .update(cx, |dock, cx| dock.step_active_tab(TabStep::Next, cx));
        }))
        .on_action(cx.listener(|shell, _: &PreviousDockTab, _, cx| {
            shell
                .dock
                .update(cx, |dock, cx| dock.step_active_tab(TabStep::Previous, cx));
        }))
        .on_action(cx.listener(|shell, _: &CloseDockTab, _, cx| {
            shell.dock.update(cx, |dock, cx| dock.close_active_tab(cx));
        }));
    let root = on_row_key::<ViewLogs>(root, RowAction::ViewLogs, cx);
    let root = on_row_key::<ViewYaml>(root, RowAction::ViewYaml, cx);
    let root = on_row_key::<OpenShell>(root, RowAction::OpenShell, cx);
    let root = on_row_key::<PortForward>(root, RowAction::PortForward, cx);
    let root = on_row_key::<Cordon>(root, RowAction::Cordon, cx);
    let root = on_row_key::<Drain>(root, RowAction::Drain, cx);
    let root = on_row_key::<EditTaints>(root, RowAction::EditTaints, cx);
    let root = on_row_key::<EditLabels>(root, RowAction::EditLabels, cx);
    let root = on_row_key::<EditYaml>(root, RowAction::EditYaml, cx);
    let root = on_row_key::<RestartRollout>(root, RowAction::RestartRollout, cx);
    let root = on_row_key::<Scale>(root, RowAction::Scale, cx);
    let root = on_row_key::<PauseRollout>(root, RowAction::PauseRollout, cx);
    let root = on_row_key::<RollBack>(root, RowAction::RollBack, cx);
    let root = on_row_key::<SuspendCronJob>(root, RowAction::SuspendCronJob, cx);
    let root = on_row_key::<TriggerCronJob>(root, RowAction::TriggerCronJob, cx);
    let root = on_row_key::<RerunJob>(root, RowAction::RerunJob, cx);
    let root = on_row_key::<EditHpaRange>(root, RowAction::EditHpaRange, cx);
    let root = on_row_key::<ExpandClaim>(root, RowAction::ExpandClaim, cx);
    let root = on_row_key::<SetDefaultStorageClass>(root, RowAction::SetDefaultStorageClass, cx);
    on_row_key::<Delete>(root, RowAction::Delete, cx)
}

fn on_step<A: Action>(root: Div, step: RowStep, cx: &Context<AppShell>) -> Div {
    root.on_action(cx.listener(move |shell, _: &A, window, cx| {
        shell.step_cursor(step, window, cx);
    }))
}

fn on_row_key<A: Action>(root: Div, action: RowAction, cx: &Context<AppShell>) -> Div {
    root.on_action(cx.listener(move |shell, _: &A, window, cx| {
        shell.run_row_key(action, window, cx);
    }))
}

/// The row the cursor would move to in `table`, or `None` when the table is empty.
fn next_cursor_row<D: TableDelegate>(
    table: &Entity<TableState<D>>,
    step: RowStep,
    cx: &App,
) -> Option<usize> {
    let table = table.read(cx);
    // One row of overlap keeps the eye on the list when it pages.
    let page = table.visible_range().rows().len().saturating_sub(1).max(1);
    step_row(
        table.selected_row(),
        table.delegate().rows_count(cx),
        step,
        page,
    )
}

impl AppShell {
    /// J, K, the arrows, Home, End, PgUp, PgDn: moves the row cursor of the visible table. The
    /// drawer follows only while it is open. Overview and Topology have no table: the graph is a
    /// canvas without a cursor, so the keys do nothing there.
    fn step_cursor(&mut self, step: RowStep, window: &mut Window, cx: &mut Context<Self>) {
        // The cursor is kept under the Edit YAML view; no key moves it there.
        if self.is_editing() {
            return;
        }
        match self.screen {
            Screen::Overview | Screen::Topology | Screen::PortForwarding => {}
            Screen::Pods => {
                let table = self.pod_table.clone();
                self.move_cursor(&table, step, window, cx);
            }
            Screen::Nodes => {
                let table = self.node_table.clone();
                self.move_cursor(&table, step, window, cx);
            }
            Screen::Kind(_) => {
                let table = self.kind_table.clone();
                self.move_cursor(&table, step, window, cx);
            }
            Screen::Issues => {
                let table = self.issue_table.clone();
                let Some(row) = next_cursor_row(&table, step, cx) else {
                    return;
                };
                // Nothing listens to this table's `SelectRow` (a click reveals the issue), so an
                // echo mark would never be consumed.
                table.update(cx, |table, cx| table.set_selected_row(row, cx));
                focus_table(&table, window, cx);
            }
        }
    }

    fn move_cursor<D: TableDelegate>(
        &mut self,
        table: &Entity<TableState<D>>,
        step: RowStep,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = next_cursor_row(table, step, cx) else {
            return;
        };
        self.select_table_row(table, row, cx);
        focus_table(table, window, cx);
    }

    /// Enter: opens the drawer on the cursor row, or on the first row without a cursor. On Issues
    /// it reveals the issue's object, as a click does. Enter on any other focused control (a
    /// button the user tabbed to) stays with that control.
    fn open_drawer_at_cursor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_editing() {
            return;
        }
        if !self.is_cursor_surface_focused(window, cx) {
            cx.propagate();
            return;
        }
        match self.screen {
            // A canvas has no cursor row to open; a node opens by click.
            Screen::Overview | Screen::Topology | Screen::PortForwarding => {}
            Screen::Pods => {
                let table = self.pod_table.clone();
                self.open_table_row(&table, window, cx);
            }
            Screen::Nodes => {
                let table = self.node_table.clone();
                self.open_table_row(&table, window, cx);
            }
            Screen::Kind(_) => {
                let table = self.kind_table.clone();
                self.open_table_row(&table, window, cx);
            }
            Screen::Issues => {
                if let Some(row) = self.issue_table.read(cx).selected_row() {
                    self.reveal_issue(row, cx);
                }
            }
        }
    }

    /// Whether the keyboard is on the shell root or on a table, where Enter means "open the row".
    fn is_cursor_surface_focused(&self, window: &Window, cx: &App) -> bool {
        self.focus_handle.is_focused(window)
            || self.pod_table.read(cx).focus_handle(cx).is_focused(window)
            || self.node_table.read(cx).focus_handle(cx).is_focused(window)
            || self
                .issue_table
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
            || self.kind_table.read(cx).focus_handle(cx).is_focused(window)
    }

    fn open_table_row<D: TableDelegate>(
        &mut self,
        table: &Entity<TableState<D>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected.is_some() {
            self.set_drawer_open(true, cx);
            focus_table(table, window, cx);
            return;
        }
        if table.read(cx).delegate().rows_count(cx) == 0 {
            return;
        }
        // No echo mark: the first row is selected as a click would, which opens the drawer.
        table.update(cx, |table, cx| table.set_selected_row(0, cx));
    }

    /// Esc: undoes one step of the ladder.
    fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.is_editing() {
            return;
        }
        let state = DismissState {
            is_dock_zoomed: self.dock.read(cx).mode() == DockMode::Zoomed,
            is_drawer_open: self.drawer.is_open,
            has_selection: self.selected.is_some(),
        };
        match dismiss_step(state) {
            DismissStep::UnzoomDock => self.dock.update(cx, |dock, cx| dock.unzoom(cx)),
            DismissStep::CloseDrawer => self.close_drawer(cx),
            DismissStep::ClearSelection => self.clear_selection(cx),
            DismissStep::Propagate => cx.propagate(),
        }
    }

    /// Esc in a text field: the focus goes to the table the screen shows, or to the shell root
    /// while no table is drawn. The field keeps its text.
    pub(super) fn focus_visible_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.scope_live(cx).is_none() {
            window.focus(&self.focus_handle, cx);
            return;
        }
        match self.screen {
            Screen::Overview | Screen::Topology | Screen::PortForwarding => {
                window.focus(&self.focus_handle, cx);
            }
            Screen::Pods => focus_table(&self.pod_table.clone(), window, cx),
            Screen::Nodes => focus_table(&self.node_table.clone(), window, cx),
            Screen::Issues => focus_table(&self.issue_table.clone(), window, cx),
            Screen::Kind(_) => focus_table(&self.kind_table.clone(), window, cx),
        }
    }

    /// `[` and `]`: selects the previous or next container of the open pod drawer and shows the
    /// Containers tab.
    fn step_container_at_cursor(&mut self, step: ContainerStep, cx: &mut Context<Self>) {
        let Some(subject) = self.drawer_subject().cloned() else {
            return;
        };
        let key = &subject.key;
        if !matches!(key, ResourceKey::Pod { .. }) {
            return;
        }
        let Some(live) = self.slot_live(&subject.cluster, cx) else {
            return;
        };
        let Some(pod) = live.pods.items().iter().find(|pod| key.is_pod(pod)) else {
            return;
        };
        let order = container_display_order(&pod.containers);
        let current = selected_container_index(pod, &self.drawer);
        let Some(next) = step_container(&order, current, step) else {
            return;
        };
        let name = pod.containers[next].name.clone();
        self.open_container(name, cx);
    }

    /// Ctrl C: copies the name of the cursor row, unless text is selected: then the kit's copy of
    /// the selection runs instead.
    fn copy_cursor_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(subject) = &self.selected else {
            cx.propagate();
            return;
        };
        if TextSelection::has_selection(window, cx) {
            cx.propagate();
            return;
        }
        let name = match &subject.key {
            ResourceKey::Pod { name, .. }
            | ResourceKey::Node { name }
            | ResourceKey::Kind { name, .. } => name.clone(),
        };
        cx.write_to_clipboard(ClipboardItem::new_string(name));
    }

    /// Roll back…: the drawer of the cursor row on its Overview, scrolled to the Revisions, where
    /// the Roll back buttons are. The scroll happens on the next paint of the drawer. A row the
    /// state blocks (paused, revisions not loaded, no older one) says why instead: a disabled menu
    /// item dispatches this too.
    fn show_revisions(
        &mut self,
        subject: &ClusterObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let blocked = self.slot_live(&subject.cluster, cx).and_then(|live| {
            let ResourceKey::Kind { kind, .. } = &subject.key else {
                return None;
            };
            let row = live.row_of(&subject.key)?;
            let replica_sets = loaded_replica_sets(*kind, row, live);
            row_block(ResourceAction::RollBack, &row.object, replica_sets)
        });
        if let Some(reason) = blocked {
            let text = unavailable_text(action_label(ResourceAction::RollBack), &reason);
            window.push_notification(Notification::warning(text).id::<RowKeyNotice>(), cx);
            return;
        }
        self.drawer.tab = DrawerTab::Overview;
        self.drawer.reveal_revisions.set(true);
        self.set_drawer_open(true, cx);
    }

    /// A single-letter row action on the cursor row, drawer open or closed. A key the subject
    /// does not offer does nothing; an offered but unavailable one says why.
    pub(crate) fn run_row_key(
        &mut self,
        row: RowAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // While Edit YAML is open the cursor is hidden, so no row key acts on it.
        if self.is_editing() {
            return;
        }
        let Some(subject) = self.selected.clone() else {
            return;
        };
        let (Some(live), Some(guard)) = (
            self.slot_live(&subject.cluster, cx),
            self.guard_for(&subject.cluster, cx),
        ) else {
            return;
        };
        match key_availability(row, &subject.key, live, &guard) {
            KeyAvailability::NotOffered => {}
            KeyAvailability::Disabled { reason } => {
                // A disabled key is an offered one, so the subject resolves it.
                let Some(action) = subject_action(row, &subject.key) else {
                    return;
                };
                let label = live
                    .row_of(&subject.key)
                    .map_or(action_label(action), |row| {
                        state_label(action, action_label(action), &row.object)
                    });
                let text = unavailable_text(label, &reason);
                window.push_notification(Notification::warning(text).id::<RowKeyNotice>(), cx);
            }
            KeyAvailability::Run(action) => self.run_available_row_key(action, subject, window, cx),
        }
    }

    /// What an available key does. A mutating action that has not shipped stays disabled, so its arm is
    /// unreachable until the owning spec wires it. The match is exhaustive so a new action cannot be
    /// forgotten.
    fn run_available_row_key(
        &mut self,
        action: ResourceAction,
        subject: ClusterObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            ResourceAction::ViewLogs => self.open_logs_of_selection(window, cx),
            ResourceAction::ViewYaml => self.open_drawer_tab(subject, DrawerTab::Yaml, cx),
            // Copy name never reaches here: `copy_cursor_name` handles it before the gate.
            ResourceAction::CopyName => {}
            // The default container of the cursor pod, in the cursor's own cluster (spec 0036).
            ResourceAction::OpenShell => self.open_default_shell(&subject, window, cx),
            // The node shell options dialog of the cursor node, in the cursor's own cluster (spec 0037).
            ResourceAction::OpenNodeShell => {
                if let ResourceKey::Node { name } = &subject.key {
                    self.open_node_shell_options(&subject.cluster, name, window, cx);
                }
            }
            // Its key is unbound; the menu item carries the pod, so this only runs from a caller that
            // resolved the action for the cursor pod.
            ResourceAction::DebugContainer => {
                if let ResourceKey::Pod { namespace, name } = &subject.key {
                    let pod = DebugPod {
                        cluster: subject.cluster.clone(),
                        namespace: namespace.clone(),
                        pod: name.clone(),
                    };
                    self.open_debug_options(pod, None, window, cx);
                }
            }
            // One TCP port starts at once; several or none open New forward (spec 0035). On the
            // subject's own cluster, never the primary.
            ResourceAction::PortForward => self.run_port_forward_key(&subject, window, cx),
            // Always on the subject's own cluster, never the primary.
            ResourceAction::Cordon => {
                if let ResourceKey::Node { name } = &subject.key {
                    self.start_cordon(&subject.cluster, name, None, window, cx);
                }
            }
            // Always the dialog, on the cursor node's own cluster; nothing runs from the key.
            ResourceAction::Drain => {
                if let ResourceKey::Node { name } = &subject.key {
                    self.start_drain(&subject.cluster, std::slice::from_ref(name), window, cx);
                }
            }
            // Only the bulk Uncordon button uses it; the key resolves a node to `Cordon`.
            ResourceAction::Uncordon => {}
            // The editors of the cursor node, in the cursor's own cluster (spec 0034).
            ResourceAction::EditTaints | ResourceAction::EditLabels => {
                if let ResourceKey::Node { name } = &subject.key {
                    let kind = if action == ResourceAction::EditTaints {
                        NodeEditKind::Taints
                    } else {
                        NodeEditKind::Labels
                    };
                    self.open_node_editor(kind, &subject.cluster, name, None, window, cx);
                }
            }
            // Opens the editor on the cursor row, in its own cluster (spec 0031).
            ResourceAction::EditYaml(_) => self.open_edit(subject, window, cx),
            // The cursor row, or the ticked set when it is one of several (spec 0033).
            ResourceAction::Delete(_) => self.delete_at_cursor(&subject, window, cx),
            // Each builds its intent from the cursor row and opens the confirm dialog.
            ResourceAction::RestartRollout(_)
            | ResourceAction::PauseRollout
            | ResourceAction::SuspendCronJob
            | ResourceAction::TriggerCronJob
            | ResourceAction::RerunJob => self.start_workload_action(action, &subject, window, cx),
            ResourceAction::Scale(_) => self.open_scale_popover(&subject, window, cx),
            ResourceAction::EditHpaRange => self.open_hpa_range_popover(&subject, window, cx),
            ResourceAction::ExpandClaim => self.open_expand_popover(&subject, window, cx),
            ResourceAction::SetDefaultStorageClass => {
                self.start_set_default(&subject, RowCheck::Enforced, window, cx);
            }
            // The buttons are in the drawer, so Roll back… takes the user to the revisions.
            ResourceAction::RollBack => self.show_revisions(&subject, window, cx),
        }
    }
}

#[cfg(test)]
#[path = "keyboard_navigation_tests.rs"]
mod keyboard_navigation_tests;
