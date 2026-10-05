use std::path::PathBuf;

use cluster::{
    AccessCheck, AccessDecision, AccessReport, AccessReview, ContainerKind, ContainerState,
    NamespacePhase, NodeReadiness, NodeScheduling, NodeStatus, PodStatus, ReplicaSetSummary,
    StatusReason,
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
        is_finished: false,
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
            terminal: cluster::ContainerTerminal::None,
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
        environment: Environment::STAGING,
        health: RowHealth::NotChecked,
        failure: None,
        note: None,
        shortcut: Some(shortcut),
        is_active,
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
        Environment::DEVELOPMENT,
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
    replica_sets: Option<Vec<ReplicaSetSummary>>,
    feeds: Vec<FeedObjects<'static>>,
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
            replica_sets: None,
            feeds: Vec::new(),
            sections: Vec::new(),
        }
    }

    fn input<'a>(&'a self, screen: Screen, cursor: Option<&'a ClusterObject>) -> PaletteInput<'a> {
        PaletteInput {
            screen,
            cursor,
            has_dock_tabs: false,
            include_resources: true,
            query_text: "",
            pair_text: None,
            session: Some(PaletteSession {
                cluster: test_cluster(),
                scope: &self.scope,
                guard: &self.guard,
                namespaces: &self.namespaces,
                pods: &self.pods,
                nodes: &self.nodes,
                kind_rows: self
                    .kind_rows
                    .as_ref()
                    .map(|(kind, rows)| (*kind, rows.as_slice())),
                replica_sets: self.replica_sets.as_deref(),
                feeds: &self.feeds,
            }),
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

/// Builds the entries for `raw` the way the shell does (the query text rides in the input) and
/// ranks them.
fn search<'a>(input: &mut PaletteInput<'a>, raw: &'a str) -> Ranked {
    let query = parse_query(raw);
    input.include_resources = lists_resources(&query);
    input.query_text = query.text;
    input.pair_text = lists_pairs(&query).then_some(query.text);
    ranked(palette_entries(input), &query)
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
    let found = search(&mut world.input(Screen::Pods, None), "");
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
    let mut input = world.input(Screen::Pods, None);
    assert_eq!(labels(&search(&mut input, ":po"))[0], "Pods");
    assert_eq!(labels(&search(&mut input, ":deploy"))[0], "Deployments");
    assert_eq!(labels(&search(&mut input, ":ns"))[0], "Namespaces");
}

#[test]
fn kind_mode_lists_every_screen_for_an_empty_text() {
    let world = World::new();
    let found = search(&mut world.input(Screen::Pods, None), ":");
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
    let found = search(&mut world.input(Screen::Pods, None), ":deploy");
    let deployments = &found.entries[0];
    assert_eq!(deployments.label.as_ref(), "Deployments");
    assert_eq!(
        reason_of(deployments),
        Some(format!("Not permitted: {check}").as_str())
    );
    let services = search(&mut world.input(Screen::Pods, None), ":svc");
    assert_eq!(reason_of(&services.entries[0]), None);
}

#[test]
fn namespace_mode_lists_all_namespaces_first_then_the_live_list() {
    let mut world = World::new();
    world.scope = NamespaceScope::Named("shop".to_owned());
    let found = search(&mut world.input(Screen::Pods, None), "#");
    assert_eq!(labels(&found), ["All namespaces", "shop", "kube-system"]);
    let current: Vec<bool> = found.entries.iter().map(|entry| entry.is_current).collect();
    assert_eq!(current, [false, true, false]);
    world.scope = NamespaceScope::All;
    let found = search(&mut world.input(Screen::Pods, None), "#");
    assert!(found.entries[0].is_current);
}

#[test]
fn commands_come_from_general_and_dock_rows() {
    let world = World::new();
    let found = search(&mut world.input(Screen::Pods, None), "");
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
    let found = search(&mut input, "dock");
    assert!(found.entries.iter().all(|entry| entry.is_enabled()));
}

#[test]
fn no_session_lists_commands_and_screens_only() {
    let world = World::new();
    let cursor = pod_key("payments-api-0");
    let mut input = world.input(Screen::Pods, Some(&cursor));
    input.session = None;
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
    let mut input = world.input(Screen::Kind(ResourceKind::Deployments), None);
    let found = search(&mut input, "pay");
    let resources: Vec<&PaletteEntry> = found
        .entries
        .iter()
        .filter(|entry| entry.group == PaletteGroup::Resources)
        .collect();
    let names: Vec<&str> = resources.iter().map(|entry| entry.label.as_ref()).collect();
    assert_eq!(names, ["payments-api", "payments-api-0"]);
    // A node matches by name too.
    let found = search(&mut input, "node-1");
    assert!(found.entries.iter().any(|entry| matches!(
        entry.target,
        PaletteTarget::Resource(ClusterObject {
            key: ResourceKey::Node { .. },
            ..
        })
    )));
    // Without an explorer kind on screen only pods and nodes are searched.
    world.kind_rows = None;
    let found = search(&mut world.input(Screen::Pods, None), "payments-api");
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
    let mut input = world.input(Screen::Kind(ResourceKind::ConfigMaps), None);
    // Cell text and labels are not indexed.
    let found = search(&mut input, "secret-ish");
    assert!(
        !found
            .entries
            .iter()
            .any(|entry| entry.group == PaletteGroup::Resources)
    );
    // Positive control: the name matches, and the entry shows exactly name and namespace/name.
    let found = search(&mut input, "api-conf");
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
    let found = search(&mut world.input(Screen::Pods, Some(&key)), "> ");
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
        // The pod's `update` check has not been asked in this world, so the gate still waits.
        Some("Checking permissions…")
    );
    // The lazy `delete pods` check has not been asked in this world either.
    assert_eq!(reason_of(action("Delete")), Some("Checking permissions…"));
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
fn unshipped_row_actions_are_never_enabled() {
    let world = World::new();
    let key = deployment_key("payments-api");
    for entry in palette_entries(&world.input(Screen::Pods, Some(&key))) {
        let PaletteTarget::RowAction(action) = entry.target else {
            continue;
        };
        // The read-only actions, and those whose spec has shipped (workloads: 0032, port-forward: 0035).
        let is_available = matches!(
            action,
            RowAction::ViewLogs
                | RowAction::ViewYaml
                | RowAction::CopyName
                | RowAction::PortForward
                | RowAction::RestartRollout
                | RowAction::PauseRollout
                | RowAction::Scale
        );
        assert_eq!(entry.is_enabled(), is_available, "{action:?}");
    }
}

#[test]
fn row_action_detail_names_the_cursor_object() {
    let world = World::new();
    let key = deployment_key("payments-api");
    let found = search(&mut world.input(Screen::Pods, Some(&key)), "> restart");
    let entry = &found.entries[0];
    assert_eq!(entry.label.as_ref(), "Restart rollout");
    assert_eq!(entry.detail.as_deref(), Some("deployment/payments-api"));
}

#[test]
fn rest_pay_matches_restart_rollout_on_a_cursor_row() {
    let world = World::new();
    let key = deployment_key("payments-api");
    let found = search(&mut world.input(Screen::Pods, Some(&key)), "> rest pay");
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
            title: "Production".into(),
            rows: vec![cluster_row("eu-ctx", "eu-prod", 1, false)],
        },
        SwitcherSection {
            title: "Staging".into(),
            rows: vec![
                cluster_row("uat-ctx", "uat", 2, true),
                cluster_row("stg-ctx", "stg", 3, false),
            ],
        },
    ];
    let mut input = world.input(Screen::Pods, None);
    let found = search(&mut input, "@");
    assert_eq!(labels(&found), ["eu-prod", "uat", "stg"]);
    let active: Vec<bool> = found.entries.iter().map(|entry| entry.is_current).collect();
    assert_eq!(active, [false, true, false]);
    // The context name is searchable, as in the switcher.
    let found = search(&mut input, "@uat-ctx");
    assert_eq!(labels(&found)[0], "uat");
    assert_eq!(search(&mut input, "@zzz").entries.len(), 0);
}

