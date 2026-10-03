//! The cluster switcher: the title-bar popover with its filter, segment, environment sections,
//! and health lines. The row model is `cluster_switcher_rows`; the probes behind the health
//! lines are `cluster_health`; the shell methods that act on a click are in `app_shell`.

use gpui_kit::Action;
use gpui_kit::assets::IconName;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    KeyBinding, ParentElement as _, Pixels, SharedString, StatefulInteractiveElement as _,
    Styled as _, WeakEntity, Window, div, prelude::FluentBuilder as _, px,
};

use crate::app_shell::AppShell;
use crate::cluster_health::{HealthBoard, RowHealth};
use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::WatchSubscription;
use crate::cluster_session::latency_millis;
use crate::cluster_switcher_rows::{
    HighlightStep, SwitcherRow, SwitcherSection, SwitcherSegment, toggle_tick,
};
use crate::cluster_view::TooManyClusters;
use crate::environment::environment_badge;
use crate::settings_window::{ManageClusters, OpenSettings};
use crate::status_tone::{StatusTone, tone_color};

gpui_kit::actions!(
    k8sboard,
    [
        OpenClusterSwitcher,
        SwitchToCluster1,
        SwitchToCluster2,
        SwitchToCluster3,
        SwitchToCluster4,
        SwitchToCluster5,
        SwitchToCluster6,
        SwitchToCluster7,
        SwitchToCluster8,
        SwitchToCluster9,
        SwitcherNext,
        SwitcherPrevious,
        SwitcherConfirm,
        CloseClusterSwitcher,
        ToggleClusterTick,
    ]
);

const SWITCHER_WIDTH: Pixels = px(380.);
const MAX_LIST_HEIGHT: Pixels = px(420.);
/// The key context of the popover content. The kit `Popover` binds Escape, Enter and Space one
/// level above it, so these bindings win while focus is inside.
const SWITCHER_CONTEXT: &str = "ClusterSwitcher";
/// The context the `Ctrl n` hints are looked up in: the shell root holds the chords.
const SHELL_CONTEXT: &str = "AppShell";

/// The popover's own keys: the arrows move the highlight, Enter switches or applies the ticks,
/// Space ticks the highlighted row, Escape closes. They are bound for a focused row and for the
/// filter input. Both contexts are deeper than the kit `Popover`, which binds Escape, Enter, and
/// Space (`space` is `Confirm`, which would close the popover), so depth decides; they are also
/// registered after the kit's bindings, so they win over the `Input` keys at equal depth.
pub(crate) fn bind_keys(cx: &mut App) {
    let contexts = [SWITCHER_CONTEXT, "ClusterSwitcher > Input"];
    cx.bind_keys(contexts.into_iter().flat_map(|context| {
        [
            KeyBinding::new("down", SwitcherNext, Some(context)),
            KeyBinding::new("up", SwitcherPrevious, Some(context)),
            KeyBinding::new("enter", SwitcherConfirm, Some(context)),
            KeyBinding::new("space", ToggleClusterTick, Some(context)),
            KeyBinding::new("escape", CloseClusterSwitcher, Some(context)),
        ]
    }));
}

/// The action behind `Ctrl n`; there is none past nine.
pub(crate) fn switch_action(shortcut: u8) -> Option<Box<dyn Action>> {
    let action: Box<dyn Action> = match shortcut {
        1 => Box::new(SwitchToCluster1),
        2 => Box::new(SwitchToCluster2),
        3 => Box::new(SwitchToCluster3),
        4 => Box::new(SwitchToCluster4),
        5 => Box::new(SwitchToCluster5),
        6 => Box::new(SwitchToCluster6),
        7 => Box::new(SwitchToCluster7),
        8 => Box::new(SwitchToCluster8),
        9 => Box::new(SwitchToCluster9),
        _ => return None,
    };
    Some(action)
}

/// What the switcher keeps between renders.
pub(crate) struct ClusterSwitcherState {
    is_open: bool,
    /// Created once with the shell; its text is cleared on every open.
    filter: Entity<InputState>,
    segment: SwitcherSegment,
    /// The row Enter switches to; the first visible row on open and after every edit.
    highlight: Option<ClusterRef>,
    /// The draft of the next view: it starts as the viewed set, and `View {n} clusters` applies it.
    ticked: Vec<ClusterRef>,
    /// Why the last tick was refused; it stays until the next tick change.
    tick_notice: Option<SharedString>,
    health: HealthBoard,
    /// Dropped on close, which aborts the probes that still run.
    probes: Vec<WatchSubscription>,
}

