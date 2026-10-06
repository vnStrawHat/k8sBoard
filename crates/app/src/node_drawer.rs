use std::rc::Rc;

use cluster::{CpuAmount, NodeCondition, NodeSummary, NodeSystemInfo, ResourceUsage};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _, relative,
};

use crate::age::format_age;
use crate::app_shell::AppShell;
use crate::clipboard_copy::copyable_mono;
use crate::cluster_metrics::FeedStatus;
use crate::cluster_session::{ClusterSession, LiveCluster};
use crate::container_detail::resource_label;
use crate::drawer::{
    DrawerBody, DrawerHeader, DrawerNavigation, DrawerSize, DrawerState, DrawerTab,
    WIDE_LABEL_WIDTH, absent_text, chips, created_text, drawer_frame, drawer_tab_bar, drawer_tabs,
    first_section_title, menu_button, section_title, shown_tab, tab_titles, truncated_text,
    value_or_absent, wide_detail_row, yaml_body,
};
use crate::kind_row::{KindObject, PodOwner};
use crate::monitor_tab::{MonitorView, monitor_tab};
use crate::node_usage::{
    node_allocatable, node_pod_count, node_pod_limit, node_quantity_text, node_requests,
};
use crate::object_events::{event_subject, recent_events};
use crate::related_pods::pods_section;
use crate::resource_actions::node_menu;
use crate::resource_kind::NODE_ICON;
use crate::row_context::RowContext;
use crate::status_tone::{
    StatusLabel, condition_status_text, node_condition_tone, node_status_label, scheduling_label,
    toned_text,
};
use crate::table_selection::ResourceKey;
use crate::usage_bar::{UsageBar, usage_bar};
use crate::usage_format::Measure;

pub(crate) fn node_drawer(
    node: &NodeSummary,
    state: &DrawerState,
    session: &Entity<ClusterSession>,
    row: &RowContext,
    navigation: DrawerNavigation,
    cx: &Context<AppShell>,
) -> AnyElement {
    let now = jiff::Timestamp::now();
    let header = DrawerHeader {
        kind_icon: NODE_ICON,
        kind_name: "Node".into(),
        name: node.name.clone().into(),
        subtitle: subtitle(node, now, cx),
        menu: node_menu_button(node, session, row, cx.weak_entity()),
        on_close: Rc::new(cx.listener(|shell, _, _, cx| shell.close_drawer(cx))),
        navigation,
    };
    let key = ResourceKey::of_node(node);
    let events =
        event_subject(&key).and_then(|subject| session.read(cx).live()?.events_of(&subject));
    let tabs = drawer_tabs(&key);
    let shown = shown_tab(tabs, state.tab);
    let body = match shown {
        DrawerTab::Events => DrawerBody::Scrolling(recent_events(events, cx)),
        DrawerTab::Monitor => DrawerBody::Scrolling(match session.read(cx).live() {
            Some(live) => monitor_tab(&MonitorView::of_nodes(state, live), cx),
            None => div().into_any_element(),
        }),
        DrawerTab::Yaml => yaml_body(state),
        // A node drawer has no Helm tabs, so `shown_tab` never yields them.
        DrawerTab::Overview
        | DrawerTab::Containers
        | DrawerTab::Values
        | DrawerTab::Manifest
        | DrawerTab::Notes => {
            DrawerBody::Scrolling(overview(node, session.read(cx).live(), now, cx))
        }
    };
    let tab_bar = drawer_tab_bar(tab_titles(tabs, 0, events), shown, cx);
    drawer_frame(header, tab_bar, body, state.width(DrawerSize::Standard), cx).into_any_element()
}

fn subtitle(node: &NodeSummary, now: jiff::Timestamp, cx: &App) -> AnyElement {
    h_flex()
        .gap_1()
        .text_sm()
        .child(toned_text(
            node_status_label(node.status, &node.conditions),
            cx,
        ))
        .child(
            div()
                .text_color(cx.theme().muted_foreground)
                .children(created_text(node.created_at, now).map(|created| format!("· {created}"))),
        )
        .into_any_element()
}

