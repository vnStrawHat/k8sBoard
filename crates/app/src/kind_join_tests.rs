use cluster::{
    ContainerKind, ContainerProbes, ContainerResource, ContainerState, ContainerSummary,
    ControllerRef, EnvEntry, EnvFromEntry, EnvFromSource, EnvSource, MountEntry, NamespacePhase,
    NamespaceScope, NamespaceSummary, PodStatus, ReadyCount, SecretDetails, ServicePortSummary,
    StatusReason, VolumeSource,
};

use super::*;
use crate::network_rows::service_row;

fn service(selector: &[&str]) -> ServiceSummary {
    ServiceSummary {
        namespace: "team-a".to_owned(),
        name: "api".to_owned(),
        created_at: None,
        labels: Vec::new(),
        service_type: "ClusterIP".to_owned(),
        cluster_ips: vec!["10.0.0.5".to_owned()],
        is_headless: false,
        external_addresses: Vec::new(),
        ports: vec![ServicePortSummary {
            name: None,
            port: 80,
            target_port: None,
            node_port: None,
            protocol: "TCP".to_owned(),
        }],
        selector: selector.iter().map(|term| (*term).to_owned()).collect(),
    }
}

fn pod(namespace: &str, name: &str, labels: &[&str]) -> PodSummary {
    PodSummary {
        annotations: cluster::AnnotationTerms::default(),
        is_finished: false,
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        status: cluster::PodStatus::Reason(StatusReason::Running),
        ready: ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: None,
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: None,
        conditions: Vec::new(),
        status_message: None,
        labels: labels.iter().map(|term| (*term).to_owned()).collect(),
        host_network: false,
        image_pull_secrets: Vec::new(),
        node_selector: Vec::new(),
        node_affinity: Vec::new(),
        containers: Vec::new(),
    }
}

fn endpoint(address: &str, pod: Option<&str>, is_ready: bool) -> EndpointSummary {
    EndpointSummary {
        address: address.to_owned(),
        is_ready,
        is_terminating: false,
        pod: pod.map(str::to_owned),
        node: None,
    }
}

fn slice(
    service: Option<&str>,
    address_type: &str,
    endpoints: Vec<EndpointSummary>,
) -> EndpointSliceSummary {
    EndpointSliceSummary {
        namespace: "team-a".to_owned(),
        name: format!("{}-abc", service.unwrap_or("orphan")),
        service: service.map(str::to_owned),
        address_type: address_type.to_owned(),
        ports: Vec::new(),
        endpoints,
    }
}

fn ready_list<T>(items: Vec<T>) -> LiveList<T> {
    LiveList::Ready {
        items,
        interruption: None,
    }
}

/// The Service row after a join. `slices: None` means the companion is not running.
fn joined(
    service: &ServiceSummary,
    pods: Vec<PodSummary>,
    slices: Option<Vec<EndpointSliceSummary>>,
) -> KindRow {
    let mut rows = vec![service_row(service)];
    let pods = ready_list(pods);
    let companion = slices.map(|slices| CompanionLists::EndpointSlices(ready_list(slices)));
    let inputs = JoinInputs {
        custom_counts: None,
        pods: &pods,
        companion: companion.as_ref(),
        kubelet: None,
        scope: &NamespaceScope::All,
    };
    join_rows(ResourceKind::Services, &mut rows, &inputs);
    rows.remove(0)
}

fn toned(text: &str, tone: StatusTone) -> StatusLabel {
    StatusLabel {
        text: text.to_owned().into(),
        tone,
    }
}

fn endpoints_cell(row: &KindRow) -> &KindCell {
    row.cells
        .get(SERVICE_ENDPOINTS)
        .expect("a Services row has an Endpoints cell")
}

fn api_pods() -> Vec<PodSummary> {
    vec![
        pod("team-a", "api-1", &["app=api"]),
        pod("team-a", "api-2", &["app=api"]),
    ]
}

fn api_endpoints(ready: [bool; 2]) -> Vec<EndpointSummary> {
    vec![
        endpoint("10.0.0.1", Some("api-1"), ready[0]),
        endpoint("10.0.0.2", Some("api-2"), ready[1]),
    ]
}

#[test]
fn joined_column_indices_name_their_columns() {
    let columns = ResourceKind::Services.columns();
    assert_eq!(
        columns.get(SERVICE_ENDPOINTS).map(|column| column.name),
        Some("Endpoints")
    );
    assert_eq!(service_row(&service(&[])).cells.len(), columns.len());
    let policy_columns = ResourceKind::NetworkPolicies.columns();
    assert_eq!(
        policy_columns
            .get(NETWORK_POLICY_AFFECTS)
            .map(|column| column.name),
        Some("Affects")
    );
    let class_columns = ResourceKind::StorageClasses.columns();
    assert_eq!(
        class_columns.get(CLASS_VOLUMES).map(|column| column.name),
        Some("PVs")
    );
    let role_columns = ResourceKind::Roles.columns();
    assert_eq!(
        role_columns.get(ROLE_BINDINGS).map(|column| column.name),
        Some("Bindings")
    );
    let cluster_role_columns = ResourceKind::ClusterRoles.columns();
    assert_eq!(
        cluster_role_columns
            .get(CLUSTER_ROLE_BINDINGS)
            .map(|column| column.name),
        Some("Bindings")
    );
    let claim_columns = ResourceKind::PersistentVolumeClaims.columns();
    assert_eq!(
        claim_columns.get(CLAIM_USED).map(|column| column.name),
        Some("Used")
    );
    let secret_columns = ResourceKind::Secrets.columns();
    assert_eq!(
        secret_columns.get(SECRET_USED_BY).map(|column| column.name),
        Some("Used by")
    );
}

#[test]
fn service_endpoints_all_ready() {
    let row = joined(
        &service(&["app=api"]),
        api_pods(),
        Some(vec![slice(Some("api"), "IPv4", api_endpoints([true; 2]))]),
    );
    assert_eq!(row.status, toned("2 endpoints ready", StatusTone::Ok));
    assert_eq!(
        endpoints_cell(&row),
        &KindCell::Toned(toned("2", StatusTone::Ok))
    );
}

#[test]
fn service_endpoints_partial() {
    let row = joined(
        &service(&["app=api"]),
        api_pods(),
        Some(vec![slice(
            Some("api"),
            "IPv4",
            api_endpoints([true, false]),
        )]),
    );
    assert_eq!(
        row.status,
        toned("1 of 2 endpoints ready", StatusTone::Warn)
    );
    assert_eq!(
        endpoints_cell(&row),
        &KindCell::Toned(toned("1 of 2", StatusTone::Warn))
    );
}

#[test]
fn service_without_ready_endpoints_is_bad() {
    let row = joined(
        &service(&["app=api"]),
        api_pods(),
        Some(vec![slice(
            Some("api"),
            "IPv4",
            api_endpoints([false, false]),
        )]),
    );
    assert_eq!(row.status, toned("0 of 2 endpoints ready", StatusTone::Bad));
    assert_eq!(
        endpoints_cell(&row),
        &KindCell::Toned(toned("0 of 2", StatusTone::Bad))
    );
}

#[test]
fn service_matches_no_pods() {
    let row = joined(
        &service(&["app=api"]),
        vec![pod("team-a", "web-1", &["app=web"])],
        Some(vec![slice(Some("api"), "IPv4", Vec::new())]),
    );
    assert_eq!(row.status, toned("Matches no pods", StatusTone::Bad));
    assert_eq!(
        endpoints_cell(&row),
        &KindCell::Toned(toned("0", StatusTone::Bad))
    );
}

#[test]
fn pods_of_another_namespace_do_not_match() {
    let row = joined(
        &service(&["app=api"]),
        vec![pod("team-b", "api-1", &["app=api"])],
        None,
    );
    assert_eq!(row.status, toned("Matches no pods", StatusTone::Bad));
}

#[test]
fn service_without_endpoints_is_bad() {
    let row = joined(
        &service(&["app=api"]),
        api_pods(),
        Some(vec![slice(Some("api"), "IPv4", Vec::new())]),
    );
    assert_eq!(row.status, toned("No endpoints", StatusTone::Bad));
    assert_eq!(
        endpoints_cell(&row),
        &KindCell::Toned(toned("0", StatusTone::Bad))
    );
}

#[test]
fn external_name_has_no_endpoints() {
    let mut external = service(&[]);
    external.service_type = "ExternalName".to_owned();
    let row = joined(&external, Vec::new(), Some(Vec::new()));
    assert_eq!(row.status, toned("ExternalName", StatusTone::Ok));
    assert_eq!(endpoints_cell(&row), &KindCell::Absent);
}

#[test]
fn selector_less_service_without_slices_warns() {
    let row = joined(&service(&[]), api_pods(), Some(Vec::new()));
    assert_eq!(row.status, toned("No endpoints", StatusTone::Warn));
    assert_eq!(
        endpoints_cell(&row),
        &KindCell::Toned(toned("0", StatusTone::Warn))
    );
}

#[test]
fn selector_less_service_does_not_match_every_pod() {
    let health = service_health(&service(&[]), &api_pods(), None);
    assert_eq!(health.matching_pods, None);
}

#[test]
fn slices_without_service_label_are_ignored() {
    let row = joined(
        &service(&["app=api"]),
        api_pods(),
        Some(vec![slice(None, "IPv4", api_endpoints([true; 2]))]),
    );
    assert_eq!(row.status, toned("No endpoints", StatusTone::Bad));
}

#[test]
fn fqdn_slices_are_ignored() {
    let row = joined(
        &service(&["app=api"]),
        api_pods(),
        Some(vec![slice(Some("api"), "FQDN", api_endpoints([true; 2]))]),
    );
    assert_eq!(row.status, toned("No endpoints", StatusTone::Bad));
}

#[test]
fn slices_of_another_service_are_ignored() {
    let row = joined(
        &service(&["app=api"]),
        api_pods(),
        Some(vec![slice(Some("web"), "IPv4", api_endpoints([true; 2]))]),
    );
    assert_eq!(row.status, toned("No endpoints", StatusTone::Bad));
}

