//! The Network › Port Forwarding page and its drawer (spec 0035): a local list of the forwards of
//! every cluster, not a Kubernetes kind. It reads `PortForwards`, so it needs no live cluster.
//!
//! A child of `app_shell`, like `workspace`. Every action of a row (Start, Retry, Restart) takes
//! its gate, lock, and confirm tier from the row's own cluster, never the primary.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, ClipboardItem, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, WeakEntity,
    div, prelude::FluentBuilder as _, px,
};

use super::AppShell;
use super::port_forward_dialogs::NewForwardPrefill;
use super::port_forward_open::target_key;
use crate::cell_truncation::{middle_truncate, mono_capacity_of_width};
use crate::drawer::{
    DrawerBody, DrawerHeader, DrawerNavigation, DrawerSize, absent_text, drawer_frame,
    first_section_title, menu_button, section_title, truncated_text, truncated_text_with_tooltip,
    wide_detail_row,
};
use crate::port_forwards::{Forward, ForwardFailure, ForwardId, ForwardState, byte_count_text};
use crate::resource_actions::{ActionAvailability, MenuItemIcon as _, disabled_menu_item};
use crate::status_tone::{tone_color, toned_text};
use crate::table_selection::ClusterObject;

/// The least the Target column keeps when the other columns leave it less.
const TARGET_MIN_WIDTH: f32 = 160.;
/// Holds `65535 → localhost:65535` in the mono font, so the local port is never cut.
const PORTS_WIDTH: f32 = 230.;
const STATUS_WIDTH: f32 = 170.;
const UPTIME_WIDTH: f32 = 80.;
const ACTION_WIDTH: f32 = 104.;
/// The gap between columns and the side padding of a row (`gap_3`, `px_4`).
const COLUMN_GAP: f32 = 12.;
const ROW_PADDING: f32 = 16.;

/// How wide the Target column is, so the middle cut of a long target knows how many characters
/// fit. It takes what the other columns leave of the list. The drawer covers the right part of the
/// list, so while it is open Target shrinks to keep Ports in view instead.
fn target_width(workspace: f32, drawer: Option<f32>) -> f32 {
    let room = match drawer {
        Some(drawer) => workspace - drawer - ROW_PADDING - PORTS_WIDTH - COLUMN_GAP,
        None => {
            let fixed = PORTS_WIDTH + STATUS_WIDTH + UPTIME_WIDTH + ACTION_WIDTH;
            workspace - 2. * ROW_PADDING - fixed - 4. * COLUMN_GAP
        }
    };
    room.max(TARGET_MIN_WIDTH)
}

/// `1 forward · 1 active`, `5 forwards · 3 active`.
fn forward_count_text(total: usize, running: usize) -> String {
    let noun = if total == 1 { "forward" } else { "forwards" };
    format!("{total} {noun} · {running} active")
}

const EMPTY_TEXT: &str = "No port forwards. Use Forward next to a port, or New forward.";
const BIND_TOOLTIP: &str = "The forward listens on this computer only (127.0.0.1 and ::1). On Windows another local program could bind the same port first.";

/// What a row's drawer menu reads, taken when the menu opens.
struct MenuSnapshot {
    is_running: bool,
    is_preset: bool,
    local_port: Option<u16>,
    /// The Restart and Start gate of the row's cluster; `None` while that cluster is not viewed.
    gate: Option<ActionAvailability>,
    cluster_label: SharedString,
    target: Option<ClusterObject>,
}

impl AppShell {
    /// The forwards that run, for the status bar.
    pub(crate) fn running_forward_count(&self, cx: &App) -> usize {
        self.port_forwards.read(cx).running_count()
    }

    /// `5 forwards · 3 active`: all rows, and the running ones.
    pub(super) fn port_forward_header_count(&self, cx: &App) -> String {
        let forwards = self.port_forwards.read(cx);
        forward_count_text(forwards.forwards().len(), forwards.running_count())
    }

