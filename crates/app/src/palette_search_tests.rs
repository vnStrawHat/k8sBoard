use std::path::PathBuf;

use cluster::{
    AccessCheck, AccessDecision, AccessReport, AccessReview, ContainerKind, ContainerState,
    NamespacePhase, NodeReadiness, NodeScheduling, NodeStatus, PodStatus, StatusReason,
};

use super::*;
use crate::cluster_health::RowHealth;
use crate::cluster_registry::ClusterRef;
use crate::cluster_session::AccessState;
use crate::environment::Environment;
use crate::kind_row::KindObject;
use crate::status_tone::StatusTone;
use crate::table_selection::ClusterObject;
use crate::write_guard::{WriteLock, test_guard};

fn known_denying(denied: &[AccessCheck]) -> AccessState {
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

fn pod(namespace: &str, name: &str) -> PodSummary {
    PodSummary {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        status: PodStatus::Reason(StatusReason::Running),
        ready: cluster::ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: None,
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: None,
        conditions: Vec::new(),
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        image_pull_secrets: Vec::new(),
        containers: vec![cluster::ContainerSummary {
            name: "app".to_owned(),
            image: "img".to_owned(),
            kind: ContainerKind::Main,
            state: ContainerState::Running { started_at: None },
            is_ready: true,
            restart_count: 0,
            last_termination: None,
            image_digest: None,
            pull_policy: None,
            is_started: None,
            ports: Vec::new(),
            resources: Vec::new(),
            probes: cluster::ContainerProbes::default(),
            env: Vec::new(),
            env_from: Vec::new(),
            mounts: Vec::new(),
        }],
    }
}

fn node(name: &str) -> NodeSummary {
    NodeSummary {
        name: name.to_owned(),
        status: NodeStatus {
            readiness: NodeReadiness::Ready,
            scheduling: NodeScheduling::Enabled,
        },
        roles: Vec::new(),
        taints: Vec::new(),
        kubelet_version: "v1.29.5".to_owned(),
        internal_ip: None,
        created_at: None,
        conditions: Vec::new(),
        addresses: Vec::new(),
        labels: Vec::new(),
        system: cluster::NodeSystemInfo::default(),
        resources: Vec::new(),
    }
}

fn namespace(name: &str) -> NamespaceSummary {
    NamespaceSummary {
        name: name.to_owned(),
        phase: NamespacePhase::Active,
        labels: Vec::new(),
        created_at: None,
        deleting_since: None,
        deletion_conditions: Vec::new(),
    }
}

/// A row with cell text and a label that the palette must never read.
fn kind_row(namespace: Option<&str>, name: &str) -> KindRow {
    KindRow {
        namespace: namespace.map(str::to_owned),
        name: name.to_owned(),
        created_at: None,
        status: StatusLabel {
            text: "Active".into(),
            tone: StatusTone::Ok,
        },
        cells: vec![crate::kind_row::KindCell::Text("secret-ish".into())],
        sections: Vec::new(),
        related_pods: None,
        event: None,
        labels: vec!["owner=secret-ish".into()],
        object: KindObject::Plain,
    }
}

fn cluster_row(context: &str, label: &str, shortcut: u8, is_active: bool) -> SwitcherRow {
    SwitcherRow {
        cluster: ClusterRef {
            kubeconfig: PathBuf::from("/home/me/.kube/config"),
            context: context.to_owned(),
        },
        label: label.to_owned(),
        environment: Environment::Staging,
        health: RowHealth::NotChecked,
        failure: None,
        shortcut: Some(shortcut),
        is_active,
        is_primary: is_active,
        is_ticked: false,
        search_text: format!("{label}\n{context}\nstg\nconfig").to_lowercase(),
    }
}

/// An unlocked development guard over `access`. The report is leaked so that the guard can sit in
/// `World` next to the lists it is used with; a test run is short and the report is small.
fn guard_of(access: AccessState) -> ClusterGuard<'static> {
    let access: &'static AccessState = Box::leak(Box::new(access));
    test_guard(
        access,
        WriteLock::Unlocked,
        "dev-1",
        Environment::Development,
    )
}