#[test]
fn dual_stack_counts_once() {
    let ipv6 = vec![
        endpoint("fd00::1", Some("api-1"), true),
        endpoint("fd00::2", Some("api-2"), true),
    ];
    let row = joined(
        &service(&["app=api"]),
        api_pods(),
        Some(vec![
            slice(Some("api"), "IPv4", api_endpoints([true; 2])),
            slice(Some("api"), "IPv6", ipv6),
        ]),
    );
    assert_eq!(row.status, toned("2 endpoints ready", StatusTone::Ok));
}

#[test]
fn endpoints_without_a_pod_dedupe_by_address() {
    let twice = vec![
        endpoint("10.0.0.9", None, true),
        endpoint("10.0.0.9", None, true),
    ];
    let row = joined(
        &service(&[]),
        Vec::new(),
        Some(vec![slice(Some("api"), "IPv4", twice)]),
    );
    assert_eq!(row.status, toned("1 endpoint ready", StatusTone::Ok));
}

#[test]
fn terminating_excluded_from_total() {
    let mut draining = endpoint("10.0.0.2", Some("api-2"), false);
    draining.is_terminating = true;
    let row = joined(
        &service(&["app=api"]),
        api_pods(),
        Some(vec![slice(
            Some("api"),
            "IPv4",
            vec![endpoint("10.0.0.1", Some("api-1"), true), draining],
        )]),
    );
    assert_eq!(row.status, toned("1 endpoint ready", StatusTone::Ok));
}

#[test]
fn denied_slices_keep_builder_status() {
    let row = joined(&service(&["app=api"]), api_pods(), None);
    assert_eq!(row.status, toned("ClusterIP", StatusTone::Ok));
    assert_eq!(endpoints_cell(&row), &KindCell::Absent);
}

#[test]
fn load_balancer_without_address_keeps_its_status() {
    let mut balancer = service(&["app=api"]);
    balancer.service_type = "LoadBalancer".to_owned();
    let row = joined(
        &balancer,
        api_pods(),
        Some(vec![slice(Some("api"), "IPv4", api_endpoints([true; 2]))]),
    );
    assert_eq!(row.status, toned("Address pending", StatusTone::Warn));
    assert_eq!(
        endpoints_cell(&row),
        &KindCell::Toned(toned("2", StatusTone::Ok))
    );
}

#[test]
fn rejoin_starts_from_the_builder_status() {
    let service = service(&["app=api"]);
    let mut rows = vec![service_row(&service)];
    let pods = ready_list(api_pods());
    let companion = CompanionLists::EndpointSlices(ready_list(vec![slice(
        Some("api"),
        "IPv4",
        api_endpoints([true; 2]),
    )]));
    let with_slices = JoinInputs {
        custom_counts: None,
        pods: &pods,
        companion: Some(&companion),
        kubelet: None,
        scope: &NamespaceScope::All,
    };
    join_rows(ResourceKind::Services, &mut rows, &with_slices);
    let without = JoinInputs {
        custom_counts: None,
        pods: &pods,
        companion: None,
        kubelet: None,
        scope: &NamespaceScope::All,
    };
    join_rows(ResourceKind::Services, &mut rows, &without);
    assert_eq!(rows[0].status, toned("ClusterIP", StatusTone::Ok));
    assert_eq!(endpoints_cell(&rows[0]), &KindCell::Absent);
}

#[test]
fn unloaded_pods_do_not_claim_no_match() {
    let mut rows = vec![service_row(&service(&["app=api"]))];
    let pods = LiveList::<PodSummary>::Loading;
    let inputs = JoinInputs {
        custom_counts: None,
        pods: &pods,
        companion: None,
        kubelet: None,
        scope: &NamespaceScope::All,
    };
    join_rows(ResourceKind::Services, &mut rows, &inputs);
    assert_eq!(rows[0].status, toned("ClusterIP", StatusTone::Ok));
}

#[test]
fn other_kinds_are_not_joined() {
    let mut rows = vec![service_row(&service(&["app=api"]))];
    let before = rows.clone();
    let pods = ready_list(Vec::new());
    let inputs = JoinInputs {
        custom_counts: None,
        pods: &pods,
        companion: None,
        kubelet: None,
        scope: &NamespaceScope::All,
    };
    join_rows(ResourceKind::Deployments, &mut rows, &inputs);
    assert_eq!(rows, before);
}

#[test]
fn service_health_takes_plain_slices() {
    let slices = [slice(Some("api"), "IPv4", api_endpoints([true, false]))];
    let health = service_health(&service(&["app=api"]), &api_pods(), Some(&slices));
    assert_eq!(
        health,
        ServiceHealth {
            matching_pods: Some(2),
            endpoints: Some(EndpointCounts { ready: 1, total: 2 }),
        }
    );
}

#[test]
fn service_health_without_slices_has_unknown_endpoints() {
    // As for a denied or unloaded list.
    let health = service_health(&service(&["app=api"]), &api_pods(), None);
    assert_eq!(health.endpoints, None);
}

#[test]
fn service_health_of_leaves_unloaded_pods_unknown() {
    let service = service(&["app=api"]);
    let slices = ready_list(vec![slice(Some("api"), "IPv4", api_endpoints([true; 2]))]);
    let health = service_health_of(&service, &LiveList::Loading, Some(&slices));
    assert_eq!(health.matching_pods, None);
    assert_eq!(
        health.endpoints,
        Some(EndpointCounts { ready: 2, total: 2 })
    );
}

#[test]
fn service_health_of_leaves_unloaded_slices_unknown() {
    let health = service_health_of(
        &service(&["app=api"]),
        &ready_list(api_pods()),
        Some(&LiveList::Loading),
    );
    assert_eq!(health.matching_pods, Some(2));
    assert_eq!(health.endpoints, None);
}

#[test]
fn unserved_means_endpoints_but_none_ready() {
    let health = |ready, total| ServiceHealth {
        matching_pods: Some(1),
        endpoints: Some(EndpointCounts { ready, total }),
    };
    assert!(health(0, 2).is_unserved());
    assert!(!health(1, 2).is_unserved());
    assert!(!health(0, 0).is_unserved());
    assert!(!ServiceHealth::default().is_unserved());
}

#[test]
fn matching_pods_use_the_selector_in_the_namespace() {
    let pods = vec![
        pod("team-a", "api-1", &["app=api", "tier=web"]),
        pod("team-a", "api-2", &["app=api"]),
        pod("team-b", "api-3", &["app=api", "tier=web"]),
    ];
    let names: Vec<&str> = matching_pods(&service(&["app=api", "tier=web"]), &pods)
        .iter()
        .map(|pod| pod.name.as_str())
        .collect();
    assert_eq!(names, ["api-1"]);
}

#[test]
fn selector_less_service_matches_no_pods() {
    assert!(matching_pods(&service(&[]), &api_pods()).is_empty());
}

#[test]
fn services_selecting_a_pod_use_its_labels_in_its_namespace() {
    let mut other_namespace = service(&["app=api"]);
    other_namespace.namespace = "team-b".to_owned();
    other_namespace.name = "api-b".to_owned();
    let mut headless_web = service(&["app=web"]);
    headless_web.name = "web".to_owned();
    let services = vec![
        service(&["app=api"]),
        headless_web,
        other_namespace,
        service(&[]),
    ];
    let selected: Vec<&str> =
        services_selecting(&pod("team-a", "api-1", &["app=api", "tier=web"]), &services)
            .iter()
            .map(|service| service.name.as_str())
            .collect();
    // The selector-less service and the other namespace's service never match.
    assert_eq!(selected, ["api"]);
}

// ---- ConfigMaps and Namespaces ----

fn container(kind: ContainerKind) -> ContainerSummary {
    ContainerSummary {
        terminal: cluster::ContainerTerminal::None,
        name: "main".to_owned(),
        image: "registry/app:1".to_owned(),
        kind,
        state: ContainerState::Running { started_at: None },
        is_ready: true,
        restart_count: 0,
        last_termination: None,
        image_digest: None,
        pull_policy: None,
        is_started: None,
        ports: Vec::new(),
        resources: Vec::new(),
        probes: ContainerProbes::default(),
        env: Vec::new(),
        env_from: Vec::new(),
        mounts: Vec::new(),
    }
}

fn owned_by(mut pod: PodSummary, kind: &str, name: &str) -> PodSummary {
    pod.controller = Some(ControllerRef {
        kind: kind.to_owned(),
        name: name.to_owned(),
    });
    pod
}

fn with_container(mut pod: PodSummary, container: ContainerSummary) -> PodSummary {
    pod.containers.push(container);
    pod
}

fn env_from_config_map(name: &str) -> ContainerSummary {
    let mut container = container(ContainerKind::Main);
    container.env_from = vec![EnvFromEntry {
        source: EnvFromSource::ConfigMap {
            name: name.to_owned(),
        },
        prefix: None,
    }];
    container
}

fn users_in_team_a(users: &ConfigMapUsers, config_map: &str) -> Vec<UsedBy> {
    users_of(users, "team-a", config_map).cloned().collect()
}

fn ways_of(used_by: &UsedBy) -> Vec<&'static str> {
    used_by.ways.iter().copied().collect()
}

#[test]
fn config_map_users_from_env_env_from_volume_projected() {
    let mut main = container(ContainerKind::Main);
    main.env = vec![
        EnvEntry {
            name: "A".to_owned(),
            source: EnvSource::ConfigMapKey {
                name: "settings".to_owned(),
                key: "a".to_owned(),
            },
        },
        // A Secret reference is not a config map.
        EnvEntry {
            name: "B".to_owned(),
            source: EnvSource::SecretKey {
                name: "settings".to_owned(),
                key: "b".to_owned(),
            },
        },
    ];
    main.env_from = env_from_config_map("settings").env_from;
    let mut init = container(ContainerKind::Init);
    init.mounts = vec![
        MountEntry {
            path: "/etc/a".to_owned(),
            volume: "a".to_owned(),
            source: VolumeSource::ConfigMap {
                name: "settings".to_owned(),
            },
            is_read_only: true,
            sub_path: None,
        },
        MountEntry {
            path: "/etc/b".to_owned(),
            volume: "b".to_owned(),
            source: VolumeSource::Projected {
                config_maps: vec!["kube-root-ca.crt".to_owned()],
                secrets: Vec::new(),
            },
            is_read_only: true,
            sub_path: None,
        },
    ];
    let pod = with_container(with_container(pod("team-a", "api-1", &[]), main), init);
    let users = config_map_users(&[pod]);
    let settings = users_in_team_a(&users, "settings");
    assert_eq!(settings.len(), 1);
    assert_eq!(ways_of(&settings[0]), ["env", "env from", "volume"]);
    let projected = users_in_team_a(&users, "kube-root-ca.crt");
    assert_eq!(ways_of(&projected[0]), ["volume"]);
}

