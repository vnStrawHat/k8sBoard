use std::rc::Rc;

use cluster::{ContainerKind, ContainerState, ContainerSummary, PodSummary, Termination};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _, px,
};

use crate::age::format_age;
use crate::app_shell::AppShell;
use crate::cluster_session::ClusterSession;
use crate::drawer::{
    DrawerHeader, DrawerState, ExpandToggle, PodDrawerTab, absent_text, created_text, detail_row,
    drawer_frame, menu_button, section_title, truncated_text, value_or_absent,
};
use crate::log_dock::LogDock;
use crate::resource_actions::pod_menu;
use crate::status_tone::{
    StatusLabel, container_state_label, pod_status_label, tone_color, toned_text,
};
use crate::table_selection::ResourceKey;

const CONTAINER_LIST_WIDTH: Pixels = px(240.);

pub(crate) fn pod_drawer(
    pod: &PodSummary,
    state: &DrawerState,
    session: &Entity<ClusterSession>,
    dock: &WeakEntity<LogDock>,
    cx: &Context<AppShell>,
) -> AnyElement {
    let now = jiff::Timestamp::now();
    let header = DrawerHeader {
        kind_badge: "Po",
        name: pod.name.clone().into(),
        subtitle: subtitle(pod, now, cx),
        menu: pod_menu_button(pod, session, dock),
        expand: Some(ExpandToggle {
            is_expanded: state.is_expanded,
            on_click: Rc::new(cx.listener(|shell, _, _, cx| shell.toggle_drawer_expanded(cx))),
        }),
        on_close: Rc::new(cx.listener(|shell, _, _, cx| shell.close_drawer(cx))),
    };
    let body = match state.tab {
        PodDrawerTab::Overview => overview(pod, cx),
        PodDrawerTab::Containers => containers_tab(pod, state, now, cx),
    };
    let tabs = tab_bar(pod, state, cx);
    drawer_frame(header, Some(tabs), body, state.width(), cx).into_any_element()
}

fn subtitle(pod: &PodSummary, now: jiff::Timestamp, cx: &App) -> AnyElement {
    h_flex()
        .gap_1()
        .text_sm()
        .child(toned_text(pod_status_label(pod), cx))
        .child(
            div()
                .text_color(cx.theme().muted_foreground)
                .child(subtitle_detail(pod, now)),
        )
        .into_any_element()
}

/// `· namespace · created 2d ago`.
fn subtitle_detail(pod: &PodSummary, now: jiff::Timestamp) -> String {
    let parts = std::iter::once(pod.namespace.clone()).chain(created_text(pod.created_at, now));
    format!("· {}", parts.collect::<Vec<_>>().join(" · "))
}

/// The menu reads the session when it opens, so it shows the access state and the pod of
/// that moment.
fn pod_menu_button(
    pod: &PodSummary,
    session: &Entity<ClusterSession>,
    dock: &WeakEntity<LogDock>,
) -> AnyElement {
    let session = session.clone();
    let dock = dock.clone();
    let key = ResourceKey::of_pod(pod);
    menu_button()
        .dropdown_menu(move |menu, _, cx| {
            let Some(live) = session.read(cx).live() else {
                return menu;
            };
            match live.pods.items().iter().find(|pod| key.is_pod(pod)) {
                Some(pod) => pod_menu(menu, pod, live, &dock),
                None => menu,
            }
        })
        .into_any_element()
}

fn tab_bar(pod: &PodSummary, state: &DrawerState, cx: &Context<AppShell>) -> AnyElement {
    let selected_index = match state.tab {
        PodDrawerTab::Overview => 0,
        PodDrawerTab::Containers => 1,
    };
    TabBar::new("pod-drawer-tabs")
        .underline()
        .selected_index(selected_index)
        .on_click(cx.listener(|shell, index: &usize, _, cx| {
            let tab = if *index == 0 {
                PodDrawerTab::Overview
            } else {
                PodDrawerTab::Containers
            };
            shell.set_drawer_tab(tab, cx);
        }))
        // Same horizontal padding as the drawer header.
        .prefix(div().w_4())
        .child(Tab::new().label("Overview"))
        .child(Tab::new().label(format!("Containers {}", pod.containers.len())))
        .into_any_element()
}

