use cluster::{AccessDecision, AccessReport, AccessReview};
use gpui_kit::Task;

use super::*;
use crate::environment::Environment;
use crate::write_guard::{ActionRisk, DialogConfirm, confirm_step, test_guard};

fn checking() -> AccessState {
    AccessState::Checking {
        _task: Task::ready(()),
    }
}

fn unknown() -> AccessState {
    AccessState::Unknown
}

/// A report that allows every check except those listed.
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

fn reason(availability: ActionAvailability) -> String {
    match availability {
        ActionAvailability::Disabled { reason } => reason.to_string(),
        ActionAvailability::Enabled => panic!("expected a disabled action"),
    }
}

/// A guard of an unlocked development cluster, which never stands in the way of a gate.
fn unlocked(access: &AccessState) -> ClusterGuard<'_> {
    test_guard(
        access,
        WriteLock::Unlocked,
        "dev-1",
        Environment::Development,
    )
}

/// A mutating action whose spec has shipped, which no `ResourceAction` is before step 4.
const SHIPPED_PATCH_NODES: ActionGate = ActionGate::Mutating {
    check: AccessCheck::PatchNodes,
    is_shipped: true,
};

fn availability_of_gate(
    gate: &ActionGate,
    access: &AccessState,
    lock: WriteLock,
) -> ActionAvailability {
    let guard = test_guard(access, lock, "dev-1", Environment::Development);
    gate_availability(gate, &guard)
}

#[test]
fn copy_name_always_enabled() {
    for access in [
        checking(),
        unknown(),
        known_denying(&[]),
        known_denying(&AccessCheck::ALL),
    ] {
        assert_eq!(
            action_availability(ResourceAction::CopyName, &unlocked(&access)),
            ActionAvailability::Enabled
        );
    }
}

#[test]
fn gate_order_table() {
    let unshipped = ActionGate::Mutating {
        check: AccessCheck::PatchNodes,
        is_shipped: false,
    };
    let allowed = known_denying(&[]);
    let denied = known_denying(&[AccessCheck::PatchNodes]);
    let (checking, unknown) = (checking(), unknown());
    let cases = [
        (
            &unshipped,
            &allowed,
            WriteLock::Unlocked,
            "Comes in a later version",
        ),
        (
            &unshipped,
            &checking,
            WriteLock::Locked,
            "Comes in a later version",
        ),
        (
            &unshipped,
            &denied,
            WriteLock::Locked,
            "Comes in a later version",
        ),
        (
            &SHIPPED_PATCH_NODES,
            &checking,
            WriteLock::Locked,
            "Checking permissions…",
        ),
        (
            &SHIPPED_PATCH_NODES,
            &unknown,
            WriteLock::Locked,
            "Permissions could not be checked",
        ),
        (
            &SHIPPED_PATCH_NODES,
            &denied,
            WriteLock::Unlocked,
            "Not permitted: patch nodes",
        ),
        (
            &SHIPPED_PATCH_NODES,
            &allowed,
            WriteLock::Locked,
            "dev-1 is read-only",
        ),
    ];
    for (gate, access, lock, expected) in cases {
        assert_eq!(
            reason(availability_of_gate(gate, access, lock)),
            expected,
            "{lock:?}"
        );
    }
    assert_eq!(
        availability_of_gate(&SHIPPED_PATCH_NODES, &allowed, WriteLock::Unlocked),
        ActionAvailability::Enabled
    );
}

#[test]
fn rbac_reason_wins_over_the_lock() {
    let denied = known_denying(&[AccessCheck::PatchNodes]);
    assert_eq!(
        reason(availability_of_gate(
            &SHIPPED_PATCH_NODES,
            &denied,
            WriteLock::Locked
        )),
        "Not permitted: patch nodes"
    );
}

#[test]
fn read_only_actions_ignore_the_lock() {
    let allowed = known_denying(&[]);
    let guard = test_guard(
        &allowed,
        WriteLock::Locked,
        "prod-eu-1",
        Environment::Production,
    );
    for action in [
        ResourceAction::ViewLogs,
        ResourceAction::ViewYaml,
        ResourceAction::CopyName,
    ] {
        assert_eq!(
            action_availability(action, &guard),
            ActionAvailability::Enabled,
            "{action:?}"
        );
    }
}

#[test]
fn gate_and_confirm_use_the_rows_cluster() {
    let allowed = known_denying(&[]);
    let locked_dev = test_guard(
        &allowed,
        WriteLock::Locked,
        "dev-1",
        Environment::Development,
    );
    let open_prod = test_guard(
        &allowed,
        WriteLock::Unlocked,
        "prod-eu-1",
        Environment::Production,
    );
    // Whichever guard is asked first, each answers for its own cluster.
    for (first, second) in [(&locked_dev, &open_prod), (&open_prod, &locked_dev)] {
        let _ = gate_availability(&SHIPPED_PATCH_NODES, first);
        let _ = gate_availability(&SHIPPED_PATCH_NODES, second);
        assert_eq!(
            reason(gate_availability(&SHIPPED_PATCH_NODES, &locked_dev)),
            "dev-1 is read-only"
        );
        assert_eq!(
            gate_availability(&SHIPPED_PATCH_NODES, &open_prod),
            ActionAvailability::Enabled
        );
    }
    assert_eq!(
        confirm_step(
            open_prod.profile.confirm,
            ActionRisk::Change,
            open_prod.display_name()
        ),
        DialogConfirm::TypeName {
            expected: "prod-eu-1".to_owned()
        }
    );
    assert_eq!(open_prod.profile.environment, Environment::Production);
    assert_eq!(
        confirm_step(
            locked_dev.profile.confirm,
            ActionRisk::Change,
            locked_dev.display_name()
        ),
        DialogConfirm::Click
    );
}