#[test]
fn owner_mapping_deployment_cronjob_bare_pod() {
    let using = |name: &str| with_container(pod("team-a", name, &[]), env_from_config_map("c"));
    let bare = using("debug");
    let deployment = owned_by(using("api-1"), "ReplicaSet", "api-7d9f8c");
    let cron_job = owned_by(using("n-1"), "Job", "nightly-29012345");
    let stateful = owned_by(using("db-0"), "StatefulSet", "db");
    // A ReplicaSet without a hash suffix and a Job without a schedule suffix keep their own names.
    let standalone = owned_by(using("w-1"), "ReplicaSet", "standalone");
    let manual = owned_by(using("m-1"), "Job", "migrate-once");
    let users = config_map_users(&[bare, deployment, cron_job, stateful, standalone, manual]);
    let owners: Vec<String> = users_in_team_a(&users, "c")
        .into_iter()
        .map(|used_by| used_by.owner)
        .collect();
    assert_eq!(
        owners,
        [
            "cronjob/nightly",
            "deployment/api",
            "job/migrate-once",
            "pod/debug",
            "replicaset/standalone",
            "statefulset/db"
        ]
    );
}

#[test]
fn owner_links_open_the_workload() {
    let pod = owned_by(
        with_container(pod("team-a", "api-1", &[]), env_from_config_map("c")),
        "ReplicaSet",
        "api-7d9f8c",
    );
    let users = config_map_users(&[pod]);
    assert_eq!(
        users_in_team_a(&users, "c")[0].target,
        ResourceKey::of_object("Deployment", Some("team-a"), "api")
    );
}

#[test]
fn pods_of_one_owner_merge_into_one_user() {
    let replica = |name: &str| {
        owned_by(
            with_container(pod("team-a", name, &[]), env_from_config_map("c")),
            "ReplicaSet",
            "api-7d9f8c",
        )
    };
    let users = config_map_users(&[replica("api-1"), replica("api-2")]);
    assert_eq!(users_in_team_a(&users, "c").len(), 1);
}

fn config_map_row_in(namespace: &str, name: &str) -> KindRow {
    crate::config_map_rows::config_map_row(&cluster::ConfigMapSummary {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        keys: Vec::new(),
        is_immutable: false,
    })
}

fn join_config_maps_of(pods: &LiveList<PodSummary>, rows: &mut [KindRow]) {
    let inputs = JoinInputs {
        custom_counts: None,
        pods,
        companion: None,
        kubelet: None,
        scope: &NamespaceScope::All,
    };
    join_rows(ResourceKind::ConfigMaps, rows, &inputs);
}

#[test]
fn used_by_cell_shows_first_and_more() {
    let user = |name: &str, owner: &str| {
        owned_by(
            with_container(pod("team-a", name, &[]), env_from_config_map("settings")),
            "StatefulSet",
            owner,
        )
    };
    let pods = ready_list(vec![user("a-0", "a"), user("b-0", "b"), user("c-0", "c")]);
    let mut rows = vec![
        config_map_row_in("team-a", "settings"),
        config_map_row_in("team-a", "unused"),
    ];
    join_config_maps_of(&pods, &mut rows);
    assert_eq!(
        rows[0].cells.get(CONFIG_MAP_USED_BY),
        Some(&KindCell::MonoWithMore {
            text: "statefulset/a".into(),
            more: 2
        })
    );
    assert_eq!(
        rows[1].cells.get(CONFIG_MAP_USED_BY),
        Some(&none_found_cell())
    );
}

#[test]
fn used_by_cell_is_absent_until_the_pods_load() {
    let mut rows = vec![config_map_row_in("team-a", "settings")];
    join_config_maps_of(&LiveList::Loading, &mut rows);
    assert_eq!(
        rows[0].cells.get(CONFIG_MAP_USED_BY),
        Some(&KindCell::Absent)
    );
}

fn requesting(mut pod: PodSummary, kind: ContainerKind, cpu: &str, memory: &str) -> PodSummary {
    let mut container = container(kind);
    container.resources = ["cpu", "memory"]
        .into_iter()
        .zip([cpu, memory])
        .map(|(name, request)| ContainerResource {
            name: name.to_owned(),
            request: Some(request.to_owned()),
            limit: None,
        })
        .collect();
    pod.containers.push(container);
    pod
}

#[test]
fn namespace_load_sums_requests_of_active_pods() {
    let main = requesting(
        pod("team-a", "a", &[]),
        ContainerKind::Main,
        "250m",
        "128Mi",
    );
    let sidecar = requesting(
        pod("team-a", "b", &[]),
        ContainerKind::Sidecar,
        "250m",
        "128Mi",
    );
    // Init containers do not run alongside the others.
    let init = requesting(pod("team-a", "c", &[]), ContainerKind::Init, "2", "1Gi");
    let mut finished = requesting(pod("team-a", "d", &[]), ContainerKind::Main, "1", "1Gi");
    finished.status = PodStatus::Reason(StatusReason::Completed);
    let pods = [main, sidecar, init, finished];
    let refs: Vec<&PodSummary> = pods.iter().collect();
    let load = namespace_load(&refs);
    // A finished pod still counts as a pod but holds no requests.
    assert_eq!(load.pods, 4);
    assert_eq!(load.cpu.nanocores(), 500_000_000);
    assert_eq!(load.memory.bytes(), 256 * 1024 * 1024);
}

fn namespace_row_named(name: &str) -> KindRow {
    crate::namespace_rows::namespace_row(&NamespaceSummary {
        name: name.to_owned(),
        phase: NamespacePhase::Active,
        labels: Vec::new(),
        created_at: None,
        deleting_since: None,
        deletion_conditions: Vec::new(),
    })
}

fn join_namespaces_of(scope: &NamespaceScope, pods: &LiveList<PodSummary>, rows: &mut [KindRow]) {
    let inputs = JoinInputs {
        custom_counts: None,
        pods,
        companion: None,
        kubelet: None,
        scope,
    };
    join_rows(ResourceKind::Namespaces, rows, &inputs);
}

fn load_cells_of(row: &KindRow) -> [Option<&KindCell>; 3] {
    [NAMESPACE_PODS, NAMESPACE_CPU, NAMESPACE_MEMORY].map(|index| row.cells.get(index))
}

#[test]
fn namespace_cells_show_pods_and_requests() {
    let pods = ready_list(vec![requesting(
        pod("team-a", "a", &[]),
        ContainerKind::Main,
        "250m",
        "128Mi",
    )]);
    let mut rows = vec![namespace_row_named("team-a")];
    join_namespaces_of(&NamespaceScope::All, &pods, &mut rows);
    assert_eq!(
        load_cells_of(&rows[0]),
        [
            Some(&KindCell::count(1)),
            Some(&KindCell::Quantity {
                text: "250m".into(),
                value: 250_000_000,
                tone: None
            }),
            Some(&KindCell::Quantity {
                text: "128Mi".into(),
                value: 128 * 1024 * 1024,
                tone: None
            }),
        ]
    );
}

#[test]
fn namespace_without_pods_reads_zero_not_unknown() {
    let mut rows = vec![namespace_row_named("empty")];
    join_namespaces_of(&NamespaceScope::All, &ready_list(Vec::new()), &mut rows);
    assert_eq!(load_cells_of(&rows[0])[0], Some(&KindCell::count(0)));
}

#[test]
fn namespace_outside_scope_is_absent() {
    let pods = ready_list(vec![pod("team-a", "a", &[])]);
    let scope = NamespaceScope::Named("team-a".to_owned());
    let mut rows = vec![namespace_row_named("team-a"), namespace_row_named("other")];
    join_namespaces_of(&scope, &pods, &mut rows);
    assert_eq!(load_cells_of(&rows[0])[0], Some(&KindCell::count(1)));
    assert_eq!(load_cells_of(&rows[1]), [Some(&KindCell::Absent); 3]);
}

#[test]
fn namespace_cells_are_absent_until_the_pods_load() {
    let mut rows = vec![namespace_row_named("team-a")];
    join_namespaces_of(&NamespaceScope::All, &LiveList::Loading, &mut rows);
    assert_eq!(load_cells_of(&rows[0]), [Some(&KindCell::Absent); 3]);
}

#[test]
fn namespace_column_indices_name_their_columns() {
    let columns = ResourceKind::Namespaces.columns();
    let names = [NAMESPACE_PODS, NAMESPACE_CPU, NAMESPACE_MEMORY]
        .map(|index| columns.get(index).map(|column| column.name));
    assert_eq!(names, [Some("Pods"), Some("CPU req"), Some("Memory req")]);
}

#[test]
fn config_map_column_index_names_used_by() {
    let columns = ResourceKind::ConfigMaps.columns();
    assert_eq!(
        columns.get(CONFIG_MAP_USED_BY).map(|column| column.name),
        Some("Used by")
    );
}

#[test]
fn cron_job_suffix_needs_eight_digits() {
    assert_eq!(cron_job_of_job("nightly-29012345"), Some("nightly"));
    assert_eq!(cron_job_of_job("nightly-123"), None);
    assert_eq!(cron_job_of_job("nightly-2901234a"), None);
    assert_eq!(cron_job_of_job("-29012345"), None);
}

// ---- NetworkPolicies ----