#[test]
fn caps_cut_each_group_and_count_the_rest() {
    let mut world = World::new();
    world.pods = (0..60)
        .map(|index| pod("shop", &format!("web-{index:02}")))
        .collect();
    world.nodes.clear();
    let found = search(&mut world.input(Screen::Pods, None), "web");
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
    let found = search(&mut world.input(Screen::Pods, None), "web");
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
    let found = search(&mut world.input(Screen::Pods, None), "po");
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
            Screen::Kind(ResourceKind::Deployments),
            &[]
        ),
        "No matches. Searched: Pods, Nodes, Deployments. Type :kind for other kinds."
    );
    assert_eq!(
        empty_text(PaletteMode::Namespaces, false, Screen::Pods, &[]),
        "No matches. Cluster not connected."
    );
    assert_eq!(
        empty_text(PaletteMode::Kinds, true, Screen::Pods, &[]),
        "No matching kind."
    );
}

#[test]
fn empty_text_names_the_live_feeds_once() {
    let feeds = [ResourceKind::Deployments, ResourceKind::Jobs];
    assert_eq!(
        empty_text(PaletteMode::All, true, Screen::Pods, &feeds),
        "No matches. Searched: Pods, Nodes, Deployments, Jobs. Type :kind for other kinds."
    );
    // The visible kind is named once, in its own place.
    assert_eq!(
        empty_text(
            PaletteMode::All,
            true,
            Screen::Kind(ResourceKind::Jobs),
            &feeds
        ),
        "No matches. Searched: Pods, Nodes, Jobs, Deployments. Type :kind for other kinds."
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
    let mut input = world.input(Screen::Kind(ResourceKind::Deployments), None);
    let found = search(&mut input, "web");
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

/// A Deployments list with one row, paused or not, in the world of a test.
fn world_with_deployment(is_paused: bool) -> World {
    let mut summary = crate::workload_actions::workload_actions_tests::deployment("payments-api");
    summary.namespace = "shop".to_owned();
    summary.is_paused = is_paused;
    let mut world = World::new();
    world.kind_rows = Some((
        ResourceKind::Deployments,
        vec![crate::workload_rows::deployment_row(&summary)],
    ));
    world
}

#[test]
fn a_paused_deployment_offers_resume_and_blocks_restart() {
    let world = world_with_deployment(true);
    let key = deployment_key("payments-api");
    let all = palette_entries(&world.input(Screen::Kind(ResourceKind::Deployments), Some(&key)));
    let entry = |label: &str| all.iter().find(|entry| entry.label.as_ref() == label);
    let restart = entry("Restart rollout").expect("Restart rollout is listed");
    assert_eq!(reason_of(restart), Some("Resume the rollout first"));
    // The pause entry reads as the way out, and it is enabled.
    let resume = entry("Resume rollout").expect("Resume rollout is listed");
    assert_eq!(reason_of(resume), None);
    assert!(entry("Pause rollout").is_none());
}

#[test]
fn a_running_deployment_offers_pause_and_restart() {
    let world = world_with_deployment(false);
    let key = deployment_key("payments-api");
    let all = palette_entries(&world.input(Screen::Kind(ResourceKind::Deployments), Some(&key)));
    for label in ["Restart rollout", "Pause rollout"] {
        let entry = all
            .iter()
            .find(|entry| entry.label.as_ref() == label)
            .unwrap_or_else(|| panic!("{label} is listed"));
        assert_eq!(reason_of(entry), None, "{label}");
    }
}

fn plain_entry(label: &str, detail: Option<&str>, keywords: &[&str]) -> PaletteEntry {
    let mut entry = PaletteEntry::new(
        PaletteGroup::GoTo,
        label.to_owned(),
        PaletteTarget::Screen(Screen::Pods),
    );
    entry.detail = detail.map(|detail| detail.to_owned().into());
    entry.keywords = keywords
        .iter()
        .map(|word| SharedString::from((*word).to_owned()))
        .collect();
    entry
}

#[test]
fn entry_match_ranges_follow_the_scoring_field() {
    let entry = plain_entry("Restart rollout", Some("deployment/payments-api"), &[]);
    let ranges = entry_match_ranges(&entry, "rest pay");
    assert_eq!(ranges.label, vec![run(0, 4)]);
    assert_eq!(ranges.detail, vec![run(11, 14)]);
}

#[test]
fn entry_match_ranges_merge_tokens_on_one_field() {
    let entry = plain_entry("payments-api", None, &[]);
    assert_eq!(
        entry_match_ranges(&entry, "pay api").label,
        vec![run(0, 3), run(9, 12)]
    );
    // Two tokens that cover the same characters give one range.
    assert_eq!(
        entry_match_ranges(&entry, "pay paym").label,
        vec![run(0, 4)]
    );
}

#[test]
fn entry_match_ranges_leave_a_keyword_match_unmarked() {
    // `po` is an exact keyword, which outscores the prefix of the label.
    let entry = plain_entry("Pods", None, &["pod", "pods", "po"]);
    assert_eq!(entry_match_ranges(&entry, "po"), EntryRanges::default());
}

#[test]
fn entry_match_ranges_of_an_empty_text_are_empty() {
    let entry = plain_entry("Restart rollout", Some("deployment/payments-api"), &[]);
    assert_eq!(entry_match_ranges(&entry, ""), EntryRanges::default());
    assert_eq!(entry_match_ranges(&entry, "  "), EntryRanges::default());
}

#[test]
fn entry_match_ranges_skip_a_token_that_matches_nothing() {
    let entry = plain_entry("Restart rollout", None, &[]);
    assert_eq!(
        entry_match_ranges(&entry, "zzz").label,
        Vec::<Range<usize>>::new()
    );
}

/// A one-range expectation; `vec![0..4]` trips `clippy::single_range_in_vec_init`.
fn run(start: usize, end: usize) -> Range<usize> {
    start..end
}

fn world_with_clusters(scope: NamespaceScope) -> World {
    let mut world = World::new();
    world.scope = scope;
    world.sections = vec![SwitcherSection {
        title: "Staging".into(),
        rows: vec![
            cluster_row("uat-ctx", "uat", 1, true),
            cluster_row("stg-ctx", "stg", 2, false),
        ],
    }];
    world
}

/// The scope each `@` row carries and its detail, in row order.
fn carried(found: &Ranked) -> Vec<(Option<NamespaceScope>, Option<&str>)> {
    found
        .entries
        .iter()
        .map(|entry| match &entry.target {
            PaletteTarget::Cluster(_, scope) => (scope.clone(), entry.note.as_deref()),
            _ => (None, None),
        })
        .collect()
}

#[test]
fn cluster_rows_carry_a_single_named_scope() {
    let world = world_with_clusters(NamespaceScope::Named("payments".to_owned()));
    let found = search(&mut world.input(Screen::Pods, None), "@");
    let payments = NamespaceScope::Named("payments".to_owned());
    // The active cluster (`uat`) carries nothing: a switch to it does nothing.
    assert_eq!(
        carried(&found),
        [
            (None, None),
            (Some(payments), Some("same namespace payments"))
        ]
    );
}

#[test]
fn cluster_rows_carry_nothing_for_all_or_several_namespaces() {
    for scope in [
        NamespaceScope::All,
        NamespaceScope::of_namespaces(["a".to_owned(), "b".to_owned()]),
    ] {
        let world = world_with_clusters(scope);
        let found = search(&mut world.input(Screen::Pods, None), "@");
        assert_eq!(carried(&found), [(None, None), (None, None)]);
    }
}

#[test]
fn cluster_rows_carry_nothing_without_a_session() {
    let world = world_with_clusters(NamespaceScope::Named("payments".to_owned()));
    let mut input = world.input(Screen::Pods, None);
    input.session = None;
    let found = search(&mut input, "@");
    assert_eq!(carried(&found), [(None, None), (None, None)]);
}

#[test]
fn the_carried_namespace_note_is_never_searched() {
    let world = world_with_clusters(NamespaceScope::Named("payments".to_owned()));
    let mut input = world.input(Screen::Pods, None);
    // `same namespace payments` is shown, but typing it must not select the rows that carry it.
    assert!(search(&mut input, "@same namespace").entries.is_empty());
    assert!(search(&mut input, "@payments").entries.is_empty());
    assert_eq!(labels(&search(&mut input, "@stg-ctx")), ["stg"]);
}

// ---- Step 5c: action x resource pairs ----

/// A Deployments list in the `shop` namespace, in the world of a test.
fn world_with_deployments(names: &[&str]) -> World {
    let mut world = World::new();
    let rows = names
        .iter()
        .map(|name| {
            let mut summary = crate::workload_actions::workload_actions_tests::deployment(name);
            summary.namespace = "shop".to_owned();
            crate::workload_rows::deployment_row(&summary)
        })
        .collect();
    world.kind_rows = Some((ResourceKind::Deployments, rows));
    world
}

fn deployments_screen() -> Screen {
    Screen::Kind(ResourceKind::Deployments)
}

/// The label and detail of every pair entry, in ranked order.
fn pairs_of(found: &Ranked) -> Vec<(&str, &str)> {
    found
        .entries
        .iter()
        .filter(|entry| matches!(entry.target, PaletteTarget::ObjectAction(..)))
        .map(|entry| (entry.label.as_ref(), entry.detail.as_deref().unwrap_or("")))
        .collect()
}

fn pair_entry<'a>(found: &'a Ranked, label: &str) -> Option<&'a PaletteEntry> {
    found.entries.iter().find(|entry| {
        entry.label.as_ref() == label && matches!(entry.target, PaletteTarget::ObjectAction(..))
    })
}