/// The menu reads the session when it opens, so it shows the access state of that moment.
fn node_menu_button(
    node: &NodeSummary,
    session: &Entity<ClusterSession>,
    row: &RowContext,
    shell: WeakEntity<AppShell>,
) -> AnyElement {
    // Weak: a rendered menu closure must not keep a session alive after a cluster switch.
    let session = session.downgrade();
    let row = row.clone();
    let key = ResourceKey::of_node(node);
    menu_button()
        .dropdown_menu(move |menu, _, cx| {
            let Some(session) = session.upgrade() else {
                return menu;
            };
            let session = session.read(cx);
            let (Some(live), Some(guard)) = (session.live(), session.guard(cx)) else {
                return menu;
            };
            match live.nodes.items().iter().find(|node| key.is_node(node)) {
                Some(node) => node_menu(menu, node, live, &guard, &row, &shell),
                None => menu,
            }
        })
        .into_any_element()
}

fn overview(
    node: &NodeSummary,
    live: Option<&LiveCluster>,
    now: jiff::Timestamp,
    cx: &Context<AppShell>,
) -> AnyElement {
    let mono = cx.theme().mono_font_family.clone();
    let taints = if node.taints.is_empty() {
        absent_text(cx).into_any_element()
    } else {
        v_flex()
            .font_family(mono.clone())
            .children(
                node.taints
                    .iter()
                    .enumerate()
                    .map(|(index, taint)| truncated_text(("taint", index), taint.to_string())),
            )
            .into_any_element()
    };
    let roles = (!node.roles.is_empty()).then(|| node.roles.join(", "));
    let created = node
        .created_at
        .map(|created_at| format!("{created_at} ({} ago)", format_age(Some(created_at), now)));
    let mut column = v_flex()
        .child(first_section_title("Node", cx))
        .child(wide_detail_row(
            "Status",
            toned_text(node_status_label(node.status, &node.conditions), cx),
            cx,
        ))
        .child(wide_detail_row(
            "Scheduling",
            toned_text(scheduling_label(node.status.scheduling), cx),
            cx,
        ))
        .child(wide_detail_row(
            "Roles",
            value_or_absent(roles.as_deref(), cx),
            cx,
        ))
        .child(wide_detail_row("Taints", taints, cx))
        .child(wide_detail_row(
            "Created",
            value_or_absent(created.as_deref(), cx),
            cx,
        ))
        .child(section_title("Labels", cx))
        .child(chips(
            "node-labels",
            &node
                .labels
                .iter()
                .map(|label| SharedString::from(label.clone()))
                .collect::<Vec<_>>(),
            cx,
        ))
        .child(section_title("Conditions", cx));
    if node.conditions.is_empty() {
        column = column.child(absent_text(cx));
    }
    for (index, condition) in node.conditions.iter().enumerate() {
        column = column.child(condition_row(index, condition, now, cx));
    }

    column = column
        .child(section_title("Allocatable used", cx))
        .child(allocatable_used(node, live, cx));

    // What runs here comes right after what it uses, before the node's addresses.
    if let Some(live) = live {
        column = column.child(pods_section(
            &PodOwner::Node {
                name: node.name.clone(),
            },
            &KindObject::Plain,
            live,
            cx,
        ));
    }

    column = column.child(section_title("Addresses", cx));
    if node.addresses.is_empty() {
        column = column.child(absent_text(cx));
    }
    for (index, address) in node.addresses.iter().enumerate() {
        column = column.child(wide_detail_row(
            address.kind.clone(),
            copyable_mono(("address", index), address.address.clone(), cx),
            cx,
        ));
    }

    let os = os_text(&node.system);
    column = column
        .child(section_title("System", cx))
        .child(wide_detail_row(
            "OS",
            value_or_absent(os.as_deref(), cx),
            cx,
        ))
        .child(wide_detail_row(
            "Kernel",
            mono_or_absent(&node.system.kernel_version, "kernel", cx),
            cx,
        ))
        .child(wide_detail_row(
            "Container runtime",
            mono_or_absent(&node.system.container_runtime, "runtime", cx),
            cx,
        ))
        .child(wide_detail_row(
            "Kubelet",
            mono_or_absent(&node.kubelet_version, "kubelet", cx),
            cx,
        ))
        .child(section_title("Resources", cx));
    if node.resources.is_empty() {
        column = column.child(absent_text(cx));
    } else {
        column = column.child(resource_row(
            ResourceRowKind::Header,
            ResourceCells {
                name: "Resource",
                capacity: "Capacity",
                allocatable: "Allocatable",
            },
            cx,
        ));
    }
    let quantity = |name: &str, value: &Option<String>| {
        value
            .as_deref()
            .map_or_else(|| "—".to_owned(), |text| node_quantity_text(name, text))
    };
    for resource in &node.resources {
        column = column.child(resource_row(
            ResourceRowKind::Quantity,
            ResourceCells {
                name: &resource_label(&resource.name),
                capacity: &quantity(&resource.name, &resource.capacity),
                allocatable: &quantity(&resource.name, &resource.allocatable),
            },
            cx,
        ));
    }

    column.into_any_element()
}

