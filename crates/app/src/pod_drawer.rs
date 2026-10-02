use std::rc::Rc;

use cluster::{
    ByteAmount, ContainerKind, ContainerResource, ContainerState, ContainerSummary, CpuAmount,
    EventSummary, PodCondition, PodSummary, ResourceUsage,
};
use gpui_kit::component::alert::Alert;
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, SharedString, StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _, px,
};

use crate::app_shell::AppShell;
use crate::cluster_session::{ClusterSession, LiveCluster, LiveList};
use crate::container_detail::{ContainerDetailInput, container_detail};
use crate::drawer::{
    DrawerBody, DrawerHeader, DrawerState, DrawerTab, absent_text, created_text, detail_row,
    drawer_frame, drawer_tab_bar, drawer_tabs, expand_toggle, link_text, menu_button,
    section_title, shown_tab, tab_titles, value_or_absent, yaml_body,
};
use crate::log_dock::LogDock;
use crate::monitor_tab::{MonitorView, monitor_tab};
use crate::object_events::{event_subject, recent_events};
use crate::pod_diagnosis::{PodDiagnosis, pod_diagnosis};
use crate::resource_actions::{pod_menu, port_forward_reason};
use crate::status_tone::{StatusTone, container_state_label, pod_status_label, toned_text};
use crate::table_selection::ResourceKey;
use crate::usage_bar::UsageBar;
use crate::usage_format::{Measure, usage_tone};

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
        menu: pod_menu_button(pod, session, dock, cx.weak_entity()),
        expand: expand_toggle(state, cx),
        on_close: Rc::new(cx.listener(|shell, _, _, cx| shell.close_drawer(cx))),
    };
    let events = pod_events(pod, session, cx);
    let tabs = drawer_tabs(&ResourceKey::of_pod(pod));
    let shown = shown_tab(tabs, state.tab);
    let loaded_events = events.and_then(LiveList::ready_items);
    let body = match shown {
        DrawerTab::Overview => DrawerBody::Scrolling(overview(pod, loaded_events, now, cx)),
        DrawerTab::Containers => DrawerBody::Scrolling(containers_tab(
            pod,
            state,
            loaded_events,
            &session
                .read(cx)
                .live()
                .map(|live| port_forward_reason(&live.access))
                .unwrap_or_default(),
            session.read(cx).live(),
            now,
            cx,
        )),
        DrawerTab::Monitor => DrawerBody::Scrolling(match session.read(cx).live() {
            Some(live) => monitor_tab(&MonitorView::of_pods(state, live), cx),
            None => div().into_any_element(),
        }),
        DrawerTab::Yaml => yaml_body(state),
        DrawerTab::Events => DrawerBody::Scrolling(recent_events(events, cx)),
    };
    let titles = tab_titles(tabs, pod.containers.len(), events);
    let tab_bar = drawer_tab_bar(titles, shown, cx);
    drawer_frame(header, tab_bar, body, state.width(), cx).into_any_element()
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
    shell: WeakEntity<AppShell>,
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
                Some(pod) => pod_menu(menu, pod, live, session.read(cx).context(), &dock, &shell),
                None => menu,
            }
        })
        .into_any_element()
}

/// The pod's events list, or `None` while the debounce or a switch is in progress.
fn pod_events<'a>(
    pod: &PodSummary,
    session: &'a Entity<ClusterSession>,
    cx: &'a App,
) -> Option<&'a LiveList<EventSummary>> {
    let subject = event_subject(&ResourceKey::of_pod(pod))?;
    session.read(cx).live()?.events_of(&subject)
}