#[test]
fn cordon_is_gated_on_patch_nodes_and_is_the_first_shipped_action() {
    assert!(matches!(
        ResourceAction::Cordon.gate(),
        ActionGate::Mutating {
            check: AccessCheck::PatchNodes,
            is_shipped: true
        }
    ));
}

#[test]
fn cordon_follows_the_gate_order() {
    let allowed = known_denying(&[]);
    let denied = known_denying(&[AccessCheck::PatchNodes]);
    let (checking, unknown) = (checking(), unknown());
    let at = |access: &AccessState, lock| {
        let guard = test_guard(access, lock, "dev-1", Environment::Development);
        action_availability(ResourceAction::Cordon, &guard)
    };
    assert_eq!(
        reason(at(&checking, WriteLock::Unlocked)),
        "Checking permissions…"
    );
    assert_eq!(
        reason(at(&unknown, WriteLock::Unlocked)),
        "Permissions could not be checked"
    );
    assert_eq!(
        reason(at(&denied, WriteLock::Locked)),
        "Not permitted: patch nodes"
    );
    assert_eq!(
        reason(at(&allowed, WriteLock::Locked)),
        "dev-1 is read-only"
    );
    assert_eq!(
        at(&allowed, WriteLock::Unlocked),
        ActionAvailability::Enabled
    );
}

#[test]
fn unshipped_mutating_actions_say_a_later_version() {
    for access in [
        checking(),
        unknown(),
        known_denying(&[]),
        known_denying(&AccessCheck::ALL),
    ] {
        for action in [
            ResourceAction::OpenShell,
            ResourceAction::PortForward,
            ResourceAction::OpenNodeShell,
            ResourceAction::Drain,
            ResourceAction::EditYaml,
            ResourceAction::Delete,
            ResourceAction::RestartRollout,
            ResourceAction::Scale,
        ] {
            assert_eq!(
                reason(action_availability(action, &unlocked(&access))),
                "Comes in a later version",
                "{action:?}"
            );
        }
    }
}

#[test]
fn logs_disabled_while_the_permissions_are_not_known() {
    assert_eq!(
        reason(action_availability(
            ResourceAction::ViewLogs,
            &unlocked(&checking())
        )),
        "Checking permissions…"
    );
    assert_eq!(
        reason(action_availability(
            ResourceAction::ViewLogs,
            &unlocked(&unknown())
        )),
        "Permissions could not be checked"
    );
}

#[test]
fn logs_allowed_is_enabled() {
    assert_eq!(
        action_availability(ResourceAction::ViewLogs, &unlocked(&known_denying(&[]))),
        ActionAvailability::Enabled
    );
}

#[test]
fn logs_denied_reason_names_access_check() {
    assert_eq!(
        reason(action_availability(
            ResourceAction::ViewLogs,
            &unlocked(&known_denying(&[AccessCheck::GetPodLogs]))
        )),
        "Not permitted: get pods/log"
    );
}

#[test]
fn forward_button_reason_says_the_feature_has_not_shipped() {
    for access in [
        known_denying(&[AccessCheck::CreatePodPortForward]),
        known_denying(&[]),
        checking(),
    ] {
        assert_eq!(port_forward_reason(&access), "Comes in a later version");
    }
}

#[test]
fn kubectl_command_quotes_only_unsafe_parts() {
    assert_eq!(
        kubectl_describe_command("readonly@Monitor", "shop", "api-0"),
        "kubectl --context readonly@Monitor -n shop describe pod api-0"
    );
    assert_eq!(
        kubectl_describe_command("my ctx", "shop", "it's"),
        "kubectl --context 'my ctx' -n shop describe pod 'it'\\''s'"
    );
}

fn pod_on(node: Option<&str>) -> PodSummary {
    PodSummary {
        namespace: "ns".to_owned(),
        name: "pod".to_owned(),
        status: cluster::PodStatus::Reason(cluster::StatusReason::Running),
        ready: cluster::ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: node.map(str::to_owned),
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: None,
        conditions: Vec::new(),
        containers: Vec::new(),
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        image_pull_secrets: Vec::new(),
    }
}

#[test]
fn view_pods_on_node_counts_pods_on_that_node() {
    let pods = [
        pod_on(Some("wk-01")),
        pod_on(Some("wk-02")),
        pod_on(Some("wk-01")),
        pod_on(None),
    ];
    assert_eq!(pods_on_node(&pods, "wk-01"), 2);
    assert_eq!(pods_on_node(&pods, "wk-02"), 1);
    // An empty node still has the item, with a zero.
    assert_eq!(pods_on_node(&pods, "wk-09"), 0);
}

fn event_with_reason(reason: Option<&str>) -> EventDetail {
    EventDetail {
        title: "t".into(),
        reason: reason.map(SharedString::from),
        object: None,
        source: None,
        message: "m".into(),
    }
}