fn network_policy(selector: &[&str]) -> cluster::NetworkPolicySummary {
    let terms: Vec<String> = selector.iter().map(|term| (*term).to_owned()).collect();
    cluster::NetworkPolicySummary {
        namespace: "team-a".to_owned(),
        name: "web".to_owned(),
        created_at: None,
        labels: Vec::new(),
        pod_selector: Selector::of_labels(&terms).unwrap_or_else(Selector::everything),
        ingress: cluster::PolicyDirection::Allowed(Vec::new()),
        egress: cluster::PolicyDirection::NotIsolated,
    }
}

fn joined_policy(policy: &cluster::NetworkPolicySummary, pods: &LiveList<PodSummary>) -> KindRow {
    let mut rows = vec![crate::network_policy_rows::network_policy_row(policy)];
    let inputs = JoinInputs {
        custom_counts: None,
        pods,
        companion: None,
        kubelet: None,
        scope: &NamespaceScope::All,
    };
    join_rows(ResourceKind::NetworkPolicies, &mut rows, &inputs);
    rows.remove(0)
}

fn affects_cell(row: &KindRow) -> &KindCell {
    row.cells
        .get(NETWORK_POLICY_AFFECTS)
        .expect("Affects cell exists")
}

#[test]
fn network_policy_affects_matching_pods_of_its_namespace() {
    let pods = ready_list(vec![
        pod("team-a", "web-1", &["app=web"]),
        pod("team-a", "web-2", &["app=web"]),
        pod("team-a", "db-1", &["app=db"]),
        pod("team-b", "web-3", &["app=web"]),
    ]);
    let row = joined_policy(&network_policy(&["app=web"]), &pods);
    assert_eq!(row.status, toned("2 pods", StatusTone::Ok));
    assert_eq!(
        affects_cell(&row),
        &KindCell::Quantity {
            text: "2 pods".into(),
            value: 2,
            tone: None,
        }
    );
    let one = joined_policy(&network_policy(&["app=db"]), &pods);
    assert_eq!(one.status, toned("1 pod", StatusTone::Ok));
}

#[test]
fn network_policy_with_empty_selector_affects_all_pods_of_its_namespace() {
    let pods = ready_list(vec![
        pod("team-a", "web-1", &["app=web"]),
        pod("team-a", "db-1", &["app=db"]),
        pod("team-b", "web-3", &["app=web"]),
    ]);
    let all = joined_policy(&network_policy(&[]), &pods);
    assert_eq!(all.status, toned("2 pods", StatusTone::Ok));
}

#[test]
fn network_policy_selecting_no_pods_warns() {
    let pods = ready_list(vec![pod("team-a", "db-1", &["app=db"])]);
    let row = joined_policy(&network_policy(&["app=web"]), &pods);
    assert_eq!(row.status, toned("Selects no pods", StatusTone::Warn));
    assert_eq!(
        affects_cell(&row),
        &KindCell::Quantity {
            text: "0 pods".into(),
            value: 0,
            tone: Some(StatusTone::Warn),
        }
    );
}

#[test]
fn network_policy_without_pods_keeps_builder_status() {
    let row = joined_policy(&network_policy(&["app=web"]), &LiveList::Loading);
    assert_eq!(row.status, toned("Ingress", StatusTone::Ok));
    assert_eq!(affects_cell(&row), &KindCell::Absent);
}

// ---- PersistentVolumeClaims ----

fn claim(phase: &str) -> cluster::PersistentVolumeClaimSummary {
    cluster::PersistentVolumeClaimSummary {
        namespace: "shop".to_owned(),
        name: "data".to_owned(),
        created_at: None,
        labels: Vec::new(),
        phase: phase.to_owned(),
        is_terminating: false,
        volume: Some("pv-1".to_owned()),
        capacity: Some("100".to_owned()),
        requested: None,
        access_modes: Vec::new(),
        storage_class: None,
        volume_mode: None,
        conditions: Vec::new(),
        class_allows_expansion: None,
    }
}

fn history_with_usage(used: Option<u64>, capacity: Option<u64>) -> KubeletHistory {
    let usage = PvcUsage {
        namespace: "shop".to_owned(),
        claim: "data".to_owned(),
        sampled_at: None,
        used: used.map(ByteAmount::from_bytes),
        capacity: capacity.map(ByteAmount::from_bytes),
        available: None,
        inodes_used: None,
        inodes: None,
    };
    let round = vec![cluster::NodeKubeletStats {
        node: "node-a".to_owned(),
        summary: Ok(cluster::KubeletSummary {
            network: None,
            pods: vec![cluster::PodKubeletStats {
                namespace: "shop".to_owned(),
                name: "web-1".to_owned(),
                uid: "web-1".to_owned(),
                network: None,
                volumes: vec![usage],
            }],
        }),
        disk_io: None,
    }];
    let mut history = KubeletHistory::default();
    history.record(
        jiff::Timestamp::from_second(15).expect("valid timestamp"),
        &round,
        &[],
        &NamespaceScope::All,
    );
    history
}

fn joined_claim(phase: &str, history: Option<&KubeletHistory>) -> KindRow {
    let mut rows = vec![crate::storage_rows::persistent_volume_claim_row(&claim(
        phase,
    ))];
    let pods = LiveList::Loading;
    let inputs = JoinInputs {
        custom_counts: None,
        pods: &pods,
        companion: None,
        kubelet: history,
        scope: &NamespaceScope::All,
    };
    join_rows(ResourceKind::PersistentVolumeClaims, &mut rows, &inputs);
    rows.remove(0)
}

fn joined_claim_with_classes(
    claim: cluster::PersistentVolumeClaimSummary,
    classes: LiveList<cluster::StorageClassSummary>,
) -> cluster::PersistentVolumeClaimSummary {
    let mut rows = vec![crate::storage_rows::persistent_volume_claim_row(&claim)];
    let pods = LiveList::Loading;
    let companion = CompanionLists::StorageClasses(classes);
    let inputs = JoinInputs {
        custom_counts: None,
        pods: &pods,
        companion: Some(&companion),
        kubelet: None,
        scope: &NamespaceScope::All,
    };
    join_rows(ResourceKind::PersistentVolumeClaims, &mut rows, &inputs);
    match rows.remove(0).object {
        KindObject::PersistentVolumeClaim(claim) => claim,
        other => panic!("a claim, not {other:?}"),
    }
}

#[test]
fn a_claim_learns_from_the_classes_whether_it_expands() {
    let classed = |name: &str| {
        let mut claim = claim("Bound");
        claim.storage_class = Some(name.to_owned());
        claim
    };
    let ready = |classes: Vec<cluster::StorageClassSummary>| LiveList::Ready {
        items: classes,
        interruption: None,
    };
    let classes = || {
        ready(vec![
            crate::resource_edits::resource_edits_tests::class("standard", true, false),
            crate::resource_edits::resource_edits_tests::class("gp3", false, true),
        ])
    };
    assert_eq!(
        joined_claim_with_classes(classed("standard"), classes()).class_allows_expansion,
        Some(false)
    );
    assert_eq!(
        joined_claim_with_classes(classed("gp3"), classes()).class_allows_expansion,
        Some(true)
    );
    // A class the list lacks, a claim with no class, and a list that has not loaded say nothing.
    assert_eq!(
        joined_claim_with_classes(classed("gone"), classes()).class_allows_expansion,
        None
    );
    assert_eq!(
        joined_claim_with_classes(claim("Bound"), classes()).class_allows_expansion,
        None
    );
    assert_eq!(
        joined_claim_with_classes(classed("standard"), LiveList::Loading).class_allows_expansion,
        None
    );
}

fn used_cell(row: &KindRow) -> &KindCell {
    row.cells.get(CLAIM_USED).expect("Used cell exists")
}

#[test]
fn claim_used_percent_and_tone() {
    let warn = joined_claim("Bound", Some(&history_with_usage(Some(83), Some(100))));
    assert_eq!(
        used_cell(&warn),
        &KindCell::Quantity {
            text: "83%".into(),
            value: 830,
            tone: Some(StatusTone::Warn),
        }
    );
}

#[test]
fn claim_used_is_bad_from_ninety_percent() {
    let bad = joined_claim("Bound", Some(&history_with_usage(Some(95), Some(100))));
    assert_eq!(
        used_cell(&bad),
        &KindCell::Quantity {
            text: "95%".into(),
            value: 950,
            tone: Some(StatusTone::Bad),
        }
    );
}

#[test]
fn claim_used_below_warn_has_no_tone() {
    let low = joined_claim("Bound", Some(&history_with_usage(Some(10), Some(100))));
    assert_eq!(
        used_cell(&low),
        &KindCell::Quantity {
            text: "10%".into(),
            value: 100,
            tone: None,
        }
    );
    assert_eq!(low.status, toned("Bound", StatusTone::Ok));
}

#[test]
fn shared_filesystem_claim_is_absent() {
    // The claim asks for 100 bytes, but the kubelet reports a 1000-byte filesystem: the
    // node's disk, which would read as a full claim.
    let history = history_with_usage(Some(950), Some(1000));
    let row = joined_claim("Bound", Some(&history));
    assert_eq!(used_cell(&row), &KindCell::Absent);
    assert_eq!(row.status, toned("Bound", StatusTone::Ok));
}

#[test]
fn shared_filesystem_needs_a_larger_reported_capacity() {
    let usage = |capacity| {
        history_with_usage(Some(1), Some(capacity))
            .pvc_usage("shop", "data")
            .cloned()
            .expect("sample")
    };
    assert!(is_shared_filesystem(&usage(1000), Some("100")));
    assert!(!is_shared_filesystem(&usage(100), Some("100")));
    assert!(!is_shared_filesystem(&usage(50), Some("100")));
    // Without a readable claim capacity there is nothing to compare.
    assert!(!is_shared_filesystem(&usage(1000), None));
    assert!(!is_shared_filesystem(&usage(1000), Some("lots")));
}

#[test]
fn claim_without_stats_is_absent() {
    let without_feed = joined_claim("Bound", None);
    assert_eq!(used_cell(&without_feed), &KindCell::Absent);
    let empty = KubeletHistory::default();
    assert_eq!(
        used_cell(&joined_claim("Bound", Some(&empty))),
        &KindCell::Absent
    );
    let no_used = history_with_usage(None, Some(100));
    assert_eq!(
        used_cell(&joined_claim("Bound", Some(&no_used))),
        &KindCell::Absent
    );
}