#[test]
fn lists_pairs_needs_two_tokens_in_all_or_actions_mode() {
    for (raw, expected) in [
        ("> rest pay", true),
        ("rest pay", true),
        ("> rest", false),
        ("rest", false),
        ("", false),
        (":rest pay", false),
        ("@rest pay", false),
        ("#rest pay", false),
    ] {
        assert_eq!(lists_pairs(&parse_query(raw)), expected, "{raw:?}");
    }
}

#[test]
fn pairs_need_one_token_on_the_action_and_another_on_the_object() {
    let world = world_with_deployments(&["payments-api", "restic-backup"]);
    let mut input = world.input(deployments_screen(), None);
    let found = search(&mut input, "> rest pay");
    assert_eq!(
        pairs_of(&found),
        [("Restart rollout", "deployment/payments-api · shop")]
    );
    let entry = pair_entry(&found, "Restart rollout").expect("the pair is listed");
    let PaletteTarget::ObjectAction(object, action) = &entry.target else {
        panic!("a pair");
    };
    assert_eq!(*object, deployment_key("payments-api"));
    assert_eq!(*action, RowAction::RestartRollout);
    // One token alone would pair Restart with every Deployment, and `restic-backup` matches both
    // the action and the object: neither is a pair.
    assert!(pairs_of(&search(&mut input, "> rest")).is_empty());
    assert!(pairs_of(&search(&mut input, "> restic")).is_empty());
    // Two tokens that both name the object leave no token for the action.
    assert!(pairs_of(&search(&mut input, "> pay api")).is_empty());
}