    /// `New forward` and `Stop all`, right-aligned in the header.
    pub(super) fn port_forward_header_buttons(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let has_cluster = self.active_cluster().is_some();
        let new = Button::new("forward-new")
            .icon(Icon::new(IconName::Plus))
            .label("New forward")
            .small()
            .outline()
            .disabled(!has_cluster)
            .on_click(cx.listener(|shell, _, window, cx| {
                let Some(cluster) = shell.active_cluster() else {
                    return;
                };
                let namespace = shell.tool_namespace(cx).unwrap_or_default();
                let prefill = NewForwardPrefill {
                    cluster,
                    namespace,
                    target: None,
                    remote_port: None,
                };
                shell.open_new_forward(prefill, window, cx);
            }));
        let new = if has_cluster {
            new
        } else {
            new.tooltip("No cluster is open")
        };
        let running = self.port_forwards.read(cx).running_count();
        let stop_all = Button::new("forward-stop-all")
            .icon(Icon::new(IconName::CircleStop))
            .label("Stop all")
            .small()
            .outline()
            .disabled(running == 0)
            .on_click(cx.listener(|shell, _, _, cx| shell.stop_all_forwards(cx)));
        vec![new.into_any_element(), stop_all.into_any_element()]
    }

    /// The list, or the empty state; the filter sits above it.
    pub(super) fn render_port_forwards(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let filter = self.forward_filter.read(cx).value().to_string();
        let forwards = self.port_forwards.read(cx);
        let selected = forwards.selected().map(|forward| forward.id);
        let rows = forwards.sorted(&filter);
        let now = jiff::Timestamp::now();
        let drawer = selected.map(|_| f32::from(self.drawer.width(DrawerSize::Standard)));
        let target = target_width(f32::from(self.drawer.workspace_width()), drawer);
        let body = if forwards.forwards().is_empty() {
            note(EMPTY_TEXT, cx)
        } else if rows.is_empty() {
            note("No forwards match the filter", cx)
        } else {
            v_flex()
                .id("forward-rows")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .children(rows.iter().enumerate().map(|(index, forward)| {
                    self.forward_row(
                        index,
                        forward,
                        selected == Some(forward.id),
                        target,
                        now,
                        cx,
                    )
                }))
                .into_any_element()
        };
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_shrink_0()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .w(px(320.))
                            .child(Input::new(&self.forward_filter).small().cleanable(true)),
                    ),
            )
            .child(self.forward_header_row(target, cx))
            .child(body)
            .into_any_element()
    }

    fn forward_header_row(&self, target: f32, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let cell = |text: &'static str, width: f32| div().w(px(width)).flex_shrink_0().child(text);
        h_flex()
            .flex_shrink_0()
            .gap_3()
            .px_4()
            .py_1()
            .text_sm()
            .bg(theme.tokens.table_head)
            .text_color(theme.table_head_foreground)
            .border_b_1()
            .border_color(theme.border)
            .child(cell("Target", target))
            .child(cell("Ports", PORTS_WIDTH))
            .child(cell("Status", STATUS_WIDTH))
            .child(cell("Uptime", UPTIME_WIDTH))
            .child(div().w(px(ACTION_WIDTH)).flex_shrink_0())
            .into_any_element()
    }

    fn forward_row(
        &self,
        index: usize,
        forward: &Forward,
        is_selected: bool,
        target_width: f32,
        now: jiff::Timestamp,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let id = forward.id;
        let mono = theme.mono_font_family.clone();
        let target = forward.spec.target_text();
        let shown_target =
            middle_truncate(&target, mono_capacity_of_width(px(target_width), cx)).into_owned();
        // The Cluster column is gone: this app shows one cluster at a time.
        let target_tooltip = format!("{target} · {}", forward.cluster_label);
        let data = h_flex()
            .id(("forward-row", index))
            .flex_1()
            .min_w_0()
            .gap_3()
            .items_center()
            .cursor_pointer()
            .on_click(cx.listener(move |shell, _, _, cx| {
                shell.port_forwards.update(cx, |forwards, cx| {
                    forwards.select(Some(id));
                    cx.notify();
                });
            }))
            .child(
                // Cut in the middle: the end of the name tells forwards apart.
                truncated_text_with_tooltip(
                    ("forward-target", index),
                    shown_target,
                    target_tooltip,
                )
                .w(px(target_width))
                .flex_shrink_0()
                .font_family(mono.clone()),
            )
            .child(
                div()
                    .w(px(PORTS_WIDTH))
                    .flex_shrink_0()
                    .font_family(mono)
                    .child(forward.ports_text()),
            )
            .child(
                div()
                    .w(px(STATUS_WIDTH))
                    .flex_shrink_0()
                    .truncate()
                    .child(toned_text(forward.status(), cx)),
            )
            .child(
                div()
                    .w(px(UPTIME_WIDTH))
                    .flex_shrink_0()
                    .text_color(theme.muted_foreground)
                    .child(forward.uptime_text(now)),
            );
        h_flex()
            .gap_3()
            .px_4()
            .py_2()
            .items_center()
            .text_sm()
            .border_b_1()
            .border_color(theme.border)
            .when(is_selected, |row| row.bg(theme.table_active))
            .child(data)
            .child(
                div()
                    .w(px(ACTION_WIDTH))
                    .flex_shrink_0()
                    .child(self.forward_action(index, forward, cx)),
            )
            .into_any_element()
    }

    /// `■ Stop` while running, `↻ Retry` after a failure, `Change port…` when the port is in use,
    /// `▶ Start` for a stopped preset. Start, Retry, and Change port read the gate of the row's own
    /// cluster.
    fn forward_action(&self, index: usize, forward: &Forward, cx: &Context<Self>) -> AnyElement {
        let id = forward.id;
        let button = |label: &'static str| {
            Button::new(("forward-action", index))
                .label(label)
                .xsmall()
                .ghost()
        };
        if forward.state.is_running() {
            return button("■ Stop")
                .on_click(cx.listener(move |shell, _, _, cx| shell.stop_forward(id, cx)))
                .into_any_element();
        }
        let restart = StoppedRowAction::of(&forward.state);
        let button = button(restart.label());
        // A cluster that is not viewed answers on click ("Open {cluster} to start this forward").
        let button = match self.forward_start_gate(&forward.cluster, cx) {
            Some(ActionAvailability::Disabled { reason }) => button.disabled(true).tooltip(reason),
            Some(ActionAvailability::Enabled) | None => {
                button.on_click(cx.listener(move |shell, _, window, cx| match restart {
                    StoppedRowAction::ChangePort => shell.open_change_local_port(id, window, cx),
                    StoppedRowAction::Retry | StoppedRowAction::Start => {
                        shell.start_forward_again(id, window, cx);
                    }
                }))
            }
        };
        button.into_any_element()
    }

    /// The drawer of the selected row, over the list.
    pub(super) fn render_port_forward_drawer(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let forward = self.port_forwards.read(cx).selected()?;
        let theme = cx.theme();
        let id = forward.id;
        let since = forward.started_at.map(|at| {
            at.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%H:%M")
                .to_string()
        });
        let mut meta = vec![
            forward.spec.namespace.clone(),
            forward.cluster_label.to_string(),
        ];
        meta.extend(since.map(|since| format!("since {since}")));
        let header = DrawerHeader {
            kind_icon: IconName::ArrowLeftRight,
            kind_name: "Port forward".into(),
            name: format!(
                "{} · {}",
                forward.spec.short_target_text(),
                forward.spec.remote_port
            )
            .into(),
            subtitle: h_flex()
                .gap_1()
                .text_sm()
                .child(toned_text(forward.status(), cx))
                .child(
                    div()
                        .text_color(theme.muted_foreground)
                        .child(format!("· {}", meta.join(" · "))),
                )
                .into_any_element(),
            menu: self.forward_menu_button(id, cx.weak_entity()),
            on_close: Rc::new(cx.listener(|shell, _, _, cx| {
                shell.port_forwards.update(cx, |forwards, cx| {
                    forwards.select(None);
                    cx.notify();
                });
            })),
            navigation: DrawerNavigation::default(),
        };
        let body = v_flex()
            .child(first_section_title("Forward", cx))
            .child(wide_detail_row("Target", target_value(forward, cx), cx))
            .child(wide_detail_row(
                "Remote port",
                div().child(format!("{}/TCP", forward.spec.remote_port)),
                cx,
            ))
            .child(wide_detail_row(
                "Local address",
                match forward.local_address_text() {
                    Some(address) => div()
                        .font_family(theme.mono_font_family.clone())
                        .child(address)
                        .into_any_element(),
                    None => absent_text(cx).into_any_element(),
                },
                cx,
            ))
            .child(wide_detail_row(
                "Bind",
                truncated_text_with_tooltip("forward-bind", "localhost only", BIND_TOOLTIP),
                cx,
            ))
            .child(wide_detail_row(
                "Auto-reconnect",
                div().child("on · up to 5 tries"),
                cx,
            ))
            .child(section_title("Traffic", cx))
            .child(wide_detail_row(
                "Open connections",
                div().child(forward.traffic.open_connections.to_string()),
                cx,
            ))
            .child(wide_detail_row(
                "Received",
                div().child(byte_count_text(forward.traffic.received)),
                cx,
            ))
            .child(wide_detail_row(
                "Sent",
                div().child(byte_count_text(forward.traffic.sent)),
                cx,
            ))
            .child(section_title("Recent events", cx))
            .child(recent_events(forward, cx))
            .into_any_element();
        Some(
            drawer_frame(
                header,
                None,
                DrawerBody::Scrolling(body),
                self.drawer.width(DrawerSize::Standard),
                &self.drawer.scroll,
                cx,
            )
            .into_any_element(),
        )
    }

    /// The ⋯ menu reads the row and the gate when it opens, so it shows the state of that moment.
    fn forward_menu_button(&self, id: ForwardId, shell: WeakEntity<AppShell>) -> AnyElement {
        menu_button()
            .dropdown_menu(move |menu, _, cx| {
                let Ok(Some(snapshot)) =
                    shell.read_with(cx, |shell, cx| shell.menu_snapshot(id, cx))
                else {
                    return menu;
                };
                forward_menu(menu, id, &snapshot, &shell)
            })
            .into_any_element()
    }

    fn menu_snapshot(&self, id: ForwardId, cx: &App) -> Option<MenuSnapshot> {
        let forward = self.port_forwards.read(cx).get(id)?;
        let viewed = self.session_of(&forward.cluster).is_some();
        Some(MenuSnapshot {
            is_running: forward.state.is_running(),
            is_preset: forward.is_preset,
            local_port: forward.local.map(|local| local.port()),
            gate: self.forward_start_gate(&forward.cluster, cx),
            cluster_label: forward.cluster_label.clone(),
            target: viewed
                .then(|| {
                    target_key(&forward.spec)
                        .map(|key| ClusterObject::new(forward.cluster.clone(), key))
                })
                .flatten(),
        })
    }
}