#[test]
fn filter_similar_disabled_without_reason() {
    assert_eq!(similar_reason(&event_with_reason(None)), None);
    assert_eq!(similar_reason(&event_with_reason(Some(""))), None);
    assert_eq!(
        similar_reason(&event_with_reason(Some("BackOff"))).map(SharedString::as_ref),
        Some("BackOff")
    );
}

fn replica_set_row_owned_by(owner: Option<(&str, &str)>) -> KindRow {
    crate::workload_rows::replica_set_row(&cluster::ReplicaSetSummary {
        namespace: "team-a".to_owned(),
        name: "api-7d9f8c".to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired: 1,
        current: 1,
        ready: 1,
        owner: owner.map(|(kind, name)| cluster::ControllerRef {
            kind: kind.to_owned(),
            name: name.to_owned(),
        }),
        revision: None,
        selector: Vec::new(),
        containers: Vec::new(),
    })
}

#[test]
fn replica_set_menu_has_go_to_owner() {
    assert!(has_go_to_owner(ResourceKind::ReplicaSets));
    assert!(!has_go_to_owner(ResourceKind::Deployments));
    assert!(!has_go_to_owner(ResourceKind::Jobs));
    let row = replica_set_row_owned_by(Some(("Deployment", "api")));
    assert_eq!(
        owner_target(&row),
        Some(ResourceKey::Kind {
            kind: ResourceKind::Deployments,
            namespace: Some("team-a".to_owned()),
            name: "api".to_owned(),
        })
    );
}

#[test]
fn go_to_owner_disabled_without_owner() {
    assert_eq!(owner_target(&replica_set_row_owned_by(None)), None);
    // An owner kind without a screen cannot be revealed either.
    let unknown = replica_set_row_owned_by(Some(("ReplicationController", "old")));
    assert_eq!(owner_target(&unknown), None);
}

fn ingress_row_with(hosts: &[&str]) -> KindRow {
    let rules = hosts
        .iter()
        .map(|host| cluster::IngressPath {
            host: Some((*host).to_owned()),
            path: Some("/".to_owned()),
            backend: "api:80".to_owned(),
            service: Some("api".to_owned()),
        })
        .collect();
    crate::network_rows::ingress_row(&cluster::IngressSummary {
        namespace: "team-a".to_owned(),
        name: "web".to_owned(),
        created_at: None,
        labels: Vec::new(),
        class: None,
        hosts: hosts.iter().map(|host| (*host).to_owned()).collect(),
        addresses: Vec::new(),
        rules,
        default_backend: None,
        default_service: None,
        tls: Vec::new(),
    })
}

#[test]
fn open_url_disabled_without_host() {
    assert_eq!(
        open_url_choice(&ingress_row_with(&[])),
        OpenUrl::Unavailable
    );
    // A wildcard host has nothing to open either.
    assert_eq!(
        open_url_choice(&ingress_row_with(&["*.example.com"])),
        OpenUrl::Unavailable
    );
}

#[test]
fn open_url_opens_the_only_url_directly() {
    assert_eq!(
        open_url_choice(&ingress_row_with(&["a.example.com"])),
        OpenUrl::One("http://a.example.com/".to_owned())
    );
}

#[test]
fn open_url_submenu_for_several() {
    assert_eq!(
        open_url_choice(&ingress_row_with(&["a.example.com", "b.example.com"])),
        OpenUrl::Several(vec![
            "http://a.example.com/".to_owned(),
            "http://b.example.com/".to_owned()
        ])
    );
}

#[test]
fn open_url_submenu_lists_at_most_ten() {
    let hosts: Vec<String> = (0..12).map(|n| format!("h{n}.example.com")).collect();
    let hosts: Vec<&str> = hosts.iter().map(String::as_str).collect();
    let OpenUrl::Several(urls) = open_url_choice(&ingress_row_with(&hosts)) else {
        panic!("expected a submenu");
    };
    assert_eq!(urls.len(), 10);
}

#[test]
fn open_url_is_unavailable_for_other_kinds() {
    let row = crate::config_map_rows::config_map_row(&cluster::ConfigMapSummary {
        namespace: "team-a".to_owned(),
        name: "settings".to_owned(),
        created_at: None,
        labels: Vec::new(),
        keys: Vec::new(),
        is_immutable: false,
    });
    assert_eq!(open_url_choice(&row), OpenUrl::Unavailable);
}

fn hpa_row_targeting(kind: &str, name: &str) -> KindRow {
    crate::policy_rows::horizontal_pod_autoscaler_row(&cluster::HorizontalPodAutoscalerSummary {
        namespace: "team-a".to_owned(),
        name: "web".to_owned(),
        created_at: None,
        labels: Vec::new(),
        target: cluster::ControllerRef {
            kind: kind.to_owned(),
            name: name.to_owned(),
        },
        min_replicas: 1,
        max_replicas: 5,
        current_replicas: 2,
        desired_replicas: 2,
        metrics: Vec::new(),
        conditions: Vec::new(),
        last_scaled_at: None,
    })
}

#[test]
fn hpa_menu_has_go_to_target() {
    assert!(has_go_to_target(ResourceKind::HorizontalPodAutoscalers));
    assert!(!has_go_to_target(ResourceKind::Deployments));
    assert_eq!(
        scale_target(&hpa_row_targeting("Deployment", "web")),
        Some(ResourceKey::Kind {
            kind: ResourceKind::Deployments,
            namespace: Some("team-a".to_owned()),
            name: "web".to_owned(),
        })
    );
}

