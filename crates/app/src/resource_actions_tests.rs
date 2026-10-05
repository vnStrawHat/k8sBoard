use cluster::{
    AccessDecision, AccessReport, AccessReview, ContainerKind, ContainerState, ContainerSummary,
    NamespacePhase, NamespaceSummary, PodDisruptionBudgetSummary,
};
use gpui_kit::Task;

use super::*;
use crate::environment::Environment;
use crate::write_guard::{ActionRisk, ConfirmMode, DialogConfirm, confirm_step, test_guard};

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
        Environment::DEVELOPMENT,
    )
}

/// A mutating action whose spec has shipped, which no `ResourceAction` is before step 4.
static SHIPPED_PATCH_NODES: std::sync::LazyLock<ActionGate> =
    std::sync::LazyLock::new(|| ActionGate::Mutating {
        checks: vec![AccessCheck::PatchNodes],
        is_shipped: true,
    });

fn availability_of_gate(
    gate: &ActionGate,
    access: &AccessState,
    lock: WriteLock,
) -> ActionAvailability {
    let guard = test_guard(access, lock, "dev-1", Environment::DEVELOPMENT);
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
        checks: vec![AccessCheck::PatchNodes],
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
        Environment::PRODUCTION,
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
        Environment::DEVELOPMENT,
    );
    let open_prod = test_guard(
        &allowed,
        WriteLock::Unlocked,
        "prod-eu-1",
        Environment::PRODUCTION,
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
    assert_eq!(open_prod.profile.environment, Environment::PRODUCTION);
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
        ActionGate::Mutating { checks, is_shipped: true } if checks == [AccessCheck::PatchNodes]
    ));
}