impl ClusterSwitcherState {
    pub(crate) fn new(filter: Entity<InputState>) -> Self {
        Self {
            is_open: false,
            filter,
            segment: SwitcherSegment::All,
            highlight: None,
            ticked: Vec::new(),
            tick_notice: None,
            health: HealthBoard::default(),
            probes: Vec::new(),
        }
    }

    pub(crate) fn ticked(&self) -> &[ClusterRef] {
        &self.ticked
    }

    /// Replaces the draft, for example with the viewed set when the popover opens.
    pub(crate) fn set_ticked(&mut self, ticked: Vec<ClusterRef>) {
        self.ticked = ticked;
        self.tick_notice = None;
    }

    /// Ticks or unticks `cluster`; a sixth tick is refused with a notice.
    pub(crate) fn toggle_tick(&mut self, cluster: &ClusterRef) {
        self.tick_notice = match toggle_tick(&mut self.ticked, cluster) {
            Ok(()) => None,
            Err(TooManyClusters) => Some(TooManyClusters.to_string().into()),
        };
    }

    pub(crate) fn tick_notice(&self) -> Option<&SharedString> {
        self.tick_notice.as_ref()
    }

    pub(crate) fn is_open(&self) -> bool {
        self.is_open
    }

    pub(crate) fn filter(&self) -> &Entity<InputState> {
        &self.filter
    }

    pub(crate) fn segment(&self) -> SwitcherSegment {
        self.segment
    }

    pub(crate) fn set_segment(&mut self, segment: SwitcherSegment) {
        self.segment = segment;
    }

    pub(crate) fn highlight(&self) -> Option<&ClusterRef> {
        self.highlight.as_ref()
    }

    pub(crate) fn set_highlight(&mut self, highlight: Option<ClusterRef>) {
        self.highlight = highlight;
    }

    pub(crate) fn health(&self) -> &HealthBoard {
        &self.health
    }

    pub(crate) fn health_mut(&mut self) -> &mut HealthBoard {
        &mut self.health
    }

    /// Keeps `probe` running until the switcher closes.
    pub(crate) fn track_probe(&mut self, probe: WatchSubscription) {
        self.probes.push(probe);
    }

    #[cfg(test)]
    pub(crate) fn probe_count(&self) -> usize {
        self.probes.len()
    }

    pub(crate) fn open(&mut self) {
        self.is_open = true;
        self.segment = SwitcherSegment::All;
        self.highlight = None;
    }

    /// Aborts the running probes; the rows they had not answered are probed again at the next
    /// open.
    pub(crate) fn close(&mut self) {
        self.is_open = false;
        self.probes.clear();
        self.health.clear_running();
    }
}

/// Whether there is anything to list, for the empty texts.
pub(crate) enum SwitcherList {
    LoadingCatalog,
    NoClusters,
    Clusters,
}

/// Everything the popover shows, copied out of the shell so the content closure owns it.
pub(crate) struct SwitcherContent {
    pub(crate) list: SwitcherList,
    /// The rows that pass the filter and the segment.
    pub(crate) sections: Vec<SwitcherSection>,
    pub(crate) all_count: usize,
    pub(crate) connected_count: usize,
    pub(crate) segment: SwitcherSegment,
    pub(crate) highlight: Option<ClusterRef>,
    /// The ticks differ from the viewed set, so the footer offers to apply them.
    pub(crate) has_pending_ticks: bool,
    pub(crate) ticked_count: usize,
    pub(crate) tick_notice: Option<SharedString>,
    pub(crate) filter: Entity<InputState>,
    pub(crate) filter_text: String,
    pub(crate) shell: WeakEntity<AppShell>,
}

/// `trigger` wrapped in the switcher popover. The shell owns the open state; the kit focuses the
/// filter on open and puts the focus back on close.
pub(crate) fn cluster_switcher(
    trigger: Button,
    shell: &AppShell,
    cx: &Context<AppShell>,
) -> AnyElement {
    let state = shell.switcher();
    let weak = cx.weak_entity();
    let on_open_change = {
        let weak = weak.clone();
        move |open: &bool, window: &mut Window, cx: &mut App| {
            let _ = weak.update(cx, |shell, cx| {
                if *open {
                    shell.open_cluster_switcher(window, cx);
                } else {
                    shell.close_cluster_switcher(cx);
                }
            });
        }
    };
    let content = state.is_open().then(|| shell.switcher_content(weak, cx));
    Popover::new("cluster-switcher")
        .open(state.is_open())
        .on_open_change(on_open_change)
        .track_focus(&state.filter().read(cx).focus_handle(cx))
        .trigger(trigger)
        .when_some(content, |popover, content| {
            popover.content(move |_, window, cx| render_content(&content, window, cx))
        })
        .into_any_element()
}