#[test]
fn go_to_target_disabled_without_screen() {
    let row = hpa_row_targeting("Rollout", "web");
    assert_eq!(scale_target(&row), None);
    assert_eq!(
        no_target_screen_reason(&row).as_ref(),
        "No screen for Rollout"
    );
}

fn claim_row() -> KindRow {
    crate::storage_rows::persistent_volume_claim_row(&cluster::PersistentVolumeClaimSummary {
        namespace: "shop".to_owned(),
        name: "data".to_owned(),
        created_at: None,
        labels: Vec::new(),
        phase: "Bound".to_owned(),
        is_terminating: false,
        volume: Some("pv-1".to_owned()),
        capacity: None,
        requested: None,
        access_modes: Vec::new(),
        storage_class: None,
        volume_mode: None,
        conditions: Vec::new(),
    })
}

fn pod_mounting_claim(namespace: &str, name: &str, claim: &str) -> PodSummary {
    let mut pod = pod_on(Some("wk-01"));
    pod.namespace = namespace.to_owned();
    pod.name = name.to_owned();
    pod.containers.push(cluster::ContainerSummary {
        name: "main".to_owned(),
        image: "img".to_owned(),
        kind: cluster::ContainerKind::Main,
        state: cluster::ContainerState::Running { started_at: None },
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
        mounts: vec![cluster::MountEntry {
            path: "/data".to_owned(),
            volume: "data".to_owned(),
            source: cluster::VolumeSource::PersistentVolumeClaim {
                claim: claim.to_owned(),
            },
            is_read_only: false,
            sub_path: None,
        }],
    });
    pod
}

#[test]
fn pvc_menu_go_to_first_mounting_pod() {
    let pods = [
        pod_mounting_claim("shop", "web-2", "data"),
        pod_mounting_claim("shop", "web-1", "data"),
    ];
    assert_eq!(
        claim_pod_target(&claim_row(), &pods),
        Some(ResourceKey::Pod {
            namespace: "shop".to_owned(),
            name: "web-1".to_owned(),
        })
    );
}

#[test]
fn pvc_go_to_pod_disabled_when_unmounted() {
    let pods = [
        pod_mounting_claim("shop", "web-1", "other"),
        pod_mounting_claim("elsewhere", "web-2", "data"),
    ];
    assert_eq!(claim_pod_target(&claim_row(), &pods), None);
    assert_eq!(claim_pod_target(&claim_row(), &[]), None);
}

#[test]
fn binding_menu_has_go_to_role() {
    let binding = |namespace: Option<&str>, kind| cluster::BindingSummary {
        namespace: namespace.map(str::to_owned),
        name: "bind".to_owned(),
        created_at: None,
        labels: Vec::new(),
        role: cluster::RoleRef {
            kind,
            name: "reader".to_owned(),
        },
        subjects: Vec::new(),
    };
    let row_of = |binding: &cluster::BindingSummary| match binding.namespace {
        Some(_) => crate::access_rows::role_binding_row(binding),
        None => crate::access_rows::cluster_role_binding_row(binding),
    };
    let role = binding(Some("shop"), cluster::RoleKind::Role);
    assert_eq!(
        binding_role_target(&row_of(&role)),
        Some(ResourceKey::Kind {
            kind: ResourceKind::Roles,
            namespace: Some("shop".to_owned()),
            name: "reader".to_owned(),
        })
    );
    let cluster_role = binding(None, cluster::RoleKind::ClusterRole);
    assert_eq!(
        binding_role_target(&row_of(&cluster_role)),
        Some(ResourceKey::Kind {
            kind: ResourceKind::ClusterRoles,
            namespace: None,
            name: "reader".to_owned(),
        })
    );
    // A kind k8sBoard has no screen for disables the item.
    let other = binding(Some("shop"), cluster::RoleKind::Other("Weird".to_owned()));
    assert_eq!(binding_role_target(&row_of(&other)), None);
}

#[test]
fn pv_menu_go_to_claim() {
    let volume = |claim: Option<(&str, &str)>| {
        crate::storage_rows::persistent_volume_row(&cluster::PersistentVolumeSummary {
            name: "pv-1".to_owned(),
            created_at: None,
            labels: Vec::new(),
            capacity: None,
            access_modes: Vec::new(),
            reclaim_policy: "Delete".to_owned(),
            phase: "Bound".to_owned(),
            is_terminating: false,
            claim: claim.map(|(namespace, name)| cluster::ClaimRef {
                namespace: namespace.to_owned(),
                name: name.to_owned(),
            }),
            storage_class: None,
            volume_mode: None,
            backend: cluster::VolumeBackend::Other { kind: "unknown" },
            node_affinity: Vec::new(),
            mount_options: Vec::new(),
            reason: None,
            message: None,
        })
    };
    assert_eq!(
        volume_claim_target(&volume(Some(("shop", "data")))),
        Some(ResourceKey::Kind {
            kind: ResourceKind::PersistentVolumeClaims,
            namespace: Some("shop".to_owned()),
            name: "data".to_owned(),
        })
    );
    assert_eq!(volume_claim_target(&volume(None)), None);
}

