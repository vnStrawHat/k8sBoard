use cluster::{ReadyCount, ServicePortSummary, StatusReason};

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
    };
    join_rows(ResourceKind::Services, &mut rows, &with_slices);
    let without = JoinInputs {
        pods: &pods,
        companion: None,
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
