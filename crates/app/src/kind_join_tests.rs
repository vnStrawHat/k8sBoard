use cluster::{
    ContainerKind, ContainerProbes, ContainerResource, ContainerState, ContainerSummary,
    ControllerRef, EnvEntry, EnvFromEntry, EnvFromSource, EnvSource, MountEntry, NamespacePhase,
    NamespaceScope, NamespaceSummary, PodStatus, ReadyCount, ServicePortSummary, StatusReason,
    VolumeSource,
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
        pods: &pods,
        companion: companion.as_ref(),
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
        pods: &pods,
        companion: Some(&companion),
        scope: &NamespaceScope::All,
    };
    join_rows(ResourceKind::Services, &mut rows, &with_slices);
    let without = JoinInputs {
        pods: &pods,
        companion: None,
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
        pods: &pods,
        companion: None,
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
        pods: &pods,
        companion: None,
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

// ---- ConfigMaps and Namespaces ----

fn container(kind: ContainerKind) -> ContainerSummary {
    ContainerSummary {
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
        pods,
        companion: None,
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
        Some(&KindCell::Absent)
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
    })
}

fn join_namespaces_of(scope: &NamespaceScope, pods: &LiveList<PodSummary>, rows: &mut [KindRow]) {
    let inputs = JoinInputs {
        pods,
        companion: None,
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
        pods,
        companion: None,
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