// ---- Secret menu ----

fn secret_key(name: &str, is_binary: bool) -> SecretKey {
    SecretKey {
        name: name.to_owned(),
        size_bytes: 8,
        is_binary,
    }
}

#[test]
fn secret_menu_order() {
    let keys = [secret_key("password", false), secret_key("username", false)];
    let model = secret_menu_model(&keys, ValueAccess::Enabled);
    assert_eq!(model.reveal, MenuState::Enabled);
    let labels: Vec<&str> = model
        .copies
        .iter()
        .map(|entry| entry.label.as_str())
        .collect();
    // One Copy entry per key, in key order.
    assert_eq!(labels, ["Copy password", "Copy username"]);
    assert!(
        model
            .copies
            .iter()
            .all(|entry| entry.state == MenuState::Enabled)
    );
    assert_eq!(model.copies[1].key.as_deref(), Some("username"));
}

#[test]
fn copy_submenu_disables_binary_keys() {
    let keys = [secret_key("blob", true), secret_key("user", false)];
    let model = secret_menu_model(&keys, ValueAccess::Enabled);
    assert_eq!(model.copies[0].state, MenuState::Disabled("Binary value"));
    assert_eq!(model.copies[1].state, MenuState::Enabled);
}

#[test]
fn secret_without_keys_offers_no_data() {
    let model = secret_menu_model(&[], ValueAccess::Enabled);
    assert_eq!(model.reveal, MenuState::Disabled("No data"));
    assert_eq!(
        model.copies,
        [CopyEntry {
            label: "No data".to_owned(),
            key: None,
            state: MenuState::Disabled("No data")
        }]
    );
}

#[test]
fn blocked_access_disables_every_secret_item() {
    let keys = [secret_key("password", false), secret_key("blob", true)];
    let model = secret_menu_model(&keys, ValueAccess::Blocked);
    let blocked = MenuState::Disabled("Disabled in screenshot runs");
    assert_eq!(model.reveal, blocked);
    assert!(model.copies.iter().all(|entry| entry.state == blocked));
    let empty = secret_menu_model(&[], ValueAccess::Blocked);
    assert_eq!(empty.reveal, blocked);
    assert_eq!(empty.copies[0].state, blocked);
}

/// The screenshot gate, end to end: launch options with a screenshot output give `Blocked`, and
/// the menu built from that same value has no enabled Reveal or Copy.
#[test]
fn secret_menu_blocked_in_screenshot_runs() {
    let options = crate::launch_options::LaunchOptions {
        kubeconfig: None,
        context: None,
        namespace: None,
        view: Vec::new(),
        filter: None,
        select: None,
        theme: None,
        config_dir: None,
        screen: crate::launch_options::LaunchScreen::Kind(ResourceKind::Secrets),
        screenshot: Some("secrets.png".into()),
        window_width: None,
        palette: None,
    };
    let access = crate::secret_values::value_access(&options);
    assert_eq!(access, ValueAccess::Blocked);
    let keys = [secret_key("password", false)];
    let model = secret_menu_model(&keys, access);
    assert!(matches!(model.reveal, MenuState::Disabled(_)));
    assert!(
        model
            .copies
            .iter()
            .all(|entry| matches!(entry.state, MenuState::Disabled(_)))
    );
}

#[test]
fn helm_release_menu_disables_rollback_and_uninstall() {
    // The menu shows the disabled items from the kind's data, and View YAML only for a key with an
    // object reference, which a release has not.
    let labels: Vec<&str> = ResourceKind::HelmReleases
        .read_only_actions()
        .iter()
        .map(|item| item.label)
        .collect();
    assert_eq!(labels, ["Roll back…"]);
    assert_eq!(
        ResourceKind::HelmReleases.delete_label(),
        "Uninstall release…"
    );
    let key = ResourceKey::Kind {
        kind: ResourceKind::HelmReleases,
        namespace: Some("shop".to_owned()),
        name: "api".to_owned(),
    };
    assert!(object_ref(&key).is_none());
}

#[test]
fn helm_release_menu_opens_values_and_manifest() {
    // The menu model: one item per Helm view tab, in order, and each tab exists on the release
    // drawer, so the click reaches a tab that is shown.
    let labels: Vec<&str> = HELM_VIEW_ITEMS.iter().map(|(label, _)| *label).collect();
    assert_eq!(labels, ["View values", "View manifest"]);
    let key = ResourceKey::Kind {
        kind: ResourceKind::HelmReleases,
        namespace: Some("shop".to_owned()),
        name: "api".to_owned(),
    };
    let tabs = crate::drawer::drawer_tabs(&key);
    for (_, tab) in HELM_VIEW_ITEMS {
        assert!(tabs.contains(&tab), "{tab:?}");
    }
    assert_eq!(HELM_VIEW_ITEMS[0].1, DrawerTab::Values);
    assert_eq!(HELM_VIEW_ITEMS[1].1, DrawerTab::Manifest);
}

