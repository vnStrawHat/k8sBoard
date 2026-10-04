//! The bottom dock that holds the log tabs: tab bar, zoom, and minimize.

use cluster::ClusterConnection;
use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Context, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};

use crate::app_shell::AppShell;
use crate::cluster_registry::ClusterRef;
use crate::cluster_session::ClusterSession;
use crate::drain_tab::DrainTab;
use crate::log_tab::{LogLayout, LogTab, LogTabEvent};
use crate::log_target::{ContainerChoice, LogTarget, NoLogTarget};
use crate::log_window::open_log_window;
use crate::resource_actions::{RowAction, disabled_menu_item};
use crate::row_context::RowContext;
use crate::shell_tab::{AttachGrant, ShellGrant, ShellKind, ShellTab, ShellTarget};
use crate::status_tone::{StatusTone, tone_color};

pub(crate) const DEFAULT_DOCK_HEIGHT: Pixels = px(280.);
pub(crate) const MIN_DOCK_HEIGHT: Pixels = px(120.);
const MAX_DOCK_FRACTION: f32 = 0.6;
const TAB_BAR_HEIGHT: Pixels = px(34.);
const TAB_LABEL_MAX_WIDTH: Pixels = px(260.);
/// How many shell tabs the dock holds: each keeps a terminal with its scrollback (about 8 MB at
/// 200 columns), so the cap bounds the memory.
pub(crate) const MAX_SHELL_TABS: usize = 8;

/// 60% of the measured workspace, never below the minimum (so the range stays valid);
/// `Pixels::MAX` before the first layout, when the container is still zero.
pub(crate) fn dock_max_height(container: Pixels) -> Pixels {
    if container <= px(0.) {
        return Pixels::MAX;
    }
    (container * MAX_DOCK_FRACTION).max(MIN_DOCK_HEIGHT)
}

/// The dock height a launch starts with: the saved one when finite and positive, clamped to
/// `MIN_DOCK_HEIGHT ..= dock_max_height(viewport)`; else `DEFAULT_DOCK_HEIGHT` under the same cap.
/// The workspace is unmeasured on the first frame, so the viewport stands in for it.
pub(crate) fn initial_dock_height(saved: Option<f32>, viewport: Pixels) -> Pixels {
    let wanted = saved
        .filter(|height| height.is_finite() && *height > 0.)
        .map_or(DEFAULT_DOCK_HEIGHT, px);
    wanted.max(MIN_DOCK_HEIGHT).min(dock_max_height(viewport))
}

/// What a resize end stores: whole pixels, `None` for the default (a reset forgets the preference).
pub(crate) fn saved_dock_height(height: Pixels) -> Option<f32> {
    let whole = f32::from(height).round();
    (whole != f32::from(DEFAULT_DOCK_HEIGHT)).then_some(whole)
}