#[test]
fn cordon_follows_the_gate_order() {
    let allowed = known_denying(&[]);
    let denied = known_denying(&[AccessCheck::PatchNodes]);
    let (checking, unknown) = (checking(), unknown());
    let at = |access: &AccessState, lock| {
        let guard = test_guard(access, lock, "dev-1", Environment::DEVELOPMENT);
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
fn node_edits_and_uncordon_share_the_cordon_gate() {
    let denied = known_denying(&[AccessCheck::PatchNodes]);
    let allowed = known_denying(&[]);
    for action in [
        ResourceAction::Uncordon,
        ResourceAction::EditTaints,
        ResourceAction::EditLabels,
    ] {
        assert!(matches!(
            action.gate(),
            ActionGate::Mutating { checks, is_shipped: true } if checks == [AccessCheck::PatchNodes]
        ));
        assert_eq!(
            reason(action_availability(action, &unlocked(&denied))),
            "Not permitted: patch nodes",
            "{action:?}"
        );
        assert_eq!(
            reason(action_availability(
                action,
                &test_guard(
                    &allowed,
                    WriteLock::Locked,
                    "dev-1",
                    Environment::DEVELOPMENT
                )
            )),
            "dev-1 is read-only",
            "{action:?}"
        );
        assert_eq!(
            action_availability(action, &unlocked(&allowed)),
            ActionAvailability::Enabled,
            "{action:?}"
        );
    }
}

#[test]
fn node_editors_are_offered_on_nodes_only_and_uncordon_has_no_key() {
    assert_eq!(
        subject_action(RowAction::EditTaints, &node_key()),
        Some(ResourceAction::EditTaints)
    );
    assert_eq!(
        subject_action(RowAction::EditLabels, &node_key()),
        Some(ResourceAction::EditLabels)
    );
    assert_eq!(subject_action(RowAction::EditTaints, &pod_key()), None);
    assert_eq!(subject_action(RowAction::EditLabels, &pod_key()), None);
    // The C key resolves to Cordon, whose label follows the node; Uncordon is the bulk button's.
    assert_eq!(
        subject_action(RowAction::Cordon, &node_key()),
        Some(ResourceAction::Cordon)
    );
    assert_eq!(
        ResourceAction::Uncordon.row_action(),
        Some(RowAction::Cordon)
    );
}

#[test]
fn node_edits_are_changes_and_labelled_for_the_menu() {
    assert_eq!(action_risk(ResourceAction::EditTaints), ActionRisk::Change);
    assert_eq!(action_risk(ResourceAction::EditLabels), ActionRisk::Change);
    assert_eq!(action_risk(ResourceAction::Uncordon), ActionRisk::Change);
    assert_eq!(action_label(ResourceAction::EditTaints), "Edit taints");
    assert_eq!(action_label(ResourceAction::EditLabels), "Edit labels");
    assert_eq!(action_label(ResourceAction::Uncordon), "Uncordon");
}

#[test]
fn unshipped_mutating_actions_say_a_later_version() {
    for access in [
        checking(),
        unknown(),
        known_denying(&[]),
        known_denying(&AccessCheck::ALL),
    ] {
        // A kind that has no such action yet has no permission check either.
        let action = ResourceAction::Scale(ObjectKind::Pod);
        assert_eq!(
            reason(action_availability(action, &unlocked(&access))),
            "Comes in a later version",
            "{action:?}"
        );
    }
}

#[test]
fn drain_needs_eviction_and_cordon_rights_in_that_order() {
    assert!(matches!(
        ResourceAction::Drain.gate(),
        ActionGate::Mutating { checks, is_shipped: true }
            if checks == [AccessCheck::CreatePodEviction, AccessCheck::PatchNodes]
    ));
    let at = |denied: &[AccessCheck]| {
        reason(action_availability(
            ResourceAction::Drain,
            &unlocked(&known_denying(denied)),
        ))
    };
    assert_eq!(at(&AccessCheck::ALL), "Not permitted: create pods/eviction");
    assert_eq!(at(&[AccessCheck::PatchNodes]), "Not permitted: patch nodes");
    assert_eq!(
        action_availability(ResourceAction::Drain, &unlocked(&known_denying(&[]))),
        ActionAvailability::Enabled
    );
    assert_eq!(
        reason(action_availability(
            ResourceAction::Drain,
            &test_guard(
                &known_denying(&[]),
                WriteLock::Locked,
                "dev-1",
                Environment::DEVELOPMENT
            )
        )),
        "dev-1 is read-only"
    );
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
fn port_forward_needs_get_and_create() {
    let guard =
        |access: &AccessState| action_availability(ResourceAction::PortForward, &unlocked(access));
    assert_eq!(guard(&known_denying(&[])), ActionAvailability::Enabled);
    for denied in [
        AccessCheck::GetPodPortForward,
        AccessCheck::CreatePodPortForward,
    ] {
        assert_eq!(
            reason(guard(&known_denying(&[denied]))),
            "Not permitted: get and create pods/portforward"
        );
    }
    assert_eq!(reason(guard(&checking())), "Checking permissions…");
    assert_eq!(
        reason(guard(&unknown())),
        "Permissions could not be checked"
    );
}

#[test]
fn port_forward_is_off_while_the_cluster_is_locked() {
    let access = known_denying(&[]);
    let locked = test_guard(
        &access,
        WriteLock::Locked,
        "prod-1",
        Environment::PRODUCTION,
    );
    assert_eq!(
        reason(action_availability(ResourceAction::PortForward, &locked)),
        "prod-1 is read-only"
    );
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
        is_finished: false,
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
        terminal: cluster::ContainerTerminal::None,
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
        filter: None,
        select: None,
        theme: None,
        color_theme: None,
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
    let labels: Vec<&str> = HELM_VIEW_ITEMS.iter().map(|(label, ..)| *label).collect();
    assert_eq!(labels, ["View values", "View manifest"]);
    let key = ResourceKey::Kind {
        kind: ResourceKind::HelmReleases,
        namespace: Some("shop".to_owned()),
        name: "api".to_owned(),
    };
    let tabs = crate::drawer::drawer_tabs(&key);
    for (_, tab, _) in HELM_VIEW_ITEMS {
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
            topology_menu(kind, Some("shop"), Some(&scope)),
            TopologyMenu::Enabled
        );
    }
    assert_eq!(
        topology_menu(ResourceKind::Deployments, Some("shop"), Some(&scope)),
        TopologyMenu::Hidden
    );
    assert_eq!(
        topology_menu(ResourceKind::Services, Some("shop"), None),
        TopologyMenu::Hidden
    );
}

#[test]
fn show_in_topology_disabled_outside_scope() {
    let scope = NamespaceScope::Named("blog".to_owned());
    assert_eq!(
        topology_menu(ResourceKind::Services, Some("shop"), Some(&scope)),
        TopologyMenu::Disabled("Namespace shop is outside the scope".to_owned())
    );
    // All namespaces include every one.
    assert_eq!(
        topology_menu(
            ResourceKind::Services,
            Some("shop"),
            Some(&NamespaceScope::All)
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
        terminal: cluster::ContainerTerminal::None,
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
        is_finished: false,
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
fn key_availability_of_the_node_keys_follows_the_gate() {
    let access = known_denying(&[]);
    for action in [
        RowAction::Drain,
        RowAction::EditTaints,
        RowAction::EditLabels,
    ] {
        assert!(
            matches!(
                availability(action, &node_key(), &access),
                KeyAvailability::Run(_)
            ),
            "{action:?}"
        );
        assert_eq!(
            availability(action, &pod_key(), &access),
            KeyAvailability::NotOffered,
            "{action:?}"
        );
    }
    let denied = known_denying(&[AccessCheck::CreatePodEviction]);
    assert_eq!(
        disabled_reason(availability(RowAction::Drain, &node_key(), &denied)),
        "Not permitted: create pods/eviction"
    );
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
fn key_availability_of_a_pod_shell_reads_both_exec_verbs_and_a_node_shell_its_own_checks() {
    let denied = known_denying(&[AccessCheck::CreatePodExec]);
    assert_eq!(
        disabled_reason(availability(RowAction::OpenShell, &pod_key(), &denied)),
        "Not permitted: get and create pods/exec"
    );
    // A node shell needs create and delete on pods and the attach pair, not the exec verbs.
    assert_eq!(
        availability(RowAction::OpenShell, &node_key(), &denied),
        KeyAvailability::Run(ResourceAction::OpenNodeShell)
    );
    assert_eq!(
        availability(RowAction::OpenShell, &pod_key(), &known_denying(&[])),
        KeyAvailability::Run(ResourceAction::OpenShell)
    );
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
    assert_eq!(
        subject_action(RowAction::Scale, &kind_key(ResourceKind::Deployments)),
        Some(ResourceAction::Scale(ObjectKind::Deployment))
    );
    assert_eq!(
        subject_action(RowAction::Scale, &kind_key(ResourceKind::StatefulSets)),
        Some(ResourceAction::Scale(ObjectKind::StatefulSet))
    );
    assert_eq!(
        subject_action(
            RowAction::RestartRollout,
            &kind_key(ResourceKind::DaemonSets)
        ),
        Some(ResourceAction::RestartRollout(ObjectKind::DaemonSet))
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
        (ResourceAction::EditTaints, node_key()),
        (ResourceAction::EditLabels, node_key()),
        (ResourceAction::CopyName, pod_key()),
        (ResourceAction::ViewYaml, pod_key()),
        (ResourceAction::EditYaml(ObjectKind::Pod), pod_key()),
        (ResourceAction::Delete(ObjectKind::Pod), pod_key()),
        (
            ResourceAction::RestartRollout(ObjectKind::Deployment),
            kind_key(ResourceKind::Deployments),
        ),
        (
            ResourceAction::Scale(ObjectKind::Deployment),
            kind_key(ResourceKind::Deployments),
        ),
        (
            ResourceAction::PauseRollout,
            kind_key(ResourceKind::Deployments),
        ),
        (
            ResourceAction::RollBack,
            kind_key(ResourceKind::Deployments),
        ),
        (
            ResourceAction::SuspendCronJob,
            kind_key(ResourceKind::CronJobs),
        ),
        (
            ResourceAction::TriggerCronJob,
            kind_key(ResourceKind::CronJobs),
        ),
        (ResourceAction::RerunJob, kind_key(ResourceKind::Jobs)),
        (
            ResourceAction::EditHpaRange,
            kind_key(ResourceKind::HorizontalPodAutoscalers),
        ),
        (
            ResourceAction::ExpandClaim,
            kind_key(ResourceKind::PersistentVolumeClaims),
        ),
        (
            ResourceAction::SetDefaultStorageClass,
            kind_key(ResourceKind::StorageClasses),
        ),
    ];
    for (action, subject) in subjects {
        assert_eq!(
            subject_action(action.row_action().expect("a row action"), &subject),
            Some(action),
            "{action:?}"
        );
    }
}

#[test]
fn an_unavailable_key_says_what_and_why() {
    assert_eq!(
        unavailable_text(
            action_label(ResourceAction::EditYaml(ObjectKind::Pod)),
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
            .expect("a row action")
            .key_action()
            .partial_eq(&OpenShell)
    );
}

// ---- Open shell: the multi-check gate and the container model (spec 0036) ----

#[test]
fn open_shell_needs_get_and_create() {
    let allowed = known_denying(&[]);
    let no_create = known_denying(&[AccessCheck::CreatePodExec]);
    let no_get = known_denying(&[AccessCheck::GetPodExec]);
    let (checking, unknown) = (checking(), unknown());
    let at = |access: &AccessState, lock| {
        let guard = test_guard(access, lock, "dev-1", Environment::DEVELOPMENT);
        action_availability(ResourceAction::OpenShell, &guard)
    };
    assert_eq!(
        at(&allowed, WriteLock::Unlocked),
        ActionAvailability::Enabled
    );
    // Either verb denied reads the same: the user needs both.
    for denied in [&no_create, &no_get] {
        assert_eq!(
            reason(at(denied, WriteLock::Unlocked)),
            "Not permitted: get and create pods/exec"
        );
    }
    // Fail closed while the answer is not known.
    assert_eq!(
        reason(at(&checking, WriteLock::Unlocked)),
        "Checking permissions…"
    );
    assert_eq!(
        reason(at(&unknown, WriteLock::Unlocked)),
        "Permissions could not be checked"
    );
    // The order: checking, then not permitted, then read-only.
    assert_eq!(
        reason(at(&no_get, WriteLock::Locked)),
        "Not permitted: get and create pods/exec"
    );
    assert_eq!(
        reason(at(&allowed, WriteLock::Locked)),
        "dev-1 is read-only"
    );
}

// ---- Workload actions (0032) ----

#[test]
fn gate_reads_the_carried_kind() {
    let denied_sets = known_denying(&[AccessCheck::PatchStatefulSets]);
    assert_eq!(
        reason(action_availability(
            ResourceAction::RestartRollout(ObjectKind::StatefulSet),
            &unlocked(&denied_sets)
        )),
        "Not permitted: patch statefulsets"
    );
    // The same denial does not touch the other kinds.
    for kind in [ObjectKind::Deployment, ObjectKind::DaemonSet] {
        assert_eq!(
            action_availability(
                ResourceAction::RestartRollout(kind),
                &unlocked(&denied_sets)
            ),
            ActionAvailability::Enabled,
            "{kind:?}"
        );
    }
    // A kind that does not restart has no check, so it is never enabled.
    assert_eq!(
        reason(action_availability(
            ResourceAction::RestartRollout(ObjectKind::Service),
            &unlocked(&known_denying(&[]))
        )),
        "Comes in a later version"
    );
}

#[test]
fn multi_check_gate_names_the_first_denied_check() {
    let gate = ActionGate::Mutating {
        checks: vec![AccessCheck::PatchNodes, AccessCheck::PatchDeployments],
        is_shipped: true,
    };
    let both_denied = known_denying(&[AccessCheck::PatchNodes, AccessCheck::PatchDeployments]);
    let second_denied = known_denying(&[AccessCheck::PatchDeployments]);
    assert_eq!(
        reason(availability_of_gate(
            &gate,
            &both_denied,
            WriteLock::Unlocked
        )),
        "Not permitted: patch nodes"
    );
    assert_eq!(
        reason(availability_of_gate(
            &gate,
            &second_denied,
            WriteLock::Unlocked
        )),
        "Not permitted: patch deployments"
    );
    assert_eq!(
        availability_of_gate(&gate, &known_denying(&[]), WriteLock::Unlocked),
        ActionAvailability::Enabled
    );
    // A one-item list reads as the single check did.
    assert_eq!(
        reason(availability_of_gate(
            &SHIPPED_PATCH_NODES,
            &known_denying(&[AccessCheck::PatchNodes]),
            WriteLock::Unlocked
        )),
        "Not permitted: patch nodes"
    );
}

#[test]
fn a_verb_pair_is_named_only_when_both_verbs_are_asked() {
    let alone = ActionGate::Mutating {
        checks: vec![AccessCheck::CreatePodExec],
        is_shipped: true,
    };
    assert_eq!(
        reason(availability_of_gate(
            &alone,
            &known_denying(&[AccessCheck::CreatePodExec]),
            WriteLock::Unlocked
        )),
        "Not permitted: create pods/exec"
    );
    let port_forward = ActionGate::Mutating {
        checks: vec![
            AccessCheck::GetPodPortForward,
            AccessCheck::CreatePodPortForward,
        ],
        is_shipped: true,
    };
    assert_eq!(
        reason(availability_of_gate(
            &port_forward,
            &known_denying(&[AccessCheck::GetPodPortForward]),
            WriteLock::Unlocked
        )),
        "Not permitted: get and create pods/portforward"
    );
}

fn container(name: &str, kind: ContainerKind, is_running: bool) -> ContainerSummary {
    let mut container = container_of(name);
    container.kind = kind;
    if !is_running {
        container.state = ContainerState::Waiting {
            reason: None,
            message: None,
        };
    }
    container
}

fn shell_state(containers: Vec<ContainerSummary>) -> ShellMenuState {
    let access = known_denying(&[]);
    shell_menu_state(&pod_with(containers), &unlocked(&access))
}

#[test]
fn a_pod_with_one_running_container_opens_it_directly() {
    let state = shell_state(vec![container("app", ContainerKind::Main, true)]);
    assert_eq!(state, ShellMenuState::One("app".to_owned()));
}

#[test]
fn container_picker_lists_running_main_and_sidecars() {
    let state = shell_state(vec![
        container("init-db", ContainerKind::Init, false),
        container("app", ContainerKind::Main, true),
        container("proxy", ContainerKind::Sidecar, true),
        container("batch", ContainerKind::Sidecar, false),
    ]);
    let ShellMenuState::Pick(choices) = state else {
        panic!("several containers must pick");
    };
    // Init containers are left out; one that is not running is listed but disabled.
    let names: Vec<(&str, &str, bool)> = choices
        .iter()
        .map(|choice| (choice.name.as_str(), choice.tag, choice.is_running))
        .collect();
    assert_eq!(
        names,
        [
            ("app", "MAIN", true),
            ("proxy", "SIDECAR", true),
            ("batch", "SIDECAR", false)
        ]
    );
}

#[test]
fn a_container_that_is_not_running_cannot_take_a_shell() {
    assert_eq!(
        shell_state(vec![container("app", ContainerKind::Main, false)]),
        ShellMenuState::Disabled("Container is not running".into())
    );
    assert_eq!(
        shell_state(vec![
            container("app", ContainerKind::Main, false),
            container("proxy", ContainerKind::Sidecar, false)
        ]),
        ShellMenuState::Disabled("No running container".into())
    );
    assert_eq!(
        shell_state(vec![container("init", ContainerKind::Init, true)]),
        ShellMenuState::Disabled("The pod has no containers".into())
    );
}

#[test]
fn the_shell_menu_reads_the_gate_of_the_pods_own_cluster() {
    let denied = known_denying(&[AccessCheck::GetPodExec]);
    let state = shell_menu_state(
        &pod_with(vec![container("app", ContainerKind::Main, true)]),
        &unlocked(&denied),
    );
    assert_eq!(
        state,
        ShellMenuState::Disabled("Not permitted: get and create pods/exec".into())
    );
    let allowed = known_denying(&[]);
    let locked = test_guard(
        &allowed,
        WriteLock::Locked,
        "prod-1",
        Environment::PRODUCTION,
    );
    assert_eq!(
        shell_menu_state(
            &pod_with(vec![container("app", ContainerKind::Main, true)]),
            &locked
        ),
        ShellMenuState::Disabled("prod-1 is read-only".into())
    );
}

#[test]
fn scale_gate_reads_the_carried_kind() {
    let denied = known_denying(&[AccessCheck::PatchStatefulSetScale]);
    assert_eq!(
        reason(action_availability(
            ResourceAction::Scale(ObjectKind::StatefulSet),
            &unlocked(&denied)
        )),
        "Not permitted: patch statefulsets/scale"
    );
    assert_eq!(
        action_availability(
            ResourceAction::Scale(ObjectKind::Deployment),
            &unlocked(&denied)
        ),
        ActionAvailability::Enabled
    );
    let denied = known_denying(&[AccessCheck::PatchDeploymentScale]);
    assert_eq!(
        reason(action_availability(
            ResourceAction::Scale(ObjectKind::Deployment),
            &unlocked(&denied)
        )),
        "Not permitted: patch deployments/scale"
    );
    // A kind that does not scale is never enabled.
    assert_eq!(
        reason(action_availability(
            ResourceAction::Scale(ObjectKind::DaemonSet),
            &unlocked(&known_denying(&[]))
        )),
        "Comes in a later version"
    );
}

#[test]
fn every_shipped_workload_action_has_its_own_check() {
    let cases = [
        (
            ResourceAction::RestartRollout(ObjectKind::Deployment),
            AccessCheck::PatchDeployments,
        ),
        (
            ResourceAction::RestartRollout(ObjectKind::DaemonSet),
            AccessCheck::PatchDaemonSets,
        ),
        (ResourceAction::PauseRollout, AccessCheck::PatchDeployments),
        (ResourceAction::RollBack, AccessCheck::PatchDeployments),
        (ResourceAction::SuspendCronJob, AccessCheck::PatchCronJobs),
        (ResourceAction::TriggerCronJob, AccessCheck::CreateJobs),
        (ResourceAction::RerunJob, AccessCheck::CreateJobs),
        (
            ResourceAction::EditHpaRange,
            AccessCheck::PatchHorizontalPodAutoscalers,
        ),
        (
            ResourceAction::ExpandClaim,
            AccessCheck::PatchPersistentVolumeClaims,
        ),
        (
            ResourceAction::SetDefaultStorageClass,
            AccessCheck::PatchStorageClasses,
        ),
    ];
    for (action, check) in cases {
        let denied = known_denying(&[check]);
        assert_eq!(
            reason(action_availability(action, &unlocked(&denied))),
            format!("Not permitted: {check}"),
            "{action:?}"
        );
        assert_eq!(
            action_availability(action, &unlocked(&known_denying(&[]))),
            ActionAvailability::Enabled,
            "{action:?}"
        );
    }
}

fn paused_deployment() -> KindObject {
    let mut deployment = crate::workload_actions::workload_actions_tests::deployment("api");
    deployment.is_paused = true;
    KindObject::Deployment(deployment)
}

#[test]
fn gate_order_then_row_block() {
    let allowed = known_denying(&[]);
    let restart = ResourceAction::RestartRollout(ObjectKind::Deployment);
    let paused = paused_deployment();
    let guard_at = |access, lock| test_guard(access, lock, "dev-1", Environment::DEVELOPMENT);
    // Locked wins over paused: the lock is what the user can change first.
    assert_eq!(
        reason(row_availability(
            restart,
            &guard_at(&allowed, WriteLock::Locked),
            &paused,
            None
        )),
        "dev-1 is read-only"
    );
    assert_eq!(
        reason(row_availability(
            restart,
            &guard_at(&allowed, WriteLock::Unlocked),
            &paused,
            None
        )),
        "Resume the rollout first"
    );
    let denied = known_denying(&[AccessCheck::PatchDeployments]);
    assert_eq!(
        reason(row_availability(
            restart,
            &guard_at(&denied, WriteLock::Unlocked),
            &paused,
            None
        )),
        "Not permitted: patch deployments"
    );
    let running = KindObject::Deployment(
        crate::workload_actions::workload_actions_tests::deployment("api"),
    );
    assert_eq!(
        row_availability(
            restart,
            &guard_at(&allowed, WriteLock::Unlocked),
            &running,
            None
        ),
        ActionAvailability::Enabled
    );
}

#[test]
fn actions_not_offered_for_other_kinds() {
    let access = known_denying(&[]);
    let offers = |kind: ResourceKind, action: RowAction| {
        availability(action, &kind_key(kind), &access) != KeyAvailability::NotOffered
    };
    assert!(offers(ResourceKind::Deployments, RowAction::PauseRollout));
    assert!(offers(ResourceKind::Deployments, RowAction::RollBack));
    assert!(!offers(ResourceKind::StatefulSets, RowAction::PauseRollout));
    assert!(!offers(ResourceKind::StatefulSets, RowAction::RollBack));
    assert!(offers(ResourceKind::CronJobs, RowAction::TriggerCronJob));
    assert!(offers(ResourceKind::CronJobs, RowAction::SuspendCronJob));
    assert!(!offers(ResourceKind::Jobs, RowAction::TriggerCronJob));
    assert!(offers(ResourceKind::Jobs, RowAction::RerunJob));
    assert!(!offers(ResourceKind::CronJobs, RowAction::RerunJob));
    assert!(offers(
        ResourceKind::HorizontalPodAutoscalers,
        RowAction::EditHpaRange
    ));
    assert!(!offers(ResourceKind::Deployments, RowAction::EditHpaRange));
    assert!(offers(
        ResourceKind::PersistentVolumeClaims,
        RowAction::ExpandClaim
    ));
    assert!(!offers(
        ResourceKind::HorizontalPodAutoscalers,
        RowAction::ExpandClaim
    ));
    // Pods and nodes carry no workload action at all.
    for subject in [pod_key(), node_key()] {
        for row in [RowAction::RestartRollout, RowAction::RerunJob] {
            assert_eq!(
                availability(row, &subject, &access),
                KeyAvailability::NotOffered
            );
        }
    }
}

#[test]
fn restart_runs_on_a_deployment_row_once_unlocked() {
    let access = known_denying(&[]);
    assert_eq!(
        availability(
            RowAction::RestartRollout,
            &kind_key(ResourceKind::Deployments),
            &access
        ),
        KeyAvailability::Run(ResourceAction::RestartRollout(ObjectKind::Deployment))
    );
}

#[test]
fn s_opens_the_first_running_main_container_else_the_first_running_one() {
    let pod = pod_with(vec![
        container("proxy", ContainerKind::Sidecar, true),
        container("init", ContainerKind::Init, true),
        container("app", ContainerKind::Main, true),
        container("worker", ContainerKind::Main, true),
    ]);
    assert_eq!(
        default_shell_container(&pod).map(|c| c.name.as_str()),
        Some("app")
    );
    let sidecars_only = pod_with(vec![
        container("down", ContainerKind::Main, false),
        container("proxy", ContainerKind::Sidecar, true),
    ]);
    assert_eq!(
        default_shell_container(&sidecars_only).map(|c| c.name.as_str()),
        Some("proxy")
    );
    let none_running = pod_with(vec![container("app", ContainerKind::Main, false)]);
    assert!(default_shell_container(&none_running).is_none());
    let init_only = pod_with(vec![container("init", ContainerKind::Init, true)]);
    assert!(default_shell_container(&init_only).is_none());
}

#[test]
fn every_new_row_action_has_a_unit_key_action() {
    use crate::keymap::{PauseRollout, RerunJob, RollBack, SuspendCronJob, TriggerCronJob};
    assert!(
        RowAction::PauseRollout
            .key_action()
            .partial_eq(&PauseRollout)
    );
    assert!(RowAction::RollBack.key_action().partial_eq(&RollBack));
    assert!(
        RowAction::SuspendCronJob
            .key_action()
            .partial_eq(&SuspendCronJob)
    );
    assert!(
        RowAction::TriggerCronJob
            .key_action()
            .partial_eq(&TriggerCronJob)
    );
    assert!(RowAction::RerunJob.key_action().partial_eq(&RerunJob));
    assert!(
        RowAction::EditHpaRange
            .key_action()
            .partial_eq(&crate::keymap::EditHpaRange)
    );
    assert!(
        RowAction::ExpandClaim
            .key_action()
            .partial_eq(&crate::keymap::ExpandClaim)
    );
    assert!(
        RowAction::SetDefaultStorageClass
            .key_action()
            .partial_eq(&crate::keymap::SetDefaultStorageClass)
    );
}

// ---- Edit YAML (spec 0031) ----

use crate::kind_access::{KindAccess, KindAccessMap};

fn update_report(kind: ObjectKind, is_allowed: bool) -> KindAccess {
    let decision = if is_allowed {
        AccessDecision::Allowed
    } else {
        AccessDecision::Denied { reason: None }
    };
    KindAccess::Known(AccessReport {
        reviews: vec![AccessReview {
            check: AccessCheck::Update(kind),
            decision,
        }],
    })
}

/// The gate of Edit YAML on a Deployment of an unlocked or locked development cluster that holds
/// `kind_access`.
fn edit_gate(kind_access: &KindAccessMap, lock: WriteLock) -> ActionAvailability {
    let access = known_denying(&[]);
    let mut guard = test_guard(&access, lock, "dev-1", Environment::DEVELOPMENT);
    guard.kind_access = kind_access;
    action_availability(ResourceAction::EditYaml(ObjectKind::Deployment), &guard)
}

#[test]
fn lazy_gate_reads_kind_access() {
    let mut map = KindAccessMap::new();
    // Nothing asked yet: the check is still to come.
    assert_eq!(
        reason(edit_gate(&map, WriteLock::Unlocked)),
        "Checking permissions…"
    );
    map.set(
        ObjectKind::Deployment,
        KindAccess::Checking {
            _task: Task::ready(()),
        },
    );
    assert_eq!(
        reason(edit_gate(&map, WriteLock::Unlocked)),
        "Checking permissions…"
    );
    map.set(ObjectKind::Deployment, KindAccess::Unknown);
    assert_eq!(
        reason(edit_gate(&map, WriteLock::Unlocked)),
        "Permissions could not be checked"
    );
    map.set(
        ObjectKind::Deployment,
        update_report(ObjectKind::Deployment, false),
    );
    assert_eq!(
        reason(edit_gate(&map, WriteLock::Unlocked)),
        "Not permitted: update deployments"
    );
    map.set(
        ObjectKind::Deployment,
        update_report(ObjectKind::Deployment, true),
    );
    assert_eq!(
        edit_gate(&map, WriteLock::Unlocked),
        ActionAvailability::Enabled
    );
}

#[test]
fn edit_yaml_respects_the_lock_after_the_permission() {
    let mut map = KindAccessMap::new();
    map.set(
        ObjectKind::Deployment,
        update_report(ObjectKind::Deployment, true),
    );
    assert_eq!(
        reason(edit_gate(&map, WriteLock::Locked)),
        "dev-1 is read-only"
    );
}

#[test]
fn the_answer_of_another_kind_does_not_open_the_gate() {
    let mut map = KindAccessMap::new();
    map.set(
        ObjectKind::ConfigMap,
        update_report(ObjectKind::ConfigMap, true),
    );
    assert_eq!(
        reason(edit_gate(&map, WriteLock::Unlocked)),
        "Checking permissions…"
    );
}

#[test]
fn edit_yaml_is_offered_only_on_editable_kinds() {
    let offered = |subject: &ResourceKey| subject_action(RowAction::EditYaml, subject);
    assert_eq!(
        offered(&pod_key()),
        Some(ResourceAction::EditYaml(ObjectKind::Pod))
    );
    assert_eq!(
        offered(&kind_key(ResourceKind::Deployments)),
        Some(ResourceAction::EditYaml(ObjectKind::Deployment))
    );
    assert_eq!(
        offered(&kind_key(ResourceKind::ConfigMaps)),
        Some(ResourceAction::EditYaml(ObjectKind::ConfigMap))
    );
    assert_eq!(
        offered(&kind_key(ResourceKind::ClusterRoles)),
        Some(ResourceAction::EditYaml(ObjectKind::ClusterRole))
    );
    // A node, a read-only kind, and a Helm release (a Secret by storage) are not edited here.
    assert_eq!(offered(&node_key()), None);
    assert_eq!(offered(&kind_key(ResourceKind::Namespaces)), None);
    assert_eq!(offered(&kind_key(ResourceKind::Events)), None);
    assert_eq!(offered(&kind_key(ResourceKind::HelmReleases)), None);
    assert_eq!(offered(&kind_key(ResourceKind::Crds)), None);
}

#[test]
fn every_editable_kind_menu_gets_edit_yaml() {
    for kind in ResourceKind::ALL {
        let editable = kind.builtin_object().is_some_and(ObjectKind::is_editable)
            && kind != ResourceKind::HelmReleases;
        assert_eq!(edit_yaml_kind(kind).is_some(), editable, "{kind:?}");
    }
    assert_eq!(
        edit_yaml_kind(ResourceKind::Secrets),
        Some(ObjectKind::Secret)
    );
    assert_eq!(edit_yaml_kind(ResourceKind::HelmReleases), None);
}

#[test]
fn edit_yaml_key_runs_the_resolved_action_when_the_gate_is_open() {
    let access = known_denying(&[]);
    let mut map = KindAccessMap::new();
    map.set(ObjectKind::Pod, update_report(ObjectKind::Pod, true));
    let mut guard = unlocked(&access);
    guard.kind_access = &map;
    let pod = pod_with(vec![container_of("app")]);
    assert_eq!(
        key_availability_of(RowAction::EditYaml, &pod_key(), Some(&pod), &guard),
        KeyAvailability::Run(ResourceAction::EditYaml(ObjectKind::Pod))
    );
    let empty = KindAccessMap::new();
    guard.kind_access = &empty;
    assert_eq!(
        disabled_reason(key_availability_of(
            RowAction::EditYaml,
            &pod_key(),
            Some(&pod),
            &guard
        )),
        "Checking permissions…"
    );
}

#[test]
fn screens_name_the_kind_their_rows_act_on() {
    use crate::app_shell::Screen;
    assert_eq!(Screen::Pods.access_kind(), Some(ObjectKind::Pod));
    assert_eq!(
        Screen::Kind(ResourceKind::Services).access_kind(),
        Some(ObjectKind::Service)
    );
    assert_eq!(Screen::Nodes.access_kind(), Some(ObjectKind::Node));
    assert_eq!(Screen::Overview.access_kind(), None);
    assert_eq!(Screen::Kind(ResourceKind::HelmReleases).access_kind(), None);
}

fn delete_report(kind: ObjectKind, is_allowed: bool) -> KindAccess {
    let decision = if is_allowed {
        AccessDecision::Allowed
    } else {
        AccessDecision::Denied { reason: None }
    };
    KindAccess::Known(AccessReport {
        reviews: vec![AccessReview {
            check: AccessCheck::Delete(kind),
            decision,
        }],
    })
}

fn delete_gate(kind_access: &KindAccessMap, lock: WriteLock) -> ActionAvailability {
    let access = known_denying(&[]);
    let mut guard = test_guard(&access, lock, "dev-1", Environment::DEVELOPMENT);
    guard.kind_access = kind_access;
    action_availability(ResourceAction::Delete(ObjectKind::Pod), &guard)
}

#[test]
fn delete_gate_order() {
    let mut map = KindAccessMap::new();
    // The shipped action waits for the lazy answer of its kind.
    assert_eq!(
        reason(delete_gate(&map, WriteLock::Unlocked)),
        "Checking permissions…"
    );
    map.set(ObjectKind::Pod, delete_report(ObjectKind::Pod, false));
    assert_eq!(
        reason(delete_gate(&map, WriteLock::Unlocked)),
        "Not permitted: delete pods"
    );
    // The permission is named before the lock.
    assert_eq!(
        reason(delete_gate(&map, WriteLock::Locked)),
        "Not permitted: delete pods"
    );
    map.set(ObjectKind::Pod, delete_report(ObjectKind::Pod, true));
    assert_eq!(
        reason(delete_gate(&map, WriteLock::Locked)),
        "dev-1 is read-only"
    );
    assert_eq!(
        delete_gate(&map, WriteLock::Unlocked),
        ActionAvailability::Enabled
    );
}

#[test]
fn delete_is_destructive_and_mutating() {
    assert_eq!(
        action_risk(ResourceAction::Delete(ObjectKind::Pod)),
        ActionRisk::Destructive
    );
    assert_eq!(
        action_label(ResourceAction::Delete(ObjectKind::Pod)),
        "Delete"
    );
}

#[test]
fn delete_is_offered_on_every_builtin_kind() {
    for kind in ResourceKind::ALL {
        let expected = match kind {
            ResourceKind::HelmReleases => None,
            kind => kind.builtin_object().map(ResourceAction::Delete),
        };
        assert_eq!(
            subject_action(RowAction::Delete, &kind_key(kind)),
            expected,
            "{kind:?}"
        );
    }
    assert_eq!(
        subject_action(RowAction::Delete, &pod_key()),
        Some(ResourceAction::Delete(ObjectKind::Pod))
    );
    assert_eq!(
        subject_action(RowAction::Delete, &node_key()),
        Some(ResourceAction::Delete(ObjectKind::Node))
    );
}

#[test]
fn delete_not_offered_on_helm_releases_or_custom_kinds() {
    let release = kind_key(ResourceKind::HelmReleases);
    assert_eq!(subject_action(RowAction::Delete, &release), None);
    let access = known_denying(&[]);
    assert_eq!(
        availability(RowAction::Delete, &release, &access),
        KeyAvailability::NotOffered
    );
    assert_eq!(delete_kind_of(ResourceKind::HelmReleases), None);
}

#[test]
fn only_a_helm_release_secret_is_refused_as_a_record() {
    use crate::kind_row::KindObject;
    let secret =
        |secret_type: &str| KindObject::Secret(crate::topology_fixtures::secret("db", secret_type));
    assert!(helm_record_reason(&secret(cluster::HELM_RELEASE_SECRET_TYPE)).is_some());
    assert!(helm_record_reason(&secret("Opaque")).is_none());
    assert!(helm_record_reason(&KindObject::Plain).is_none());
}

#[test]
fn delete_menu_hint_is_the_delete_key() {
    use crate::keymap::Delete;
    assert!(RowAction::Delete.key_action().partial_eq(&Delete));
    assert_eq!(
        ResourceAction::Delete(ObjectKind::Secret).row_action(),
        Some(RowAction::Delete)
    );
}

// ---- 0037: Debug container ----

fn debug_gate(access: &AccessState) -> ActionAvailability {
    action_availability(ResourceAction::DebugContainer, &unlocked(access))
}

#[test]
fn debug_container_gate_table() {
    assert_eq!(debug_gate(&known_denying(&[])), ActionAvailability::Enabled);
    // Each missing check names itself; the attach pair names both verbs.
    let cases = [
        (
            AccessCheck::PatchPodEphemeralContainers,
            "Not permitted: patch pods/ephemeralcontainers",
        ),
        (AccessCheck::WatchPods, "Not permitted: watch pods"),
        (
            AccessCheck::GetPodAttach,
            "Not permitted: get and create pods/attach",
        ),
        (
            AccessCheck::CreatePodAttach,
            "Not permitted: get and create pods/attach",
        ),
    ];
    for (denied, text) in cases {
        assert_eq!(reason(debug_gate(&known_denying(&[denied]))), text);
    }
    assert_eq!(reason(debug_gate(&checking())), "Checking permissions…");
    assert_eq!(
        reason(debug_gate(&unknown())),
        "Permissions could not be checked"
    );
}

#[test]
fn debug_container_is_off_while_the_cluster_is_locked() {
    let access = known_denying(&[]);
    let locked = test_guard(
        &access,
        WriteLock::Locked,
        "prod-1",
        Environment::PRODUCTION,
    );
    assert_eq!(
        reason(action_availability(ResourceAction::DebugContainer, &locked)),
        "prod-1 is read-only"
    );
}

#[test]
fn debug_container_is_a_change_that_the_tier_confirms() {
    assert_eq!(
        action_risk(ResourceAction::DebugContainer),
        ActionRisk::Change
    );
    assert_eq!(
        confirm_step(
            ConfirmMode::Click,
            action_risk(ResourceAction::DebugContainer),
            "stg-1"
        ),
        DialogConfirm::Click
    );
}

#[test]
fn a_pod_without_a_running_container_cannot_take_a_debug_container() {
    let running = pod_with(vec![container_of("app")]);
    assert_eq!(debug_container_block(&running), None);
    let mut waiting = container_of("app");
    waiting.state = cluster::ContainerState::Waiting {
        reason: None,
        message: None,
    };
    let mut init = container_of("init");
    init.kind = cluster::ContainerKind::Init;
    let idle = pod_with(vec![waiting, init]);
    assert_eq!(
        debug_container_block(&idle).as_deref(),
        Some("The pod has no running container")
    );
    assert!(debug_container_block(&pod_with(Vec::new())).is_some());
}

#[test]
fn debug_targets_skip_init_and_stopped_containers() {
    let mut init = container_of("init");
    init.kind = cluster::ContainerKind::Init;
    let mut sidecar = container_of("proxy");
    sidecar.kind = cluster::ContainerKind::Sidecar;
    let mut stopped = container_of("old");
    stopped.state = cluster::ContainerState::Waiting {
        reason: None,
        message: None,
    };
    let pod = pod_with(vec![init, container_of("web"), sidecar, stopped]);
    let names: Vec<_> = debug_targets(&pod)
        .map(|container| container.name.as_str())
        .collect();
    assert_eq!(names, ["web", "proxy"]);
}

#[test]
fn the_debug_menu_state_is_the_gate_then_the_pod() {
    let access = known_denying(&[]);
    let guard = unlocked(&access);
    let running = pod_with(vec![container_of("app")]);
    assert_eq!(debug_menu_state(&running, &guard), DebugMenuState::Ready);
    assert_eq!(
        debug_menu_state(&pod_with(Vec::new()), &guard),
        DebugMenuState::Disabled("The pod has no running container".into())
    );
    // The gate says no first, whatever the pod is.
    let denied = known_denying(&[AccessCheck::WatchPods]);
    assert_eq!(
        debug_menu_state(&running, &unlocked(&denied)),
        DebugMenuState::Disabled("Not permitted: watch pods".into())
    );
}

#[test]
fn the_debug_container_key_resolves_for_pods_only() {
    assert_eq!(
        subject_action(RowAction::DebugContainer, &pod_key()),
        Some(ResourceAction::DebugContainer)
    );
    assert_eq!(subject_action(RowAction::DebugContainer, &node_key()), None);
    assert_eq!(
        ResourceAction::DebugContainer.row_action(),
        Some(RowAction::DebugContainer)
    );
    assert_eq!(
        action_label(ResourceAction::DebugContainer),
        "Debug container"
    );
}

#[test]
fn the_attach_pair_reads_as_one_right() {
    let checks = [AccessCheck::GetPodAttach, AccessCheck::CreatePodAttach];
    assert_eq!(
        denied_text(AccessCheck::GetPodAttach, &checks),
        "Not permitted: get and create pods/attach"
    );
    // A lone attach check (not paired in the action) names itself.
    assert_eq!(
        denied_text(AccessCheck::GetPodAttach, &[AccessCheck::GetPodAttach]),
        "Not permitted: get pods/attach"
    );
}

// ---- 0037: Open node shell ----

fn node_shell_gate(access: &AccessState) -> ActionAvailability {
    action_availability(ResourceAction::OpenNodeShell, &unlocked(access))
}

#[test]
fn node_shell_gate_table() {
    assert_eq!(
        node_shell_gate(&known_denying(&[])),
        ActionAvailability::Enabled
    );
    // The first missing check names itself; the attach pair names both verbs.
    let cases = [
        (AccessCheck::CreatePods, "Not permitted: create pods"),
        (AccessCheck::DeletePods, "Not permitted: delete pods"),
        (AccessCheck::WatchPods, "Not permitted: watch pods"),
        (
            AccessCheck::GetPodAttach,
            "Not permitted: get and create pods/attach",
        ),
        (
            AccessCheck::CreatePodAttach,
            "Not permitted: get and create pods/attach",
        ),
    ];
    for (denied, text) in cases {
        assert_eq!(reason(node_shell_gate(&known_denying(&[denied]))), text);
    }
    // With several missing, the first of the action's own list is the reason.
    let many = known_denying(&[AccessCheck::WatchPods, AccessCheck::CreatePods]);
    assert_eq!(reason(node_shell_gate(&many)), "Not permitted: create pods");
    assert_eq!(
        reason(node_shell_gate(&checking())),
        "Checking permissions…"
    );
}

#[test]
fn node_shell_off_for_the_cluster_says_so_after_the_permissions() {
    let allowed = known_denying(&[]);
    let mut guard = unlocked(&allowed);
    guard.profile.allow_node_shell = false;
    assert_eq!(
        reason(action_availability(ResourceAction::OpenNodeShell, &guard)),
        "Node shell is off for dev-1 (Settings › Clusters › Safety)"
    );
    // The permission reason still comes first: the setting is not blamed for a missing right.
    let denied = known_denying(&[AccessCheck::CreatePods]);
    let mut guard = unlocked(&denied);
    guard.profile.allow_node_shell = false;
    assert_eq!(
        reason(action_availability(ResourceAction::OpenNodeShell, &guard)),
        "Not permitted: create pods"
    );
}

#[test]
fn the_setting_is_read_before_the_lock() {
    let allowed = known_denying(&[]);
    let mut guard = test_guard(
        &allowed,
        WriteLock::Locked,
        "prod-1",
        Environment::PRODUCTION,
    );
    guard.profile.allow_node_shell = false;
    assert_eq!(
        reason(action_availability(ResourceAction::OpenNodeShell, &guard)),
        "Node shell is off for prod-1 (Settings › Clusters › Safety)"
    );
    guard.profile.allow_node_shell = true;
    assert_eq!(
        reason(action_availability(ResourceAction::OpenNodeShell, &guard)),
        "prod-1 is read-only"
    );
}

#[test]
fn node_shell_is_a_privileged_action() {
    assert_eq!(
        action_risk(ResourceAction::OpenNodeShell),
        ActionRisk::Privileged
    );
    // Even a click-tier development cluster types the node name.
    assert_eq!(
        confirm_step(
            ConfirmMode::Click,
            action_risk(ResourceAction::OpenNodeShell),
            "wk-03"
        ),
        DialogConfirm::TypeName {
            expected: "wk-03".to_owned()
        }
    );
}

fn node_with_os(operating_system: &str) -> NodeSummary {
    NodeSummary {
        name: "wk-03".to_owned(),
        status: cluster::NodeStatus {
            readiness: cluster::NodeReadiness::Ready,
            scheduling: cluster::NodeScheduling::Enabled,
        },
        roles: Vec::new(),
        taints: Vec::new(),
        kubelet_version: "v1.29.5".to_owned(),
        internal_ip: None,
        created_at: None,
        conditions: Vec::new(),
        addresses: Vec::new(),
        system: cluster::NodeSystemInfo {
            operating_system: operating_system.to_owned(),
            ..cluster::NodeSystemInfo::default()
        },
        resources: Vec::new(),
        labels: Vec::new(),
    }
}

#[test]
fn a_windows_node_takes_no_node_shell() {
    assert_eq!(node_shell_block(&node_with_os("linux")), None);
    assert_eq!(
        node_shell_block(&node_with_os("windows")).as_deref(),
        Some("Node shell needs a Linux node")
    );
    assert_eq!(
        node_shell_block(&node_with_os("")).as_deref(),
        Some("Node shell needs a Linux node")
    );
}

#[test]
fn the_uat_answers_disable_both_debug_actions_with_their_reasons() {
    // What the read-only UAT user is told (spec 0037 AC 12): every new check but `get pods/attach`
    // is denied.
    let uat = known_denying(&[
        AccessCheck::CreatePods,
        AccessCheck::DeletePods,
        AccessCheck::PatchPodEphemeralContainers,
        AccessCheck::CreatePodAttach,
    ]);
    assert_eq!(
        reason(action_availability(
            ResourceAction::DebugContainer,
            &unlocked(&uat)
        )),
        "Not permitted: patch pods/ephemeralcontainers"
    );
    assert_eq!(
        reason(action_availability(
            ResourceAction::OpenNodeShell,
            &unlocked(&uat)
        )),
        "Not permitted: create pods"
    );
}

// ---- Resource edits (spec 0032b) ----

#[test]
fn expand_row_block_table() {
    use crate::resource_edits::resource_edits_tests::claim;
    let expand = |claim: cluster::PersistentVolumeClaimSummary| {
        crate::workload_actions::row_block(
            ResourceAction::ExpandClaim,
            &KindObject::PersistentVolumeClaim(claim),
            None,
        )
        .map(|reason| reason.to_string())
    };
    assert_eq!(expand(claim("data", "100Gi", "100Gi")), None);
    let mut pending = claim("data", "100Gi", "100Gi");
    pending.phase = "Pending".to_owned();
    assert_eq!(
        expand(pending).as_deref(),
        Some("Only a bound claim can be expanded")
    );
    let mut terminating = claim("data", "100Gi", "100Gi");
    terminating.is_terminating = true;
    assert_eq!(
        expand(terminating).as_deref(),
        Some("The claim is being deleted")
    );
    // Another kind of row is never blocked by the claim rules.
    assert_eq!(
        crate::workload_actions::row_block(ResourceAction::ExpandClaim, &KindObject::Plain, None),
        None
    );
}

#[test]
fn expand_item_is_gated_then_blocked_by_the_row() {
    use crate::resource_edits::resource_edits_tests::claim;
    let allowed = known_denying(&[]);
    let mut pending = claim("data", "100Gi", "100Gi");
    pending.phase = "Pending".to_owned();
    let pending = KindObject::PersistentVolumeClaim(pending);
    let bound = KindObject::PersistentVolumeClaim(claim("data", "100Gi", "100Gi"));
    let guard_at = |access, lock| test_guard(access, lock, "dev-1", Environment::DEVELOPMENT);
    assert_eq!(
        row_availability(
            ResourceAction::ExpandClaim,
            &guard_at(&allowed, WriteLock::Unlocked),
            &bound,
            None
        ),
        ActionAvailability::Enabled
    );
    assert_eq!(
        reason(row_availability(
            ResourceAction::ExpandClaim,
            &guard_at(&allowed, WriteLock::Unlocked),
            &pending,
            None
        )),
        "Only a bound claim can be expanded"
    );
    // The lock wins over the state of the row.
    assert_eq!(
        reason(row_availability(
            ResourceAction::ExpandClaim,
            &guard_at(&allowed, WriteLock::Locked),
            &pending,
            None
        )),
        "dev-1 is read-only"
    );
    let denied = known_denying(&[AccessCheck::PatchPersistentVolumeClaims]);
    assert_eq!(
        reason(row_availability(
            ResourceAction::ExpandClaim,
            &guard_at(&denied, WriteLock::Unlocked),
            &bound,
            None
        )),
        "Not permitted: patch persistentvolumeclaims"
    );
}

#[test]
fn set_default_row_block_table() {
    use crate::resource_edits::resource_edits_tests::class;
    let block = |class: cluster::StorageClassSummary| {
        crate::workload_actions::row_block(
            ResourceAction::SetDefaultStorageClass,
            &KindObject::StorageClass(class),
            None,
        )
        .map(|reason| reason.to_string())
    };
    assert_eq!(block(class("gp3", false, true)), None);
    assert_eq!(
        block(class("io2", true, true)).as_deref(),
        Some("Already the default")
    );
    // A class that does not expand can still be the default.
    assert_eq!(block(class("st1", false, false)), None);
}

#[test]
fn set_default_item_is_gated_then_blocked_by_the_row() {
    use crate::resource_edits::resource_edits_tests::class;
    let allowed = known_denying(&[]);
    let default = KindObject::StorageClass(class("io2", true, true));
    let other = KindObject::StorageClass(class("gp3", false, true));
    let guard_at = |access, lock| test_guard(access, lock, "dev-1", Environment::DEVELOPMENT);
    let item = |guard: &ClusterGuard<'_>, object: &KindObject| {
        reason(row_availability(
            ResourceAction::SetDefaultStorageClass,
            guard,
            object,
            None,
        ))
    };
    assert_eq!(
        row_availability(
            ResourceAction::SetDefaultStorageClass,
            &guard_at(&allowed, WriteLock::Unlocked),
            &other,
            None
        ),
        ActionAvailability::Enabled
    );
    assert_eq!(
        item(&guard_at(&allowed, WriteLock::Unlocked), &default),
        "Already the default"
    );
    assert_eq!(
        item(&guard_at(&allowed, WriteLock::Locked), &default),
        "dev-1 is read-only"
    );
    let denied = known_denying(&[AccessCheck::PatchStorageClasses]);
    assert_eq!(
        item(&guard_at(&denied, WriteLock::Unlocked), &other),
        "Not permitted: patch storageclasses"
    );
}

// ---- Spec 0039 step 1: View logs submenu, container menu, quota Edit ----

#[test]
fn quota_menu_has_edit_yaml_and_no_stale_edit() {
    assert!(ResourceKind::ResourceQuotas.read_only_actions().is_empty());
    assert_eq!(
        edit_yaml_kind(ResourceKind::ResourceQuotas),
        Some(ObjectKind::ResourceQuota)
    );
    // Secrets keep their placeholder: value editing is a later spec.
    assert!(!ResourceKind::Secrets.read_only_actions().is_empty());
}

fn logs_menu_of(containers: Vec<ContainerSummary>, access: &AccessState) -> LogsMenuState {
    LogsMenu::of(&pod_with(containers), access).state
}

#[test]
fn logs_menu_one_container_is_direct() {
    let state = logs_menu_of(
        vec![container("app", ContainerKind::Main, true)],
        &known_denying(&[]),
    );
    assert!(matches!(state, LogsMenuState::One(_)));
}

#[test]
fn logs_menu_lists_every_container_with_tags() {
    let state = logs_menu_of(
        vec![
            container("init-db", ContainerKind::Init, false),
            container("proxy", ContainerKind::Sidecar, true),
            container("app", ContainerKind::Main, true),
        ],
        &known_denying(&[]),
    );
    let LogsMenuState::Pick(choices) = state else {
        panic!("several containers must pick");
    };
    // Spec order, init included, none left out for its state.
    let entries: Vec<(&str, &str)> = choices
        .iter()
        .map(|choice| (choice.name.as_str(), choice.tag))
        .collect();
    assert_eq!(
        entries,
        [("init-db", "INIT"), ("proxy", "SIDECAR"), ("app", "MAIN")]
    );
}

#[test]
fn logs_menu_disabled_when_logs_not_permitted() {
    let denied = known_denying(&[AccessCheck::GetPodLogs]);
    let state = logs_menu_of(
        vec![
            container("app", ContainerKind::Main, true),
            container("proxy", ContainerKind::Sidecar, true),
        ],
        &denied,
    );
    let LogsMenuState::Disabled(reason) = state else {
        panic!("denied logs must disable the item");
    };
    assert_eq!(
        reason,
        format!("Not permitted: {}", AccessCheck::GetPodLogs)
    );
    let none = logs_menu_of(Vec::new(), &known_denying(&[]));
    assert!(matches!(
        none,
        LogsMenuState::Disabled(reason) if reason == "The pod has no containers"
    ));
}

#[test]
fn logs_choice_opens_that_container() {
    let pod = pod_with(vec![
        container("app", ContainerKind::Main, true),
        container("proxy", ContainerKind::Sidecar, true),
    ]);
    let Some(LogTarget::Pod(target)) = LogTarget::of_container(&pod, "proxy") else {
        panic!("the pod has a container named proxy");
    };
    assert_eq!(target.initial_container, "proxy");
    // An explicit pick, so a reopen switches the tab to it (the dock keeps the user's own pick otherwise).
    assert_eq!(target.choice, crate::log_target::ContainerChoice::Explicit);
}

#[test]
fn container_menu_items_in_order() {
    assert_eq!(
        CONTAINER_MENU,
        [
            ContainerMenuEntry::ViewLogs,
            ContainerMenuEntry::OpenShell,
            ContainerMenuEntry::Attach,
            ContainerMenuEntry::CopyImage
        ]
    );
}

#[test]
fn container_shell_disabled_when_not_running() {
    let access = known_denying(&[]);
    let guard = unlocked(&access);
    assert_eq!(
        container_shell_availability(&container("app", ContainerKind::Main, true), &guard),
        ActionAvailability::Enabled
    );
    assert_eq!(
        reason(container_shell_availability(
            &container("app", ContainerKind::Main, false),
            &guard
        )),
        "Container is not running"
    );
}

#[test]
fn container_shell_follows_the_gate() {
    let denied = known_denying(&[AccessCheck::GetPodExec]);
    let running = container("app", ContainerKind::Main, true);
    // The gate speaks before the container's state.
    assert_eq!(
        reason(container_shell_availability(&running, &unlocked(&denied))),
        "Not permitted: get and create pods/exec"
    );
    let allowed = known_denying(&[]);
    let locked = test_guard(
        &allowed,
        WriteLock::Locked,
        "prod-1",
        Environment::PRODUCTION,
    );
    assert_eq!(
        reason(container_shell_availability(&running, &locked)),
        "prod-1 is read-only"
    );
}

// ---- Spec 0039 step 2: L on workload kinds, View logs of last job ----

fn scheduled_cron_job_row(last_schedule: Option<i64>) -> KindRow {
    let mut cron_job =
        crate::workload_actions::workload_actions_tests::cron_job("nightly", "Allow", 0);
    cron_job.last_schedule_at =
        last_schedule.map(|seconds| jiff::Timestamp::from_second(seconds).expect("a timestamp"));
    crate::batch_rows::cron_job_row(&cron_job)
}

/// A pod of `team-a` owned by the Job `job`.
fn job_pod(job: &str) -> PodSummary {
    let mut pod = pod_with(vec![container_of("main")]);
    pod.namespace = "team-a".to_owned();
    pod.controller = Some(cluster::ControllerRef {
        kind: "Job".to_owned(),
        name: job.to_owned(),
    });
    pod
}

#[test]
fn view_logs_is_offered_on_workload_kinds() {
    for kind in [
        ResourceKind::Deployments,
        ResourceKind::StatefulSets,
        ResourceKind::DaemonSets,
        ResourceKind::ReplicaSets,
        ResourceKind::Jobs,
        ResourceKind::CronJobs,
    ] {
        assert_eq!(
            subject_action(RowAction::ViewLogs, &kind_key(kind)),
            Some(ResourceAction::ViewLogs),
            "{kind:?}"
        );
    }
}

#[test]
fn view_logs_not_offered_on_services() {
    for kind in [
        ResourceKind::Services,
        ResourceKind::ConfigMaps,
        ResourceKind::Namespaces,
    ] {
        assert_eq!(subject_action(RowAction::ViewLogs, &kind_key(kind)), None);
    }
}

#[test]
fn cron_job_view_logs_item_has_l_hint() {
    // The item carries the key action of `ResourceAction::ViewLogs`, which is L.
    assert_eq!(
        ResourceAction::ViewLogs
            .row_action()
            .expect("a row action")
            .key_action()
            .name(),
        RowAction::ViewLogs.key_action().name()
    );
    // A CronJob row has a logs entry although it owns no pods.
    let row = scheduled_cron_job_row(Some(29_000_000 * 60));
    assert!(workload_logs_owner(&row, &[job_pod("nightly-29000000")]).is_some());
}

#[test]
fn cron_job_key_disabled_without_pods_says_why() {
    let access = known_denying(&[]);
    let row = scheduled_cron_job_row(Some(29_000_000 * 60));
    assert_eq!(
        disabled_reason(workload_logs_key(Some(&row), &[], &access)),
        "Job nightly-29000000 has no pods left"
    );
    let never = scheduled_cron_job_row(None);
    assert_eq!(
        disabled_reason(workload_logs_key(Some(&never), &[], &access)),
        "No job has run yet"
    );
    assert_eq!(
        workload_logs_key(Some(&row), &[job_pod("nightly-29000000")], &access),
        KeyAvailability::Run(ResourceAction::ViewLogs)
    );
}

#[test]
fn workload_logs_key_asks_the_gate_before_the_row() {
    let denied = known_denying(&[AccessCheck::GetPodLogs]);
    let row = scheduled_cron_job_row(None);
    assert_eq!(
        disabled_reason(workload_logs_key(Some(&row), &[], &denied)),
        format!("Not permitted: {}", AccessCheck::GetPodLogs)
    );
    // A row the list no longer holds is not offered.
    assert_eq!(
        workload_logs_key(None, &[], &known_denying(&[])),
        KeyAvailability::NotOffered
    );
}

#[test]
fn workload_logs_item_has_l_hint() {
    // A deployment row opens its own pods; the item reads the same owner the key does.
    let summary = crate::workload_actions::workload_actions_tests::deployment("api");
    let row = crate::workload_rows::deployment_row(&summary);
    assert!(matches!(
        workload_logs_owner(&row, &[]),
        Some(Ok(PodOwner::Deployment { .. }))
    ));
}

// ---- Edit values (spec 0047) ----

fn patch_report(kind: ObjectKind, is_allowed: bool) -> KindAccess {
    let decision = if is_allowed {
        AccessDecision::Allowed
    } else {
        AccessDecision::Denied { reason: None }
    };
    KindAccess::Known(AccessReport {
        reviews: vec![AccessReview {
            check: AccessCheck::Patch(kind),
            decision,
        }],
    })
}

fn values_gate(kind_access: &KindAccessMap, lock: WriteLock) -> ActionAvailability {
    let access = known_denying(&[]);
    let mut guard = test_guard(&access, lock, "dev-1", Environment::DEVELOPMENT);
    guard.kind_access = kind_access;
    action_availability(ResourceAction::EditValues(ObjectKind::Secret), &guard)
}

#[test]
fn denied_patch_disables_with_reason() {
    let mut map = KindAccessMap::new();
    assert_eq!(
        reason(values_gate(&map, WriteLock::Unlocked)),
        "Checking permissions…"
    );
    map.set(ObjectKind::Secret, patch_report(ObjectKind::Secret, false));
    assert_eq!(
        reason(values_gate(&map, WriteLock::Unlocked)),
        "Not permitted: patch secrets"
    );
    map.set(ObjectKind::Secret, patch_report(ObjectKind::Secret, true));
    assert_eq!(
        values_gate(&map, WriteLock::Unlocked),
        ActionAvailability::Enabled
    );
    assert_eq!(
        reason(values_gate(&map, WriteLock::Locked)),
        "dev-1 is read-only"
    );
}

#[test]
fn an_update_answer_does_not_open_the_values_gate() {
    let mut map = KindAccessMap::new();
    map.set(ObjectKind::Secret, update_report(ObjectKind::Secret, true));
    assert_eq!(
        reason(values_gate(&map, WriteLock::Unlocked)),
        "Not permitted: patch secrets"
    );
}

#[test]
fn edit_values_key_action_is_edit_values() {
    assert_eq!(
        RowAction::EditValues.key_action().name(),
        "k8sboard::EditValues"
    );
    assert_eq!(
        ResourceAction::EditValues(ObjectKind::Secret).row_action(),
        Some(RowAction::EditValues)
    );
    assert_eq!(
        action_label(ResourceAction::EditValues(ObjectKind::Secret)),
        "Edit values"
    );
    assert_eq!(
        action_risk(ResourceAction::EditValues(ObjectKind::Secret)),
        ActionRisk::Change
    );
}

#[test]
fn edit_values_resolves_on_two_kinds_only() {
    let resolved = |subject: &ResourceKey| subject_action(RowAction::EditValues, subject);
    assert_eq!(
        resolved(&kind_key(ResourceKind::ConfigMaps)),
        Some(ResourceAction::EditValues(ObjectKind::ConfigMap))
    );
    assert_eq!(
        resolved(&kind_key(ResourceKind::Secrets)),
        Some(ResourceAction::EditValues(ObjectKind::Secret))
    );
    // A Helm release reads as a Secret by storage; it never offers the editor.
    assert_eq!(resolved(&kind_key(ResourceKind::HelmReleases)), None);
    assert_eq!(resolved(&kind_key(ResourceKind::Deployments)), None);
    assert_eq!(resolved(&pod_key()), None);
    assert_eq!(resolved(&node_key()), None);
    assert_eq!(resolved(&kind_key(ResourceKind::Crds)), None);
}

#[test]
fn edit_yaml_still_resolves_on_the_two_kinds() {
    assert_eq!(
        subject_action(RowAction::EditYaml, &kind_key(ResourceKind::Secrets)),
        Some(ResourceAction::EditYaml(ObjectKind::Secret))
    );
    assert_eq!(
        subject_action(RowAction::EditYaml, &kind_key(ResourceKind::ConfigMaps)),
        Some(ResourceAction::EditYaml(ObjectKind::ConfigMap))
    );
}

#[test]
fn the_two_kind_menus_list_edit_values_and_no_stale_edit() {
    for (kind, object) in [
        (ResourceKind::ConfigMaps, ObjectKind::ConfigMap),
        (ResourceKind::Secrets, ObjectKind::Secret),
    ] {
        let actions = kind.read_only_actions();
        assert_eq!(actions.len(), 1, "{kind:?}");
        assert_eq!(actions[0].label, "Edit values…");
        assert_eq!(actions[0].action, Some(ResourceAction::EditValues(object)));
    }
    assert_eq!(edit_values_kind(ResourceKind::HelmReleases), None);
}

fn secret_summary(
    secret_type: &str,
    labels: &[&str],
    is_immutable: bool,
) -> cluster::SecretSummary {
    cluster::SecretSummary {
        namespace: "shop".to_owned(),
        name: "credentials".to_owned(),
        created_at: None,
        labels: labels.iter().map(|label| (*label).to_owned()).collect(),
        secret_type: secret_type.to_owned(),
        keys: Vec::new(),
        details: if secret_type == "kubernetes.io/service-account-token" {
            cluster::SecretDetails::ServiceAccountToken { account: None }
        } else {
            cluster::SecretDetails::None
        },
        is_immutable,
        is_owned: false,
    }
}

fn config_map_summary(labels: &[&str], is_immutable: bool) -> cluster::ConfigMapSummary {
    cluster::ConfigMapSummary {
        namespace: "shop".to_owned(),
        name: "settings".to_owned(),
        created_at: None,
        labels: labels.iter().map(|label| (*label).to_owned()).collect(),
        keys: Vec::new(),
        is_immutable,
    }
}

#[test]
fn refused_objects_disable_the_item() {
    let block = |object: KindObject| values_edit_block(&object).map(|reason| reason.to_string());
    let secret = |secret_type: &str, labels: &[&str], immutable: bool| {
        block(KindObject::Secret(secret_summary(
            secret_type,
            labels,
            immutable,
        )))
    };
    assert_eq!(secret("Opaque", &[], false), None);
    assert_eq!(
        secret("helm.sh/release.v1", &[], false).as_deref(),
        Some("Helm release records cannot be edited")
    );
    assert_eq!(
        secret("Opaque", &["owner=helm", "name=x"], false).as_deref(),
        Some("Helm release records cannot be edited")
    );
    assert_eq!(
        secret("kubernetes.io/service-account-token", &[], false).as_deref(),
        Some("Service account tokens are managed by Kubernetes")
    );
    assert_eq!(
        secret("Opaque", &[], true).as_deref(),
        Some("Immutable Secret")
    );
    // A ConfigMap labelled owner=helm is a Helm release record of the ConfigMap driver.
    assert_eq!(
        block(KindObject::ConfigMap(config_map_summary(
            &["owner=helm"],
            false
        )))
        .as_deref(),
        Some("Helm release records cannot be edited")
    );
    assert_eq!(
        block(KindObject::ConfigMap(config_map_summary(&[], true))).as_deref(),
        Some("Immutable ConfigMap")
    );
    assert_eq!(
        block(KindObject::ConfigMap(config_map_summary(&[], false))),
        None
    );
    assert_eq!(block(KindObject::Plain), None);
    // The same reasons reach the palette and the menus through `row_block`.
    assert_eq!(
        row_block(
            ResourceAction::EditValues(ObjectKind::Secret),
            &KindObject::Secret(secret_summary("Opaque", &[], true)),
            None
        )
        .as_deref(),
        Some("Immutable Secret")
    );
}

/// Every action, with the kind-carrying ones once for a kind that resolves and once for one that
/// has no check (`Planned`).
fn every_action() -> Vec<ResourceAction> {
    vec![
        ResourceAction::ViewLogs,
        ResourceAction::OpenShell,
        ResourceAction::PortForward,
        ResourceAction::OpenNodeShell,
        ResourceAction::DebugContainer,
        ResourceAction::Cordon,
        ResourceAction::Uncordon,
        ResourceAction::Drain,
        ResourceAction::EditTaints,
        ResourceAction::EditLabels,
        ResourceAction::CopyName,
        ResourceAction::ViewYaml,
        ResourceAction::EditYaml(ObjectKind::Pod),
        ResourceAction::EditValues(ObjectKind::ConfigMap),
        ResourceAction::Delete(ObjectKind::Pod),
        ResourceAction::RestartRollout(ObjectKind::Deployment),
        ResourceAction::RestartRollout(ObjectKind::Pod),
        ResourceAction::Scale(ObjectKind::Deployment),
        ResourceAction::Scale(ObjectKind::Pod),
        ResourceAction::PauseRollout,
        ResourceAction::RollBack,
        ResourceAction::SuspendCronJob,
        ResourceAction::TriggerCronJob,
        ResourceAction::RerunJob,
        ResourceAction::EditHpaRange,
        ResourceAction::ExpandClaim,
        ResourceAction::SetDefaultStorageClass,
    ]
}

#[test]
fn needs_confirm_matches_the_mutating_gate() {
    for action in every_action() {
        assert_eq!(
            needs_confirm(action),
            matches!(action.gate(), ActionGate::Mutating { .. }),
            "{action:?}"
        );
    }
    // The read-only actions never reach a confirm; a shipped write does.
    for action in [
        ResourceAction::ViewLogs,
        ResourceAction::ViewYaml,
        ResourceAction::CopyName,
    ] {
        assert!(!needs_confirm(action), "{action:?}");
    }
    for action in [
        ResourceAction::RestartRollout(ObjectKind::Deployment),
        ResourceAction::OpenShell,
        ResourceAction::EditYaml(ObjectKind::Pod),
    ] {
        assert!(needs_confirm(action), "{action:?}");
    }
}

#[test]
fn is_planned_matches_the_unshipped_gates() {
    for action in every_action() {
        let is_unshipped = matches!(
            action.gate(),
            ActionGate::Planned
                | ActionGate::Mutating {
                    is_shipped: false,
                    ..
                }
        );
        assert_eq!(is_planned(action), is_unshipped, "{action:?}");
    }
    // A kind with no restart or scale check has no action to run: it stays planned.
    assert!(is_planned(ResourceAction::RestartRollout(ObjectKind::Pod)));
    assert!(is_planned(ResourceAction::Scale(ObjectKind::Pod)));
    // Drain shipped with spec 0034, so it is not planned any more.
    assert!(!is_planned(ResourceAction::Drain));
    assert!(!is_planned(ResourceAction::ViewLogs));
}

// ---- Attach (spec 0040) ----

fn attachable(name: &str, kind: ContainerKind) -> ContainerSummary {
    let mut container = container(name, kind, true);
    container.terminal = cluster::ContainerTerminal::Interactive;
    container
}

#[test]
fn attach_gate_needs_both_attach_verbs() {
    let allowed = known_denying(&[]);
    assert_eq!(
        action_availability(ResourceAction::Attach, &unlocked(&allowed)),
        ActionAvailability::Enabled
    );
    assert_eq!(
        reason(action_availability(
            ResourceAction::Attach,
            &unlocked(&checking())
        )),
        "Checking permissions…"
    );
    for verb in [AccessCheck::GetPodAttach, AccessCheck::CreatePodAttach] {
        let denied = known_denying(&[verb]);
        assert_eq!(
            reason(action_availability(
                ResourceAction::Attach,
                &unlocked(&denied)
            )),
            "Not permitted: get and create pods/attach",
            "{verb:?}"
        );
    }
    let locked = test_guard(
        &allowed,
        WriteLock::Locked,
        "prod-1",
        Environment::PRODUCTION,
    );
    assert_eq!(
        reason(action_availability(ResourceAction::Attach, &locked)),
        "prod-1 is read-only"
    );
    assert_eq!(action_risk(ResourceAction::Attach), ActionRisk::Change);
    assert_eq!(action_label(ResourceAction::Attach), "Attach");
}

#[test]
fn attach_block_reasons() {
    let ok = attachable("app", ContainerKind::Main);
    assert_eq!(attach_block(&ok), None);
    let mut stopped = ok.clone();
    stopped.state = ContainerState::Waiting {
        reason: None,
        message: None,
    };
    assert_eq!(
        attach_block(&stopped).as_deref(),
        Some("Container is not running")
    );
    let init = attachable("init", ContainerKind::Init);
    assert_eq!(
        attach_block(&init).as_deref(),
        Some("Init containers cannot be attached")
    );
    let plain = container("app", ContainerKind::Main, true);
    assert_eq!(
        attach_block(&plain).as_deref(),
        Some("The container has no terminal (stdin and tty); use View logs")
    );
    // A sidecar with a terminal can be attached, and `stdinOnce` is still a terminal.
    let mut once = attachable("proxy", ContainerKind::Sidecar);
    once.terminal = cluster::ContainerTerminal::InteractiveOnce;
    assert_eq!(attach_block(&once), None);
}

#[test]
fn default_attach_container_prefers_running_main() {
    let pod = pod_with(vec![
        attachable("proxy", ContainerKind::Sidecar),
        attachable("app", ContainerKind::Main),
    ]);
    assert_eq!(
        default_attach_container(&pod).map(|container| container.name.as_str()),
        Ok("app")
    );
    // A main container without a terminal does not hide a sidecar that has one.
    let pod = pod_with(vec![
        attachable("proxy", ContainerKind::Sidecar),
        container("app", ContainerKind::Main, true),
    ]);
    assert_eq!(
        default_attach_container(&pod).map(|container| container.name.as_str()),
        Ok("proxy")
    );
    let none = pod_with(vec![container("app", ContainerKind::Main, true)]);
    assert_eq!(
        default_attach_container(&none).err().as_deref(),
        Some("No running container has a terminal (stdin and tty); use View logs")
    );
}

#[test]
fn a_key_reads_the_gate_then_the_default_container() {
    let subject = ResourceKey::Pod {
        namespace: "shop".to_owned(),
        name: "api-0".to_owned(),
    };
    let allowed = known_denying(&[]);
    let with_terminal = pod_with(vec![attachable("app", ContainerKind::Main)]);
    let without = pod_with(vec![container("app", ContainerKind::Main, true)]);
    let key = |pod: Option<&PodSummary>, access: &AccessState| {
        key_availability_of(RowAction::Attach, &subject, pod, &unlocked(access))
    };
    assert_eq!(
        key(Some(&with_terminal), &allowed),
        KeyAvailability::Run(ResourceAction::Attach)
    );
    assert_eq!(
        disabled_reason(key(Some(&without), &allowed)),
        "No running container has a terminal (stdin and tty); use View logs"
    );
    // The gate wins over the pod: the reason the user can act on first comes first.
    assert_eq!(
        disabled_reason(key(
            Some(&without),
            &known_denying(&[AccessCheck::GetPodAttach])
        )),
        "Not permitted: get and create pods/attach"
    );
    assert_eq!(key(None, &allowed), KeyAvailability::NotOffered);
}

// ---- Restart pod and Evict (spec 0040) ----

fn owned_by(kind: &str) -> Option<cluster::ControllerRef> {
    Some(cluster::ControllerRef {
        kind: kind.to_owned(),
        name: "owner".to_owned(),
    })
}

fn removal_pod(controller: Option<cluster::ControllerRef>) -> PodSummary {
    PodSummary {
        controller,
        ..pod_with(Vec::new())
    }
}

#[test]
fn pod_menu_follows_w4_order() {
    use PodMenuEntry::*;
    assert_eq!(
        POD_MENU,
        [
            ViewLogs,
            OpenShell,
            DebugContainer,
            PortForward,
            Attach,
            Separator,
            EditYaml,
            ViewYaml,
            RestartPod,
            EvictPod,
            Separator,
            CopyName,
            CopyKubectlCommand,
            Separator,
            DeletePod,
        ]
    );
}

#[test]
fn restart_pod_refuses_bare_static_finished_and_terminating_pods() {
    let restart = |pod: &PodSummary| pod_block(ResourceAction::RestartPod, pod);
    assert_eq!(restart(&removal_pod(owned_by("ReplicaSet"))), None);
    assert_eq!(restart(&removal_pod(owned_by("StatefulSet"))), None);
    assert_eq!(restart(&removal_pod(owned_by("MyOperatorKind"))), None);
    assert_eq!(
        restart(&removal_pod(None)).as_deref(),
        Some("Not managed by a controller; it would not come back. Use Delete pod…")
    );
    assert_eq!(
        restart(&removal_pod(owned_by("Node"))).as_deref(),
        Some("Static pod: the kubelet owns it")
    );
    let mut terminating = removal_pod(owned_by("ReplicaSet"));
    terminating.status = cluster::PodStatus::Terminating;
    assert_eq!(
        restart(&terminating).as_deref(),
        Some("Already terminating")
    );
    // The phase decides, never the status reason: an `Error` reason on a running pod restarts.
    let mut errored = removal_pod(owned_by("ReplicaSet"));
    errored.status = cluster::PodStatus::Reason(cluster::StatusReason::Error);
    assert_eq!(restart(&errored), None);
    let mut finished = removal_pod(owned_by("Job"));
    finished.is_finished = true;
    assert_eq!(
        restart(&finished).as_deref(),
        Some("The pod has finished; its controller does not restart it")
    );
}

#[test]
fn evict_refuses_static_and_terminating_pods_only() {
    let evict = |pod: &PodSummary| pod_block(ResourceAction::EvictPod, pod);
    let static_pod = removal_pod(owned_by("Node"));
    assert_eq!(evict(&static_pod).as_deref(), Some(STATIC_POD_TEXT));
    // One text for both actions.
    assert_eq!(
        pod_block(ResourceAction::RestartPod, &static_pod).as_deref(),
        Some(STATIC_POD_TEXT)
    );
    let mut terminating = removal_pod(owned_by("ReplicaSet"));
    terminating.status = cluster::PodStatus::Terminating;
    assert_eq!(evict(&terminating).as_deref(), Some("Already terminating"));
    // A bare pod, a DaemonSet pod, and a finished pod can be evicted.
    assert_eq!(evict(&removal_pod(None)), None);
    assert_eq!(evict(&removal_pod(owned_by("DaemonSet"))), None);
    let mut finished = removal_pod(owned_by("Job"));
    finished.is_finished = true;
    assert_eq!(evict(&finished), None);
}

#[test]
fn restart_and_evict_gates_and_risks() {
    assert_eq!(
        action_risk(ResourceAction::RestartPod),
        ActionRisk::Destructive
    );
    assert_eq!(
        action_risk(ResourceAction::EvictPod),
        ActionRisk::Destructive
    );
    assert_eq!(action_label(ResourceAction::RestartPod), "Restart pod");
    assert_eq!(action_label(ResourceAction::EvictPod), "Evict");
    // Evict reads the session report: create pods/eviction.
    let denied = known_denying(&[AccessCheck::CreatePodEviction]);
    assert_eq!(
        reason(action_availability(
            ResourceAction::EvictPod,
            &unlocked(&denied)
        )),
        "Not permitted: create pods/eviction"
    );
    // Restart reads the lazy review Delete pod reads, so the two items agree.
    assert_eq!(
        action_availability(ResourceAction::RestartPod, &unlocked(&known_denying(&[]))),
        action_availability(
            ResourceAction::Delete(ObjectKind::Pod),
            &unlocked(&known_denying(&[]))
        ),
    );
    let allowed = known_denying(&[]);
    let guard = unlocked(&allowed);
    assert_eq!(
        reason(action_availability(ResourceAction::RestartPod, &guard)),
        "Checking permissions…"
    );
    // Neither has a single key: the unit actions are in no binding.
    assert_eq!(
        RowAction::RestartPod.key_action().name(),
        "k8sboard::RestartPod"
    );
    assert_eq!(
        RowAction::EvictPod.key_action().name(),
        "k8sboard::EvictPod"
    );
}

#[test]
fn a_removal_key_reads_the_gate_then_the_pod() {
    let subject = ResourceKey::Pod {
        namespace: "shop".to_owned(),
        name: "api-0".to_owned(),
    };
    let allowed = known_denying(&[]);
    let bare = removal_pod(None);
    let key = |row: RowAction, pod: Option<&PodSummary>, access: &AccessState| {
        key_availability_of(row, &subject, pod, &unlocked(access))
    };
    // Evict of a bare pod runs; Restart says why not.
    assert_eq!(
        key(RowAction::EvictPod, Some(&bare), &allowed),
        KeyAvailability::Run(ResourceAction::EvictPod)
    );
    // The lazy Delete review has not answered here, so Restart reads `Checking` first.
    assert_eq!(
        disabled_reason(key(RowAction::RestartPod, Some(&bare), &allowed)),
        "Checking permissions…"
    );
    assert_eq!(
        disabled_reason(key(
            RowAction::EvictPod,
            Some(&bare),
            &known_denying(&[AccessCheck::CreatePodEviction])
        )),
        "Not permitted: create pods/eviction"
    );
    assert_eq!(
        key(RowAction::EvictPod, None, &allowed),
        KeyAvailability::NotOffered
    );
    let node = ResourceKey::Node {
        name: "wk-01".to_owned(),
    };
    assert_eq!(subject_action(RowAction::RestartPod, &node), None);
    assert_eq!(subject_action(RowAction::EvictPod, &node), None);
}

// ---- New from templates (spec 0042) ----

#[test]
fn the_new_button_is_a_gated_header_action_with_no_row() {
    let action = ResourceAction::CreateObject(ObjectKind::ConfigMap);
    assert_eq!(action.row_action(), None);
    assert_eq!(action_label(action), "New ConfigMap");
    assert_eq!(action_risk(action), ActionRisk::Change);
    assert!(needs_confirm(action));
    assert!(!is_planned(action));
}

fn create_gate(kind_access: &KindAccessMap, lock: WriteLock) -> ActionAvailability {
    let access = known_denying(&[]);
    let mut guard = test_guard(&access, lock, "dev-1", Environment::DEVELOPMENT);
    guard.kind_access = kind_access;
    action_availability(ResourceAction::CreateObject(ObjectKind::ConfigMap), &guard)
}

fn create_report(kind: ObjectKind, is_allowed: bool) -> KindAccess {
    let decision = if is_allowed {
        AccessDecision::Allowed
    } else {
        AccessDecision::Denied { reason: None }
    };
    KindAccess::Known(AccessReport {
        reviews: vec![AccessReview {
            check: AccessCheck::Create(kind),
            decision,
        }],
    })
}

#[test]
fn new_button_disabled_with_gate_reason() {
    let mut map = KindAccessMap::new();
    assert_eq!(
        reason(create_gate(&map, WriteLock::Unlocked)),
        "Checking permissions…"
    );
    map.set(ObjectKind::ConfigMap, KindAccess::Unknown);
    assert_eq!(
        reason(create_gate(&map, WriteLock::Unlocked)),
        "Permissions could not be checked"
    );
    map.set(
        ObjectKind::ConfigMap,
        create_report(ObjectKind::ConfigMap, false),
    );
    assert_eq!(
        reason(create_gate(&map, WriteLock::Unlocked)),
        "Not permitted: create configmaps"
    );
    map.set(
        ObjectKind::ConfigMap,
        create_report(ObjectKind::ConfigMap, true),
    );
    assert_eq!(
        create_gate(&map, WriteLock::Unlocked),
        ActionAvailability::Enabled
    );
    // The lock comes after the permission.
    assert_eq!(
        reason(create_gate(&map, WriteLock::Locked)),
        "dev-1 is read-only"
    );
}

// ---- 0018 step 6: Certificate Renew now ----

/// A Certificates kind served at `version`; its CRD name is the cert-manager one whatever the
/// version, like a cluster that serves only an old API.
fn served_certificate(group: &str, version: &str) -> crate::custom_kind::CustomKind {
    let crd = cluster::CrdSummary {
        name: format!("certificates.{group}"),
        group: group.to_owned(),
        kind: "Certificate".to_owned(),
        plural: "certificates".to_owned(),
        singular: "certificate".to_owned(),
        scope: cluster::ResourceScope::Namespaced,
        versions: vec![cluster::CrdVersion {
            name: version.to_owned(),
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

fn renew_key_availability(
    kind: crate::custom_kind::CustomKind,
    access: &AccessState,
    lock: WriteLock,
) -> KeyAvailability {
    let guard = test_guard(access, lock, "dev-1", Environment::DEVELOPMENT);
    key_availability_of(
        RowAction::RenewCertificate,
        &kind_key(ResourceKind::Custom(kind)),
        None,
        &guard,
    )
}

#[test]
fn renew_item_only_on_cert_manager_kind() {
    let certificates = ResourceKind::Custom(served_certificate("cert-manager.io", "v1"));
    let items = certificates.read_only_actions();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].label, "Renew now");
    assert_eq!(items[0].action, Some(ResourceAction::RenewCertificate));
    assert_eq!(
        subject_action(RowAction::RenewCertificate, &kind_key(certificates)),
        Some(ResourceAction::RenewCertificate)
    );
    // Argo CD Applications, other custom kinds, and built-in kinds offer none.
    let applications = ResourceKind::Custom(served_widget("applications.argoproj.io"));
    assert!(applications.read_only_actions().is_empty());
    for kind in [
        applications,
        ResourceKind::Custom(served_widget("certificates.example.org")),
        ResourceKind::Deployments,
    ] {
        assert_eq!(
            subject_action(RowAction::RenewCertificate, &kind_key(kind)),
            None,
            "{kind:?}"
        );
    }
    assert_eq!(
        subject_action(RowAction::RenewCertificate, &pod_key()),
        None
    );
}

#[test]
fn renew_item_disabled_reasons() {
    let v1 = served_certificate("cert-manager.io", "v1");
    assert_eq!(
        disabled_reason(renew_key_availability(v1, &checking(), WriteLock::Unlocked)),
        "Checking permissions…"
    );
    assert_eq!(
        disabled_reason(renew_key_availability(
            v1,
            &known_denying(&[AccessCheck::UpdateCertificateStatus]),
            WriteLock::Unlocked
        )),
        "Not permitted: update certificates/status"
    );
    assert_eq!(
        disabled_reason(renew_key_availability(
            v1,
            &known_denying(&[]),
            WriteLock::Locked
        )),
        "dev-1 is read-only"
    );
    assert_eq!(
        renew_key_availability(v1, &known_denying(&[]), WriteLock::Unlocked),
        KeyAvailability::Run(ResourceAction::RenewCertificate)
    );
    // The gate says no first; only an open gate reaches the version.
    let old = served_certificate("cert-manager.io", "v1alpha2");
    assert_eq!(
        disabled_reason(renew_key_availability(
            old,
            &known_denying(&[]),
            WriteLock::Unlocked
        )),
        "Needs cert-manager.io/v1"
    );
    assert_eq!(
        disabled_reason(renew_key_availability(
            old,
            &known_denying(&[AccessCheck::UpdateCertificateStatus]),
            WriteLock::Unlocked
        )),
        "Not permitted: update certificates/status"
    );
}

#[test]
fn renew_is_a_change_with_a_key_and_a_label() {
    let action = ResourceAction::RenewCertificate;
    assert_eq!(action_risk(action), ActionRisk::Change);
    assert_eq!(action_label(action), "Renew now");
    assert_eq!(action.row_action(), Some(RowAction::RenewCertificate));
    assert!(needs_confirm(action));
    assert!(!is_planned(action));
    assert!(
        RowAction::RenewCertificate
            .key_action()
            .partial_eq(&crate::keymap::RenewCertificate)
    );
}

#[test]
fn only_the_cert_manager_kind_is_v1_gated() {
    let v1 = served_certificate("cert-manager.io", "v1");
    let action = ResourceAction::RenewCertificate;
    assert_eq!(kind_block(action, ResourceKind::Custom(v1)), None);
    assert_eq!(
        kind_block(
            action,
            ResourceKind::Custom(served_certificate("cert-manager.io", "v1beta1"))
        )
        .as_deref(),
        Some(NEEDS_CERT_MANAGER_V1)
    );
    assert_eq!(kind_block(action, ResourceKind::Deployments), None);
    assert_eq!(
        kind_block(ResourceAction::PauseRollout, ResourceKind::Custom(v1)),
        None
    );
}

// ---- Show remaining resources, Show selected pods ----

fn namespace_in(phase: NamespacePhase) -> KindRow {
    crate::namespace_rows::namespace_row(&NamespaceSummary {
        name: "team-a".to_owned(),
        phase,
        labels: Vec::new(),
        created_at: None,
        deleting_since: None,
        deletion_conditions: Vec::new(),
    })
}

fn budget_row() -> KindRow {
    crate::policy_rows::pod_disruption_budget_row(&PodDisruptionBudgetSummary {
        namespace: "shop".to_owned(),
        name: "web".to_owned(),
        created_at: None,
        labels: Vec::new(),
        min_available: Some("2".to_owned()),
        max_unavailable: None,
        selector: None,
        current_healthy: 2,
        desired_healthy: 2,
        expected_pods: 2,
        disruptions_allowed: 0,
        unhealthy_pod_eviction_policy: None,
        conditions: Vec::new(),
        is_status_stale: false,
    })
}

#[test]
fn a_terminating_namespace_shows_its_remaining_resources() {
    let row = namespace_in(NamespacePhase::Terminating);
    let show = show_section(&row).expect("namespaces have the item");
    assert_eq!(show.label, "Show remaining resources");
    assert_eq!(show.block, None);
    assert!(row.section(show.title).is_some());
}

#[test]
fn an_active_namespace_has_no_remaining_resources_to_show() {
    let row = namespace_in(NamespacePhase::Active);
    let show = show_section(&row).expect("namespaces have the item");
    assert_eq!(show.block, Some("The namespace is not terminating"));
    assert!(row.section(show.title).is_none());
}

#[test]
fn a_budget_shows_its_selected_pods() {
    let row = budget_row();
    let show = show_section(&row).expect("budgets have the item");
    assert_eq!(show.label, "Show selected pods");
    assert_eq!(show.block, None);
    assert!(row.section(show.title).is_some());
}

#[test]
fn other_kinds_have_no_show_section_item() {
    let row = crate::kind_row::KindRow {
        namespace: Some("shop".to_owned()),
        name: "api".to_owned(),
        created_at: None,
        status: crate::status_tone::StatusLabel {
            text: "Ready".into(),
            tone: crate::status_tone::StatusTone::Ok,
        },
        cells: Vec::new(),
        sections: Vec::new(),
        event: None,
        related_pods: None,
        labels: Vec::new(),
        object: KindObject::Plain,
    };
    assert_eq!(show_section(&row), None);
}

#[test]
fn row_keyed_sets_key_and_icon() {
    let item = row_keyed(PopupMenuItem::new("Delete"), RowAction::Delete);
    assert!(matches!(
        item,
        PopupMenuItem::Item {
            icon: Some(_),
            action: Some(_),
            ..
        }
    ));
    let disabled = row_keyed(
        disabled_menu_item("Copy name", "Not connected".into()),
        RowAction::CopyName,
    );
    assert!(matches!(
        disabled,
        PopupMenuItem::ElementItem {
            icon: Some(_),
            action: Some(_),
            ..
        }
    ));
}