#[test]
fn pairs_skip_the_cursor_roll_back_delete_and_unshipped_actions() {
    let world = world_with_deployments(&["payments-api", "payments-web"]);
    let cursor = deployment_key("payments-api");
    let mut input = world.input(deployments_screen(), Some(&cursor));
    // The cursor keeps its own entry, with the state of the loaded row, and no duplicate.
    let found = search(&mut input, "> rest pay");
    assert_eq!(
        pairs_of(&found),
        [("Restart rollout", "deployment/payments-web · shop")]
    );
    assert_eq!(found.entries.len(), 2);
    // `roll` also names "Pause rollout", so look at the actions the pairs carry. for raw in ["> roll pay", "> delete pay", "> del pay", "> back pay"] { let found = search(&mut input, raw); let has_excluded = found.entries.iter().any(|entry| { matches!( entry.target, PaletteTarget::ObjectAction(_, RowAction::Delete | RowAction::RollBack) ) }); assert!(!has_excluded, "{raw}"); } assert!(pairs_of(&search(&mut input, "> delete pay")).is_empty());
    for row in ROW_ACTIONS {
        assert_eq!(
            is_pairable(row),
            !matches!(
                row,
                RowAction::Delete
                    | RowAction::RollBack
                    | RowAction::RestartPod
                    | RowAction::EvictPod
            ),
            "{row:?}"
        );
    }
}