/// `pod/{pod}`, and for a Service or a workload the pod it resolved to.
fn target_value(forward: &Forward, cx: &App) -> AnyElement {
    let mono = cx.theme().mono_font_family.clone();
    let mut text = forward.spec.short_target_text();
    if let Some(pod) = &forward.pod
        && format!("pod/{pod}") != text
    {
        text.push_str(&format!(" → pod/{pod}"));
    }
    truncated_text("forward-target-value", text)
        .font_family(mono)
        .into_any_element()
}

fn recent_events(forward: &Forward, cx: &App) -> AnyElement {
    if forward.events.is_empty() {
        return absent_text(cx).into_any_element();
    }
    v_flex()
        .gap_1()
        .children(forward.events.iter().rev().map(|line| {
            let color = line
                .tone
                .map_or(cx.theme().foreground, |tone| tone_color(tone, cx));
            h_flex()
                .gap_3()
                .items_start()
                .text_sm()
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(cx.theme().muted_foreground)
                        .child(line.time_text()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(color)
                        .child(line.text.clone()),
                )
        }))
        .into_any_element()
}

fn note(text: &'static str, cx: &App) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

/// The ⋯ items in the order of the wireframe. Stop-type items follow the state; Restart and Start
/// show the gate of the row's cluster.
fn forward_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    id: ForwardId,
    row: &MenuSnapshot,
    shell: &WeakEntity<AppShell>,
) -> gpui_kit::component::menu::PopupMenu {
    let on_shell =
        |run: fn(&mut AppShell, ForwardId, &mut gpui_kit::Window, &mut Context<AppShell>)| {
            let shell = shell.clone();
            move |_: &gpui_kit::ClickEvent, window: &mut gpui_kit::Window, cx: &mut App| {
                let _ = shell.update(cx, |shell, cx| run(shell, id, window, cx));
            }
        };
    let is_stop = row.is_running || row.is_preset;
    let stop_label = if is_stop {
        "Stop forward"
    } else {
        "Remove from list"
    };
    let stop = if row.is_running || !row.is_preset {
        PopupMenuItem::new(stop_label).on_click(on_shell(|shell, id, _, cx| {
            shell.stop_forward(id, cx);
        }))
    } else {
        disabled_menu_item(stop_label, "Not running".into())
    }
    .menu_icon(if is_stop {
        IconName::CircleStop
    } else {
        IconName::X
    });
    let start_label = if row.is_running { "Restart" } else { "Start" };
    let start = match &row.gate {
        Some(ActionAvailability::Disabled { reason }) => {
            disabled_menu_item(start_label, reason.clone())
        }
        // A cluster that is not viewed answers on click with the notice.
        Some(ActionAvailability::Enabled) | None => {
            PopupMenuItem::new(start_label).on_click(on_shell(|shell, id, window, cx| {
                shell.start_forward_again(id, window, cx);
            }))
        }
    }
    .menu_icon(if row.is_running {
        IconName::RotateCw
    } else {
        IconName::Play
    });
    let browser = match (row.is_running, row.local_port) {
        (true, Some(port)) => PopupMenuItem::new("Open in browser")
            .on_click(move |_, _, cx| cx.open_url(&format!("http://127.0.0.1:{port}"))),
        _ => disabled_menu_item("Open in browser", "Not listening".into()),
    }
    .menu_icon(IconName::ExternalLink);
    let copy = match row.local_port {
        Some(port) => PopupMenuItem::new("Copy local address").on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(format!("127.0.0.1:{port}")));
        }),
        None => disabled_menu_item("Copy local address", "Not listening".into()),
    }
    .menu_icon(IconName::Copy);
    let change = PopupMenuItem::new("Change local port…")
        .on_click(on_shell(|shell, id, window, cx| {
            shell.open_change_local_port(id, window, cx);
        }))
        .menu_icon(IconName::Pencil);
    let save = if row.is_preset {
        disabled_menu_item("Save as preset", "Already a preset".into())
    } else {
        PopupMenuItem::new("Save as preset").on_click(on_shell(|shell, id, _, cx| {
            shell.save_forward_preset(id, cx);
        }))
    }
    .menu_icon(IconName::Star);
    let go = match &row.target {
        Some(target) => {
            let (shell, target) = (shell.clone(), target.clone());
            PopupMenuItem::new("Go to target").on_click(move |_, _, cx| {
                let _ = shell.update(cx, |shell, cx| shell.reveal_object(target.clone(), cx));
            })
        }
        None => disabled_menu_item(
            "Go to target",
            format!("Open {} first", row.cluster_label).into(),
        ),
    }
    .menu_icon(IconName::CornerDownRight);
    let menu = menu
        .item(stop)
        .item(start)
        .item(browser)
        .item(copy)
        .item(change)
        .item(save)
        .item(go);
    if !row.is_preset {
        return menu;
    }
    menu.separator().item(
        PopupMenuItem::new("Remove preset…")
            .on_click(on_shell(|shell, id, window, cx| {
                shell.open_remove_preset(id, window, cx);
            }))
            .menu_icon(IconName::X),
    )
}