/// The loaded data a test borrows into a `PaletteInput`.
struct World {
    scope: NamespaceScope,
    guard: ClusterGuard<'static>,
    namespaces: Vec<NamespaceSummary>,
    pods: Vec<PodSummary>,
    nodes: Vec<NodeSummary>,
    kind_rows: Option<(ResourceKind, Vec<KindRow>)>,
    sections: Vec<SwitcherSection>,
}

impl World {
    fn new() -> Self {
        Self {
            scope: NamespaceScope::All,
            guard: guard_of(known_denying(&[])),
            namespaces: vec![namespace("shop"), namespace("kube-system")],
            pods: vec![pod("shop", "payments-api-0")],
            nodes: vec![node("node-1")],
            kind_rows: None,
            sections: Vec::new(),
        }
    }

    fn input<'a>(&'a self, screen: Screen, cursor: Option<&'a ClusterObject>) -> PaletteInput<'a> {
        PaletteInput {
            screen,
            cursor,
            has_dock_tabs: false,
            include_resources: true,
            sessions: vec![PaletteSession {
                cluster: test_cluster(),
                label: None,
                is_primary: true,
                scope: &self.scope,
                guard: &self.guard,
                namespaces: &self.namespaces,
                pods: &self.pods,
                nodes: &self.nodes,
                kind_rows: self
                    .kind_rows
                    .as_ref()
                    .map(|(kind, rows)| (*kind, rows.as_slice())),
            }],
            clusters: &self.sections,
        }
    }
}

fn labels(ranked: &Ranked) -> Vec<&str> {
    ranked
        .entries
        .iter()
        .map(|entry| entry.label.as_ref())
        .collect()
}

fn search(input: &PaletteInput<'_>, raw: &str) -> Ranked {
    ranked(palette_entries(input), &parse_query(raw))
}

fn reason_of(entry: &PaletteEntry) -> Option<&str> {
    match &entry.state {
        EntryState::Enabled => None,
        EntryState::Disabled { reason } => Some(reason.as_ref()),
    }
}

fn test_cluster() -> ClusterRef {
    ClusterRef {
        kubeconfig: PathBuf::from("/home/me/.kube/config"),
        context: "uat-ctx".to_owned(),
    }
}

fn in_test_cluster(key: ResourceKey) -> ClusterObject {
    ClusterObject::new(test_cluster(), key)
}

fn pod_key(name: &str) -> ClusterObject {
    in_test_cluster(ResourceKey::Pod {
        namespace: "shop".to_owned(),
        name: name.to_owned(),
    })
}

fn deployment_key(name: &str) -> ClusterObject {
    in_test_cluster(ResourceKey::Kind {
        kind: ResourceKind::Deployments,
        namespace: Some("shop".to_owned()),
        name: name.to_owned(),
    })
}

#[test]
fn parse_query_reads_each_prefix() {
    let cases = [
        (":po", PaletteMode::Kinds, "po"),
        ("@prod", PaletteMode::Clusters, "prod"),
        ("#web", PaletteMode::Namespaces, "web"),
        ("> rest pay", PaletteMode::Actions, "rest pay"),
        ("api", PaletteMode::All, "api"),
        ("", PaletteMode::All, ""),
        (":", PaletteMode::Kinds, ""),
    ];
    for (raw, mode, text) in cases {
        let query = parse_query(raw);
        assert_eq!((query.mode, query.text), (mode, text), "{raw:?}");
    }
}

#[test]
fn parse_query_skips_leading_spaces() {
    let query = parse_query("  :po");
    assert_eq!((query.mode, query.text), (PaletteMode::Kinds, "po"));
}

#[test]
fn entry_score_needs_every_token() {
    let fields = ["Restart rollout", "deployment/payments-api"];
    assert!(entry_score("rest pay", &fields).is_some());
    assert_eq!(entry_score("rest zzz", &fields), None);
}

#[test]
fn entry_score_lets_tokens_share_a_field() {
    assert!(entry_score("pay api", &["payments-api"]).is_some());
}

