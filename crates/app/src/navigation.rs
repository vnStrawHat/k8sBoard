use std::collections::HashMap;

use cluster::{CrdSummary, NamespaceScope};
use gpui_kit::assets::IconName;
use gpui_kit::component::sidebar::{Sidebar, SidebarMenu, SidebarMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use crate::app_shell::{AppShell, Screen};
use crate::cluster_session::{AccessState, CustomGate, LiveCluster, LiveList, namespaces_label};
use crate::custom_kind::CustomKind;
use crate::resource_kind::ResourceKind;

pub(crate) const SIDEBAR_WIDTH: Pixels = px(220.);

/// The items above the groups.
const TOP_ITEMS: [&str; 3] = ["Overview", "Issues", "Topology"];

/// The section whose items are followed by one submenu per API group of the custom kinds.
const CUSTOM_RESOURCES: &str = "Custom Resources";

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
        _ => ResourceKind::from_label(item).map(Screen::Kind),
    }
}

/// Snapshot lengths shown next to Pods, Nodes, and the visible kind; `None` while a list is not
/// loaded. Other kinds show their counted number, when they have one.
pub(crate) struct NavigationCounts {
    pub(crate) pods: Option<usize>,
    pub(crate) nodes: Option<usize>,
    pub(crate) explorer: Option<(ResourceKind, usize)>,
    /// Counted numbers of kinds, from one-shot requests.
    pub(crate) kinds: HashMap<ResourceKind, usize>,
}

impl NavigationCounts {
    /// The number of `kind`: the live list of the visible screen wins over a counted one.
    fn of_kind(&self, kind: ResourceKind) -> Option<usize> {
        let live = self
            .explorer
            .filter(|(listed, _)| *listed == kind)
            .map(|(_, count)| count);
        live.or_else(|| self.kinds.get(&kind).copied())
    }
}

/// Whether a kind item can be opened.
#[derive(Clone, Debug, PartialEq, Eq)]
enum KindAvailability {
    Enabled,
    Denied { reason: SharedString },
}

/// A kind is disabled only when the access review is known and denies listing it. While the
/// review runs or has failed the item stays enabled; the list's error state explains a 403.
fn kind_availability(
    kind: ResourceKind,
    access: &AccessState,
    scope: &NamespaceScope,
) -> KindAvailability {
    // A custom kind has no list check here: its review is per resource, and `item` reads its gate.
    let Some(check) = kind.access_check() else {
        return KindAvailability::Enabled;
    };
    let AccessState::Known(report) = access else {
        return KindAvailability::Enabled;
    };
    if report.is_allowed(check) {
        return KindAvailability::Enabled;
    }
    let reason = match scope {
        // The review asked cluster-wide, so say so.
        NamespaceScope::All if kind.is_namespaced() => {
            format!("Not permitted: {check} in all namespaces")
        }
        NamespaceScope::Several(names) if kind.is_namespaced() => {
            format!("Not permitted: {check} in {}", namespaces_label(names))
        }
        NamespaceScope::All | NamespaceScope::Named(_) | NamespaceScope::Several(_) => {
            format!("Not permitted: {check}")
        }
    };
    KindAvailability::Denied {
        reason: reason.into(),
    }
}

/// Whether a group starts open: the open-by-default groups, and the group that holds the active
/// screen so its entry is never hidden. The kit keeps the toggle state of a group after its first
/// render, so this decides the state at launch only.
fn is_section_open(section: &NavigationSection, active: Screen) -> bool {
    section.is_open_by_default
        || section
            .items
            .iter()
            .any(|name| screen_of(name) == Some(active))
        || (section.name == CUSTOM_RESOURCES
            && matches!(active, Screen::Kind(ResourceKind::Custom(_))))
}