#[test]
fn zero_capacity_is_absent() {
    let history = history_with_usage(Some(0), Some(0));
    assert_eq!(
        used_cell(&joined_claim("Bound", Some(&history))),
        &KindCell::Absent
    );
}

#[test]
fn full_claim_raises_status() {
    let history = history_with_usage(Some(95), Some(100));
    let row = joined_claim("Bound", Some(&history));
    assert_eq!(row.status, toned("95% used", StatusTone::Bad));
    // A claim that is not Bound keeps its phase.
    let pending = joined_claim("Pending", Some(&history));
    assert_eq!(pending.status, toned("Pending", StatusTone::Warn));
}

#[test]
fn rejoin_without_stats_restores_the_builder_status() {
    let full = history_with_usage(Some(95), Some(100));
    let mut rows = vec![crate::storage_rows::persistent_volume_claim_row(&claim(
        "Bound",
    ))];
    let pods = LiveList::Loading;
    for history in [Some(&full), None] {
        let inputs = JoinInputs {
            custom_counts: None,
            pods: &pods,
            companion: None,
            kubelet: history,
            scope: &NamespaceScope::All,
        };
        join_rows(ResourceKind::PersistentVolumeClaims, &mut rows, &inputs);
    }
    assert_eq!(rows[0].status, toned("Bound", StatusTone::Ok));
    assert_eq!(used_cell(&rows[0]), &KindCell::Absent);
}

// ---- StorageClasses ----

fn class(name: &str) -> cluster::StorageClassSummary {
    cluster::StorageClassSummary {
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        provisioner: "ebs.csi.aws.com".to_owned(),
        reclaim_policy: "Delete".to_owned(),
        binding_mode: "Immediate".to_owned(),
        allows_expansion: false,
        is_default: false,
        parameters: Vec::new(),
        mount_options: Vec::new(),
    }
}

fn volume_of(name: &str, class: Option<&str>) -> cluster::PersistentVolumeSummary {
    cluster::PersistentVolumeSummary {
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        capacity: None,
        access_modes: Vec::new(),
        reclaim_policy: "Delete".to_owned(),
        phase: "Bound".to_owned(),
        is_terminating: false,
        claim: None,
        storage_class: class.map(str::to_owned),
        volume_mode: None,
        backend: cluster::VolumeBackend::Other { kind: "unknown" },
        node_affinity: Vec::new(),
        mount_options: Vec::new(),
        reason: None,
        message: None,
    }
}

fn joined_classes(names: &[&str], companion: Option<&CompanionLists>) -> Vec<KindRow> {
    let mut rows: Vec<KindRow> = names
        .iter()
        .map(|name| crate::storage_rows::storage_class_row(&class(name)))
        .collect();
    let pods = LiveList::Loading;
    let inputs = JoinInputs {
        custom_counts: None,
        pods: &pods,
        companion,
        kubelet: None,
        scope: &NamespaceScope::All,
    };
    join_rows(ResourceKind::StorageClasses, &mut rows, &inputs);
    rows
}

#[test]
fn class_volumes_count_by_class() {
    let companion = CompanionLists::PersistentVolumes(ready_list(vec![
        volume_of("pv-1", Some("gp3")),
        volume_of("pv-2", Some("gp3")),
        volume_of("pv-3", Some("fast")),
        // A volume without a class (static provisioning) belongs to no class row.
        volume_of("pv-4", None),
    ]));
    let rows = joined_classes(&["gp3", "fast", "empty"], Some(&companion));
    let count = |row: &KindRow| row.cells[CLASS_VOLUMES].clone();
    let quantity = |count: u64| KindCell::Quantity {
        text: count.to_string().into(),
        value: count,
        tone: None,
    };
    assert_eq!(count(&rows[0]), quantity(2));
    assert_eq!(count(&rows[1]), quantity(1));
    assert_eq!(count(&rows[2]), quantity(0));
}

#[test]
fn class_volumes_absent_without_companion() {
    let rows = joined_classes(&["gp3"], None);
    assert_eq!(rows[0].cells[CLASS_VOLUMES], KindCell::Absent);
    // A companion that has not loaded yet reads the same.
    let loading = CompanionLists::PersistentVolumes(LiveList::Loading);
    let rows = joined_classes(&["gp3"], Some(&loading));
    assert_eq!(rows[0].cells[CLASS_VOLUMES], KindCell::Absent);
}

#[test]
fn class_join_ignores_the_endpoint_slice_companion() {
    let slices = CompanionLists::EndpointSlices(ready_list(Vec::new()));
    let rows = joined_classes(&["gp3"], Some(&slices));
    assert_eq!(rows[0].cells[CLASS_VOLUMES], KindCell::Absent);
}

// ---- Roles and ClusterRoles ----

fn role_of(
    namespace: Option<&str>,
    name: &str,
    rules: Vec<cluster::RbacRule>,
) -> cluster::RoleSummary {
    cluster::RoleSummary {
        namespace: namespace.map(str::to_owned),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        rules,
        aggregation: Vec::new(),
    }
}

fn wildcard_rule() -> cluster::RbacRule {
    let star = || vec!["*".to_owned()];
    cluster::RbacRule {
        api_groups: star(),
        resources: star(),
        resource_names: Vec::new(),
        verbs: star(),
        non_resource_urls: Vec::new(),
    }
}

fn binding_of(
    namespace: Option<&str>,
    name: &str,
    role: (cluster::RoleKind, &str),
) -> cluster::BindingSummary {
    cluster::BindingSummary {
        namespace: namespace.map(str::to_owned),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        role: cluster::RoleRef {
            kind: role.0,
            name: role.1.to_owned(),
        },
        subjects: Vec::new(),
    }
}

fn bindings_companion(
    role_bindings: LiveList<cluster::BindingSummary>,
    cluster_role_bindings: Option<LiveList<cluster::BindingSummary>>,
) -> CompanionLists {
    CompanionLists::Bindings {
        role_bindings,
        cluster_role_bindings,
    }
}

fn joined_roles(
    kind: ResourceKind,
    roles: &[cluster::RoleSummary],
    companion: Option<&CompanionLists>,
) -> Vec<KindRow> {
    let mut rows: Vec<KindRow> = roles
        .iter()
        .map(|role| match kind {
            ResourceKind::Roles => crate::access_rows::role_row(role),
            _ => crate::access_rows::cluster_role_row(role),
        })
        .collect();
    let pods = LiveList::Loading;
    let inputs = JoinInputs {
        custom_counts: None,
        pods: &pods,
        companion,
        kubelet: None,
        scope: &NamespaceScope::All,
    };
    join_rows(kind, &mut rows, &inputs);
    rows
}

fn count_cell(count: u64, tone: Option<StatusTone>) -> KindCell {
    KindCell::Quantity {
        text: count.to_string().into(),
        value: count,
        tone,
    }
}

#[test]
fn binding_counts_per_role() {
    let companion = bindings_companion(
        ready_list(vec![
            binding_of(Some("shop"), "a", (cluster::RoleKind::Role, "reader")),
            binding_of(Some("shop"), "b", (cluster::RoleKind::Role, "reader")),
            binding_of(Some("other"), "c", (cluster::RoleKind::Role, "reader")),
            binding_of(Some("shop"), "d", (cluster::RoleKind::ClusterRole, "view")),
        ]),
        None,
    );
    let roles = [
        role_of(Some("shop"), "reader", Vec::new()),
        role_of(Some("shop"), "lonely", Vec::new()),
    ];
    let rows = joined_roles(ResourceKind::Roles, &roles, Some(&companion));
    assert_eq!(rows[0].cells[ROLE_BINDINGS], count_cell(2, None));
    assert_eq!(rows[1].cells[ROLE_BINDINGS], count_cell(0, None));

    let companion = bindings_companion(
        ready_list(vec![binding_of(
            Some("shop"),
            "d",
            (cluster::RoleKind::ClusterRole, "view"),
        )]),
        Some(ready_list(vec![binding_of(
            None,
            "e",
            (cluster::RoleKind::ClusterRole, "view"),
        )])),
    );
    let rows = joined_roles(
        ResourceKind::ClusterRoles,
        &[role_of(None, "view", Vec::new())],
        Some(&companion),
    );
    assert_eq!(rows[0].cells[CLUSTER_ROLE_BINDINGS], count_cell(2, None));
}

#[test]
fn wildcard_role_bindings_warn() {
    let companion = bindings_companion(
        ready_list(Vec::new()),
        Some(ready_list(vec![binding_of(
            None,
            "root",
            (cluster::RoleKind::ClusterRole, "super"),
        )])),
    );
    let roles = [
        role_of(None, "super", vec![wildcard_rule()]),
        role_of(None, "unbound-super", vec![wildcard_rule()]),
    ];
    let rows = joined_roles(ResourceKind::ClusterRoles, &roles, Some(&companion));
    assert_eq!(
        rows[0].cells[CLUSTER_ROLE_BINDINGS],
        count_cell(1, Some(StatusTone::Warn))
    );
    // Nothing to warn about when no binding uses the role.
    assert_eq!(rows[1].cells[CLUSTER_ROLE_BINDINGS], count_cell(0, None));
}

#[test]
fn bindings_absent_until_companion_ready() {
    let roles = [role_of(None, "view", Vec::new())];
    let rows = joined_roles(ResourceKind::ClusterRoles, &roles, None);
    assert_eq!(rows[0].cells[CLUSTER_ROLE_BINDINGS], KindCell::Absent);
    let loading = bindings_companion(ready_list(Vec::new()), Some(LiveList::Loading));
    let rows = joined_roles(ResourceKind::ClusterRoles, &roles, Some(&loading));
    assert_eq!(rows[0].cells[CLUSTER_ROLE_BINDINGS], KindCell::Absent);
    let unrelated = CompanionLists::EndpointSlices(ready_list(Vec::new()));
    let rows = joined_roles(ResourceKind::ClusterRoles, &roles, Some(&unrelated));
    assert_eq!(rows[0].cells[CLUSTER_ROLE_BINDINGS], KindCell::Absent);
}