#[test]
fn entry_score_of_empty_text_is_zero() {
    assert_eq!(entry_score("  ", &["anything"]), Some(0));
}

#[test]
fn empty_query_lists_actions_only() {
    let world = World::new();
    let found = search(&world.input(Screen::Pods, None), "");
    assert!(!found.entries.is_empty());
    assert!(
        found
            .entries
            .iter()
            .all(|entry| entry.group == PaletteGroup::Actions)
    );
}

#[test]
fn kind_mode_ranks_an_exact_alias_first() {
    let world = World::new();
    let input = world.input(Screen::Pods, None);
    assert_eq!(labels(&search(&input, ":po"))[0], "Pods");
    assert_eq!(labels(&search(&input, ":deploy"))[0], "Deployments");
    assert_eq!(labels(&search(&input, ":ns"))[0], "Namespaces");
}

#[test]
fn kind_mode_lists_every_screen_for_an_empty_text() {
    let world = World::new();
    let found = search(&world.input(Screen::Pods, None), ":");
    // Pods and Nodes, then the 26 explorer kinds, capped at 30.
    assert_eq!(found.entries.len(), 28);
    assert!(
        found
            .entries
            .iter()
            .all(|entry| matches!(entry.target, PaletteTarget::Screen(_)))
    );
}

#[test]
fn kind_mode_disables_denied_kinds_with_the_sidebar_reason() {
    let mut world = World::new();
    let check = ResourceKind::Deployments
        .access_check()
        .expect("a built-in kind has a list check");
    world.guard = guard_of(known_denying(&[check]));
    world.scope = NamespaceScope::Named("shop".to_owned());
    let found = search(&world.input(Screen::Pods, None), ":deploy");
    let deployments = &found.entries[0];
    assert_eq!(deployments.label.as_ref(), "Deployments");
    assert_eq!(
        reason_of(deployments),
        Some(format!("Not permitted: {check}").as_str())
    );
    let services = search(&world.input(Screen::Pods, None), ":svc");
    assert_eq!(reason_of(&services.entries[0]), None);
}

#[test]
fn namespace_mode_lists_all_namespaces_first_then_the_live_list() {
    let mut world = World::new();
    world.scope = NamespaceScope::Named("shop".to_owned());
    let found = search(&world.input(Screen::Pods, None), "#");
    assert_eq!(labels(&found), ["All namespaces", "shop", "kube-system"]);
    let current: Vec<bool> = found.entries.iter().map(|entry| entry.is_current).collect();
    assert_eq!(current, [false, true, false]);
    world.scope = NamespaceScope::All;
    let found = search(&world.input(Screen::Pods, None), "#");
    assert!(found.entries[0].is_current);
}

#[test]
fn commands_come_from_general_and_dock_rows() {
    let world = World::new();
    let found = search(&world.input(Screen::Pods, None), "");
    let commands: Vec<&PaletteEntry> = found
        .entries
        .iter()
        .filter(|entry| matches!(entry.target, PaletteTarget::Command(_)))
        .collect();
    let has = |label: &str| commands.iter().any(|entry| entry.label.as_ref() == label);
    assert!(has("Show all shortcuts"));
    assert!(has("Open Settings"));
    assert!(has("Toggle the dock"));
    for excluded in [
        "Command palette",
        "Jump to a resource kind",
        "Import kubeconfig file (Settings window)",
        "Switch to cluster 1–9",
    ] {
        assert!(!has(excluded), "{excluded} is listed");
    }
    let action_name = |entry: &PaletteEntry| match &entry.target {
        PaletteTarget::Command(action) => action.name(),
        _ => "",
    };
    assert!(
        commands
            .iter()
            .all(|entry| !action_name(entry).ends_with("OpenPalette")
                && !action_name(entry).ends_with("OpenKindPalette")
                && !action_name(entry).ends_with("ImportKubeconfig"))
    );
    // The dock has no tab in this fixture.
    let dock = commands
        .iter()
        .find(|entry| entry.label.as_ref() == "Toggle the dock")
        .expect("a dock row");
    assert_eq!(reason_of(dock), Some("No dock tabs"));
    let shortcuts = commands
        .iter()
        .find(|entry| entry.label.as_ref() == "Show all shortcuts")
        .expect("a general row");
    assert_eq!(reason_of(shortcuts), None);
}