/// What the button of a row without a running stream offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StoppedRowAction {
    /// The same port would fail again, so the button asks for another one.
    ChangePort,
    Retry,
    Start,
}

impl StoppedRowAction {
    fn of(state: &ForwardState) -> Self {
        match state {
            ForwardState::Failed(ForwardFailure::PortInUse(_)) => Self::ChangePort,
            ForwardState::Failed(_) => Self::Retry,
            _ => Self::Start,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::ChangePort => "Change port…",
            Self::Retry => "↻ Retry",
            Self::Start => "▶ Start",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_counts_forwards_in_singular_and_plural() {
        assert_eq!(forward_count_text(1, 1), "1 forward · 1 active");
        assert_eq!(forward_count_text(0, 0), "0 forwards · 0 active");
        assert_eq!(forward_count_text(5, 3), "5 forwards · 3 active");
    }
    #[test]
    fn target_takes_what_the_fixed_columns_leave() {
        // 1100 and 1320 px windows, less the 250 px sidebar, drawer closed.
        assert_eq!(target_width(880., None), 880. - 32. - 584. - 48.);
        assert_eq!(target_width(1100., None), 1100. - 32. - 584. - 48.);
    }

    #[test]
    fn target_keeps_ports_in_view_beside_the_drawer() {
        // The drawer covers the right half of an 1100 px list: Target ends where Ports can still
        // end before the drawer starts.
        let target = target_width(1100., Some(550.));
        assert!(ROW_PADDING + target + COLUMN_GAP + PORTS_WIDTH <= 1100. - 550.);
    }

    #[test]
    fn target_never_shrinks_below_its_minimum() {
        assert_eq!(target_width(500., None), TARGET_MIN_WIDTH);
        assert_eq!(target_width(800., Some(480.)), TARGET_MIN_WIDTH);
    }

    #[test]
    fn ports_width_holds_the_longest_ports_text() {
        // 23 mono characters of about 9.6 px each.
        assert!(PORTS_WIDTH >= "65535 → localhost:65535".chars().count() as f32 * 9.6);
    }

    #[test]
    fn a_port_in_use_offers_change_port_instead_of_retry() {
        let label = |state| StoppedRowAction::of(&state).label();
        assert_eq!(
            label(ForwardState::Failed(ForwardFailure::PortInUse(3000))),
            "Change port…"
        );
        assert_eq!(
            label(ForwardState::Failed(ForwardFailure::TargetLost)),
            "↻ Retry"
        );
        assert_eq!(label(ForwardState::Stopped), "▶ Start");
    }
}