/// What the pods on a node request and how many there are; only known for the All scope, since
/// a namespace scope would understate both.
struct NodePods {
    cpu_request: CpuAmount,
    memory_request: cluster::ByteAmount,
    count: usize,
}

/// One line of the "Allocatable used" section.
#[derive(Debug, PartialEq)]
struct AllocatableRow {
    label: &'static str,
    value: String,
    bar: Option<UsageBar>,
    /// Names the numbers behind the bar: used, requested, and allocatable.
    tooltip: Option<String>,
}

/// CPU, Memory, and (with `pods`) Pods rows. A value without a sample or without an allocatable
/// is "—" and has no bar. Requests are part of the value when the pods are known: a node can be
/// idle and still full, and only the requests say so.
fn allocatable_rows(
    node: &NodeSummary,
    latest: Option<ResourceUsage>,
    pods: Option<&NodePods>,
) -> Vec<AllocatableRow> {
    let (cpu, memory) = node_allocatable(node);
    let share = |label: &'static str,
                 measure: Measure,
                 used: Option<f64>,
                 total: Option<f64>,
                 request: Option<f64>| {
        let (Some(used), Some(total)) = (used, total.filter(|total| *total > 0.)) else {
            return AllocatableRow {
                label,
                value: ABSENT_VALUE.to_owned(),
                bar: None,
                tooltip: None,
            };
        };
        let bar = UsageBar::of_ratio(used / total, request.map(|request| request / total));
        let (value, tooltip) = match request {
            Some(request) => {
                // The unit is printed once, on the last number, when all three share it.
                let mut texts = measure.format_shared(&[used, request, total]).into_iter();
                let mut next = || texts.next().unwrap_or_default();
                let (used, request, total) = (next(), next(), next());
                (
                    format!("{used} used · {request} requested / {total}"),
                    Some(format!(
                        "{label}: {used} used, {request} requested, {total} allocatable"
                    )),
                )
            }
            None => (measure.format_pair(used, total, " / "), None),
        };
        AllocatableRow {
            label,
            value,
            bar: Some(bar),
            tooltip,
        }
    };
    let mut rows = vec![
        share(
            "CPU",
            Measure::Cpu,
            latest.map(|usage| usage.cpu.cores()),
            cpu.map(CpuAmount::cores),
            pods.map(|pods| pods.cpu_request.cores()),
        ),
        share(
            "Memory",
            Measure::Bytes,
            latest.map(|usage| usage.memory.bytes() as f64),
            memory.map(|memory| memory.bytes() as f64),
            pods.map(|pods| pods.memory_request.bytes() as f64),
        ),
    ];
    if let Some(pods) = pods {
        rows.push(pod_count_row(pods.count, node_pod_limit(node)));
    }
    rows
}

fn pod_count_row(count: usize, limit: Option<u64>) -> AllocatableRow {
    let Some(limit) = limit.filter(|limit| *limit > 0) else {
        return AllocatableRow {
            label: "Pods",
            value: count.to_string(),
            bar: None,
            tooltip: None,
        };
    };
    AllocatableRow {
        label: "Pods",
        value: format!("{count} / {limit}"),
        bar: Some(UsageBar::of_ratio(count as f64 / limit as f64, None)),
        tooltip: None,
    }
}

const ABSENT_VALUE: &str = "—";