#[test]
fn roles_join_without_the_cluster_role_bindings_list() {
    // Roles run no cluster-wide list, and their count does not need one.
    let companion = bindings_companion(ready_list(Vec::new()), None);
    let rows = joined_roles(
        ResourceKind::Roles,
        &[role_of(Some("shop"), "reader", Vec::new())],
        Some(&companion),
    );
    assert_eq!(rows[0].cells[ROLE_BINDINGS], count_cell(0, None));
}

// ---- ServiceAccounts ----

fn account_of(namespace: &str, name: &str) -> cluster::ServiceAccountSummary {
    cluster::ServiceAccountSummary {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        secrets: Vec::new(),
        image_pull_secrets: Vec::new(),
        automount_token: None,
        cloud_identities: Vec::new(),
    }
}

fn running_as(namespace: &str, name: &str, account: Option<&str>) -> PodSummary {
    let mut pod = pod(namespace, name, &[]);
    pod.service_account = account.map(str::to_owned);
    pod
}

fn account_subject(namespace: &str, name: &str) -> cluster::Subject {
    cluster::Subject {
        kind: cluster::SubjectKind::ServiceAccount,
        name: name.to_owned(),
        namespace: Some(namespace.to_owned()),
    }
}

fn group_subject(name: &str) -> cluster::Subject {
    cluster::Subject {
        kind: cluster::SubjectKind::Group,
        name: name.to_owned(),
        namespace: None,
    }
}

fn binding_for(
    namespace: Option<&str>,
    name: &str,
    role: (cluster::RoleKind, &str),
    subjects: Vec<cluster::Subject>,
) -> cluster::BindingSummary {
    cluster::BindingSummary {
        subjects,
        ..binding_of(namespace, name, role)
    }
}

/// The ServiceAccounts rows after a join; `None` leaves that list unloaded.
fn joined_accounts(
    accounts: &[cluster::ServiceAccountSummary],
    pods: Option<Vec<PodSummary>>,
    bindings: Option<(Vec<cluster::BindingSummary>, Vec<cluster::BindingSummary>)>,
) -> Vec<KindRow> {
    let mut rows: Vec<KindRow> = accounts
        .iter()
        .map(crate::access_rows::service_account_row)
        .collect();
    let pods = pods.map_or(LiveList::Loading, ready_list);
    let companion = bindings.map(|(role_bindings, cluster_role_bindings)| {
        bindings_companion(
            ready_list(role_bindings),
            Some(ready_list(cluster_role_bindings)),
        )
    });
    let inputs = JoinInputs {
        custom_counts: None,
        pods: &pods,
        companion: companion.as_ref(),
        kubelet: None,
        scope: &NamespaceScope::All,
    };
    join_rows(ResourceKind::ServiceAccounts, &mut rows, &inputs);
    rows
}

fn text_cell(text: &str) -> KindCell {
    KindCell::Text(text.to_owned().into())
}

fn tone_cell(text: &str, tone: StatusTone) -> KindCell {
    KindCell::Toned(StatusLabel {
        text: text.to_owned().into(),
        tone,
    })
}

#[test]
fn service_account_joined_column_indices() {
    let columns = ResourceKind::ServiceAccounts.columns();
    assert_eq!(columns[ACCOUNT_BOUND_ROLES].name, "Bound roles");
    assert_eq!(columns[ACCOUNT_USED_BY].name, "Used by");
}

#[test]
fn service_account_bound_roles_cell() {
    let bindings = (
        vec![binding_for(
            Some("shop"),
            "read",
            (cluster::RoleKind::Role, "reader"),
            vec![account_subject("shop", "api")],
        )],
        vec![binding_for(
            None,
            "view-all",
            (cluster::RoleKind::ClusterRole, "view"),
            vec![account_subject("shop", "api")],
        )],
    );
    let rows = joined_accounts(
        &[account_of("shop", "api"), account_of("shop", "idle")],
        None,
        Some(bindings),
    );
    assert_eq!(
        rows[0].cells[ACCOUNT_BOUND_ROLES],
        text_cell("clusterrole/view, role/reader")
    );
    // An account nothing binds shows a muted dash, not a missing value.
    assert_eq!(
        rows[1].cells[ACCOUNT_BOUND_ROLES],
        tone_cell("—", StatusTone::Done)
    );
    // Without the bindings the cell is absent and the status stays the builder one.
    let rows = joined_accounts(&[account_of("shop", "api")], None, None);
    assert_eq!(rows[0].cells[ACCOUNT_BOUND_ROLES], KindCell::Absent);
    assert_eq!(rows[0].status.text.as_ref(), "Service account");
}

#[test]
fn service_account_cluster_admin_warns() {
    let bindings = (
        Vec::new(),
        vec![binding_for(
            None,
            "root",
            (cluster::RoleKind::ClusterRole, "cluster-admin"),
            vec![account_subject("shop", "api")],
        )],
    );
    let rows = joined_accounts(&[account_of("shop", "api")], None, Some(bindings));
    assert_eq!(
        rows[0].cells[ACCOUNT_BOUND_ROLES],
        tone_cell("clusterrole/cluster-admin", StatusTone::Warn)
    );
    assert_eq!(rows[0].status.text.as_ref(), "Cluster admin");
    assert_eq!(rows[0].status.tone, StatusTone::Warn);
}

#[test]
fn cluster_admin_through_service_accounts_group_warns() {
    let bindings = (
        Vec::new(),
        vec![binding_for(
            None,
            "everyone",
            (cluster::RoleKind::ClusterRole, "cluster-admin"),
            vec![group_subject("system:serviceaccounts:shop")],
        )],
    );
    let rows = joined_accounts(
        &[account_of("shop", "api"), account_of("other", "api")],
        None,
        Some(bindings),
    );
    assert_eq!(rows[0].status.text.as_ref(), "Cluster admin");
    // The group names one namespace.
    assert_eq!(rows[1].status.text.as_ref(), "Service account");
    assert_eq!(
        rows[1].cells[ACCOUNT_BOUND_ROLES],
        tone_cell("—", StatusTone::Done)
    );
}

#[test]
fn group_wide_roles_follow_the_accounts_own_and_fold_into_a_count() {
    let group = |name: &str, role: &str, subject: &str| {
        binding_for(
            None,
            name,
            (cluster::RoleKind::ClusterRole, role),
            vec![group_subject(subject)],
        )
    };
    let bindings = (
        Vec::new(),
        vec![
            group(
                "discovery",
                "system:service-account-issuer-discovery",
                "system:serviceaccounts",
            ),
            group(
                "also",
                "system:public-info-viewer",
                "system:serviceaccounts:shop",
            ),
            binding_for(
                None,
                "own",
                (cluster::RoleKind::ClusterRole, "view"),
                vec![account_subject("shop", "api")],
            ),
        ],
    );
    let rows = joined_accounts(
        &[account_of("shop", "api"), account_of("shop", "idle")],
        None,
        Some(bindings),
    );
    assert_eq!(
        rows[0].cells[ACCOUNT_BOUND_ROLES],
        text_cell("clusterrole/view, +2 via group")
    );
    // An account with only group grants shows just the count.
    assert_eq!(
        rows[1].cells[ACCOUNT_BOUND_ROLES],
        text_cell("+2 via group")
    );
}
#[test]
fn service_account_used_by_counts_pods() {
    let pods = vec![
        running_as("shop", "a-1", Some("api")),
        running_as("shop", "a-2", Some("api")),
        running_as("shop", "w-1", Some("worker")),
        running_as("other", "a-3", Some("api")),
    ];
    let rows = joined_accounts(
        &[
            account_of("shop", "api"),
            account_of("shop", "worker"),
            account_of("shop", "idle"),
        ],
        Some(pods),
        None,
    );
    let used = |count: u64, text: &str| KindCell::Quantity {
        text: text.to_owned().into(),
        value: count,
        tone: None,
    };
    assert_eq!(rows[0].cells[ACCOUNT_USED_BY], used(2, "2 pods"));
    assert_eq!(rows[1].cells[ACCOUNT_USED_BY], used(1, "1 pod"));
    assert_eq!(rows[2].cells[ACCOUNT_USED_BY], used(0, "0 pods"));
    assert_eq!(
        (rows[0].status.text.as_ref(), rows[0].status.tone),
        ("2 pods", StatusTone::Ok)
    );
    assert_eq!(
        (rows[2].status.text.as_ref(), rows[2].status.tone),
        ("No pods", StatusTone::Done)
    );
    // Without the pods the cell is absent.
    let rows = joined_accounts(&[account_of("shop", "api")], None, None);
    assert_eq!(rows[0].cells[ACCOUNT_USED_BY], KindCell::Absent);
}

#[test]
fn pod_without_service_account_counts_for_default() {
    let pods = vec![
        running_as("shop", "bare", None),
        running_as("shop", "named", Some("default")),
        running_as("shop", "other", Some("api")),
    ];
    let rows = joined_accounts(&[account_of("shop", "default")], Some(pods), None);
    assert_eq!(rows[0].status.text.as_ref(), "2 pods");
}

#[test]
fn cluster_admin_to_authenticated_marks_every_account() {
    let bindings = (
        Vec::new(),
        vec![
            // The stock grants to everyone are not listed on each account.
            binding_for(
                None,
                "basic-user",
                (cluster::RoleKind::ClusterRole, "system:basic-user"),
                vec![group_subject("system:authenticated")],
            ),
            binding_for(
                None,
                "open-door",
                (cluster::RoleKind::ClusterRole, "cluster-admin"),
                vec![group_subject("system:authenticated")],
            ),
        ],
    );
    let rows = joined_accounts(
        &[account_of("shop", "api"), account_of("other", "web")],
        None,
        Some(bindings),
    );
    for row in &rows {
        assert_eq!(row.status.text.as_ref(), "Cluster admin");
        assert_eq!(
            row.cells[ACCOUNT_BOUND_ROLES],
            tone_cell("clusterrole/cluster-admin", StatusTone::Warn)
        );
    }
}