fn overview(
    pod: &PodSummary,
    events: Option<&[EventSummary]>,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> AnyElement {
    let running = pod
        .containers
        .iter()
        .filter(|container| matches!(container.state, ContainerState::Running { .. }))
        .count();
    let node = match &pod.node_name {
        Some(name) => link_text(
            1,
            &name.clone().into(),
            ResourceKey::Node { name: name.clone() },
            cx,
        )
        .into_any_element(),
        None => absent_text(cx).into_any_element(),
    };
    let controller = pod.controller.as_ref().map(|controller| {
        let text = format!("{}/{}", controller.kind, controller.name);
        let target =
            ResourceKey::of_object(&controller.kind, Some(&pod.namespace), &controller.name);
        match target {
            Some(target) => link_text(2, &text.into(), target, cx).into_any_element(),
            None => div().truncate().child(text).into_any_element(),
        }
    });
    v_flex()
        .children(pod_diagnosis(pod, events, now).map(|diagnosis| why_box(&diagnosis, cx)))
        .child(section_title("Pod", cx))
        .child(detail_row("Node", node, cx))
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
            controller.unwrap_or_else(|| absent_text(cx).into_any_element()),
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

/// The WHY box. `Alert` has no children, so the link that opens the container is a sibling
/// right under it.
fn why_box(diagnosis: &PodDiagnosis, cx: &Context<AppShell>) -> AnyElement {
    let title = match &diagnosis.container {
        Some(name) => format!("WHY · CONTAINER {name}"),
        None => "WHY · POD".to_owned(),
    };
    let text = diagnosis.text.clone();
    let alert = match diagnosis.tone {
        StatusTone::Bad => Alert::error("why-box", text),
        _ => Alert::warning("why-box", text),
    };
    v_flex()
        .gap_1()
        .child(alert.title(title))
        .children(diagnosis.container.clone().map(|name| {
            let label = format!("Open container {name} →");
            div()
                .id("why-open-container")
                .cursor_pointer()
                .text_sm()
                .text_color(cx.theme().link)
                .underline()
                .on_click(
                    cx.listener(move |shell, _, _, cx| shell.open_container(name.clone(), cx)),
                )
                .child(label)
        }))
        .into_any_element()
}

/// `{reason}: {message}`, either part may be missing; `None` when both are.
fn condition_tooltip(condition: &PodCondition) -> Option<String> {
    match (&condition.reason, &condition.message) {
        (Some(reason), Some(message)) => Some(format!("{reason}: {message}")),
        (Some(text), None) | (None, Some(text)) => Some(text.clone()),
        (None, None) => None,
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
        .children(pod.conditions.iter().enumerate().map(|(index, condition)| {
            let dot_color = if condition.is_true {
                theme.success
            } else {
                theme.muted_foreground
            };
            let tooltip = (!condition.is_true)
                .then(|| condition_tooltip(condition))
                .flatten();
            h_flex()
                .id(("condition", index))
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
                .when_some(tooltip, |this, text| {
                    this.tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
                })
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

pub(crate) fn kind_tag(kind: ContainerKind, cx: &App) -> AnyElement {
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

/// How a container's `cpu` or `memory` row reads once usage is known.
#[derive(Debug, PartialEq)]
pub(crate) struct UsageRow {
    pub(crate) value: String,
    pub(crate) tone: Option<StatusTone>,
    pub(crate) bar: Option<UsageBar>,
    pub(crate) note: Option<String>,
}

/// `None` when the row keeps its request and limit text: no usage, or not `cpu` or `memory`.
pub(crate) fn container_usage_row(
    resource: &ContainerResource,
    usage: Option<ResourceUsage>,
) -> Option<UsageRow> {
    let usage = usage?;
    let (measure, used) = match resource.name.as_str() {
        "cpu" => (Measure::Cpu, usage.cpu.cores()),
        "memory" => (Measure::Bytes, usage.memory.bytes() as f64),
        _ => return None,
    };
    let quantity = |text: &Option<String>| {
        let text = text.as_deref()?;
        match measure {
            Measure::Cpu => CpuAmount::parse(text).map(CpuAmount::cores),
            Measure::Bytes => ByteAmount::parse(text).map(|bytes| bytes.bytes() as f64),
        }
    };
    let request = quantity(&resource.request);
    // A zero limit has no ratio, so it reads like no limit.
    let limit = quantity(&resource.limit).filter(|limit| *limit > 0.);
    let Some(limit) = limit else {
        let request = request.map_or_else(
            || "no request".to_owned(),
            |request| format!("request {}", measure.format(request)),
        );
        return Some(UsageRow {
            value: format!("{} used", measure.format(used)),
            tone: None,
            bar: None,
            note: Some(format!("{request} · no limit")),
        });
    };
    let ratio = used / limit;
    Some(UsageRow {
        value: measure.format_pair(used, limit, " of "),
        tone: usage_tone(ratio),
        bar: Some(UsageBar::of_ratio(
            ratio,
            request.map(|request| request / limit),
        )),
        note: request.map(|request| format!("request {}", measure.format(request))),
    })
}

/// The index of the container the Containers tab shows: the clicked one, else the default.
pub(crate) fn selected_container_index(pod: &PodSummary, state: &DrawerState) -> Option<usize> {
    state
        .selected_container
        .as_deref()
        .and_then(|name| {
            pod.containers
                .iter()
                .position(|container| container.name == name)
        })
        .or_else(|| default_container(&pod.containers))
}

fn containers_tab(
    pod: &PodSummary,
    state: &DrawerState,
    events: Option<&[EventSummary]>,
    forward_reason: &SharedString,
    live: Option<&LiveCluster>,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> AnyElement {
    let Some(selected) = selected_container_index(pod, state) else {
        return absent_text(cx).into_any_element();
    };
    let list = container_list(&pod.containers, selected, cx);
    let detail = container_detail(
        &ContainerDetailInput {
            pod,
            container: &pod.containers[selected],
            tab: state.container_tab,
            events,
            forward_reason,
            usage: live.and_then(|live| {
                let name = &pod.containers[selected].name;
                live.metrics
                    .pods
                    .history
                    .latest_container(&pod.namespace, &pod.name, name)
            }),
            kubelet: live.map(|live| &live.metrics.kubelet.history),
            monitor: live.map(|live| MonitorView::of_container(state, live)),
            now,
        },
        cx,
    );
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

#[cfg(test)]
#[path = "pod_drawer_tests.rs"]
mod pod_drawer_tests;
