//! The bottom dock that holds the log tabs: tab bar, zoom, and minimize.

use cluster::ClusterConnection;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, Render, SharedString, StatefulInteractiveElement as _, Styled as _, WeakEntity, Window,
    div, prelude::FluentBuilder as _, px,
};

use crate::app_shell::AppShell;
use crate::cluster_session::ClusterSession;
use crate::log_tab::{LogLayout, LogTab};
use crate::log_target::{ContainerChoice, LogTarget, NoLogTarget};
use crate::resource_actions::{READ_ONLY_MODE_REASON, disabled_menu_item};
use crate::status_tone::tone_color;

pub(crate) const DEFAULT_DOCK_HEIGHT: Pixels = px(280.);
pub(crate) const MIN_DOCK_HEIGHT: Pixels = px(120.);
const MAX_DOCK_FRACTION: f32 = 0.6;
const TAB_BAR_HEIGHT: Pixels = px(34.);
const TAB_LABEL_MAX_WIDTH: Pixels = px(260.);

/// 60% of the measured workspace, never below the minimum (so the range stays valid);
/// `Pixels::MAX` before the first layout, when the container is still zero.
pub(crate) fn dock_max_height(container: Pixels) -> Pixels {
    if container <= px(0.) {
        return Pixels::MAX;
    }
    (container * MAX_DOCK_FRACTION).max(MIN_DOCK_HEIGHT)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DockMode {
    Normal,
    Minimized,
    Zoomed,
}

pub(crate) struct LogDock {
    tabs: Vec<Entity<LogTab>>,
    /// `None` exactly when `tabs` is empty.
    active: Option<usize>,
    mode: DockMode,
    /// Weak: workload tabs observe the session, but a tab never keeps it alive.
    session: Option<WeakEntity<ClusterSession>>,
    /// The "+ ▾" menu reads the selection and opens tabs through the shell.
    shell: WeakEntity<AppShell>,
}

impl LogDock {
    pub(crate) fn new(shell: WeakEntity<AppShell>) -> Self {
        Self {
            tabs: Vec::new(),
            active: None,
            mode: DockMode::Normal,
            session: None,
            shell,
        }
    }

    pub(crate) fn set_session(&mut self, session: Option<WeakEntity<ClusterSession>>) {
        self.session = session;
    }

    /// Activates the target's tab if one exists, else adds one. Minimized becomes Normal.
    /// Nothing opens without a session.
    pub(crate) fn open(
        &mut self,
        connection: ClusterConnection,
        target: LogTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session.as_ref().and_then(WeakEntity::upgrade) else {
            return;
        };
        let existing = self
            .tabs
            .iter()
            .position(|tab| tab.read(cx).is_for(&target));
        let index = match existing {
            Some(index) => {
                // A menu reopen must not undo the container the user picked in the tab, so only
                // an explicit container switches it.
                if let LogTarget::Pod(pod) = &target
                    && pod.choice == ContainerChoice::Explicit
                {
                    let container = pod.initial_container.clone();
                    self.tabs[index].update(cx, |tab, cx| tab.pick_container(container, cx));
                }
                index
            }
            None => {
                let tab = cx.new(|cx| LogTab::new(connection, target, &session, window, cx));
                let layout = self.layout();
                tab.update(cx, |tab, cx| tab.set_layout(layout, cx));
                self.tabs.push(tab);
                self.tabs.len() - 1
            }
        };
        self.active = Some(index);
        if self.mode == DockMode::Minimized {
            self.mode = DockMode::Normal;
        }
        cx.notify();
    }

    pub(crate) fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        let remaining = self.tabs.len();
        self.active = self
            .active
            .and_then(|active| active_after_close(active, index, remaining));
        if remaining == 0 {
            self.mode = DockMode::Normal;
        }
        cx.notify();
    }

    /// A context switch: every stream belongs to the old connection.
    pub(crate) fn close_all(&mut self, cx: &mut Context<Self>) {
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
            tab.update(cx, |tab, cx| tab.set_layout(layout, cx));
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

    /// Whether the active tab is still waiting for its stream to open.
    #[cfg(feature = "screenshot")]
    pub(crate) fn is_connecting(&self, cx: &gpui_kit::App) -> bool {
        self.active_tab()
            .is_some_and(|tab| tab.read(cx).is_connecting())
    }

    fn active_tab(&self) -> Option<&Entity<LogTab>> {
        self.tabs.get(self.active?)
    }

    fn activate(&mut self, index: usize, cx: &mut Context<Self>) {
        self.active = Some(index);
        if self.mode == DockMode::Minimized {
            self.mode = DockMode::Normal;
        }
        cx.notify();
    }

    fn toggle_zoom(&mut self, cx: &mut Context<Self>) {
        let mode = match self.mode {
            DockMode::Zoomed => DockMode::Normal,
            DockMode::Normal | DockMode::Minimized => DockMode::Zoomed,
        };
        self.set_mode(mode, cx);
    }

    fn toggle_minimize(&mut self, cx: &mut Context<Self>) {
        let mode = match self.mode {
            DockMode::Minimized => DockMode::Normal,
            DockMode::Normal | DockMode::Zoomed => DockMode::Minimized,
        };
        self.set_mode(mode, cx);
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
                    .map(|(index, entity)| self.render_tab(index, entity, cx)),
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
                    .on_click(cx.listener(|dock, _, _, cx| dock.toggle_minimize(cx))),
            )
            .child(div().w_1())
    }

    /// "+ ▾": a new tab for the selection. The shell slot is visible but never enabled in this
    /// version, so there is no placeholder tab.
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
                    Some(entity) => entity.read(cx).open_shell_unavailable_reason(cx),
                    None => READ_ONLY_MODE_REASON.into(),
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
                menu.item(logs_item)
                    .item(disabled_menu_item("Shell into selected", shell_reason))
            })
    }

    fn render_tab(
        &self,
        index: usize,
        entity: &Entity<LogTab>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let tab = entity.read(cx);
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
                    label: tab.label().into(),
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
                    .child(Icon::new(IconName::FileText).size_3())
                    .child(
                        div()
                            .flex_shrink_0()
                            .size_2()
                            .rounded_full()
                            .bg(tone_color(tab.tone(), cx)),
                    )
                    .child(
                        div()
                            .max_w(TAB_LABEL_MAX_WIDTH)
                            .truncate()
                            .font_family(theme.mono_font_family.clone())
                            .text_xs()
                            .when(!is_active, |this| this.text_color(theme.muted_foreground))
                            .child(tab.label()),
                    ),
            )
            .child(
                Button::new(("log-tab-close", index))
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::X))
                    .tooltip("Close")
                    .on_click(cx.listener(move |dock, _, _, cx| dock.close_tab(index, cx))),
            )
    }
}

impl Render for LogDock {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let is_minimized = self.mode == DockMode::Minimized;
        let body = self.active_tab().filter(|_| !is_minimized);
        v_flex()
            .flex_shrink_0()
            .w_full()
            // Minimized leaves just the tab bar, so the dock keeps its natural height.
            .when(!is_minimized, |this| this.h_full())
            .bg(theme.background)
            .border_t_1()
            .border_color(theme.border)
            .when(self.mode == DockMode::Normal, |this| this.shadow_md())
            .child(self.render_tab_bar(cx))
            .children(body.map(|tab| div().flex_1().min_h_0().child(tab.clone())))
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
}