// ---- Secrets ----

const NO_PODS: &[PodSummary] = &[];
const NO_INGRESSES: &[IngressSummary] = &[];

fn secret_of(secret_type: &str, name: &str) -> cluster::SecretSummary {
    cluster::SecretSummary {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        secret_type: secret_type.to_owned(),
        keys: Vec::new(),
        details: SecretDetails::None,
        is_immutable: false,
        is_owned: false,
    }
}

fn ingress_using(name: &str, secrets: &[Option<&str>]) -> IngressSummary {
    IngressSummary {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        class: None,
        hosts: Vec::new(),
        addresses: Vec::new(),
        rules: Vec::new(),
        default_backend: None,
        default_service: None,
        tls: secrets
            .iter()
            .map(|secret| cluster::IngressTls {
                hosts: Vec::new(),
                secret_name: secret.map(str::to_owned),
            })
            .collect(),
    }
}

fn secret_pod(name: &str, container: ContainerSummary) -> PodSummary {
    with_container(pod("team-a", name, &[]), container)
}

fn env_from_secret(name: &str) -> ContainerSummary {
    let mut container = container(ContainerKind::Main);
    container.env_from = vec![EnvFromEntry {
        source: EnvFromSource::Secret {
            name: name.to_owned(),
        },
        prefix: None,
    }];
    container
}

/// Every kind of reference a pod can hold to a secret named `db`.
fn pod_using_secrets() -> PodSummary {
    let mut main = container(ContainerKind::Main);
    main.env = vec![EnvEntry {
        name: "PASSWORD".to_owned(),
        source: EnvSource::SecretKey {
            name: "db".to_owned(),
            key: "password".to_owned(),
        },
    }];
    main.env_from = env_from_secret("db").env_from;
    main.mounts = vec![
        MountEntry {
            path: "/etc/db".to_owned(),
            volume: "db".to_owned(),
            source: VolumeSource::Secret {
                name: "db".to_owned(),
            },
            is_read_only: true,
            sub_path: None,
        },
        MountEntry {
            path: "/etc/bundle".to_owned(),
            volume: "bundle".to_owned(),
            source: VolumeSource::Projected {
                config_maps: vec!["settings".to_owned()],
                secrets: vec!["projected".to_owned()],
            },
            is_read_only: true,
            sub_path: None,
        },
    ];
    let mut pod = secret_pod("api-1", main);
    pod.image_pull_secrets = vec!["registry".to_owned()];
    pod
}

#[test]
fn secret_users_by_env_env_from_volume_projected_pull() {
    let users = secret_users(&[pod_using_secrets()], NO_INGRESSES);
    let db = users_in_team_a(&users, "db");
    assert_eq!(db.len(), 1);
    assert_eq!(ways_of(&db[0]), ["env", "env from", "volume"]);
    assert_eq!(db[0].owner, "pod/api-1");
    assert_eq!(
        ways_of(&users_in_team_a(&users, "projected")[0]),
        ["volume"]
    );
    assert_eq!(
        ways_of(&users_in_team_a(&users, "registry")[0]),
        ["image pull"]
    );
    // A config map in a projected volume is not a secret.
    assert!(users_in_team_a(&users, "settings").is_empty());
}

#[test]
fn secret_users_ignore_empty_names() {
    let mut main = container(ContainerKind::Main);
    main.mounts = vec![MountEntry {
        path: "/etc".to_owned(),
        volume: "v".to_owned(),
        source: VolumeSource::Secret {
            name: String::new(),
        },
        is_read_only: true,
        sub_path: None,
    }];
    let mut pod = secret_pod("api-1", main);
    pod.image_pull_secrets = vec![String::new()];
    assert!(secret_users(&[pod], NO_INGRESSES).is_empty());
}

#[test]
fn secret_users_include_ingress_tls() {
    let ingress = ingress_using("shop", &[Some("shop-tls"), None, Some("shop-tls")]);
    let users = secret_users(NO_PODS, &[ingress]);
    let tls = users_in_team_a(&users, "shop-tls");
    assert_eq!(tls.len(), 1);
    assert_eq!(tls[0].owner, "ingress/shop");
    assert_eq!(ways_of(&tls[0]), ["tls"]);
    assert_eq!(
        tls[0].target,
        ResourceKey::of_object("Ingress", Some("team-a"), "shop")
    );
}

#[test]
fn secret_users_list_the_token_account() {
    let mut token = secret_of("kubernetes.io/service-account-token", "builder-token");
    token.details = SecretDetails::ServiceAccountToken {
        account: Some("builder".to_owned()),
    };
    let list = secret_user_list(&token, &SecretUsers::new());
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].owner, "serviceaccount/builder");
    assert_eq!(ways_of(&list[0]), ["token"]);
}

/// The Secrets rows after a join; `pods` and `ingresses` are `None` while not loaded.
fn joined_secrets(
    secrets: &[cluster::SecretSummary],
    pods: Option<Vec<PodSummary>>,
    ingresses: Option<Vec<IngressSummary>>,
) -> Vec<KindRow> {
    let mut rows: Vec<KindRow> = secrets.iter().map(crate::secret_rows::secret_row).collect();
    let pods = match pods {
        Some(pods) => ready_list(pods),
        None => LiveList::Loading,
    };
    let companion = ingresses.map(|ingresses| CompanionLists::Ingresses(ready_list(ingresses)));
    let inputs = JoinInputs {
        custom_counts: None,
        pods: &pods,
        companion: companion.as_ref(),
        kubelet: None,
        scope: &NamespaceScope::All,
    };
    join_rows(ResourceKind::Secrets, &mut rows, &inputs);
    rows
}

fn none_found_cell() -> KindCell {
    KindCell::Toned(toned("none found", StatusTone::Done))
}

#[test]
fn secret_used_by_absent_until_lists_ready() {
    let secrets = [secret_of("Opaque", "orphan"), secret_of("Opaque", "db")];
    let pods = vec![pod_using_secrets()];
    // Pods not loaded: nothing at all, not even the ingress users.
    let rows = joined_secrets(&secrets, None, Some(vec![ingress_using("shop", &[])]));
    assert!(
        rows.iter()
            .all(|row| row.cells[SECRET_USED_BY] == KindCell::Absent)
    );
    // Ingresses not loaded: users show, but `none found` waits.
    let rows = joined_secrets(&secrets, Some(pods.clone()), None);
    assert_eq!(rows[0].cells[SECRET_USED_BY], KindCell::Absent);
    assert_eq!(
        rows[1].cells[SECRET_USED_BY],
        KindCell::Mono("pod/api-1".into())
    );
    // Both loaded.
    let rows = joined_secrets(&secrets, Some(pods), Some(Vec::new()));
    assert_eq!(rows[0].cells[SECRET_USED_BY], none_found_cell());
}

#[test]
fn secret_used_by_cell_names_first_owner_and_count() {
    let secrets = [secret_of("Opaque", "db")];
    let second = secret_pod("web-1", env_from_secret("db"));
    let rows = joined_secrets(
        &secrets,
        Some(vec![pod_using_secrets(), second]),
        Some(Vec::new()),
    );
    assert_eq!(
        rows[0].cells[SECRET_USED_BY],
        KindCell::MonoWithMore {
            text: "pod/api-1".into(),
            more: 1
        }
    );
}

#[test]
fn secret_in_use_by_an_ingress_has_a_user() {
    let secrets = [secret_of("Opaque", "shop-cert")];
    let ingress = ingress_using("shop", &[Some("shop-cert")]);
    let rows = joined_secrets(&secrets, Some(Vec::new()), Some(vec![ingress]));
    assert_eq!(
        rows[0].cells[SECRET_USED_BY],
        KindCell::Mono("ingress/shop".into())
    );
}

#[test]
fn a_tls_secret_nobody_mounts_says_none_found() {
    let secrets = [secret_of("kubernetes.io/tls", "shop-tls")];
    let rows = joined_secrets(&secrets, Some(Vec::new()), Some(Vec::new()));
    assert_eq!(rows[0].cells[SECRET_USED_BY], none_found_cell());
}

// ---- Ingress TLS ----

fn tls_secret_with(name: &str, details: SecretDetails) -> cluster::SecretSummary {
    let mut secret = secret_of("kubernetes.io/tls", name);
    secret.details = details;
    secret
}

fn leaf_expiring(not_after_second: i64) -> SecretDetails {
    SecretDetails::Certificate {
        chain: vec![cluster::CertificateInfo {
            subject: "CN=shop".to_owned(),
            issuer: "CN=ca".to_owned(),
            alt_names: Vec::new(),
            not_before: jiff::Timestamp::from_second(0).expect("timestamp"),
            not_after: jiff::Timestamp::from_second(not_after_second).expect("timestamp"),
        }],
    }
}

/// The TLS cell of one ingress after a join; `secrets` is `None` while the companion is absent.
fn tls_cell_of(ingress: &IngressSummary, secrets: Option<Vec<cluster::SecretSummary>>) -> KindCell {
    let mut rows = vec![crate::network_rows::ingress_row(ingress)];
    let pods = ready_list(Vec::new());
    let companion = secrets.map(|secrets| CompanionLists::TlsSecrets(ready_list(secrets)));
    let inputs = JoinInputs {
        custom_counts: None,
        pods: &pods,
        companion: companion.as_ref(),
        kubelet: None,
        scope: &NamespaceScope::All,
    };
    join_rows(ResourceKind::Ingresses, &mut rows, &inputs);
    rows.remove(0).cells.remove(INGRESS_TLS)
}

#[test]
fn ingress_tls_column_is_named_tls() {
    let columns = ResourceKind::Ingresses.columns();
    assert_eq!(
        columns.get(INGRESS_TLS).map(|column| column.name),
        Some("TLS")
    );
}