/// How far above the handle the 60 % line sits; `None` before the first layout.
pub(crate) fn max_line_offset(container: Pixels, dock: Pixels) -> Option<Pixels> {
    if container <= px(0.) {
        return None;
    }
    Some((dock_max_height(container) - dock).max(px(0.)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DockMode {
    Normal,
    Minimized,
    Zoomed,
}

/// The cluster a log tab reads from: its connection, and the session its workload tabs follow.
pub(crate) struct LogOrigin {
    pub(crate) cluster: ClusterRef,
    /// Weak: workload tabs observe the session, but a tab never keeps it alive.
    pub(crate) session: WeakEntity<ClusterSession>,
    pub(crate) connection: ClusterConnection,
}

impl LogOrigin {
    pub(crate) fn new(row: &RowContext, connection: ClusterConnection) -> Self {
        Self {
            cluster: row.cluster.clone(),
            session: row.session.clone(),
            connection,
        }
    }
}

/// One tab of the dock: a log view, a shell, or a drain. Never side by side: the dock shows one at
/// a time.
pub(crate) enum DockTab {
    Logs(Entity<LogTab>),
    Shell(Entity<ShellTab>),
    Drain(Entity<DrainTab>),
}

impl DockTab {
    /// A running drain stays: closing its tab would hide a live run (`Cancel the drain first`).
    fn is_pinned(&self, cx: &App) -> bool {
        match self {
            Self::Drain(tab) => tab.read(cx).is_running(),
            Self::Logs(_) | Self::Shell(_) => false,
        }
    }

    fn title(&self, cx: &App) -> String {
        match self {
            Self::Logs(tab) => tab.read(cx).label(),
            Self::Shell(tab) => tab.read(cx).label(),
            Self::Drain(tab) => tab.read(cx).label(),
        }
    }

    fn tone(&self, cx: &App) -> StatusTone {
        match self {
            Self::Logs(tab) => tab.read(cx).tone(),
            Self::Shell(tab) => tab.read(cx).tone(),
            Self::Drain(tab) => tab.read(cx).tone(),
        }
    }

    fn icon(&self) -> IconName {
        match self {
            Self::Logs(_) => IconName::FileText,
            Self::Shell(_) => IconName::SquareTerminal,
            Self::Drain(_) => IconName::ArrowDown,
        }
    }

    fn view(&self) -> AnyElement {
        match self {
            Self::Logs(tab) => tab.clone().into_any_element(),
            Self::Shell(tab) => tab.clone().into_any_element(),
            Self::Drain(tab) => tab.clone().into_any_element(),
        }
    }
}

/// A log tab that moved to a window of its own. The window owns the tab; the dock keeps a weak
/// handle, so closing the window ends the stream with no bookkeeping here.
struct PoppedTab {
    tab: WeakEntity<LogTab>,
    window: AnyWindowHandle,
}

pub(crate) struct Dock {
    tabs: Vec<DockTab>,
    /// Pop-outs of the current cluster; closed windows are dropped before each use.
    popped: Vec<PoppedTab>,
    /// `None` exactly when `tabs` is empty.
    active: Option<usize>,
    mode: DockMode,
    /// The "+ ▾" menu reads the selection and opens tabs through the shell.
    shell: WeakEntity<AppShell>,
}

impl Dock {
    pub(crate) fn new(shell: WeakEntity<AppShell>) -> Self {
        Self {
            tabs: Vec::new(),
            popped: Vec::new(),
            active: None,
            mode: DockMode::Normal,
            shell,
        }
    }

    /// Activates the target's tab of that cluster if one exists, else adds one. Minimized becomes
    /// Normal. Nothing opens once the session of the origin is gone.
    pub(crate) fn open(
        &mut self,
        origin: LogOrigin,
        target: LogTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = origin.session.upgrade() else {
            return;
        };
        if self.activate_popped(&origin, &target, cx) {
            return;
        }
        let existing = self.tabs.iter().position(|tab| {
            matches!(tab, DockTab::Logs(tab) if tab.read(cx).is_for(&origin.cluster, &target))
        });
        let index = match existing {
            Some(index) => {
                // A menu reopen must not undo the container the user picked in the tab, so only
                // an explicit container switches it.
                if let LogTarget::Pod(pod) = &target
                    && pod.choice == ContainerChoice::Explicit
                    && let DockTab::Logs(tab) = &self.tabs[index]
                {
                    let container = pod.initial_container.clone();
                    tab.update(cx, |tab, cx| tab.pick_container(container, cx));
                }
                index
            }
            None => {
                let tab = cx.new(|cx| LogTab::new(origin, target, &session, window, cx));
                cx.subscribe_in(&tab, window, Self::on_log_tab_event)
                    .detach();
                let layout = self.layout();
                tab.update(cx, |tab, cx| tab.set_layout(layout, cx));
                self.tabs.push(DockTab::Logs(tab));
                self.tabs.len() - 1
            }
        };
        self.active = Some(index);
        if self.mode == DockMode::Minimized {
            self.mode = DockMode::Normal;
        }
        cx.notify();
    }

    /// A pop-out already shows `target` of that cluster: its window is brought forward, and an
    /// explicit container pick applies as it does to a dock tab. No tab, no stream.
    fn activate_popped(
        &mut self,
        origin: &LogOrigin,
        target: &LogTarget,
        cx: &mut Context<Self>,
    ) -> bool {
        self.popped.retain(|popped| popped.tab.upgrade().is_some());
        let shown = self.popped.iter().find_map(|popped| {
            let tab = popped.tab.upgrade()?;
            tab.read(cx)
                .is_for(&origin.cluster, target)
                .then_some((tab, popped.window))
        });
        let Some((tab, window)) = shown else {
            return false;
        };
        if let LogTarget::Pod(pod) = target
            && pod.choice == ContainerChoice::Explicit
        {
            let container = pod.initial_container.clone();
            tab.update(cx, |tab, cx| tab.pick_container(container, cx));
        }
        let _ = window.update(cx, |_, window, _| window.activate_window());
        true
    }

    /// Moves a log tab out of the dock into a window of its own; the same entity keeps its
    /// stream. The dock keeps a weak handle (decision 19). If the window cannot open, the tab goes
    /// back where it was and a notice says so.
    fn pop_out(&mut self, tab: &Entity<LogTab>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self
            .tabs
            .iter()
            .position(|open| matches!(open, DockTab::Logs(logs) if logs == tab))
        else {
            return;
        };
        let (active, mode) = (self.active, self.mode);
        let Some(taken) = self.take_tab(index, cx) else {
            return;
        };
        let title = tab.read(cx).label();
        match open_log_window(tab.clone(), title, cx) {
            Some(handle) => self.popped.push(PoppedTab {
                tab: tab.downgrade(),
                window: handle,
            }),
            None => {
                self.tabs.insert(index, taken);
                self.active = active;
                self.mode = mode;
                window.push_notification(Notification::warning("Could not open a new window"), cx);
                cx.notify();
            }
        }
    }

    fn on_log_tab_event(
        &mut self,
        tab: &Entity<LogTab>,
        event: &LogTabEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            LogTabEvent::PopOut => self.pop_out(tab, window, cx),
        }
    }

    /// `--screen logs-popout`: the active log tab moves to its window.
    #[cfg(feature = "screenshot")]
    pub(crate) fn pop_out_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(DockTab::Logs(tab)) = self.active_tab() {
            let tab = tab.clone();
            self.pop_out(&tab, window, cx);
        }
    }

    /// The window of the first live pop-out.
    #[cfg(any(feature = "screenshot", test))]
    pub(crate) fn popped_window(&self) -> Option<AnyWindowHandle> {
        self.popped
            .iter()
            .find(|popped| popped.tab.upgrade().is_some())
            .map(|popped| popped.window)
    }

    /// Opens a shell tab on `connection`, the connection of the target's own cluster, with the
    /// proof that both exec verbs are allowed. A new tab every time (two shells into one container
    /// are normal), activated; Minimized becomes Normal. `None`, with a notice, past the cap.
    pub(crate) fn open_shell(
        &mut self,
        target: ShellTarget,
        cluster_label: String,
        grant: ShellGrant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<ShellTab>> {
        if !self.has_room_for_shell(cx) {
            window.push_notification(Notification::warning(shell_cap_text()), cx);
            return None;
        }
        let app = self.shell.clone();
        let tab = cx.new(|cx| ShellTab::new(target, cluster_label, app, window, cx));
        tab.update(cx, |tab, cx| tab.connect(grant, cx));
        self.tabs.push(DockTab::Shell(tab.clone()));
        self.activate(self.tabs.len() - 1, cx);
        Some(tab)
    }

    /// Opens the tab of a debug start (a debug container or a node shell pod) and attaches it, on
    /// the connection of the target's own cluster, with the proof that both attach verbs are
    /// allowed. Like `open_shell` it adds a new tab each time, counts toward the cap, and answers
    /// `None` past it.
    pub(crate) fn open_attach(
        &mut self,
        target: ShellTarget,
        kind: ShellKind,
        cluster_label: String,
        grant: AttachGrant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<ShellTab>> {
        if !self.has_room_for_shell(cx) {
            window.push_notification(Notification::warning(shell_cap_text()), cx);
            return None;
        }
        let app = self.shell.clone();
        let tab =
            cx.new(|cx| ShellTab::new(target, cluster_label, app, window, cx).with_kind(kind));
        tab.update(cx, |tab, cx| tab.connect_attach(grant, cx));
        self.tabs.push(DockTab::Shell(tab.clone()));
        self.activate(self.tabs.len() - 1, cx);
        Some(tab)
    }

    /// `--screen shell-fixture`: a shell tab that never connects, fed `transcript`.
    #[cfg(feature = "screenshot")]
    pub(crate) fn open_shell_fixture(
        &mut self,
        fixture: crate::screenshot::ShellTabFixture,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ShellTab> {
        let app = self.shell.clone();
        let tab = cx.new(|cx| {
            ShellTab::new(fixture.target, fixture.cluster_label, app, window, cx)
                .with_kind(fixture.kind)
        });
        tab.update(cx, |tab, cx| tab.show_fixture(fixture.transcript, cx));
        self.tabs.push(DockTab::Shell(tab.clone()));
        self.activate(self.tabs.len() - 1, cx);
        tab
    }

    /// Adds the tab of a drain and activates it; Minimized becomes Normal.
    pub(crate) fn open_drain(&mut self, tab: Entity<DrainTab>, cx: &mut Context<Self>) {
        self.tabs.push(DockTab::Drain(tab));
        self.activate(self.tabs.len() - 1, cx);
    }

    /// The drain tabs, in tab order.
    pub(crate) fn drain_tabs(&self) -> impl Iterator<Item = &Entity<DrainTab>> {
        self.tabs.iter().filter_map(|tab| match tab {
            DockTab::Drain(tab) => Some(tab),
            DockTab::Logs(_) | DockTab::Shell(_) => None,
        })
    }

    /// Every running drain, whatever its cluster, each with the cluster's display name: what the
    /// quit stops.
    pub(crate) fn running_drains(&self, cx: &App) -> Vec<(Entity<DrainTab>, SharedString)> {
        self.drain_tabs()
            .filter_map(|tab| {
                let state = tab.read(cx);
                state
                    .is_running()
                    .then(|| (tab.clone(), state.cluster_name().clone()))
            })
            .collect()
    }

    /// The running drains of `clusters`: what leaving them would stop.
    pub(crate) fn running_drains_of(
        &self,
        clusters: &[ClusterRef],
        cx: &App,
    ) -> Vec<(Entity<DrainTab>, SharedString)> {
        let mut running = self.running_drains(cx);
        running.retain(|(tab, _)| clusters.contains(tab.read(cx).cluster()));
        running
    }

    /// Whether the dock holds a running drain of `cluster`: one per cluster at a time.
    pub(crate) fn has_running_drain(&self, cluster: &ClusterRef, cx: &App) -> bool {
        !self
            .running_drains_of(std::slice::from_ref(cluster), cx)
            .is_empty()
    }

    /// Removes the tab of a finished drain. A running one stays.
    pub(crate) fn close_drain(&mut self, tab: &Entity<DrainTab>, cx: &mut Context<Self>) {
        let index = self
            .tabs
            .iter()
            .position(|open| matches!(open, DockTab::Drain(drain) if drain == tab));
        if let Some(index) = index {
            self.close_tab(index, cx);
        }
    }

    /// Whether another shell tab fits under the cap.
    pub(crate) fn has_room_for_shell(&self, cx: &App) -> bool {
        self.shell_tabs(cx).count() < MAX_SHELL_TABS
    }

    /// The shell tabs, for the checks that count them.
    fn shell_tabs<'a>(&'a self, _: &'a App) -> impl Iterator<Item = &'a Entity<ShellTab>> {
        self.tabs.iter().filter_map(|tab| match tab {
            DockTab::Shell(tab) => Some(tab),
            DockTab::Logs(_) | DockTab::Drain(_) => None,
        })
    }

    /// How many open shell tabs belong to `clusters`: what releasing them would end. A node shell
    /// counts apart (`node_shell_count_of`), because closing it also deletes its pod.
    pub(crate) fn shell_count_of(&self, clusters: &[ClusterRef], cx: &App) -> usize {
        self.shell_tabs(cx)
            .filter(|tab| {
                let tab = tab.read(cx);
                clusters.contains(tab.cluster()) && !tab.kind().is_node_shell()
            })
            .count()
    }

    /// How many open node shells belong to `clusters`.
    pub(crate) fn node_shell_count_of(&self, clusters: &[ClusterRef], cx: &App) -> usize {
        self.shell_tabs(cx)
            .filter(|tab| {
                let tab = tab.read(cx);
                clusters.contains(tab.cluster()) && tab.kind().is_node_shell()
            })
            .count()
    }

    /// The user closes a tab: a running drain refuses (`Cancel the drain first`).
    pub(crate) fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.tabs.get(index).is_some_and(|tab| tab.is_pinned(cx)) {
            return;
        }
        self.take_tab(index, cx);
    }

    /// Takes the tab out whatever it is doing, and moves the active index and the mode as a
    /// close does.
    fn take_tab(&mut self, index: usize, cx: &mut Context<Self>) -> Option<DockTab> {
        if index >= self.tabs.len() {
            return None;
        }
        let taken = self.tabs.remove(index);
        let remaining = self.tabs.len();
        self.active = self
            .active
            .and_then(|active| active_after_close(active, index, remaining));
        if remaining == 0 {
            self.mode = DockMode::Normal;
        }
        cx.notify();
        Some(taken)
    }

    /// A context switch: every stream belongs to the old connection, in the dock and in the
    /// pop-out windows alike.
    pub(crate) fn close_all(&mut self, cx: &mut Context<Self>) {
        for popped in self.popped.drain(..) {
            let _ = popped
                .window
                .update(cx, |_, window, _| window.remove_window());
        }
        self.tabs.clear();
        self.active = None;
        self.mode = DockMode::Normal;
        cx.notify();
    }

    /// A navigation click returns the dock to its split; the tabs stay.
    pub(crate) fn unzoom(&mut self, cx: &mut Context<Self>) {
        if self.mode == DockMode::Zoomed {
            self.set_mode(DockMode::Normal, cx);
        }
    }

    pub(crate) fn set_mode(&mut self, mode: DockMode, cx: &mut Context<Self>) {
        self.mode = mode;
        let layout = self.layout();
        for tab in &self.tabs {
            if let DockTab::Logs(tab) = tab {
                tab.update(cx, |tab, cx| tab.set_layout(layout, cx));
            }
        }
        cx.notify();
    }

    /// The zoomed dock has room for the legend and the histogram.
    fn layout(&self) -> LogLayout {
        match self.mode {
            DockMode::Zoomed => LogLayout::Full,
            DockMode::Normal | DockMode::Minimized => LogLayout::Compact,
        }
    }

    /// Moves a tab by drag and drop; the active tab stays active.
    fn move_tab(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        self.active = move_tab(&mut self.tabs, from, to, self.active);
        cx.notify();
    }

    pub(crate) fn mode(&self) -> DockMode {
        self.mode
    }

    pub(crate) fn has_tabs(&self) -> bool {
        !self.tabs.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// The open shell tabs, in tab order.
    #[cfg(test)]
    pub(crate) fn shell_tab_entities(&self) -> Vec<Entity<ShellTab>> {
        self.tabs
            .iter()
            .filter_map(|tab| match tab {
                DockTab::Shell(tab) => Some(tab.clone()),
                DockTab::Logs(_) | DockTab::Drain(_) => None,
            })
            .collect()
    }

    /// The open log tabs, in tab order.
    #[cfg(test)]
    pub(crate) fn log_tab_entities(&self) -> Vec<Entity<LogTab>> {
        self.tabs
            .iter()
            .filter_map(|tab| match tab {
                DockTab::Logs(tab) => Some(tab.clone()),
                DockTab::Shell(_) | DockTab::Drain(_) => None,
            })
            .collect()
    }

    /// The labels of the log tabs, in tab order.
    #[cfg(test)]
    pub(crate) fn log_tab_labels(&self, cx: &gpui_kit::App) -> Vec<String> {
        self.tabs
            .iter()
            .filter_map(|tab| match tab {
                DockTab::Logs(tab) => Some(tab.read(cx).label()),
                DockTab::Shell(_) | DockTab::Drain(_) => None,
            })
            .collect()
    }

    /// Whether the active tab is still waiting for its stream to open.
    #[cfg(feature = "screenshot")]
    pub(crate) fn is_connecting(&self, cx: &gpui_kit::App) -> bool {
        match self.active_tab() {
            Some(DockTab::Logs(tab)) => tab.read(cx).is_connecting(),
            Some(DockTab::Shell(tab)) => {
                *tab.read(cx).state() == crate::shell_tab::ShellState::Connecting
            }
            Some(DockTab::Drain(_)) | None => false,
        }
    }

    fn active_tab(&self) -> Option<&DockTab> {
        self.tabs.get(self.active?)
    }

    fn activate(&mut self, index: usize, cx: &mut Context<Self>) {
        self.active = Some(index);
        if self.mode == DockMode::Minimized {
            self.mode = DockMode::Normal;
        }
        cx.notify();
    }

    /// Zooms the dock in, or back to its split. Nothing happens without tabs: the dock is not drawn.
    pub(crate) fn toggle_zoom(&mut self, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        self.set_mode(toggled_zoom(self.mode), cx);
    }

    /// Minimizes the dock to its tab bar, or restores it. Nothing happens without tabs.
    pub(crate) fn toggle_visibility(&mut self, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        self.set_mode(toggled_visibility(self.mode), cx);
    }

    /// Activates the next or previous tab, wrapping. A minimized dock is restored, like a click.
    pub(crate) fn step_active_tab(&mut self, step: TabStep, cx: &mut Context<Self>) {
        let Some(active) = self.active else {
            return;
        };
        self.activate(step_tab(active, self.tabs.len(), step), cx);
    }

    /// Closes the active tab.
    pub(crate) fn close_active_tab(&mut self, cx: &mut Context<Self>) {
        if let Some(active) = self.active {
            self.close_tab(active, cx);
        }
    }

    fn render_tab_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let is_zoomed = self.mode == DockMode::Zoomed;
        let is_minimized = self.mode == DockMode::Minimized;
        h_flex()
            .flex_shrink_0()
            .h(TAB_BAR_HEIGHT)
            .items_center()
            .bg(theme.muted)
            .border_b_1()
            .border_color(theme.border)
            .children(
                self.tabs
                    .iter()
                    .enumerate()
                    .map(|(index, tab)| self.render_tab(index, tab, cx)),
            )
            .child(self.render_new_tab_button())
            .child(div().flex_1())
            .child(
                Button::new("log-dock-zoom")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(if is_zoomed {
                        IconName::Minimize2
                    } else {
                        IconName::Maximize2
                    }))
                    .tooltip(if is_zoomed { "Zoom out" } else { "Zoom in" })
                    .on_click(cx.listener(|dock, _, _, cx| dock.toggle_zoom(cx))),
            )
            .child(
                Button::new("log-dock-minimize")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(if is_minimized {
                        IconName::ChevronUp
                    } else {
                        IconName::ChevronDown
                    }))
                    .tooltip(if is_minimized { "Restore" } else { "Minimize" })
                    .on_click(cx.listener(|dock, _, _, cx| dock.toggle_visibility(cx))),
            )
            .child(div().w_1())
    }

    /// "+ ▾": a new tab for the selection, a log view or a shell.
    fn render_new_tab_button(&self) -> impl IntoElement {
        let shell = self.shell.clone();
        Button::new("log-dock-new")
            .ghost()
            .xsmall()
            .child(
                h_flex()
                    .items_center()
                    .gap_0p5()
                    .child(Icon::new(IconName::Plus).size_3())
                    .child(Icon::new(IconName::ChevronDown).size_3()),
            )
            .tooltip("New tab")
            .dropdown_menu(move |menu, _, cx| {
                // Built when the menu opens (popover render) and cached until it is dismissed;
                // not inside an AppShell update.
                let entity = shell.upgrade();
                let target = match &entity {
                    Some(entity) => entity.read(cx).selected_log_target(cx),
                    None => Err(NoLogTarget::NotConnected),
                };
                let shell_reason = match &entity {
                    Some(entity) => entity.read(cx).selected_shell_reason(cx),
                    None => Some("Not connected".into()),
                };
                let logs_item = match target {
                    Ok(_) => {
                        let shell = shell.clone();
                        PopupMenuItem::new("Logs of selected").on_click(move |_, window, cx| {
                            let _ = shell.update(cx, |shell, cx| {
                                shell.open_logs_of_selection(window, cx);
                            });
                        })
                    }
                    Err(reason) => {
                        disabled_menu_item("Logs of selected", reason.to_string().into())
                    }
                };
                // The same arm as the S key and the palette: the cursor row, in its own cluster.
                let shell_item = match shell_reason {
                    None => {
                        let shell = shell.clone();
                        PopupMenuItem::new("Shell into selected").on_click(move |_, window, cx| {
                            let _ = shell.update(cx, |shell, cx| {
                                shell.run_row_key(RowAction::OpenShell, window, cx);
                            });
                        })
                    }
                    Some(reason) => disabled_menu_item("Shell into selected", reason),
                };
                menu.item(logs_item).item(shell_item)
            })
    }

    fn render_tab(&self, index: usize, tab: &DockTab, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let title = tab.title(cx);
        let is_active = self.active == Some(index);
        h_flex()
            .h_full()
            .items_center()
            .gap_1()
            .pl_3()
            .pr_1()
            .border_r_1()
            .border_color(theme.border)
            .when(is_active, |this| this.bg(theme.background))
            .id(("log-tab-slot", index))
            .on_drag(
                DraggedTab {
                    index,
                    label: title.clone().into(),
                },
                |dragged, _, _, cx| cx.new(|_| dragged.clone()),
            )
            .drag_over::<DraggedTab>(|style, _, _, cx| style.bg(cx.theme().accent))
            .on_drop(cx.listener(move |dock, dragged: &DraggedTab, _, cx| {
                dock.move_tab(dragged.index, index, cx);
            }))
            .child(
                h_flex()
                    .id(("log-tab", index))
                    .items_center()
                    .gap_1p5()
                    .min_w_0()
                    .cursor_pointer()
                    .on_click(cx.listener(move |dock, _, _, cx| dock.activate(index, cx)))
                    .child(Icon::new(tab.icon()).size_3())
                    .child(
                        div()
                            .flex_shrink_0()
                            .size_2()
                            .rounded_full()
                            .bg(tone_color(tab.tone(cx), cx)),
                    )
                    .child(
                        div()
                            .max_w(TAB_LABEL_MAX_WIDTH)
                            .truncate()
                            .font_family(theme.mono_font_family.clone())
                            .text_xs()
                            .when(!is_active, |this| this.text_color(theme.muted_foreground))
                            .child(title),
                    ),
            )
            .child(
                Button::new(("log-tab-close", index))
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::X))
                    .disabled(tab.is_pinned(cx))
                    .tooltip(if tab.is_pinned(cx) {
                        "Cancel the drain first"
                    } else {
                        "Close"
                    })
                    .on_click(cx.listener(move |dock, _, _, cx| dock.close_tab(index, cx))),
            )
    }
}