fn render_content(content: &SwitcherContent, window: &Window, cx: &App) -> AnyElement {
    let [next, previous, confirm, close, tick] = std::array::from_fn(|_| content.shell.clone());
    v_flex()
        .key_context(SWITCHER_CONTEXT)
        .w(SWITCHER_WIDTH)
        .gap_2()
        .on_action(move |_: &SwitcherNext, _, cx| {
            let _ = next.update(cx, |shell, cx| {
                shell.move_switcher_highlight(HighlightStep::Next, cx)
            });
        })
        .on_action(move |_: &SwitcherPrevious, _, cx| {
            let _ = previous.update(cx, |shell, cx| {
                shell.move_switcher_highlight(HighlightStep::Previous, cx)
            });
        })
        .on_action(move |_: &SwitcherConfirm, _, cx| {
            let _ = confirm.update(cx, |shell, cx| shell.confirm_switcher_highlight(cx));
        })
        .on_action(move |_: &ToggleClusterTick, _, cx| {
            let _ = tick.update(cx, |shell, cx| shell.toggle_highlight_tick(cx));
        })
        .on_action(move |_: &CloseClusterSwitcher, _, cx| {
            let _ = close.update(cx, |shell, cx| shell.close_cluster_switcher(cx));
        })
        .child(header(content))
        .child(list(content, window, cx))
        .children(tick_notice_line(content.tick_notice.as_ref()))
        .children(ticks_footer(content, window, cx))
        .child(footer(content, window, cx))
        .into_any_element()
}

fn header(content: &SwitcherContent) -> AnyElement {
    // The chosen segment is filled with the primary color, the other one is a ghost button.
    let segment_button = |id: &'static str, label: String, segment: SwitcherSegment| {
        let shell = content.shell.clone();
        let button = Button::new(id).small().label(label);
        let button = if content.segment == segment {
            button.primary()
        } else {
            button.ghost()
        };
        button.on_click(move |_, _, cx| {
            let _ = shell.update(cx, |shell, cx| shell.set_switcher_segment(segment, cx));
        })
    };
    h_flex()
        .gap_2()
        .items_center()
        .child(
            div().flex_1().min_w_0().child(
                Input::new(&content.filter)
                    .small()
                    .prefix(Icon::new(IconName::Search)),
            ),
        )
        .child(
            h_flex()
                .gap_1()
                .child(segment_button(
                    "switcher-all",
                    format!("All {}", content.all_count),
                    SwitcherSegment::All,
                ))
                .child(segment_button(
                    "switcher-connected",
                    format!("Connected {}", content.connected_count),
                    SwitcherSegment::Connected,
                )),
        )
        .into_any_element()
}

fn list(content: &SwitcherContent, window: &Window, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let scroll = v_flex()
        .id("switcher-list")
        .max_h(MAX_LIST_HEIGHT)
        .overflow_y_scroll()
        .gap_1();
    if let Some(text) = empty_text(content) {
        return scroll
            .child(
                div()
                    .px_2()
                    .py_1()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(text),
            )
            .into_any_element();
    }
    let mut index = 0;
    let mut sections = Vec::new();
    for section in &content.sections {
        let rows: Vec<AnyElement> = section
            .rows
            .iter()
            .map(|row| {
                index += 1;
                render_row(index, row, content, window, cx)
            })
            .collect();
        sections.push(
            v_flex()
                .gap_0p5()
                .child(
                    h_flex()
                        .px_2()
                        .justify_between()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(section.title)
                        .child(section.rows.len().to_string()),
                )
                .children(rows),
        );
    }
    scroll.children(sections).into_any_element()
}

/// The text that replaces the rows, if any.
fn empty_text(content: &SwitcherContent) -> Option<String> {
    match content.list {
        SwitcherList::LoadingCatalog => return Some("Loading kubeconfigs…".to_owned()),
        SwitcherList::NoClusters => {
            return Some("No clusters. Add one in Settings.".to_owned());
        }
        SwitcherList::Clusters => {}
    }
    if !content.sections.is_empty() {
        return None;
    }
    let filter = content.filter_text.trim();
    Some(if !filter.is_empty() {
        format!("No cluster matches '{filter}'.")
    } else {
        "No connected cluster yet. Open the list with All to check them.".to_owned()
    })
}