/// The section body: the rows, or one muted line when the nodes feed has no data to show.
fn allocatable_used(node: &NodeSummary, live: Option<&LiveCluster>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let feed = live.map(|live| &live.metrics.nodes);
    if let Some(FeedStatus::Unavailable(reason) | FeedStatus::Failed(reason)) =
        feed.map(|feed| &feed.status)
    {
        return div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(format!("Usage unavailable: {reason}"))
            .into_any_element();
    }
    let latest = feed.and_then(|feed| feed.history.latest(&node.name));
    // Requests and the pod count come from the pods list, which holds every namespace only when
    // the scope is All.
    let pods = live.and_then(|live| {
        let items = live.pods.ready_items()?;
        if live.scope != cluster::NamespaceScope::All {
            return None;
        }
        let (cpu_request, memory_request) = node_requests(&node.name, items);
        Some(NodePods {
            cpu_request,
            memory_request,
            count: node_pod_count(&node.name, items),
        })
    });
    let mono = theme.mono_font_family.clone();
    v_flex()
        .children(
            allocatable_rows(node, latest, pods.as_ref())
                .into_iter()
                .map(|row| {
                    let value = v_flex()
                        .w_full()
                        .gap_1()
                        .child(div().font_family(mono.clone()).child(row.value))
                        .children(row.bar.map(|bar| {
                            let bar = usage_bar(bar, relative(1.), cx);
                            let Some(text) = row.tooltip else {
                                return bar.into_any_element();
                            };
                            div()
                                .id(row.label)
                                .w_full()
                                .tooltip(move |window, cx| {
                                    Tooltip::new(text.clone()).build(window, cx)
                                })
                                .child(bar)
                                .into_any_element()
                        }));
                    wide_detail_row(row.label, value, cx)
                }),
        )
        .into_any_element()
}

/// A condition: its status in the condition's tone, then why and since when. The message is the
/// tooltip, because it can be long.
fn condition_row(
    index: usize,
    condition: &NodeCondition,
    now: jiff::Timestamp,
    cx: &App,
) -> AnyElement {
    let status = StatusLabel {
        text: condition_status_text(condition.status).into(),
        tone: node_condition_tone(condition),
    };
    let detail = condition_detail(condition, now);
    let value = h_flex()
        .gap_2()
        .child(toned_text(status, cx).flex_shrink_0())
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_color(cx.theme().muted_foreground)
                .child(detail),
        );
    let row = div().id(("node-condition", index)).child(wide_detail_row(
        condition.name.clone(),
        value,
        cx,
    ));
    match condition.message.clone() {
        Some(message) => row
            .tooltip(move |window, cx| Tooltip::new(message.clone()).build(window, cx))
            .into_any_element(),
        None => row.into_any_element(),
    }
}

/// `{reason} · since {age}`; either part may be missing.
fn condition_detail(condition: &NodeCondition, now: jiff::Timestamp) -> String {
    let since = condition
        .changed_at
        .map(|changed_at| format!("since {}", format_age(Some(changed_at), now)));
    [condition.reason.clone(), since]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

/// `linux/amd64 · Ubuntu 22.04.4 LTS`; empty parts and their separators are left out.
fn os_text(system: &NodeSystemInfo) -> Option<String> {
    let platform = [
        system.operating_system.as_str(),
        system.architecture.as_str(),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("/");
    let text = [platform.as_str(), system.os_image.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    (!text.is_empty()).then_some(text)
}

fn mono_or_absent(value: &str, id: &'static str, cx: &App) -> AnyElement {
    if value.is_empty() {
        return absent_text(cx).into_any_element();
    }
    truncated_text(id, value.to_owned())
        .font_family(cx.theme().mono_font_family.clone())
        .into_any_element()
}

#[derive(Clone, Copy, PartialEq)]
enum ResourceRowKind {
    Header,
    Quantity,
}

struct ResourceCells<'a> {
    name: &'a str,
    capacity: &'a str,
    allocatable: &'a str,
}

/// The header or one row of the Resources table. Quantities are mono and shown as written.
fn resource_row(kind: ResourceRowKind, cells: ResourceCells, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let is_header = kind == ResourceRowKind::Header;
    let cell = |text: &str| {
        let cell = div().flex_1().min_w_0().truncate().child(text.to_owned());
        if is_header {
            cell
        } else {
            cell.font_family(theme.mono_font_family.clone())
        }
    };
    h_flex()
        .gap_3()
        .py_1()
        .text_sm()
        .when(is_header, |this| {
            this.text_xs().text_color(theme.muted_foreground)
        })
        .child(
            div()
                .w(WIDE_LABEL_WIDTH)
                .flex_shrink_0()
                .truncate()
                .child(cells.name.to_owned()),
        )
        .child(cell(cells.capacity))
        .child(cell(cells.allocatable))
        .into_any_element()
}

#[cfg(test)]
#[path = "node_drawer_tests.rs"]
mod node_drawer_tests;