#[test]
fn dock_commands_are_enabled_with_a_tab() {
    let world = World::new();
    let mut input = world.input(Screen::Pods, None);
    input.has_dock_tabs = true;
    let found = search(&input, "dock");
    assert!(found.entries.iter().all(|entry| entry.is_enabled()));
}

#[test]
fn no_session_lists_commands_and_screens_only() {
    let world = World::new();
    let cursor = pod_key("payments-api-0");
    let mut input = world.input(Screen::Pods, Some(&cursor));
    input.sessions.clear();
    let all = palette_entries(&input);
    assert!(all.iter().all(|entry| matches!(
        entry.target,
        PaletteTarget::Command(_) | PaletteTarget::Screen(_)
    )));
    assert!(
        all.iter()
            .any(|entry| matches!(entry.target, PaletteTarget::Screen(_)))
    );
    // A cursor without a session offers no row action: the gates need the access report.
    assert!(
        !all.iter()
            .any(|entry| matches!(entry.target, PaletteTarget::RowAction(_)))
    );
}

#[test]
fn resources_search_pods_nodes_and_the_visible_kind_only() {
    let mut world = World::new();
    world.kind_rows = Some((
        ResourceKind::Deployments,
        vec![kind_row(Some("shop"), "payments-api")],
    ));
    let input = world.input(Screen::Kind(ResourceKind::Deployments), None);
    let found = search(&input, "pay");
    let resources: Vec<&PaletteEntry> = found
        .entries
        .iter()
        .filter(|entry| entry.group == PaletteGroup::Resources)
        .collect();
    let names: Vec<&str> = resources.iter().map(|entry| entry.label.as_ref()).collect();
    assert_eq!(names, ["payments-api", "payments-api-0"]);
    // A node matches by name too.
    let found = search(&input, "node-1");
    assert!(found.entries.iter().any(|entry| matches!(
        entry.target,
        PaletteTarget::Resource(ClusterObject {
            key: ResourceKey::Node { .. },
            ..
        })
    )));
    // Without an explorer kind on screen only pods and nodes are searched.
    world.kind_rows = None;
    let found = search(&world.input(Screen::Pods, None), "payments-api");
    assert_eq!(
        found
            .entries
            .iter()
            .filter(|entry| entry.group == PaletteGroup::Resources)
            .count(),
        1
    );
}

#[test]
fn resource_entries_carry_name_and_status_only() {
    let mut world = World::new();
    world.pods.clear();
    world.nodes.clear();
    world.kind_rows = Some((
        ResourceKind::ConfigMaps,
        vec![
            kind_row(Some("shop"), "api-config"),
            kind_row(Some("shop"), "web"),
        ],
    ));
    let input = world.input(Screen::Kind(ResourceKind::ConfigMaps), None);
    // Cell text and labels are not indexed.
    let found = search(&input, "secret-ish");
    assert!(
        !found
            .entries
            .iter()
            .any(|entry| entry.group == PaletteGroup::Resources)
    );
    // Positive control: the name matches, and the entry shows exactly name and namespace/name.
    let found = search(&input, "api-conf");
    let entry = found
        .entries
        .iter()
        .find(|entry| entry.group == PaletteGroup::Resources)
        .expect("the row named api-config matches");
    assert_eq!(entry.label.as_ref(), "api-config");
    assert_eq!(entry.detail.as_deref(), Some("shop/api-config"));
    assert_eq!(
        entry.status.as_ref().map(|status| status.text.as_ref()),
        Some("Active")
    );
    assert_eq!(
        entry
            .keywords
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<&str>>(),
        ["configmap", "configmaps"]
    );
}