#[test]
fn a_node_pairs_with_its_shipped_actions() {
    let world = World::new();
    let mut input = world.input(Screen::Nodes, None);
    let found = search(&mut input, "> cordon node");
    assert_eq!(pairs_of(&found), [("Cordon", "node/node-1")]);
    // Drain shipped with spec 0034, so it is paired like any other shipped action.
    let found = search(&mut input, "> drain node");
    assert_eq!(pairs_of(&found), [("Drain", "node/node-1")]);
}

#[test]
fn pairs_come_from_the_loaded_lists_only() {
    // No Deployments list is loaded (any other screen): no Deployment pair, but a pod pair.
    let world = World::new();
    let mut input = world.input(Screen::Pods, None);
    assert!(pairs_of(&search(&mut input, "> rest pay")).is_empty());
    let found = search(&mut input, "> logs pay");
    assert_eq!(
        pairs_of(&found),
        [("View logs", "pod/payments-api-0 · shop")]
    );
}

#[test]
fn all_mode_pairs_reuse_the_resource_scores() {
    let mut world = world_with_deployments(&["payments-api"]);
    world.pods = vec![
        pod("shop", "payments-api-0"),
        pod("shop", "restic-payments-0"),
    ];
    let mut input = world.input(deployments_screen(), None);
    let query = parse_query("rest pay");
    input.include_resources = true;
    input.query_text = query.text;
    input.pair_text = Some(query.text);
    let entries = palette_entries(&input);
    let resources: Vec<&PaletteEntry> = entries
        .iter()
        .filter(|entry| entry.group == PaletteGroup::Resources)
        .collect();
    // Only the pod that matches both tokens lists; the misses are dropped.
    assert_eq!(
        resources
            .iter()
            .map(|entry| entry.label.as_ref())
            .collect::<Vec<_>>(),
        ["restic-payments-0"]
    );
    for entry in &resources {
        assert_eq!(entry.score, score_of(entry, query.text));
        assert!(entry.score.is_some());
    }
    // The same scan found the pair object that matches one token (`pay`).
    let found = ranked(entries, &query);
    assert_eq!(
        pairs_of(&found),
        [("Restart rollout", "deployment/payments-api · shop")]
    );
}