fn overview(pod: &PodSummary, cx: &Context<AppShell>) -> AnyElement {
    let running = pod
        .containers
        .iter()
        .filter(|container| matches!(container.state, ContainerState::Running { .. }))
        .count();
    let controller = pod
        .controller
        .as_ref()
        .map(|controller| format!("{}/{}", controller.kind, controller.name));
    v_flex()
        .child(section_title("Pod", cx))
        .child(detail_row(
            "Node",
            value_or_absent(pod.node_name.as_deref(), cx),
            cx,
        ))
        .child(detail_row(
            "Pod IP",
            mono_or_absent(pod.pod_ip.as_deref(), cx),
            cx,
        ))
        .child(detail_row(
            "QoS class",
            value_or_absent(pod.qos_class.as_deref(), cx),
            cx,
        ))
        .child(detail_row(
            "Service account",
            value_or_absent(pod.service_account.as_deref(), cx),
            cx,
        ))
        .child(detail_row(
            "Controlled by",
            value_or_absent(controller.as_deref(), cx),
            cx,
        ))
        .child(section_title("Conditions", cx))
        .child(conditions(pod, cx))
        .child(section_title(
            format!("Containers · {running} of {} running", pod.containers.len()),
            cx,
        ))
        .children(
            pod.containers
                .iter()
                .enumerate()
                .map(|(index, container)| container_row(index, container, cx)),
        )
        .into_any_element()
}

fn mono_or_absent(value: Option<&str>, cx: &App) -> AnyElement {
    match value {
        Some(text) => div()
            .truncate()
            .font_family(cx.theme().mono_font_family.clone())
            .child(text.to_owned())
            .into_any_element(),
        None => absent_text(cx).into_any_element(),
    }
}

fn conditions(pod: &PodSummary, cx: &App) -> AnyElement {
    if pod.conditions.is_empty() {
        return absent_text(cx).into_any_element();
    }
    let theme = cx.theme();
    h_flex()
        .flex_wrap()
        .gap_2()
        .children(pod.conditions.iter().map(|condition| {
            let dot_color = if condition.is_true {
                theme.success
            } else {
                theme.muted_foreground
            };
            h_flex()
                .gap_1p5()
                .items_center()
                .px_2()
                .py_0p5()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .text_xs()
                .child(div().size_2().rounded_full().bg(dot_color))
                .child(condition.name.clone())
        }))
        .into_any_element()
}

/// One container of the Overview tab; clicking it opens the Containers tab on it.
fn container_row(index: usize, container: &ContainerSummary, cx: &Context<AppShell>) -> AnyElement {
    let theme = cx.theme();
    let name = container.name.clone();
    let mono = theme.mono_font_family.clone();
    let hover_bg = theme.secondary_hover;
    h_flex()
        .id(("container-row", index))
        .gap_2()
        .items_center()
        .px_2()
        .py_1()
        .rounded(theme.radius)
        .text_sm()
        .cursor_pointer()
        .hover(move |style| style.bg(hover_bg))
        .on_click(cx.listener(move |shell, _, _, cx| shell.open_container(name.clone(), cx)))
        .child(kind_tag(container.kind, cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_family(mono.clone())
                .child(container.name.clone()),
        )
        .child(toned_text(container_state_label(container), cx))
        .child(
            div()
                .w(px(32.))
                .text_right()
                .font_family(mono)
                .child(container.restart_count.to_string()),
        )
        .into_any_element()
}

fn kind_tag(kind: ContainerKind, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .px_1()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .text_color(theme.muted_foreground)
        .text_xs()
        .child(kind_tag_text(kind))
        .into_any_element()
}

pub(crate) fn kind_tag_text(kind: ContainerKind) -> &'static str {
    match kind {
        ContainerKind::Init => "INIT",
        ContainerKind::Sidecar => "SIDECAR",
        ContainerKind::Main => "MAIN",
    }
}

/// The first main container that is not ready, else the first main one, else the first.
pub(crate) fn default_container(containers: &[ContainerSummary]) -> Option<usize> {
    containers
        .iter()
        .position(|container| container.kind == ContainerKind::Main && !container.is_ready)
        .or_else(|| {
            containers
                .iter()
                .position(|container| container.kind == ContainerKind::Main)
        })
        .or_else(|| (!containers.is_empty()).then_some(0))
}