fn render_row(
    index: usize,
    row: &SwitcherRow,
    content: &SwitcherContent,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let is_highlighted = content.highlight.as_ref() == Some(&row.cluster);
    let hover_background = theme.list_hover;
    let target = row.cluster.clone();
    let shell = content.shell.clone();
    let tick = {
        let (shell, target) = (content.shell.clone(), row.cluster.clone());
        // A separate sibling of the switch button, so ticking never switches.
        Checkbox::new(("switcher-tick", index))
            .checked(row.is_ticked)
            .on_click(move |_: &bool, _, cx| {
                let _ = shell.update(cx, |shell, cx| shell.toggle_cluster_tick(&target, cx));
            })
    };
    // A kit button: focusable with Tab, and Enter on it clicks it.
    let switch_area = Button::new(("switcher-row", index))
        .ghost()
        .small()
        .flex_1()
        .min_w_0()
        .when(row.health == RowHealth::Unreachable, |button| {
            button.opacity(0.55)
        })
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(
                    div().w(px(16.)).children(
                        row.is_active
                            .then(|| Icon::new(IconName::Check).size_4().into_any_element()),
                    ),
                )
                .child(environment_badge(row.environment, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_left()
                        .font_family(theme.mono_font_family.clone())
                        .when(row.is_primary, |label| label.font_bold())
                        .child(row.label.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(health_color(row.health, cx))
                        .child(health_text(row.health)),
                ),
        )
        .on_click(move |_, _, cx| {
            let _ = shell.update(cx, |shell, cx| shell.switch_from_switcher(&target, cx));
        });
    h_flex()
        .w_full()
        .px_2()
        .py_1()
        .gap_2()
        .items_center()
        .rounded(theme.radius)
        .when(row.is_active, |this| this.bg(theme.accent))
        .when(is_highlighted, |this| this.bg(hover_background))
        .hover(move |style| style.bg(hover_background))
        .child(tick)
        .child(switch_area)
        .child(row_action(index, row, content, window))
        .into_any_element()
}

/// Retry or Check where a probe can help, and the `Ctrl n` hint of the first nine rows.
fn row_action(
    index: usize,
    row: &SwitcherRow,
    content: &SwitcherContent,
    window: &Window,
) -> AnyElement {
    let probe_label = match row.health {
        RowHealth::Unreachable => Some("Retry"),
        RowHealth::NotChecked => Some("Check"),
        RowHealth::Live(_)
        | RowHealth::Connecting
        | RowHealth::Interrupted
        | RowHealth::Reachable(_)
        | RowHealth::Checking => None,
    };
    let probe = probe_label.map(|label| {
        let (shell, target) = (content.shell.clone(), row.cluster.clone());
        Button::new(("switcher-probe", index))
            .ghost()
            .small()
            .label(label)
            .when_some(row.failure.clone(), |button, reason| button.tooltip(reason))
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                let _ = shell.update(cx, |shell, cx| shell.probe_cluster(&target, cx));
            })
    });
    let hint = row
        .shortcut
        .and_then(switch_action)
        .and_then(|action| Kbd::binding_for_action(action.as_ref(), Some(SHELL_CONTEXT), window));
    h_flex()
        .gap_1()
        .items_center()
        .children(probe)
        .children(hint)
        .into_any_element()
}

/// Why the last tick was refused. It shows whether or not the footer does: a full view that
/// refuses a sixth tick has nothing to apply, so no footer, and the user still needs the reason.
fn tick_notice_line(notice: Option<&SharedString>) -> Option<AnyElement> {
    let notice = notice?.clone();
    Some(
        Alert::warning("switcher-tick-notice", notice)
            .small()
            .into_any_element(),
    )
}