#[test]
fn ranked_keeps_the_order_a_fresh_score_gives() {
    let mut world = World::new();
    world.pods = vec![
        pod("shop", "web-api-0"),
        pod("shop", "api-0"),
        pod("shop", "x-api-0"),
    ];
    let mut input = world.input(Screen::Pods, None);
    let found = search(&mut input, "api");
    let resources: Vec<&PaletteEntry> = found
        .entries
        .iter()
        .filter(|entry| entry.group == PaletteGroup::Resources)
        .collect();
    let scores: Vec<Option<u32>> = resources.iter().map(|entry| entry.score).collect();
    assert!(scores.iter().all(Option::is_some));
    assert!(scores.windows(2).all(|pair| pair[0] >= pair[1]));
    assert_eq!(resources[0].label.as_ref(), "api-0");
    for entry in &resources {
        assert_eq!(entry.score, score_of(entry, "api"));
    }
}

#[test]
fn ranked_uses_a_stored_score_instead_of_scoring_again() {
    let mut stored = plain_entry("Pods", None, &[]);
    stored.score = Some(7);
    let fresh = plain_entry("Pods", None, &[]);
    let found = ranked(vec![stored, fresh], &parse_query("pods"));
    // The stored 7 is below any real score of `pods` against `Pods`, so the fresh entry leads.
    assert_eq!(found.entries[0].score, None);
    assert_eq!(found.entries[1].score, Some(7));
}

#[test]
fn pairs_keep_the_top_fifty_objects() {
    let mut world = World::new();
    world.pods = (0..60)
        .map(|index| pod("shop", &format!("web-{index:02}")))
        .collect();
    let mut input = world.input(Screen::Pods, None);
    let query = parse_query("> logs web");
    input.include_resources = false;
    input.query_text = query.text;
    input.pair_text = Some(query.text);
    let pairs = palette_entries(&input)
        .into_iter()
        .filter(|entry| matches!(entry.target, PaletteTarget::ObjectAction(..)))
        .count();
    assert_eq!(pairs, RESOURCES_CAP);
}

#[test]
fn pair_state_follows_the_gate() {
    let mut world = world_with_deployments(&["payments-api"]);
    world.guard = guard_of(known_denying(&[AccessCheck::PatchDeployments]));
    let mut input = world.input(deployments_screen(), None);
    let found = search(&mut input, "> rest pay");
    let entry = pair_entry(&found, "Restart rollout").expect("the pair is listed");
    assert_eq!(reason_of(entry), Some("Not permitted: patch deployments"));
    assert!(!entry.needs_confirm);

    // A paused Deployment blocks its restart through the loaded row.
    let mut summary = crate::workload_actions::workload_actions_tests::deployment("payments-api");
    summary.namespace = "shop".to_owned();
    summary.is_paused = true;
    let mut world = World::new();
    world.kind_rows = Some((
        ResourceKind::Deployments,
        vec![crate::workload_rows::deployment_row(&summary)],
    ));
    let mut input = world.input(deployments_screen(), None);
    let found = search(&mut input, "> rest pay");
    let entry = pair_entry(&found, "Restart rollout").expect("the pair is listed");
    assert_eq!(reason_of(entry), Some("Resume the rollout first"));
    assert!(!entry.needs_confirm);
}

#[test]
fn pair_detail_names_the_namespace_unless_scoped_to_one() {
    let mut world = world_with_deployments(&["api"]);
    let detail_in = |world: &World| {
        let mut input = world.input(deployments_screen(), None);
        let found = search(&mut input, "> rest api");
        pair_entry(&found, "Restart rollout")
            .and_then(|entry| entry.detail.as_deref().map(str::to_owned))
    };
    assert_eq!(detail_in(&world).as_deref(), Some("deployment/api · shop"));
    world.scope = NamespaceScope::of_namespaces(["shop".to_owned(), "web".to_owned()]);
    assert_eq!(detail_in(&world).as_deref(), Some("deployment/api · shop"));
    world.scope = NamespaceScope::Named("shop".to_owned());
    assert_eq!(detail_in(&world).as_deref(), Some("deployment/api"));
}

#[test]
fn needs_confirm_marks_enabled_mutating_entries_only() {
    let world = world_with_deployments(&["payments-api", "payments-web"]);
    let cursor = deployment_key("payments-api");
    let mut input = world.input(deployments_screen(), Some(&cursor));
    // The cursor entries: Restart (a write) is marked; Copy name and View YAML never are.
    let found = search(&mut input, "> pay");
    let cursor_entry = |label: &str| {
        found.entries.iter().find(|entry| {
            entry.label.as_ref() == label && matches!(entry.target, PaletteTarget::RowAction(_))
        })
    };
    assert!(cursor_entry("Restart rollout").is_some_and(|entry| entry.needs_confirm));
    assert!(cursor_entry("View YAML").is_some_and(|entry| !entry.needs_confirm));
    assert!(cursor_entry("Copy name").is_some_and(|entry| !entry.needs_confirm));
    // The pairs of the other object follow the same rule.
    let found = search(&mut input, "> rest pay");
    assert!(pair_entry(&found, "Restart rollout").is_some_and(|entry| entry.needs_confirm));
    let found = search(&mut input, "> copy pay");
    assert!(pair_entry(&found, "Copy name").is_some_and(|entry| !entry.needs_confirm));
    let found = search(&mut input, "> yaml pay");
    assert!(pair_entry(&found, "View YAML").is_some_and(|entry| !entry.needs_confirm));
}