pub(crate) fn sidebar(
    active: Screen,
    counts: &NavigationCounts,
    live: Option<&LiveCluster>,
    cx: &Context<AppShell>,
) -> impl IntoElement {
    let top =
        SidebarMenu::new().children(TOP_ITEMS.map(|name| SidebarMenuItem::new(name).disable(true)));
    let sections = SidebarMenu::new().children(SECTIONS.iter().map(|section| {
        SidebarMenuItem::new(section.name)
            .default_open(is_section_open(section, active))
            .click_to_toggle(true)
            .children(
                section
                    .items
                    .iter()
                    .map(|name| item(name, active, counts, live, cx))
                    .chain(custom_groups(section, active, counts, live, cx)),
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
    live: Option<&LiveCluster>,
    cx: &Context<AppShell>,
) -> SidebarMenuItem {
    screen_item(name, screen_of(name), active, counts, live, cx)
}

/// One API-group submenu per group of the custom kinds, under Custom Resources, kinds sorted by
/// label. The group of the shown kind starts open. Nothing before the CRD list has loaded.
fn custom_groups(
    section: &NavigationSection,
    active: Screen,
    counts: &NavigationCounts,
    live: Option<&LiveCluster>,
    cx: &Context<AppShell>,
) -> Vec<SidebarMenuItem> {
    if section.name != CUSTOM_RESOURCES {
        return Vec::new();
    }
    let Some(crds) = live.and_then(|live| live.crds.as_ref()) else {
        return Vec::new();
    };
    kind_groups(ready_kinds(&crds.list, &crds.kinds))
        .into_iter()
        .map(|(group, kinds)| {
            let is_open = kinds
                .iter()
                .any(|kind| Screen::Kind(ResourceKind::Custom(*kind)) == active);
            SidebarMenuItem::new(group_label(group))
                .default_open(is_open)
                .click_to_toggle(true)
                .children(kinds.into_iter().map(|kind| {
                    let screen = Screen::Kind(ResourceKind::Custom(kind));
                    screen_item(kind.spec().label, Some(screen), active, counts, live, cx)
                }))
        })
        .collect()
}

/// The kinds the sidebar may group: none until the CRD list has loaded, so a stale set never
/// shows while the list is loading or failed.
fn ready_kinds<'a>(list: &LiveList<CrdSummary>, kinds: &'a [CustomKind]) -> &'a [CustomKind] {
    if list.ready_count().is_some() {
        kinds
    } else {
        &[]
    }
}

/// Longest API group name the sidebar shows whole: more would run under the chevron.
const MAX_GROUP_LABEL_CHARS: usize = 20;

/// The group name, ending in an ellipsis when it is too long for the sidebar.
fn group_label(group: &str) -> String {
    if group.chars().count() <= MAX_GROUP_LABEL_CHARS {
        return group.to_owned();
    }
    let mut label: String = group.chars().take(MAX_GROUP_LABEL_CHARS - 1).collect();
    label.push('…');
    label
}

/// The kinds grouped by API group, in the order given (sorted by group).
fn kind_groups(kinds: &[CustomKind]) -> Vec<(&'static str, Vec<CustomKind>)> {
    let mut groups: Vec<(&'static str, Vec<CustomKind>)> = Vec::new();
    for kind in kinds {
        let group = kind.resource().group.as_str();
        match groups.last_mut() {
            Some((last, members)) if *last == group => members.push(*kind),
            _ => groups.push((group, vec![*kind])),
        }
    }
    groups
}

/// A kind is denied by the access review (built-in) or by its own list review (custom).
fn denial(kind: ResourceKind, live: &LiveCluster) -> Option<SharedString> {
    if let Some(custom) = kind.custom() {
        return match live.custom_gates.get(&custom) {
            Some(CustomGate::Denied { reason }) => Some(reason.clone().into()),
            _ => None,
        };
    }
    match kind_availability(kind, &live.access, &live.scope) {
        KindAvailability::Enabled => None,
        KindAvailability::Denied { reason } => Some(reason),
    }
}

fn screen_item(
    name: &'static str,
    screen: Option<Screen>,
    active: Screen,
    counts: &NavigationCounts,
    live: Option<&LiveCluster>,
    cx: &Context<AppShell>,
) -> SidebarMenuItem {
    let Some(screen) = screen else {
        return SidebarMenuItem::new(name).disable(true);
    };
    if let Screen::Kind(kind) = screen
        && let Some(live) = live
        && let Some(reason) = denial(kind, live)
    {
        return denied_item(name, reason, screen == active);
    }
    let count = match screen {
        Screen::Pods => counts.pods,
        Screen::Nodes => counts.nodes,
        Screen::Kind(kind) => counts.of_kind(kind),
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

/// A greyed-out item with a lock whose tooltip says what is missing.
fn denied_item(name: &'static str, reason: SharedString, is_active: bool) -> SidebarMenuItem {
    SidebarMenuItem::new(name)
        .active(is_active)
        .disable(true)
        .suffix(move |_, cx| {
            let reason = reason.clone();
            div()
                .id(name)
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::Lock))
                .tooltip(move |window, cx| Tooltip::new(reason.clone()).build(window, cx))
        })
}

#[cfg(test)]
mod tests {
    use cluster::{AccessCheck, AccessDecision, AccessReport, AccessReview};
    use gpui_kit::Task;

    use super::*;

    #[test]
    fn enabled_items_are_pods_nodes_and_explorer_kinds() {
        let enabled: Vec<&str> = TOP_ITEMS
            .iter()
            .chain(SECTIONS.iter().flat_map(|section| section.items.iter()))
            .copied()
            .filter(|name| screen_of(name).is_some())
            .collect();
        assert_eq!(
            enabled,
            [
                "Nodes",
                "Namespaces",
                "Events",
                "Pods",
                "Deployments",
                "StatefulSets",
                "DaemonSets",
                "ReplicaSets",
                "Jobs",
                "CronJobs",
                "Services",
                "Ingresses",
                "NetworkPolicies",
                "ConfigMaps",
                "Secrets",
                "HPAs",
                "ResourceQuotas",
                "PDBs",
                "PVCs",
                "PVs",
                "StorageClasses",
                "ServiceAccounts",
                "Roles",
                "ClusterRoles",
                "RoleBindings",
                "ClusterRoleBindings",
                "Releases",
                "CRDs",
            ]
        );
    }

    #[test]
    fn the_group_of_the_active_screen_starts_open() {
        let is_open = |name: &str, active: Screen| {
            let section = SECTIONS
                .iter()
                .find(|section| section.name == name)
                .expect("section exists");
            is_section_open(section, active)
        };
        let policies = Screen::Kind(ResourceKind::NetworkPolicies);
        assert!(is_open("Network", policies));
        assert!(!is_open("Config", policies));
        assert!(is_open(
            "Config",
            Screen::Kind(ResourceKind::PodDisruptionBudgets)
        ));
        // The groups that open by default stay open, whatever is active.
        assert!(is_open("Workloads", policies));
        assert!(!is_open("Network", Screen::Pods));
    }

    #[test]
    fn kind_labels_match_navigation_items() {
        let items: Vec<&str> = SECTIONS
            .iter()
            .flat_map(|section| section.items.iter())
            .copied()
            .collect();
        for kind in ResourceKind::ALL {
            assert!(items.contains(&kind.label()), "{}", kind.label());
            assert_eq!(screen_of(kind.label()), Some(Screen::Kind(kind)));
        }
    }

    fn report_denying(denied: &[AccessCheck]) -> AccessState {
        let reviews = AccessCheck::ALL
            .into_iter()
            .map(|check| AccessReview {
                check,
                decision: if denied.contains(&check) {
                    AccessDecision::Denied { reason: None }
                } else {
                    AccessDecision::Allowed
                },
            })
            .collect();
        AccessState::Known(AccessReport { reviews })
    }

    fn served(group: &str, plural: &str) -> CustomKind {
        let crd = cluster::CrdSummary {
            name: format!("{plural}.{group}"),
            group: group.to_owned(),
            kind: plural.trim_end_matches('s').to_owned(),
            plural: plural.to_owned(),
            singular: plural.trim_end_matches('s').to_owned(),
            scope: cluster::ResourceScope::Namespaced,
            versions: vec![cluster::CrdVersion {
                name: "v1".to_owned(),
                is_served: true,
                is_storage: true,
                is_deprecated: false,
                deprecation_warning: None,
                printer_columns: Vec::new(),
                schema: cluster::SchemaOutline::default(),
            }],
            state: cluster::CrdState::Established,
            created_at: None,
        };
        crate::custom_kind::custom_kinds(
            &[crd],
            &mut crate::custom_kind::CustomKindCache::default(),
        )[0]
    }

    #[test]
    fn custom_kinds_group_by_api_group() {
        let kinds = [
            served("a.io", "alphas"),
            served("a.io", "betas"),
            served("b.io", "gammas"),
        ];
        let groups: Vec<_> = kind_groups(&kinds)
            .into_iter()
            .map(|(group, members)| (group, members.len()))
            .collect();
        assert_eq!(groups, [("a.io", 2), ("b.io", 1)]);
        assert!(kind_groups(&[]).is_empty());
    }

    #[test]
    fn no_groups_without_a_ready_crd_list() {
        let kinds = [served("a.io", "alphas")];
        let failed = LiveList::<CrdSummary>::Failed {
            message: "denied".to_owned(),
        };
        let ready = LiveList::<CrdSummary>::Ready {
            items: Vec::new(),
            interruption: None,
        };
        assert!(ready_kinds(&LiveList::Loading, &kinds).is_empty());
        assert!(ready_kinds(&failed, &kinds).is_empty());
        assert_eq!(ready_kinds(&ready, &kinds).len(), 1);
    }

    #[test]
    fn long_group_labels_end_in_an_ellipsis() {
        assert_eq!(group_label("argoproj.io"), "argoproj.io");
        let exact = "a".repeat(MAX_GROUP_LABEL_CHARS);
        assert_eq!(group_label(&exact), exact);
        let label = group_label("clickhouse.altinity.com");
        assert_eq!(label.chars().count(), MAX_GROUP_LABEL_CHARS);
        assert!(label.ends_with('…'), "{label}");
        assert!(label.starts_with("clickhouse.altinity"));
    }

    #[test]
    fn crds_item_resolves_through_label() {
        assert_eq!(screen_of("CRDs"), Some(Screen::Kind(ResourceKind::Crds)));
    }

    #[test]
    fn custom_kinds_have_no_list_check_to_deny() {
        let kind = ResourceKind::Custom(served("a.io", "alphas"));
        let access = report_denying(&AccessCheck::ALL);
        assert_eq!(
            kind_availability(kind, &access, &NamespaceScope::All),
            KindAvailability::Enabled
        );
    }

    #[test]
    fn crds_are_denied_by_their_list_check() {
        let access = report_denying(&[AccessCheck::ListCustomResourceDefinitions]);
        assert_eq!(
            kind_availability(ResourceKind::Crds, &access, &NamespaceScope::All),
            KindAvailability::Denied {
                reason: "Not permitted: list customresourcedefinitions".into()
            }
        );
    }

    #[test]
    fn the_custom_resources_section_opens_for_a_custom_screen() {
        let section = SECTIONS
            .iter()
            .find(|section| section.name == CUSTOM_RESOURCES)
            .expect("section exists");
        let custom = Screen::Kind(ResourceKind::Custom(served("a.io", "alphas")));
        assert!(is_section_open(section, custom));
        assert!(!is_section_open(section, Screen::Pods));
    }

    fn denied(reason: &str) -> KindAvailability {
        KindAvailability::Denied {
            reason: reason.to_owned().into(),
        }
    }

    #[test]
    fn denied_kind_reason_names_all_namespaces_scope() {
        let access = report_denying(&[
            AccessCheck::ListDeployments,
            AccessCheck::ListNamespaces,
            AccessCheck::ListEvents,
        ]);
        let all = NamespaceScope::All;
        let named = NamespaceScope::Named("team-a".to_owned());
        assert_eq!(
            kind_availability(ResourceKind::Deployments, &access, &all),
            denied("Not permitted: list deployments in all namespaces")
        );
        assert_eq!(
            kind_availability(ResourceKind::Deployments, &access, &named),
            denied("Not permitted: list deployments")
        );
        assert_eq!(
            kind_availability(ResourceKind::Events, &access, &all),
            denied("Not permitted: list events in all namespaces")
        );
        // Namespaces are cluster-scoped, so the scope never applies to them.
        assert_eq!(
            kind_availability(ResourceKind::Namespaces, &access, &all),
            denied("Not permitted: list namespaces")
        );
        assert_eq!(
            kind_availability(ResourceKind::Deployments, &report_denying(&[]), &all),
            KindAvailability::Enabled
        );
    }

    #[test]
    fn kind_availability_names_the_picked_namespaces_for_several() {
        let access = report_denying(&[AccessCheck::ListDeployments, AccessCheck::ListNamespaces]);
        let several = NamespaceScope::of_namespaces(["b".to_owned(), "a".to_owned()]);
        assert_eq!(
            kind_availability(ResourceKind::Deployments, &access, &several),
            denied("Not permitted: list deployments in a, b")
        );
        assert_eq!(
            kind_availability(ResourceKind::Namespaces, &access, &several),
            denied("Not permitted: list namespaces")
        );
    }

    #[test]
    fn checking_and_unknown_access_keep_kinds_enabled() {
        let checking = AccessState::Checking {
            _task: Task::ready(()),
        };
        for access in [checking, AccessState::Unknown] {
            for kind in ResourceKind::ALL {
                assert_eq!(
                    kind_availability(kind, &access, &NamespaceScope::All),
                    KindAvailability::Enabled
                );
            }
        }
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

    #[test]
    fn live_count_wins_over_counted() {
        let counts = NavigationCounts {
            pods: None,
            nodes: None,
            explorer: Some((ResourceKind::Services, 71)),
            kinds: HashMap::from([
                (ResourceKind::Services, 70),
                (ResourceKind::Deployments, 31),
            ]),
        };
        // The visible kind shows its live list; the others show what was counted.
        assert_eq!(counts.of_kind(ResourceKind::Services), Some(71));
        assert_eq!(counts.of_kind(ResourceKind::Deployments), Some(31));
        assert_eq!(counts.of_kind(ResourceKind::Jobs), None);
    }
}
