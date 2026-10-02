use std::rc::Rc;

use cluster::{NodeCondition, NodeSummary, NodeSystemInfo};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _,
};

use crate::age::format_age;
use crate::app_shell::AppShell;
use crate::cluster_session::{ClusterSession, LiveCluster};
use crate::container_detail::resource_label;
use crate::drawer::{
    DrawerBody, DrawerHeader, DrawerState, DrawerTab, WIDE_LABEL_WIDTH, absent_text, chips,
    created_text, drawer_frame, drawer_tab_bar, drawer_tabs, expand_toggle, menu_button,
    section_title, shown_tab, tab_titles, truncated_text, value_or_absent, wide_detail_row,
    yaml_body,
};
use crate::kind_row::PodOwner;
use crate::object_events::{event_subject, recent_events};
use crate::related_pods::pods_section;
use crate::resource_actions::node_menu;
use crate::status_tone::{
    StatusLabel, condition_status_text, node_condition_tone, node_status_label, toned_text,
};
use crate::table_selection::ResourceKey;

pub(crate) fn node_drawer(
    node: &NodeSummary,
    state: &DrawerState,
    session: &Entity<ClusterSession>,
    cx: &Context<AppShell>,
) -> AnyElement {
    let now = jiff::Timestamp::now();
    let header = DrawerHeader {
        kind_badge: "No",
        name: node.name.clone().into(),
        subtitle: subtitle(node, now, cx),
        menu: node_menu_button(node, session, cx.weak_entity()),
        expand: expand_toggle(state, cx),
        on_close: Rc::new(cx.listener(|shell, _, _, cx| shell.close_drawer(cx))),
    };
    let key = ResourceKey::of_node(node);
    let events =
        event_subject(&key).and_then(|subject| session.read(cx).live()?.events_of(&subject));
    let tabs = drawer_tabs(&key);
    let shown = shown_tab(tabs, state.tab);
    let body = match shown {
        DrawerTab::Events => DrawerBody::Scrolling(recent_events(events, cx)),
        DrawerTab::Yaml => yaml_body(state),
        DrawerTab::Overview | DrawerTab::Containers => {
            DrawerBody::Scrolling(overview(node, session.read(cx).live(), now, cx))
        }
    };
    let tab_bar = drawer_tab_bar(tab_titles(tabs, 0, events), shown, cx);
    drawer_frame(header, tab_bar, body, state.width(), cx).into_any_element()
}

fn subtitle(node: &NodeSummary, now: jiff::Timestamp, cx: &App) -> AnyElement {
    h_flex()
        .gap_1()
        .text_sm()
        .child(toned_text(node_status_label(node.status), cx))
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
    shell: WeakEntity<AppShell>,
) -> AnyElement {
    let session = session.clone();
    let key = ResourceKey::of_node(node);
    menu_button()
        .dropdown_menu(move |menu, _, cx| {
            let Some(live) = session.read(cx).live() else {
                return menu;
            };
            match live.nodes.items().iter().find(|node| key.is_node(node)) {
                Some(node) => node_menu(menu, node, live, &shell),
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
        .child(section_title("Node", cx))
        .child(wide_detail_row(
            "Status",
            toned_text(node_status_label(node.status), cx),
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
        .child(section_title("Conditions", cx));
    if node.conditions.is_empty() {
        column = column.child(absent_text(cx));
    }
    for (index, condition) in node.conditions.iter().enumerate() {
        column = column.child(condition_row(index, condition, now, cx));
    }

    column = column.child(section_title("Addresses", cx));
    if node.addresses.is_empty() {
        column = column.child(absent_text(cx));
    }
    for (index, address) in node.addresses.iter().enumerate() {
        column = column.child(wide_detail_row(
            address.kind.clone(),
            truncated_text(("address", index), address.address.clone()).font_family(mono.clone()),
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
    let quantity = |value: &Option<String>| value.as_deref().unwrap_or("—").to_owned();
    for resource in &node.resources {
        column = column.child(resource_row(
            ResourceRowKind::Quantity,
            ResourceCells {
                name: &resource_label(&resource.name),
                capacity: &quantity(&resource.capacity),
                allocatable: &quantity(&resource.allocatable),
            },
            cx,
        ));
    }

    if let Some(live) = live {
        column = column.child(pods_section(
            &PodOwner::Node {
                name: node.name.clone(),
            },
            live,
            cx,
        ));
    }
    column
        .child(section_title("Labels", cx))
        .child(chips(
            &node
                .labels
                .iter()
                .map(|label| SharedString::from(label.clone()))
                .collect::<Vec<_>>(),
            cx,
        ))
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
