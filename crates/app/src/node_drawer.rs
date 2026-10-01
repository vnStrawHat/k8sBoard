use std::rc::Rc;

use cluster::NodeSummary;
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, IntoElement, ParentElement as _, Styled as _, div,
};

use crate::age::format_age;
use crate::app_shell::AppShell;
use crate::cluster_session::ClusterSession;
use crate::drawer::{
    DRAWER_WIDTH, DrawerHeader, absent_text, created_text, detail_row, drawer_frame, menu_button,
    truncated_text, value_or_absent,
};
use crate::resource_actions::node_menu;
use crate::status_tone::{node_status_label, toned_text};
use crate::table_selection::ResourceKey;

pub(crate) fn node_drawer(
    node: &NodeSummary,
    session: &Entity<ClusterSession>,
    cx: &Context<AppShell>,
) -> AnyElement {
    let now = jiff::Timestamp::now();
    let header = DrawerHeader {
        kind_badge: "No",
        name: node.name.clone().into(),
        subtitle: subtitle(node, now, cx),
        menu: node_menu_button(node, session),
        // One column is enough for a node, so there is nothing to expand.
        expand: None,
        on_close: Rc::new(cx.listener(|shell, _, _, cx| shell.close_drawer(cx))),
    };
    drawer_frame(header, None, body(node, now, cx), DRAWER_WIDTH, cx).into_any_element()
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
fn node_menu_button(node: &NodeSummary, session: &Entity<ClusterSession>) -> AnyElement {
    let session = session.clone();
    let key = ResourceKey::of_node(node);
    menu_button()
        .dropdown_menu(move |menu, _, cx| {
            let Some(live) = session.read(cx).live() else {
                return menu;
            };
            match live.nodes.items().iter().find(|node| key.is_node(node)) {
                Some(node) => node_menu(menu, node, &live.access),
                None => menu,
            }
        })
        .into_any_element()
}

fn body(node: &NodeSummary, now: jiff::Timestamp, cx: &App) -> AnyElement {
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
    v_flex()
        .child(detail_row(
            "Status",
            toned_text(node_status_label(node.status), cx),
            cx,
        ))
        .child(detail_row(
            "Roles",
            value_or_absent(roles.as_deref(), cx),
            cx,
        ))
        .child(detail_row("Taints", taints, cx))
        .child(detail_row(
            "Kubelet version",
            div()
                .truncate()
                .font_family(mono.clone())
                .child(node.kubelet_version.clone()),
            cx,
        ))
        .child(detail_row(
            "Internal IP",
            match &node.internal_ip {
                Some(ip) => div()
                    .truncate()
                    .font_family(mono)
                    .child(ip.clone())
                    .into_any_element(),
                None => absent_text(cx).into_any_element(),
            },
            cx,
        ))
        .child(detail_row(
            "Created",
            value_or_absent(created.as_deref(), cx),
            cx,
        ))
        .into_any_element()
}