#[test]
fn a_disabled_entry_never_needs_confirm() {
    let mut world = world_with_deployments(&["payments-api"]);
    world.guard = guard_of(known_denying(&[AccessCheck::PatchDeployments]));
    let cursor = deployment_key("payments-api");
    let all = palette_entries(&world.input(deployments_screen(), Some(&cursor)));
    let restart = all
        .iter()
        .find(|entry| entry.label.as_ref() == "Restart rollout")
        .expect("Restart rollout is listed");
    assert!(!restart.is_enabled());
    assert!(!restart.needs_confirm);
}

#[test]
fn commands_screens_resources_namespaces_and_clusters_never_need_confirm() {
    let mut world = World::new();
    world.sections = vec![SwitcherSection {
        title: "Staging".into(),
        rows: vec![cluster_row("stg-ctx", "stg", 1, false)],
    }];
    let all = palette_entries(&world.input(Screen::Pods, None));
    assert!(all.iter().any(|entry| entry.group == PaletteGroup::GoTo));
    assert!(all.iter().all(|entry| !entry.needs_confirm));
}

#[test]
fn objects_that_only_match_the_action_token_do_not_crowd_out_the_named_one() {
    let mut world = World::new();
    // Sixty pods match `logs` better than `payments-api-0` matches `pay`.
    world.pods = (0..60)
        .map(|index| pod("shop", &format!("logstash-{index:02}")))
        .chain(std::iter::once(pod("shop", "payments-api-0")))
        .collect();
    let mut input = world.input(Screen::Pods, None);
    for raw in ["> logs pay", "logs pay"] {
        let found = search(&mut input, raw);
        assert_eq!(
            pairs_of(&found),
            [("View logs", "pod/payments-api-0 · shop")],
            "{raw}"
        );
    }
}

#[test]
fn object_tokens_leave_out_the_tokens_that_name_an_action() {
    assert_eq!(object_tokens(&["logs", "pay"]), [false, true]);
    assert_eq!(object_tokens(&["rest", "api", "pay"]), [false, true, true]);
    // Every token reads as an action: any of them may name the object.
    assert_eq!(object_tokens(&["rest", "logs"]), [true, true]);
    assert_eq!(object_tokens(&["pay", "api"]), [true, true]);
}

#[test]
fn the_pair_label_actions_are_all_pairable() {
    for action in PAIR_LABEL_ACTIONS {
        assert!(
            is_pairable(action.row_action().expect("a row action")),
            "{action:?}"
        );
    }
}

#[test]
fn attach_restart_pod_and_evict_are_cursor_entries_with_their_states() {
    let world = World::new();
    let cursor = pod_key("payments-api-0");
    let mut input = world.input(Screen::Pods, Some(&cursor));
    let found = search(&mut input, "> pod");
    let entry = |action: RowAction| {
        found
            .entries
            .iter()
            .find(|entry| matches!(&entry.target, PaletteTarget::RowAction(row) if *row == action))
    };
    // Evict of a bare pod is allowed and needs a confirm.
    let evict = entry(RowAction::EvictPod).expect("Evict is listed");
    assert_eq!(evict.label.as_ref(), "Evict");
    assert!(evict.is_enabled() && evict.needs_confirm);
    // Restart pod reads the lazy Delete pod review (unanswered here) and then the bare pod.
    let restart = entry(RowAction::RestartPod).expect("Restart pod is listed");
    assert_eq!(restart.label.as_ref(), "Restart pod");
    assert_eq!(reason_of(restart), Some("Checking permissions…"));
    assert!(!restart.needs_confirm);
    // The test pod has no terminal, so Attach says so.
    let attach = entry(RowAction::Attach).expect("Attach is listed");
    assert_eq!(
        reason_of(attach),
        Some("No running container has a terminal (stdin and tty); use View logs")
    );
    // They are cursor entries only: no pair for another pod.
    let found = search(&mut input, "> evict pay");
    assert!(
        pairs_of(&found)
            .iter()
            .all(|(label, _)| *label != "Evict" && *label != "Restart pod")
    );
}

// ---- Condition feeds (spec 0056 C1) ----