fn served_widget(crd_name: &str) -> crate::custom_kind::CustomKind {
    let (plural, group) = crd_name.split_once('.').expect("a CRD name has a group");
    let crd = cluster::CrdSummary {
        name: crd_name.to_owned(),
        group: group.to_owned(),
        kind: "Widget".to_owned(),
        plural: plural.to_owned(),
        singular: "widget".to_owned(),
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
    crate::custom_kind::custom_kinds(&[crd], &mut crate::custom_kind::CustomKindCache::default())[0]
}

#[test]
fn browse_instances_opens_the_custom_kind() {
    let widgets = served_widget("widgets.x.io");
    let gadgets = served_widget("gadgets.x.io");
    assert_eq!(
        browse_target("widgets.x.io", &[gadgets, widgets]),
        Some(widgets)
    );
}

#[test]
fn browse_instances_disabled_without_a_kind() {
    let gadgets = served_widget("gadgets.x.io");
    assert_eq!(browse_target("widgets.x.io", &[gadgets]), None);
    assert_eq!(browse_target("widgets.x.io", &[]), None);
}

#[test]
fn custom_menus_have_no_read_only_actions() {
    let kind = ResourceKind::Custom(served_widget("widgets.x.io"));
    assert!(kind.read_only_actions().is_empty());
    assert!(!kind.has_port_forward());
    assert_eq!(kind.delete_label(), "Delete widget…");
}

fn deployment_owner() -> PodOwner {
    PodOwner::Deployment {
        namespace: "team-a".to_owned(),
        name: "api".to_owned(),
    }
}

#[test]
fn workload_menu_offers_view_logs_when_allowed() {
    let entry = workload_logs_entry(Some(&deployment_owner()), &known_denying(&[]));
    assert_eq!(
        entry,
        Some(("View logs (all pods)", ActionAvailability::Enabled))
    );
}

#[test]
fn job_menu_labels_view_logs() {
    let owner = PodOwner::Controller {
        namespace: "team-a".to_owned(),
        kind: JOB_KIND,
        name: "migrate".to_owned(),
    };
    let entry = workload_logs_entry(Some(&owner), &known_denying(&[]));
    assert_eq!(entry, Some(("View logs", ActionAvailability::Enabled)));
}

#[test]
fn workload_view_logs_denied_reason_names_access_check() {
    let access = known_denying(&[AccessCheck::GetPodLogs]);
    let (_, availability) =
        workload_logs_entry(Some(&deployment_owner()), &access).expect("a workload entry");
    assert_eq!(
        reason(availability),
        format!("Not permitted: {}", AccessCheck::GetPodLogs)
    );
}

#[test]
fn non_workload_kind_menu_has_no_view_logs() {
    // A ConfigMap row has no related pods; a node has them but is not one workload.
    assert_eq!(workload_logs_entry(None, &known_denying(&[])), None);
    let node = PodOwner::Node { name: "n1".into() };
    assert_eq!(workload_logs_entry(Some(&node), &known_denying(&[])), None);
}

fn role_with_rules(rules: Vec<cluster::RbacRule>) -> KindRow {
    crate::access_rows::role_row(&cluster::RoleSummary {
        namespace: Some("shop".to_owned()),
        name: "reader".to_owned(),
        created_at: None,
        labels: Vec::new(),
        rules,
        aggregation: Vec::new(),
    })
}

#[test]
fn role_menus_start_with_who_can() {
    assert!(has_who_can(ResourceKind::Roles));
    assert!(has_who_can(ResourceKind::ClusterRoles));
    assert!(!has_who_can(ResourceKind::RoleBindings));
    assert!(!has_who_can(ResourceKind::ServiceAccounts));
}

#[test]
fn network_policy_menu_starts_with_test_traffic() {
    assert!(has_test_traffic(ResourceKind::NetworkPolicies));
    assert!(!has_test_traffic(ResourceKind::Ingresses));
    assert!(!has_test_traffic(ResourceKind::Services));
}

#[test]
fn service_account_menu_starts_with_check_permissions() {
    assert!(has_check_permissions(ResourceKind::ServiceAccounts));
    assert!(!has_check_permissions(ResourceKind::Roles));
    assert!(!has_check_permissions(ResourceKind::Secrets));
}

#[test]
fn who_can_query_prefills_from_the_first_resource_rule() {
    let row = role_with_rules(vec![cluster::RbacRule {
        api_groups: vec!["apps".to_owned()],
        resources: vec!["deployments".to_owned()],
        resource_names: Vec::new(),
        verbs: vec!["get".to_owned()],
        non_resource_urls: Vec::new(),
    }]);
    assert_eq!(who_can_query(&row).as_deref(), Some("get deployments.apps"));
    assert_eq!(who_can_query(&role_with_rules(Vec::new())), None);
    assert_eq!(who_can_query(&claim_row()), None);
}

#[test]
fn namespaces_menu_has_set_as_default() {
    assert_eq!(
        default_namespace_state(ResourceKind::Namespaces, "team-a", None),
        Some(false)
    );
    assert_eq!(
        default_namespace_state(ResourceKind::Namespaces, "team-a", Some("team-b")),
        Some(false)
    );
}

#[test]
fn set_as_default_is_checked_for_the_default() {
    assert_eq!(
        default_namespace_state(ResourceKind::Namespaces, "team-a", Some("team-a")),
        Some(true)
    );
}

#[test]
fn other_kinds_have_no_set_as_default() {
    for kind in [ResourceKind::Deployments, ResourceKind::Services] {
        assert_eq!(default_namespace_state(kind, "x", None), None);
    }
}

#[test]
fn logs_launch_denied_without_log_access() {
    let pod = pod_mounting_claim("shop", "web-1", "data");
    let denied = known_denying(&[AccessCheck::GetPodLogs]);
    let Err(reason) = logs_launch(&pod, None, &denied) else {
        panic!("logs must be denied");
    };
    assert!(reason.contains("get pods/log"), "{reason}");
    assert!(logs_launch(&pod, None, &known_denying(&[])).is_ok());
}

#[test]
fn logs_launch_needs_containers() {
    let pod = pod_on(Some("wk-01"));
    let Err(reason) = logs_launch(&pod, None, &known_denying(&[])) else {
        panic!("a pod without containers has nothing to read");
    };
    assert_eq!(reason, "The pod has no containers");
}

#[test]
fn show_in_topology_is_offered_for_services_and_ingresses_in_scope() {
    let scope = NamespaceScope::Named("shop".to_owned());
    for kind in [ResourceKind::Services, ResourceKind::Ingresses] {
        assert_eq!(
            topology_menu(kind, Some("shop"), Some(&scope), None),
            TopologyMenu::Enabled
        );
    }
    assert_eq!(
        topology_menu(ResourceKind::Deployments, Some("shop"), Some(&scope), None),
        TopologyMenu::Hidden
    );
    assert_eq!(
        topology_menu(ResourceKind::Services, Some("shop"), None, None),
        TopologyMenu::Hidden
    );
}

#[test]
fn show_in_topology_disabled_outside_scope() {
    let scope = NamespaceScope::Named("blog".to_owned());
    assert_eq!(
        topology_menu(ResourceKind::Services, Some("shop"), Some(&scope), None),
        TopologyMenu::Disabled("Namespace shop is outside the scope".to_owned())
    );
    // All namespaces include every one.
    assert_eq!(
        topology_menu(
            ResourceKind::Services,
            Some("shop"),
            Some(&NamespaceScope::All),
            None
        ),
        TopologyMenu::Enabled
    );
}

// ---- Row keys ----

fn pod_key() -> ResourceKey {
    ResourceKey::Pod {
        namespace: "shop".to_owned(),
        name: "api-0".to_owned(),
    }
}

fn node_key() -> ResourceKey {
    ResourceKey::Node {
        name: "node-1".to_owned(),
    }
}

fn kind_key(kind: ResourceKind) -> ResourceKey {
    ResourceKey::Kind {
        kind,
        namespace: Some("shop".to_owned()),
        name: "api".to_owned(),
    }
}

fn container_of(name: &str) -> cluster::ContainerSummary {
    cluster::ContainerSummary {
        name: name.to_owned(),
        image: "img".to_owned(),
        kind: cluster::ContainerKind::Main,
        state: cluster::ContainerState::Running { started_at: None },
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
    }
}

fn pod_with(containers: Vec<cluster::ContainerSummary>) -> PodSummary {
    PodSummary {
        namespace: "shop".to_owned(),
        name: "api-0".to_owned(),
        status: cluster::PodStatus::Reason(cluster::StatusReason::Running),
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
        containers,
    }
}

fn availability(row: RowAction, subject: &ResourceKey, access: &AccessState) -> KeyAvailability {
    let pod = pod_with(vec![container_of("app")]);
    key_availability_of(
        row,
        subject,
        subject.is_pod(&pod).then_some(&pod),
        &unlocked(access),
    )
}

fn disabled_reason(availability: KeyAvailability) -> String {
    match availability {
        KeyAvailability::Disabled { reason } => reason.to_string(),
        other => panic!("expected a disabled key, got {other:?}"),
    }
}

#[test]
fn key_availability_offers_logs_only_for_pods() {
    let access = known_denying(&[]);
    assert_eq!(
        availability(RowAction::ViewLogs, &pod_key(), &access),
        KeyAvailability::Run(ResourceAction::ViewLogs)
    );
    for subject in [node_key(), kind_key(ResourceKind::Services)] {
        assert_eq!(
            availability(RowAction::ViewLogs, &subject, &access),
            KeyAvailability::NotOffered
        );
    }
}

#[test]
fn key_availability_explains_a_pod_without_containers() {
    let pod = pod_with(Vec::new());
    let access = known_denying(&[]);
    let availability = key_availability_of(
        RowAction::ViewLogs,
        &pod_key(),
        Some(&pod),
        &unlocked(&access),
    );
    assert_eq!(disabled_reason(availability), "The pod has no containers");
}

#[test]
fn key_availability_disables_unshipped_keys_with_the_later_version_reason() {
    let access = known_denying(&[]);
    let offered = [
        (RowAction::EditYaml, pod_key()),
        (RowAction::Delete, pod_key()),
        (RowAction::Drain, node_key()),
        (
            RowAction::RestartRollout,
            kind_key(ResourceKind::Deployments),
        ),
        (RowAction::Scale, kind_key(ResourceKind::Deployments)),
    ];
    for (action, subject) in offered {
        assert_eq!(
            disabled_reason(availability(action, &subject, &access)),
            "Comes in a later version",
            "{action:?}"
        );
    }
    // Where a subject has no such action, the key is silent.
    assert_eq!(
        availability(RowAction::Cordon, &pod_key(), &access),
        KeyAvailability::NotOffered
    );
}

#[test]
fn key_availability_uses_the_access_gate() {
    let denied = known_denying(&[AccessCheck::GetPodLogs]);
    assert_eq!(
        disabled_reason(availability(RowAction::ViewLogs, &pod_key(), &denied)),
        "Not permitted: get pods/log"
    );
    assert_eq!(
        disabled_reason(availability(RowAction::ViewLogs, &pod_key(), &checking())),
        "Checking permissions…"
    );
}

#[test]
fn key_availability_of_a_shell_waits_for_its_spec_before_its_permission() {
    let denied = known_denying(&[AccessCheck::CreatePodExec]);
    for subject in [pod_key(), node_key()] {
        assert_eq!(
            disabled_reason(availability(RowAction::OpenShell, &subject, &denied)),
            "Comes in a later version"
        );
    }
}

#[test]
fn key_availability_offers_restart_and_scale_from_kind_actions() {
    let access = known_denying(&[]);
    let offers = |kind: ResourceKind, action: RowAction| {
        availability(action, &kind_key(kind), &access) != KeyAvailability::NotOffered
    };
    assert!(offers(ResourceKind::Deployments, RowAction::RestartRollout));
    assert!(offers(ResourceKind::Deployments, RowAction::Scale));
    assert!(offers(ResourceKind::DaemonSets, RowAction::RestartRollout));
    assert!(!offers(ResourceKind::DaemonSets, RowAction::Scale));
    assert!(!offers(ResourceKind::Services, RowAction::RestartRollout));
    assert!(!offers(ResourceKind::Services, RowAction::Scale));
}

#[test]
fn view_yaml_and_copy_name_always_run() {
    for access in [checking(), unknown(), known_denying(&AccessCheck::ALL)] {
        for subject in [pod_key(), node_key(), kind_key(ResourceKind::ConfigMaps)] {
            for (row, resolved) in [
                (RowAction::ViewYaml, ResourceAction::ViewYaml),
                (RowAction::CopyName, ResourceAction::CopyName),
            ] {
                assert_eq!(
                    availability(row, &subject, &access),
                    KeyAvailability::Run(resolved)
                );
            }
        }
    }
}

#[test]
fn a_helm_release_has_no_yaml_key() {
    // Its YAML would show the release Secret, which holds values.
    assert_eq!(
        availability(
            RowAction::ViewYaml,
            &kind_key(ResourceKind::HelmReleases),
            &known_denying(&[])
        ),
        KeyAvailability::NotOffered
    );
}

#[test]
fn subject_action_resolves_the_carried_kind() {
    // `Scale` carries no kind yet, so the kind table decides only whether it is offered.
    assert_eq!(
        subject_action(RowAction::Scale, &kind_key(ResourceKind::Deployments)),
        Some(ResourceAction::Scale)
    );
    assert_eq!(
        subject_action(RowAction::Scale, &kind_key(ResourceKind::DaemonSets)),
        None
    );
    assert_eq!(
        subject_action(RowAction::OpenShell, &node_key()),
        Some(ResourceAction::OpenNodeShell)
    );
    assert_eq!(
        subject_action(RowAction::OpenShell, &pod_key()),
        Some(ResourceAction::OpenShell)
    );
    assert_eq!(subject_action(RowAction::ViewLogs, &node_key()), None);
}

#[test]
fn every_resource_action_has_a_row_action() {
    let subjects = [
        (ResourceAction::ViewLogs, pod_key()),
        (ResourceAction::OpenShell, pod_key()),
        (ResourceAction::PortForward, pod_key()),
        (ResourceAction::OpenNodeShell, node_key()),
        (ResourceAction::Cordon, node_key()),
        (ResourceAction::Drain, node_key()),
        (ResourceAction::CopyName, pod_key()),
        (ResourceAction::ViewYaml, pod_key()),
        (ResourceAction::EditYaml, pod_key()),
        (ResourceAction::Delete, pod_key()),
        (
            ResourceAction::RestartRollout,
            kind_key(ResourceKind::Deployments),
        ),
        (ResourceAction::Scale, kind_key(ResourceKind::Deployments)),
    ];
    for (action, subject) in subjects {
        assert_eq!(
            subject_action(action.row_action(), &subject),
            Some(action),
            "{action:?}"
        );
    }
}

#[test]
fn an_unavailable_key_says_what_and_why() {
    assert_eq!(
        unavailable_text(
            action_label(ResourceAction::EditYaml),
            "Comes in a later version"
        ),
        "Edit YAML is unavailable: Comes in a later version"
    );
}

#[test]
fn menu_hints_name_the_key_action() {
    use crate::keymap::{OpenShell, ViewLogs};
    assert!(RowAction::ViewLogs.key_action().partial_eq(&ViewLogs));
    // The node shell shares the pod shell key.
    assert!(
        ResourceAction::OpenNodeShell
            .row_action()
            .key_action()
            .partial_eq(&OpenShell)
    );
}

#[test]
fn show_in_topology_is_disabled_for_a_row_of_another_cluster() {
    // Topology draws the primary cluster alone, so a row of any other viewed cluster cannot be
    // shown there.
    assert_eq!(
        topology_menu(
            ResourceKind::Services,
            Some("shop"),
            Some(&NamespaceScope::All),
            Some("prod-eu")
        ),
        TopologyMenu::Disabled("Topology draws only the primary cluster (prod-eu)".to_owned())
    );
}