#[test]
fn row_actions_follow_key_availability() {
    let world = World::new();
    let key = pod_key("payments-api-0");
    let found = search(&world.input(Screen::Pods, Some(&key)), "> ");
    let action = |label: &str| {
        found
            .entries
            .iter()
            .find(|entry| entry.label.as_ref() == label)
            .unwrap_or_else(|| panic!("{label} is listed"))
    };
    assert_eq!(reason_of(action("View logs")), None);
    assert_eq!(
        reason_of(action("Edit YAML")),
        Some("Comes in a later version")
    );
    assert_eq!(
        reason_of(action("Delete")),
        Some("Comes in a later version")
    );
    // A pod has no node-only action.
    assert!(
        !found
            .entries
            .iter()
            .any(|entry| entry.label.as_ref() == "Cordon")
    );
}

#[test]
fn no_cursor_offers_no_row_actions() {
    let world = World::new();
    let all = palette_entries(&world.input(Screen::Pods, None));
    assert!(
        !all.iter()
            .any(|entry| matches!(entry.target, PaletteTarget::RowAction(_)))
    );
}

#[test]
fn mutating_row_actions_are_never_enabled() {
    let world = World::new();
    let key = deployment_key("payments-api");
    for entry in palette_entries(&world.input(Screen::Pods, Some(&key))) {
        let PaletteTarget::RowAction(action) = entry.target else {
            continue;
        };
        let is_read_only = matches!(
            action,
            RowAction::ViewLogs | RowAction::ViewYaml | RowAction::CopyName
        );
        assert_eq!(entry.is_enabled(), is_read_only, "{action:?}");
    }
}

#[test]
fn row_action_detail_names_the_cursor_object() {
    let world = World::new();
    let key = deployment_key("payments-api");
    let found = search(&world.input(Screen::Pods, Some(&key)), "> restart");
    let entry = &found.entries[0];
    assert_eq!(entry.label.as_ref(), "Restart rollout");
    assert_eq!(entry.detail.as_deref(), Some("deployment/payments-api"));
}

#[test]
fn rest_pay_matches_restart_rollout_on_a_cursor_row() {
    let world = World::new();
    let key = deployment_key("payments-api");
    let found = search(&world.input(Screen::Pods, Some(&key)), "> rest pay");
    assert_eq!(labels(&found)[0], "Restart rollout");
}

#[test]
fn every_offered_row_action_maps() {
    let world = World::new();
    let subjects = [
        pod_key("payments-api-0"),
        in_test_cluster(ResourceKey::Node {
            name: "node-1".to_owned(),
        }),
        deployment_key("payments-api"),
    ];
    let mut offered = 0;
    for subject in &subjects {
        for entry in palette_entries(&world.input(Screen::Pods, Some(subject))) {
            let PaletteTarget::RowAction(action) = entry.target else {
                continue;
            };
            offered += 1;
            // The key action is the same unit action the key binds, by name.
            let expected = match action {
                RowAction::OpenShell => "k8sboard::OpenShell",
                other => &format!("k8sboard::{other:?}"),
            };
            assert_eq!(action.key_action().name(), expected);
        }
    }
    assert!(offered > 0);
}

#[test]
fn a_node_cursor_offers_the_node_shell_under_its_own_label() {
    let world = World::new();
    let key = in_test_cluster(ResourceKey::Node {
        name: "node-1".to_owned(),
    });
    let all = palette_entries(&world.input(Screen::Nodes, Some(&key)));
    assert!(
        all.iter()
            .any(|entry| entry.label.as_ref() == "Open node shell")
    );
    assert!(all.iter().any(|entry| entry.label.as_ref() == "Cordon"));
}

#[test]
fn cluster_mode_lists_switcher_rows_in_order() {
    let mut world = World::new();
    world.sections = vec![
        SwitcherSection {
            title: "Production",
            rows: vec![cluster_row("eu-ctx", "eu-prod", 1, false)],
        },
        SwitcherSection {
            title: "Staging",
            rows: vec![
                cluster_row("uat-ctx", "uat", 2, true),
                cluster_row("stg-ctx", "stg", 3, false),
            ],
        },
    ];
    let input = world.input(Screen::Pods, None);
    let found = search(&input, "@");
    assert_eq!(labels(&found), ["eu-prod", "uat", "stg"]);
    let active: Vec<bool> = found.entries.iter().map(|entry| entry.is_current).collect();
    assert_eq!(active, [false, true, false]);
    // The context name is searchable, as in the switcher.
    let found = search(&input, "@uat-ctx");
    assert_eq!(labels(&found)[0], "uat");
    assert_eq!(search(&input, "@zzz").entries.len(), 0);
}