impl Render for Dock {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let is_minimized = self.mode == DockMode::Minimized;
        let body = self.active_tab().filter(|_| !is_minimized);
        v_flex()
            .key_context("Dock")
            .flex_shrink_0()
            .w_full()
            // Minimized leaves just the tab bar, so the dock keeps its natural height.
            .when(!is_minimized, |this| this.h_full())
            .bg(theme.background)
            .border_t_1()
            .border_color(theme.border)
            .when(self.mode == DockMode::Normal, |this| this.shadow_md())
            .child(self.render_tab_bar(cx))
            .children(body.map(|tab| div().flex_1().min_h_0().child(tab.view())))
    }
}

/// The payload of a tab drag, and the chip that follows the pointer.
#[derive(Clone)]
struct DraggedTab {
    index: usize,
    label: SharedString,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px_2()
            .py_1()
            .rounded_sm()
            .bg(theme.muted)
            .border_1()
            .border_color(theme.border)
            .font_family(theme.mono_font_family.clone())
            .text_xs()
            .child(self.label.clone())
    }
}

/// The notice of a ninth shell tab.
pub(crate) fn shell_cap_text() -> String {
    format!("Close a shell tab first ({MAX_SHELL_TABS} open)")
}

/// The mode after the minimize key: a visible dock (split or zoomed) minimizes, a minimized one
/// returns to its split.
pub(crate) fn toggled_visibility(mode: DockMode) -> DockMode {
    match mode {
        DockMode::Minimized => DockMode::Normal,
        DockMode::Normal | DockMode::Zoomed => DockMode::Minimized,
    }
}

