use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::sidebar::{Sidebar, SidebarMenu, SidebarMenuItem};
use gpui_kit::{Context, IntoElement, ParentElement as _, Pixels, Styled as _, div, px};

use crate::app_shell::{AppShell, Screen};

pub(crate) const SIDEBAR_WIDTH: Pixels = px(220.);

/// The items above the groups.
const TOP_ITEMS: [&str; 3] = ["Overview", "Issues", "Topology"];

struct NavigationSection {
    name: &'static str,
    items: &'static [&'static str],
    is_open_by_default: bool,
}

/// The navigation model of the wireframes. Items that are not listed in `screen_of` are
/// shown disabled so users can see what is coming.
const SECTIONS: [NavigationSection; 8] = [
    NavigationSection {
        name: "Cluster",
        items: &["Nodes", "Namespaces", "Events"],
        is_open_by_default: true,
    },
    NavigationSection {
        name: "Workloads",
        items: &[
            "Pods",
            "Deployments",
            "StatefulSets",
            "DaemonSets",
            "ReplicaSets",
            "Jobs",
            "CronJobs",
        ],
        is_open_by_default: true,
    },
    NavigationSection {
        name: "Network",
        items: &[
            "Services",
            "Ingresses",
            "NetworkPolicies",
            "Port Forwarding",
        ],
        is_open_by_default: false,
    },
    NavigationSection {
        name: "Config",
        items: &["ConfigMaps", "Secrets", "HPAs", "ResourceQuotas", "PDBs"],
        is_open_by_default: false,
    },
    NavigationSection {
        name: "Storage",
        items: &["PVCs", "PVs", "StorageClasses"],
        is_open_by_default: false,
    },
    NavigationSection {
        name: "Access Control",
        items: &[
            "ServiceAccounts",
            "Roles",
            "ClusterRoles",
            "RoleBindings",
            "ClusterRoleBindings",
        ],
        is_open_by_default: false,
    },
    NavigationSection {
        name: "Helm",
        items: &["Releases"],
        is_open_by_default: false,
    },
    NavigationSection {
        name: "Custom Resources",
        items: &["CRDs"],
        is_open_by_default: false,
    },
];

/// The screen an item opens, or `None` for an item that is not built yet.
fn screen_of(item: &str) -> Option<Screen> {
    match item {
        "Pods" => Some(Screen::Pods),
        "Nodes" => Some(Screen::Nodes),
        _ => None,
    }
}

/// Snapshot lengths shown next to Pods and Nodes; `None` while a list is not loaded.
pub(crate) struct NavigationCounts {
    pub(crate) pods: Option<usize>,
    pub(crate) nodes: Option<usize>,
}

pub(crate) fn sidebar(
    active: Screen,
    counts: &NavigationCounts,
    cx: &Context<AppShell>,
) -> impl IntoElement {
    let top =
        SidebarMenu::new().children(TOP_ITEMS.map(|name| SidebarMenuItem::new(name).disable(true)));
    let sections = SidebarMenu::new().children(SECTIONS.iter().map(|section| {
        SidebarMenuItem::new(section.name)
            .default_open(section.is_open_by_default)
            .click_to_toggle(true)
            .children(
                section
                    .items
                    .iter()
                    .map(|name| item(name, active, counts, cx)),
            )
    }));
    Sidebar::<SidebarMenu>::new("navigation")
        .w(SIDEBAR_WIDTH)
        .collapsible(false)
        .child(top)
        .child(sections)
}

fn item(
    name: &'static str,
    active: Screen,
    counts: &NavigationCounts,
    cx: &Context<AppShell>,
) -> SidebarMenuItem {
    let Some(screen) = screen_of(name) else {
        return SidebarMenuItem::new(name).disable(true);
    };
    let count = match screen {
        Screen::Pods => counts.pods,
        Screen::Nodes => counts.nodes,
    };
    SidebarMenuItem::new(name)
        .active(screen == active)
        .on_click(cx.listener(move |shell, _, _, cx| shell.show_screen(screen, cx)))
        .suffix(move |_, cx| {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .children(count.map(|count| count.to_string()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pods_and_nodes_are_enabled() {
        let enabled: Vec<&str> = TOP_ITEMS
            .iter()
            .chain(SECTIONS.iter().flat_map(|section| section.items.iter()))
            .copied()
            .filter(|name| screen_of(name).is_some())
            .collect();
        assert_eq!(enabled, ["Nodes", "Pods"]);
    }

    #[test]
    fn cluster_and_workloads_start_open() {
        let open: Vec<&str> = SECTIONS
            .iter()
            .filter(|section| section.is_open_by_default)
            .map(|section| section.name)
            .collect();
        assert_eq!(open, ["Cluster", "Workloads"]);
    }
}