fn feed_deployment(namespace: &str, name: &str) -> KindObject {
    let mut summary = crate::workload_actions::workload_actions_tests::deployment(name);
    summary.namespace = namespace.to_owned();
    KindObject::Deployment(summary)
}

/// A feed whose objects live for the rest of the test, like `guard_of`'s report.
fn feed_of(kind: ResourceKind, objects: Vec<KindObject>) -> FeedObjects<'static> {
    FeedObjects {
        kind,
        objects: Box::leak(objects.into_boxed_slice()),
    }
}

fn resource_labels(ranked: &Ranked) -> Vec<&str> {
    ranked
        .entries
        .iter()
        .filter(|entry| entry.group == PaletteGroup::Resources)
        .map(|entry| entry.label.as_ref())
        .collect()
}

#[test]
fn a_feed_deployment_is_found_by_name_without_a_status() {
    let mut world = World::new();
    world.feeds = vec![feed_of(
        ResourceKind::Deployments,
        vec![feed_deployment("shop", "checkout")],
    )];
    let found = search(&mut world.input(Screen::Pods, None), "checkout");
    let entry = found
        .entries
        .iter()
        .find(|entry| entry.group == PaletteGroup::Resources)
        .expect("the deployment is listed");
    assert_eq!(entry.label.as_ref(), "checkout");
    assert_eq!(entry.detail.as_deref(), Some("shop/checkout"));
    assert!(entry.status.is_none());
    assert!(matches!(
        &entry.target,
        PaletteTarget::Resource(object) if object.key == ResourceKey::Kind {
            kind: ResourceKind::Deployments,
            namespace: Some("shop".to_owned()),
            name: "checkout".to_owned(),
        }
    ));
    // The kind words narrow to the kind.
    let by_kind = search(&mut world.input(Screen::Pods, None), "deploy checkout");
    assert_eq!(resource_labels(&by_kind), ["checkout"]);
}

#[test]
fn a_feed_object_outside_the_scope_is_skipped() {
    let mut world = World::new();
    world.scope = NamespaceScope::Named("shop".to_owned());
    world.feeds = vec![feed_of(
        ResourceKind::Deployments,
        vec![
            feed_deployment("shop", "checkout"),
            feed_deployment("billing", "checkout-batch"),
        ],
    )];
    let found = search(&mut world.input(Screen::Pods, None), "checkout");
    assert_eq!(resource_labels(&found), ["checkout"]);
}

#[test]
fn the_on_screen_row_wins_over_its_feed_copy() {
    let mut world = World::new();
    let summary = crate::workload_actions::workload_actions_tests::deployment("checkout");
    let mut row = kind_row(Some("team-a"), "checkout");
    row.object = KindObject::Deployment(summary.clone());
    world.kind_rows = Some((ResourceKind::Deployments, vec![row]));
    world.feeds = vec![feed_of(
        ResourceKind::Deployments,
        vec![KindObject::Deployment(summary)],
    )];
    let found = search(
        &mut world.input(Screen::Kind(ResourceKind::Deployments), None),
        "checkout",
    );
    assert_eq!(resource_labels(&found), ["checkout"]);
    let entry = found
        .entries
        .iter()
        .find(|entry| entry.group == PaletteGroup::Resources)
        .expect("listed");
    assert!(entry.status.is_some(), "the row keeps its status");
}

#[test]
fn a_feed_of_another_kind_is_kept_beside_the_visible_kind() {
    let mut world = World::new();
    world.kind_rows = Some((
        ResourceKind::Services,
        vec![kind_row(Some("shop"), "checkout")],
    ));
    world.feeds = vec![feed_of(
        ResourceKind::Deployments,
        vec![feed_deployment("shop", "checkout")],
    )];
    let found = search(
        &mut world.input(Screen::Kind(ResourceKind::Services), None),
        "checkout",
    );
    assert_eq!(resource_labels(&found), ["checkout", "checkout"]);
}

#[test]
fn feed_objects_carry_no_action_pairs() {
    let mut world = World::new();
    world.feeds = vec![feed_of(
        ResourceKind::Deployments,
        vec![feed_deployment("shop", "checkout")],
    )];
    let found = search(&mut world.input(Screen::Pods, None), "restart checkout");
    assert!(
        found
            .entries
            .iter()
            .all(|entry| !matches!(entry.target, PaletteTarget::ObjectAction(..)))
    );
}

#[test]
fn feed_objects_of_other_summaries_are_ignored() {
    let mut world = World::new();
    world.feeds = vec![feed_of(ResourceKind::Deployments, vec![KindObject::Plain])];
    let found = search(&mut world.input(Screen::Pods, None), "plain");
    assert!(resource_labels(&found).is_empty());
}