/// `{n} selected`, Clear, and `View {n} clusters ⏎`; shown only while the ticks differ from the
/// viewed set, so there is no footer when applying would change nothing.
fn ticks_footer(content: &SwitcherContent, window: &Window, cx: &App) -> Option<AnyElement> {
    if !content.has_pending_ticks {
        return None;
    }
    let theme = cx.theme();
    let count = content.ticked_count;
    let apply_label = if count == 1 {
        "View 1 cluster".to_owned()
    } else {
        format!("View {count} clusters")
    };
    let (clear_shell, apply_shell) = (content.shell.clone(), content.shell.clone());
    let hint = Kbd::binding_for_action(&SwitcherConfirm, Some(SWITCHER_CONTEXT), window);
    Some(
        v_flex()
            .gap_1()
            .pt_2()
            .border_t_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .child(format!("{count} selected")),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Button::new("switcher-clear")
                                    .ghost()
                                    .small()
                                    .label("Clear")
                                    .on_click(move |_, _, cx| {
                                        let _ = clear_shell
                                            .update(cx, |shell, cx| shell.clear_switcher_ticks(cx));
                                    }),
                            )
                            .child(
                                Button::new("switcher-apply")
                                    .primary()
                                    .small()
                                    .label(apply_label)
                                    .disabled(count == 0)
                                    .on_click(move |_, _, cx| {
                                        let _ = apply_shell
                                            .update(cx, |shell, cx| shell.apply_switcher_ticks(cx));
                                    }),
                            )
                            .children(hint),
                    ),
            )
            .into_any_element(),
    )
}

fn footer(content: &SwitcherContent, window: &Window, cx: &App) -> AnyElement {
    let shell = content.shell.clone();
    let hint = Kbd::binding_for_action(&OpenSettings, Some(SHELL_CONTEXT), window)
        .or_else(|| Kbd::global_binding_for_action(&OpenSettings, window));
    h_flex()
        .pt_2()
        .justify_between()
        .items_center()
        .border_t_1()
        .border_color(cx.theme().border)
        .child(
            Button::new("switcher-manage")
                .ghost()
                .small()
                .label("Manage clusters…")
                .on_click(move |_, window, cx| {
                    let _ = shell.update(cx, |shell, cx| shell.close_cluster_switcher(cx));
                    window.dispatch_action(Box::new(ManageClusters), cx);
                }),
        )
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("opens Settings")
                .children(hint),
        )
        .into_any_element()
}

/// `Live · 38 ms`, `Unreachable`, and so on (decision 12 of the spec).
pub(crate) fn health_text(health: RowHealth) -> String {
    match health {
        RowHealth::Live(latency) => format!("Live · {} ms", latency_millis(latency)),
        RowHealth::Reachable(latency) => format!("Reachable · {} ms", latency_millis(latency)),
        RowHealth::Connecting => "Connecting…".to_owned(),
        RowHealth::Interrupted => "Interrupted".to_owned(),
        RowHealth::Unreachable => "Unreachable".to_owned(),
        RowHealth::Checking => "Checking…".to_owned(),
        RowHealth::NotChecked => "Not checked".to_owned(),
    }
}

pub(crate) fn health_color(health: RowHealth, cx: &App) -> gpui_kit::Hsla {
    match health {
        RowHealth::Live(_) | RowHealth::Reachable(_) => tone_color(StatusTone::Ok, cx),
        RowHealth::Connecting | RowHealth::Interrupted | RowHealth::Checking => {
            tone_color(StatusTone::Info, cx)
        }
        RowHealth::Unreachable | RowHealth::NotChecked => cx.theme().muted_foreground,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn health_lines_follow_the_spec_texts() {
        let ms = |millis| Duration::from_millis(millis);
        assert_eq!(health_text(RowHealth::Live(ms(38))), "Live · 38 ms");
        assert_eq!(health_text(RowHealth::Reachable(ms(7))), "Reachable · 7 ms");
        assert_eq!(health_text(RowHealth::Unreachable), "Unreachable");
        assert_eq!(health_text(RowHealth::Checking), "Checking…");
        assert_eq!(health_text(RowHealth::NotChecked), "Not checked");
        assert_eq!(health_text(RowHealth::Connecting), "Connecting…");
        assert_eq!(health_text(RowHealth::Interrupted), "Interrupted");
    }

    #[test]
    fn the_notice_line_needs_only_a_notice() {
        // Not the footer: a full view that refuses a sixth tick has nothing to apply.
        let notice = SharedString::from("View at most 5 clusters at once.");
        assert!(tick_notice_line(Some(&notice)).is_some());
        assert!(tick_notice_line(None).is_none());
    }

    #[test]
    fn every_digit_up_to_nine_has_an_action() {
        for shortcut in 1..=9 {
            assert!(switch_action(shortcut).is_some(), "{shortcut}");
        }
        assert!(switch_action(0).is_none());
        assert!(switch_action(10).is_none());
    }
}
