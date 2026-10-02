use cluster::{AccessDecision, AccessReport, AccessReview};
use gpui_kit::Task;

use super::*;

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

#[test]
fn copy_name_always_enabled() {
    for access in [
        checking(),
        unknown(),
        known_denying(&[]),
        known_denying(&AccessCheck::ALL),
    ] {
        assert_eq!(
            action_availability(ResourceAction::CopyName, &access),
            ActionAvailability::Enabled
        );
    }
}

#[test]
fn cordon_and_drain_always_disabled_read_only_mode() {
    for access in [checking(), unknown(), known_denying(&[])] {
        for action in [ResourceAction::Cordon, ResourceAction::Drain] {
            assert_eq!(
                reason(action_availability(action, &access)),
                "Read-only mode"
            );
        }
    }
}

#[test]
fn gated_actions_disabled_while_checking() {
    for action in [
        ResourceAction::ViewLogs,
        ResourceAction::OpenShell,
        ResourceAction::PortForward,
        ResourceAction::OpenNodeShell,
    ] {
        assert_eq!(
            reason(action_availability(action, &checking())),
            "Checking permissions…"
        );
        assert_eq!(
            reason(action_availability(action, &unknown())),
            "Permissions could not be checked"
        );
    }
}

#[test]
fn shell_denied_reason_names_access_check() {
    let access = known_denying(&[
        AccessCheck::CreatePodExec,
        AccessCheck::CreatePodPortForward,
    ]);
    assert_eq!(
        reason(action_availability(ResourceAction::OpenShell, &access)),
        "Not permitted: create pods/exec"
    );
    assert_eq!(
        reason(action_availability(ResourceAction::PortForward, &access)),
        "Not permitted: create pods/portforward"
    );
}

#[test]
fn shell_allowed_still_disabled_in_read_only_mode() {
    let access = known_denying(&[]);
    assert_eq!(
        reason(action_availability(ResourceAction::OpenShell, &access)),
        "Not available in read-only mode"
    );
    assert_eq!(
        reason(action_availability(ResourceAction::PortForward, &access)),
        "Not available in read-only mode"
    );
}

#[test]
fn node_shell_gated_by_create_pods_exec() {
    assert_eq!(
        reason(action_availability(
            ResourceAction::OpenNodeShell,
            &known_denying(&[AccessCheck::CreatePodExec])
        )),
        "Not permitted: create pods/exec"
    );
    assert_eq!(
        reason(action_availability(
            ResourceAction::OpenNodeShell,
            &known_denying(&[AccessCheck::CreatePodPortForward])
        )),
        "Not available in read-only mode"
    );
}

#[test]
fn logs_allowed_is_enabled() {
    assert_eq!(
        action_availability(ResourceAction::ViewLogs, &known_denying(&[])),
        ActionAvailability::Enabled
    );
}

#[test]
fn logs_denied_reason_names_access_check() {
    assert_eq!(
        reason(action_availability(
            ResourceAction::ViewLogs,
            &known_denying(&[AccessCheck::GetPodLogs])
        )),
        "Not permitted: get pods/log"
    );
}

#[test]
fn forward_button_reason_follows_the_port_forward_gate() {
    assert_eq!(
        port_forward_reason(&known_denying(&[AccessCheck::CreatePodPortForward])),
        "Not permitted: create pods/portforward"
    );
    assert_eq!(
        port_forward_reason(&known_denying(&[])),
        "Not available in read-only mode"
    );
    assert_eq!(port_forward_reason(&checking()), "Checking permissions…");
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
        filter: None,
        select: None,
        theme: None,
        screen: crate::launch_options::LaunchScreen::Kind(ResourceKind::Secrets),
        screenshot: Some("secrets.png".into()),
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
    assert_eq!(
        ResourceKind::HelmReleases.read_only_actions(),
        ["Roll back…"]
    );
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