fn containers_tab(
    pod: &PodSummary,
    state: &DrawerState,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> AnyElement {
    let selected = state
        .selected_container
        .as_deref()
        .and_then(|name| {
            pod.containers
                .iter()
                .position(|container| container.name == name)
        })
        .or_else(|| default_container(&pod.containers));
    let Some(selected) = selected else {
        return absent_text(cx).into_any_element();
    };
    let list = container_list(&pod.containers, selected, cx);
    let detail = container_detail(&pod.containers[selected], now, cx);
    // At the default width the list stacks above the detail; expanded, they sit side by side.
    if state.is_expanded {
        h_flex()
            .items_start()
            .gap_4()
            .child(div().w(CONTAINER_LIST_WIDTH).flex_shrink_0().child(list))
            .child(div().flex_1().min_w_0().child(detail))
            .into_any_element()
    } else {
        v_flex()
            .gap_4()
            .child(list)
            .child(detail)
            .into_any_element()
    }
}

fn container_list(
    containers: &[ContainerSummary],
    selected: usize,
    cx: &Context<AppShell>,
) -> AnyElement {
    let groups = [
        ContainerKind::Init,
        ContainerKind::Sidecar,
        ContainerKind::Main,
    ];
    v_flex()
        .children(groups.into_iter().filter_map(|kind| {
            let members: Vec<(usize, &ContainerSummary)> = containers
                .iter()
                .enumerate()
                .filter(|(_, container)| container.kind == kind)
                .collect();
            if members.is_empty() {
                return None;
            }
            let refs: Vec<&ContainerSummary> =
                members.iter().map(|(_, container)| *container).collect();
            let title = group_title(kind, &refs);
            let theme = cx.theme();
            Some(
                v_flex()
                    .child(
                        div()
                            .py_1()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(title),
                    )
                    .children(members.into_iter().map(|(index, container)| {
                        list_item(index, container, index == selected, cx)
                    })),
            )
        }))
        .into_any_element()
}

fn group_title(kind: ContainerKind, members: &[&ContainerSummary]) -> String {
    let total = members.len();
    match kind {
        ContainerKind::Init => {
            let done = members
                .iter()
                .filter(|container| {
                    matches!(&container.state, ContainerState::Terminated(termination)
                        if termination.exit_code == 0)
                })
                .count();
            format!("Init · ran in order {done}/{total}")
        }
        ContainerKind::Sidecar => format!("Sidecars {total}"),
        ContainerKind::Main => {
            let ready = members
                .iter()
                .filter(|container| container.is_ready)
                .count();
            format!("Containers {ready}/{total}")
        }
    }
}

fn list_item(
    index: usize,
    container: &ContainerSummary,
    is_selected: bool,
    cx: &Context<AppShell>,
) -> AnyElement {
    let theme = cx.theme();
    let name = container.name.clone();
    // The same colour the tables use for their selected row.
    let selected_bg = theme.table_active;
    let hover_bg = theme.secondary_hover;
    let mono = theme.mono_font_family.clone();
    h_flex()
        .id(("container-item", index))
        .gap_2()
        .items_center()
        .px_2()
        .py_1()
        .rounded(theme.radius)
        .text_sm()
        .cursor_pointer()
        .when(is_selected, move |this| this.bg(selected_bg))
        .when(!is_selected, move |this| {
            this.hover(move |s| s.bg(hover_bg))
        })
        .on_click(cx.listener(move |shell, _, _, cx| shell.select_container(name.clone(), cx)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_family(mono.clone())
                .child(container.name.clone()),
        )
        .child(
            div()
                .font_family(mono)
                .text_color(theme.muted_foreground)
                .child(container.restart_count.to_string()),
        )
        .child(toned_text(container_state_label(container), cx))
        .into_any_element()
}

fn container_detail(
    container: &ContainerSummary,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> AnyElement {
    let theme = cx.theme();
    let mono = theme.mono_font_family.clone();
    let label = container_state_label(container);
    let tone = label.tone;
    let state = StatusLabel {
        text: state_text(container, &label, now).into(),
        tone,
    };
    let last_state = container
        .last_termination
        .as_ref()
        .map(|termination| last_state_text(termination, now));
    v_flex()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .pb_2()
                .child(
                    div()
                        .font_semibold()
                        .font_family(mono.clone())
                        .child(container.name.clone()),
                )
                .child(div().text_color(tone_color(tone, cx)).child(label.text))
                .child(kind_tag(container.kind, cx)),
        )
        .child(detail_row("State", toned_text(state, cx), cx))
        .child(detail_row(
            "Last state",
            value_or_absent(last_state.as_deref(), cx),
            cx,
        ))
        .child(detail_row(
            "Restarts",
            div()
                .font_family(mono.clone())
                .child(container.restart_count.to_string()),
            cx,
        ))
        .child(detail_row(
            "Image",
            truncated_text("container-image", container.image.clone()).font_family(mono),
            cx,
        ))
        .into_any_element()
}

/// The state label, plus how long a running container has been up.
fn state_text(container: &ContainerSummary, label: &StatusLabel, now: jiff::Timestamp) -> String {
    match &container.state {
        ContainerState::Running {
            started_at: Some(started_at),
        } => format!(
            "{} · started {} ago",
            label.text,
            format_age(Some(*started_at), now)
        ),
        _ => label.text.to_string(),
    }
}

fn last_state_text(termination: &Termination, now: jiff::Timestamp) -> String {
    let reason = termination
        .reason
        .as_ref()
        .map_or_else(|| "Terminated".to_owned(), ToString::to_string);
    let mut text = format!("{reason} · exit {}", termination.exit_code);
    if let Some(signal) = termination.signal {
        text.push_str(&format!(" · signal {signal}"));
    }
    if let Some(finished_at) = termination.finished_at {
        text.push_str(&format!(
            " · ended {} ago",
            format_age(Some(finished_at), now)
        ));
    }
    text
}

#[cfg(test)]
#[path = "pod_drawer_tests.rs"]
mod pod_drawer_tests;