#[test]
fn ingress_tls_earliest_leaf_expiry() {
    let ingress = ingress_using("shop", &[Some("late"), Some("early"), Some("early")]);
    let mut elsewhere = tls_secret_with("early", leaf_expiring(100));
    elsewhere.namespace = "elsewhere".to_owned();
    let secrets = vec![
        tls_secret_with("late", leaf_expiring(900)),
        tls_secret_with("early", leaf_expiring(500)),
        // A secret of the same name in another namespace is not the one named.
        elsewhere,
    ];
    assert_eq!(
        tls_cell_of(&ingress, Some(secrets)),
        KindCell::Expiry {
            not_after: jiff::Timestamp::from_second(500).expect("timestamp")
        }
    );
}

#[test]
fn ingress_tls_missing_secret() {
    let ingress = ingress_using("shop", &[Some("gone"), Some("present")]);
    let secrets = vec![tls_secret_with("present", leaf_expiring(500))];
    assert_eq!(
        tls_cell_of(&ingress, Some(secrets)),
        KindCell::Toned(toned("no TLS secret", StatusTone::Warn))
    );
}

#[test]
fn ingress_tls_not_parsed() {
    let ingress = ingress_using("shop", &[Some("broken")]);
    let secrets = vec![tls_secret_with(
        "broken",
        SecretDetails::NoCertificate(cluster::CertificateIssue::Unparsed),
    )];
    assert_eq!(
        tls_cell_of(&ingress, Some(secrets)),
        KindCell::Toned(toned("not parsed", StatusTone::Warn))
    );
}

#[test]
fn ingress_tls_default_cert() {
    let ingress = ingress_using("shop", &[None]);
    assert_eq!(
        tls_cell_of(&ingress, Some(Vec::new())),
        KindCell::Text("default cert".into())
    );
}

#[test]
fn ingress_tls_unjoined_without_companion() {
    let ingress = ingress_using("shop", &[Some("shop-tls")]);
    assert_eq!(tls_cell_of(&ingress, None), KindCell::Text("yes".into()));
}

#[test]
fn ingress_without_tls_keeps_an_absent_cell() {
    let ingress = ingress_using("shop", &[]);
    assert_eq!(tls_cell_of(&ingress, Some(Vec::new())), KindCell::Absent);
}

fn crd_row_named(name: &str) -> KindRow {
    crate::crd_rows::crd_row(&cluster::CrdSummary {
        name: name.to_owned(),
        group: "x.io".to_owned(),
        kind: "Widget".to_owned(),
        plural: "widgets".to_owned(),
        singular: "widget".to_owned(),
        scope: cluster::ResourceScope::Namespaced,
        versions: Vec::new(),
        state: cluster::CrdState::Established,
        created_at: None,
    })
}

fn widget_kind_named(crd_name: &str) -> CustomKind {
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

fn join_crd_rows(rows: &mut [KindRow], counts: Option<&HashMap<CustomKind, u64>>) {
    let pods = LiveList::Loading;
    let inputs = JoinInputs {
        custom_counts: counts,
        pods: &pods,
        companion: None,
        kubelet: None,
        scope: &NamespaceScope::All,
    };
    join_rows(ResourceKind::Crds, rows, &inputs);
}

#[test]
fn crd_instances_index_names_the_column() {
    let columns = crate::resource_kind::kind_columns(ResourceKind::Crds);
    // The Name column is not a cell.
    assert_eq!(columns[CRD_INSTANCES + 1].name, "Instances");
}

#[test]
fn crd_instances_fill_from_counts() {
    let mut rows = vec![crd_row_named("widgets.x.io"), crd_row_named("gadgets.x.io")];
    let counts = HashMap::from([(widget_kind_named("widgets.x.io"), 12)]);
    join_crd_rows(&mut rows, Some(&counts));
    assert_eq!(
        rows[0].cells[CRD_INSTANCES],
        KindCell::Quantity {
            text: "12".into(),
            value: 12,
            tone: None
        }
    );
    // A CRD without a count (not counted yet, failed, or denied) stays absent.
    assert_eq!(rows[1].cells[CRD_INSTANCES], KindCell::Absent);
}

#[test]
fn missing_counts_stay_absent() {
    let mut rows = vec![crd_row_named("widgets.x.io")];
    join_crd_rows(&mut rows, None);
    assert_eq!(rows[0].cells[CRD_INSTANCES], KindCell::Absent);
    // A rerun without counts clears an earlier join instead of keeping it.
    let counts = HashMap::from([(widget_kind_named("widgets.x.io"), 3)]);
    join_crd_rows(&mut rows, Some(&counts));
    join_crd_rows(&mut rows, None);
    assert_eq!(rows[0].cells[CRD_INSTANCES], KindCell::Absent);
}

#[test]
fn service_health_core_matches_list_wrapper() {
    // The slice core is what Topology calls; the wrapper reads the session's lists. On loaded
    // lists they must agree, for a Service that selects pods and for one that selects none.
    let pods = vec![
        pod("team-a", "api-1", &["app=api"]),
        pod("team-a", "api-2", &["app=api"]),
        pod("team-a", "db-1", &["app=db"]),
        pod("team-b", "api-3", &["app=api"]),
    ];
    let slices = vec![slice(
        Some("api"),
        "IPv4",
        vec![endpoint("10.0.0.1", Some("api-1"), true)],
    )];
    let pods_list = ready_list(pods.clone());
    let slices_list = ready_list(slices.clone());
    for selector in [&["app=api"][..], &["app=none"], &[]] {
        let service = service(selector);
        let core = service_health(&service, &pods, Some(&slices));
        let wrapped = service_health_of(&service, &pods_list, Some(&slices_list));
        assert_eq!(core, wrapped, "{selector:?}");
    }
}

// ---- Last job of a CronJob ----

fn scheduled_cron_job(last: Option<i64>) -> cluster::CronJobSummary {
    let mut cron_job =
        crate::workload_actions::workload_actions_tests::cron_job("nightly", "Allow", 0);
    cron_job.last_schedule_at =
        last.map(|seconds| jiff::Timestamp::from_second(seconds).expect("a valid timestamp"));
    cron_job
}

#[test]
fn last_job_name_uses_unix_minutes() {
    // 59 s into minute 29_000_000: the controller names the Job by the whole minute.
    let cron_job = scheduled_cron_job(Some(29_000_000 * 60 + 59));
    let pods = [owned_by(
        pod("team-a", "nightly-29000000-x", &[]),
        "Job",
        "nightly-29000000",
    )];
    assert_eq!(
        last_job_owner(&cron_job, &pods),
        Ok(PodOwner::Controller {
            namespace: "team-a".to_owned(),
            kind: JOB_KIND,
            name: "nightly-29000000".to_owned(),
        })
    );
}

#[test]
fn last_job_needs_a_schedule() {
    let cron_job = scheduled_cron_job(None);
    assert_eq!(
        last_job_owner(&cron_job, &[]),
        Err("No job has run yet".into())
    );
}

#[test]
fn last_job_needs_pods_left() {
    let cron_job = scheduled_cron_job(Some(29_000_000 * 60));
    // A pod of an earlier run does not count.
    let pods = [owned_by(
        pod("team-a", "nightly-28999999-x", &[]),
        "Job",
        "nightly-28999999",
    )];
    assert_eq!(
        last_job_owner(&cron_job, &pods),
        Err("Job nightly-29000000 has no pods left".into())
    );
}

#[test]
fn last_job_ignores_other_namespaces() {
    let cron_job = scheduled_cron_job(Some(29_000_000 * 60));
    let pods = [owned_by(
        pod("team-b", "nightly-29000000-x", &[]),
        "Job",
        "nightly-29000000",
    )];
    assert!(last_job_owner(&cron_job, &pods).is_err());
}

fn mounting_config_map(name: &str) -> ContainerSummary {
    let mut container = container(ContainerKind::Main);
    container.mounts = vec![MountEntry {
        path: "/etc/c".to_owned(),
        volume: "c".to_owned(),
        source: VolumeSource::ConfigMap {
            name: name.to_owned(),
        },
        is_read_only: true,
        sub_path: None,
    }];
    container
}

#[test]
fn env_consumers_are_the_env_reading_deployments_statefulsets_and_daemonsets_of_the_namespace() {
    let using = |namespace: &str, name: &str, container: ContainerSummary| {
        with_container(pod(namespace, name, &[]), container)
    };
    let pods = [
        owned_by(
            using("team-a", "api-1", env_from_config_map("c")),
            "ReplicaSet",
            "api-7d9f8c",
        ),
        owned_by(
            using("team-a", "db-0", env_from_config_map("c")),
            "StatefulSet",
            "db",
        ),
        // Mounted files update on their own, a CronJob run starts fresh, a bare pod is not
        // restarted, and another namespace's `c` is another object.
        owned_by(
            using("team-a", "agent-1", mounting_config_map("c")),
            "DaemonSet",
            "agent",
        ),
        owned_by(
            using("team-a", "n-1", env_from_config_map("c")),
            "Job",
            "nightly-29012345",
        ),
        using("team-a", "debug", env_from_config_map("c")),
        owned_by(
            using("team-b", "web-1", env_from_config_map("c")),
            "ReplicaSet",
            "web-7d9f8c",
        ),
    ];
    let consumers = env_consumers(&pods, ObjectKind::ConfigMap, "team-a", "c");
    assert_eq!(
        consumers,
        [
            (
                ObjectKind::Deployment,
                ResourceKey::of_object("Deployment", Some("team-a"), "api").unwrap()
            ),
            (
                ObjectKind::StatefulSet,
                ResourceKey::of_object("StatefulSet", Some("team-a"), "db").unwrap()
            ),
        ]
    );
}

#[test]
fn env_consumers_of_a_secret_read_its_own_references() {
    let pods = [
        owned_by(
            with_container(pod("team-a", "agent-1", &[]), env_from_secret("s")),
            "DaemonSet",
            "agent",
        ),
        // A ConfigMap of the same name is not the Secret.
        owned_by(
            with_container(pod("team-a", "api-1", &[]), env_from_config_map("s")),
            "ReplicaSet",
            "api-7d9f8c",
        ),
    ];
    let consumers = env_consumers(&pods, ObjectKind::Secret, "team-a", "s");
    assert_eq!(
        consumers,
        [(
            ObjectKind::DaemonSet,
            ResourceKey::of_object("DaemonSet", Some("team-a"), "agent").unwrap()
        )]
    );
}