/// The mode after the zoom key: a zoomed dock returns to its split, any other zooms.
pub(crate) fn toggled_zoom(mode: DockMode) -> DockMode {
    match mode {
        DockMode::Zoomed => DockMode::Normal,
        DockMode::Normal | DockMode::Minimized => DockMode::Zoomed,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabStep {
    Next,
    Previous,
}

/// The tab a tab key activates, wrapping at both ends. `len` is at least 1.
pub(crate) fn step_tab(active: usize, len: usize, step: TabStep) -> usize {
    match step {
        TabStep::Next => (active + 1) % len,
        TabStep::Previous => (active + len - 1) % len,
    }
}

/// Moves `from` to `to`; returns the active index so the same tab stays active. An equal or
/// out-of-range index changes nothing.
fn move_tab<T>(tabs: &mut Vec<T>, from: usize, to: usize, active: Option<usize>) -> Option<usize> {
    if from == to || from >= tabs.len() || to >= tabs.len() {
        return active;
    }
    let tab = tabs.remove(from);
    tabs.insert(to, tab);
    active.map(|active| {
        if active == from {
            to
        } else if from < active && active <= to {
            active - 1
        } else if to <= active && active < from {
            active + 1
        } else {
            active
        }
    })
}

/// The new active index after closing `closed`: the right neighbor, else the left; `None`
/// when no tab is left.
fn active_after_close(active: usize, closed: usize, remaining: usize) -> Option<usize> {
    if remaining == 0 {
        return None;
    }
    if closed < active {
        return Some(active - 1);
    }
    if closed > active {
        return Some(active);
    }
    // The right neighbor slides into the closed index; the last tab falls back to the left.
    Some(closed.min(remaining - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_max_height_is_sixty_percent_of_workspace() {
        let max = dock_max_height(px(1000.));
        assert!((max - px(600.)).abs() < px(0.01), "{max:?}");
    }

    #[test]
    fn dock_max_height_never_below_minimum() {
        assert_eq!(dock_max_height(px(150.)), MIN_DOCK_HEIGHT);
    }

    #[test]
    fn initial_dock_height_uses_a_valid_saved_height() {
        let viewport = px(1000.);
        assert_eq!(initial_dock_height(None, viewport), px(280.));
        assert_eq!(initial_dock_height(Some(400.), viewport), px(400.));
        assert_eq!(initial_dock_height(Some(50.), viewport), px(120.));
        for invalid in [0., -1., f32::INFINITY, f32::NAN] {
            assert_eq!(initial_dock_height(Some(invalid), viewport), px(280.));
        }
        assert_eq!(initial_dock_height(Some(900.), viewport), px(600.));
        assert_eq!(initial_dock_height(Some(400.), px(300.)), px(180.));
    }

    #[test]
    fn saved_dock_height_rounds_and_forgets_the_default() {
        assert_eq!(saved_dock_height(px(401.6)), Some(402.));
        assert_eq!(saved_dock_height(px(280.)), None);
    }

    #[test]
    fn max_line_offset_is_the_room_left_to_sixty_percent() {
        assert_eq!(max_line_offset(px(1000.), px(280.)), Some(px(320.)));
        assert_eq!(max_line_offset(px(1000.), px(600.)), Some(px(0.)));
        assert_eq!(max_line_offset(px(0.), px(280.)), None);
    }

    #[test]
    fn dock_max_height_unbounded_before_first_layout() {
        assert_eq!(dock_max_height(px(0.)), Pixels::MAX);
    }

    #[test]
    fn closing_active_tab_selects_right_then_left_neighbor() {
        // Tabs [a, b, c] with b active: c slides into index 1.
        assert_eq!(active_after_close(1, 1, 2), Some(1));
        // c active and last: the left neighbor b becomes index 1.
        assert_eq!(active_after_close(2, 2, 2), Some(1));
    }

    #[test]
    fn closing_tab_before_active_shifts_active() {
        assert_eq!(active_after_close(2, 0, 2), Some(1));
    }

    #[test]
    fn closing_tab_after_active_keeps_active() {
        assert_eq!(active_after_close(0, 2, 2), Some(0));
    }

    #[test]
    fn move_tab_keeps_active_tab_active() {
        // [a, b, c, d] with the active tab at 1 (b).
        let mut tabs = vec!['a', 'b', 'c', 'd'];
        assert_eq!(move_tab(&mut tabs, 1, 3, Some(1)), Some(3));
        assert_eq!(tabs, ['a', 'c', 'd', 'b']);
        // Another tab passes over the active one, forward and backward.
        let mut tabs = vec!['a', 'b', 'c', 'd'];
        assert_eq!(move_tab(&mut tabs, 0, 2, Some(1)), Some(0));
        assert_eq!(tabs, ['b', 'c', 'a', 'd']);
        let mut tabs = vec!['a', 'b', 'c', 'd'];
        assert_eq!(move_tab(&mut tabs, 3, 0, Some(1)), Some(2));
        assert_eq!(tabs, ['d', 'a', 'b', 'c']);
        // A move that does not cross the active tab leaves its index.
        let mut tabs = vec!['a', 'b', 'c', 'd'];
        assert_eq!(move_tab(&mut tabs, 2, 3, Some(0)), Some(0));
    }

    #[test]
    fn move_tab_ignores_same_or_out_of_range_index() {
        let mut tabs = vec!['a', 'b'];
        assert_eq!(move_tab(&mut tabs, 1, 1, Some(0)), Some(0));
        assert_eq!(move_tab(&mut tabs, 0, 5, Some(1)), Some(1));
        assert_eq!(move_tab(&mut tabs, 5, 0, None), None);
        assert_eq!(tabs, ['a', 'b']);
    }
    #[test]
    fn closing_last_tab_leaves_no_active() {
        assert_eq!(active_after_close(0, 0, 0), None);
    }

    #[test]
    fn toggled_visibility_minimizes_then_restores() {
        assert_eq!(toggled_visibility(DockMode::Normal), DockMode::Minimized);
        assert_eq!(toggled_visibility(DockMode::Minimized), DockMode::Normal);
        assert_eq!(toggled_visibility(DockMode::Zoomed), DockMode::Minimized);
    }

    #[test]
    fn toggled_zoom_zooms_then_restores() {
        assert_eq!(toggled_zoom(DockMode::Normal), DockMode::Zoomed);
        assert_eq!(toggled_zoom(DockMode::Minimized), DockMode::Zoomed);
        assert_eq!(toggled_zoom(DockMode::Zoomed), DockMode::Normal);
    }

    #[test]
    fn step_tab_wraps_both_ways() {
        assert_eq!(step_tab(0, 3, TabStep::Previous), 2);
        assert_eq!(step_tab(2, 3, TabStep::Next), 0);
        assert_eq!(step_tab(1, 3, TabStep::Next), 2);
        assert_eq!(step_tab(0, 1, TabStep::Next), 0);
    }
}

#[cfg(test)]
#[path = "dock_tests.rs"]
mod dock_tests;