#[test]
fn caps_cut_each_group_and_count_the_rest() {
    let mut world = World::new();
    world.pods = (0..60)
        .map(|index| pod("shop", &format!("web-{index:02}")))
        .collect();
    world.nodes.clear();
    let found = search(&world.input(Screen::Pods, None), "web");
    let resources = found
        .entries
        .iter()
        .filter(|entry| entry.group == PaletteGroup::Resources)
        .count();
    assert_eq!(resources, 50);
    assert_eq!(found.more, 10);
}

#[test]
fn ranking_is_stable_for_equal_scores() {
    let mut world = World::new();
    world.pods = ["web-b", "web-a", "web-c"]
        .iter()
        .map(|name| pod("shop", name))
        .collect();
    world.nodes.clear();
    let found = search(&world.input(Screen::Pods, None), "web");
    let names: Vec<&str> = found
        .entries
        .iter()
        .filter(|entry| entry.group == PaletteGroup::Resources)
        .map(|entry| entry.label.as_ref())
        .collect();
    assert_eq!(names, ["web-b", "web-a", "web-c"]);
}

#[test]
fn groups_keep_the_wireframe_order() {
    let world = World::new();
    let found = search(&world.input(Screen::Pods, None), "po");
    let groups: Vec<usize> = found
        .entries
        .iter()
        .map(|entry| entry.group.index())
        .collect();
    let mut sorted = groups.clone();
    sorted.sort_unstable();
    assert_eq!(groups, sorted);
    assert!(groups.contains(&1) && groups.contains(&2));
}

#[test]
fn empty_text_hints_name_what_was_searched() {
    assert_eq!(
        empty_text(
            PaletteMode::All,
            true,
            Screen::Kind(ResourceKind::Deployments)
        ),
        "No matches. Searched: Pods, Nodes, Deployments. Type :kind to open another kind."
    );
    assert_eq!(
        empty_text(PaletteMode::Namespaces, false, Screen::Pods),
        "No matches. Cluster not connected."
    );
    assert_eq!(
        empty_text(PaletteMode::Kinds, true, Screen::Pods),
        "No matching kind."
    );
}

#[test]
fn resources_are_not_built_when_the_query_cannot_list_them() {
    let world = World::new();
    let mut input = world.input(Screen::Pods, None);
    input.include_resources = false;
    let all = palette_entries(&input);
    assert!(
        !all.iter()
            .any(|entry| entry.group == PaletteGroup::Resources)
    );
    // Everything else is still there.
    assert!(
        all.iter()
            .any(|entry| matches!(entry.target, PaletteTarget::Screen(_)))
    );
}

#[test]
fn the_visible_kind_ranks_before_pods_for_equal_scores() {
    let mut world = World::new();
    world.pods = vec![pod("shop", "web-pod")];
    world.nodes.clear();
    world.kind_rows = Some((
        ResourceKind::Deployments,
        vec![kind_row(Some("shop"), "web-deploy")],
    ));
    let input = world.input(Screen::Kind(ResourceKind::Deployments), None);
    let found = search(&input, "web");
    let names: Vec<&str> = found
        .entries
        .iter()
        .filter(|entry| entry.group == PaletteGroup::Resources)
        .map(|entry| entry.label.as_ref())
        .collect();
    assert_eq!(names, ["web-deploy", "web-pod"]);
}

#[test]
fn only_text_in_all_mode_lists_resources() {
    assert!(lists_resources(&parse_query("pay")));
    assert!(!lists_resources(&parse_query("")));
    for raw in [":pay", "@pay", "#pay", "> pay"] {
        assert!(!lists_resources(&parse_query(raw)), "{raw}");
    }
}
