//! The offline Traffic fixture (spec 0049): the namespace of W11 with Istio and pod network
//! readings, for the `topology-traffic-fixture` screens and the tests. The graph is written out by
//! hand so the screenshot build needs none of the test builders; a test checks that it equals what
//! the real row builders and `build_topology` make of the same objects.

use std::collections::HashMap;

use cluster::{
    ControllerRef, MetricsError, PodStatus, PodSummary, ReadyCount, StatusReason, TrafficEnd,
    TrafficMetricSource, TrafficRate, TrafficReading, TrafficSourceKind,
};

use crate::status_tone::StatusTone;
use crate::table_selection::ResourceKey;
use crate::topology_checks::{CheckRule, ConfigCheck};
use crate::topology_graph::{
    NodeId, NodeLook, Relation, TopologyEdge, TopologyGraph, TopologyKind, TopologyNode,
};
use crate::topology_traffic::TrafficSample;

pub(crate) const NAMESPACE: &str = "shop";

/// One node of the fixture: kind, name, caption, tone, and app group.
type NodeRow = (
    TopologyKind,
    &'static str,
    &'static str,
    Option<StatusTone>,
    Option<&'static str>,
);

const NODES: [NodeRow; 13] = [
    (
        TopologyKind::Ingress,
        "shop-api",
        "Ingress \u{b7} shop-api.example.com",
        Some(StatusTone::Warn),
        None,
    ),
    (
        TopologyKind::Service,
        "ledger",
        "Service \u{b7} ClusterIP",
        None,
        Some("ledger"),
    ),
    (
        TopologyKind::Service,
        "payments-api",
        "Service \u{b7} /",
        None,
        Some("api"),
    ),
    (
        TopologyKind::Service,
        "payments-legacy",
        "Service \u{b7} ClusterIP",
        None,
        None,
    ),
    (
        TopologyKind::Deployment,
        "api",
        "Deployment \u{b7} 2/3",
        Some(StatusTone::Warn),
        None,
    ),
    (
        TopologyKind::StatefulSet,
        "ledger",
        "StatefulSet \u{b7} 1/1",
        None,
        Some("ledger"),
    ),
    (
        TopologyKind::DaemonSet,
        "node-agent",
        "DaemonSet \u{b7} 1/1",
        None,
        Some("node-agent"),
    ),
    (
        TopologyKind::ReplicaSet,
        "api-7d9f",
        "ReplicaSet \u{b7} rev 3",
        None,
        Some("api"),
    ),
    (
        TopologyKind::Pod,
        "api-7d9f-4xk2p",
        "Running \u{b7} 1/1",
        None,
        Some("api"),
    ),
    (
        TopologyKind::Pod,
        "api-7d9f-9qz7v",
        "Running \u{b7} 1/1",
        None,
        Some("api"),
    ),
    (
        TopologyKind::Pod,
        "api-7d9f-t5bcm",
        "CrashLoopBackOff \u{b7} api",
        Some(StatusTone::Bad),
        Some("api"),
    ),
    (
        TopologyKind::Pod,
        "ledger-0",
        "Running \u{b7} 1/1",
        None,
        Some("ledger"),
    ),
    (
        TopologyKind::Pod,
        "node-agent-x7k2p",
        "Running \u{b7} 1/1",
        None,
        Some("node-agent"),
    ),
];

/// The nodes by index: the 13 above, then the `NoPods` ghost of `payments-legacy` (13).
const EDGES: [(usize, usize, Relation); 12] = [
    (0, 2, Relation::RoutesTo),
    (1, 11, Relation::RoutesTo),
    (2, 8, Relation::RoutesTo),
    (2, 9, Relation::RoutesTo),
    (2, 10, Relation::RoutesTo),
    (3, 13, Relation::RoutesTo),
    (4, 7, Relation::Owns),
    (5, 11, Relation::Owns),
    (6, 12, Relation::Owns),
    (7, 8, Relation::Owns),
    (7, 9, Relation::Owns),
    (7, 10, Relation::Owns),
];

fn node_of(row: &NodeRow) -> TopologyNode {
    let (kind, name, caption, tone, group) = *row;
    let key = match kind {
        TopologyKind::Pod => Some(ResourceKey::Pod {
            namespace: NAMESPACE.to_owned(),
            name: name.to_owned(),
        }),
        _ => ResourceKey::of_object(kind.object_kind(), Some(NAMESPACE), name),
    };
    TopologyNode {
        id: NodeId::Object {
            kind,
            name: name.to_owned(),
        },
        kind,
        look: NodeLook::Plain,
        name: name.into(),
        caption: caption.into(),
        tone,
        group: group.map(str::to_owned),
        objects: 1,
        key,
    }
}

/// The graph `build_topology` makes of the `traffic_namespace()` objects.
pub(crate) fn traffic_fixture_graph() -> TopologyGraph {
    let ghost = NodeId::NoPods {
        service: "payments-legacy".to_owned(),
    };
    let mut nodes: Vec<TopologyNode> = NODES.iter().map(node_of).collect();
    nodes.push(TopologyNode {
        id: ghost.clone(),
        kind: TopologyKind::Pod,
        look: NodeLook::Ghost,
        name: "selector app=legacy".into(),
        caption: "0 pods match".into(),
        tone: Some(StatusTone::Bad),
        group: None,
        objects: 0,
        key: None,
    });
    TopologyGraph {
        nodes,
        edges: EDGES
            .iter()
            .map(|(from, to, relation)| TopologyEdge {
                from: *from,
                to: *to,
                relation: *relation,
            })
            .collect(),
        // Every fixture Service has the one port 80 (the builders' default).
        port_labels: EDGES
            .iter()
            .filter(|(_, _, relation)| *relation == Relation::RoutesTo)
            .map(|(from, to, _)| ((*from, *to), "80".to_owned()))
            .collect::<HashMap<_, _>>(),
        checks: vec![
            ConfigCheck {
                rule: CheckRule::ServiceNoPods,
                node: ghost,
                tone: StatusTone::Bad,
                text: "Service payments-legacy matches no pods (selector app=legacy).".to_owned(),
            },
            ConfigCheck {
                rule: CheckRule::ObjectDiagnosis,
                node: NodeId::Object {
                    kind: TopologyKind::Deployment,
                    name: "api".to_owned(),
                },
                tone: StatusTone::Warn,
                text: "Deployment api: 1 of 3 not ready.".to_owned(),
            },
        ],
        resources: 13,
    }
}

fn pod(name: &str, owner: (&str, &str), app: &str, is_host_network: bool) -> PodSummary {
    PodSummary {
        annotations: cluster::AnnotationTerms::default(),
        is_finished: false,
        namespace: NAMESPACE.to_owned(),
        name: name.to_owned(),
        status: PodStatus::Reason(StatusReason::Running),
        ready: ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: None,
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: Some(ControllerRef {
            kind: owner.0.to_owned(),
            name: owner.1.to_owned(),
        }),
        conditions: Vec::new(),
        containers: Vec::new(),
        status_message: None,
        labels: vec![format!("app={app}")],
        host_network: is_host_network,
        image_pull_secrets: Vec::new(),
        node_selector: Vec::new(),
        node_affinity: Vec::new(),
    }
}

/// The pods behind the fixture graph, which the traffic names are resolved against.
pub(crate) fn traffic_fixture_pods() -> Vec<PodSummary> {
    let api = |name: &str| pod(name, ("ReplicaSet", "api-7d9f"), "api", false);
    vec![
        api("api-7d9f-4xk2p"),
        api("api-7d9f-9qz7v"),
        api("api-7d9f-t5bcm"),
        pod("ledger-0", ("StatefulSet", "ledger"), "ledger", false),
        pod(
            "node-agent-x7k2p",
            ("DaemonSet", "node-agent"),
            "node-agent",
            true,
        ),
    ]
}

fn traffic_source(kind: TrafficSourceKind) -> TrafficMetricSource {
    let metric = match kind {
        TrafficSourceKind::Istio => "istio_requests_total",
        TrafficSourceKind::PodNetwork => "container_network_receive_bytes_total",
    };
    TrafficMetricSource::detect(&std::collections::BTreeSet::from([metric.to_owned()]))[0]
}

fn flow(from: TrafficEnd, service: &str, requests: f64, errors: f64) -> TrafficRate {
    TrafficRate {
        from: Some(from),
        to: TrafficEnd::Service(service.to_owned()),
        requests: Some(requests),
        errors: Some(errors),
        receive: None,
        transmit: None,
    }
}

pub(crate) fn pod_rate(pod: &str, receive: f64, transmit: f64) -> TrafficRate {
    TrafficRate {
        from: None,
        to: TrafficEnd::Pod(pod.to_owned()),
        requests: None,
        errors: None,
        receive: Some(receive),
        transmit: Some(transmit),
    }
}

fn reading(
    kind: TrafficSourceKind,
    rates: Vec<TrafficRate>,
) -> (TrafficMetricSource, Result<TrafficReading, MetricsError>) {
    (
        traffic_source(kind),
        Ok(TrafficReading {
            rates,
            was_cut: false,
        }),
    )
}

fn sample(
    readings: Vec<(TrafficMetricSource, Result<TrafficReading, MetricsError>)>,
) -> TrafficSample {
    TrafficSample {
        at: jiff::Timestamp::now(),
        readings,
    }
}

fn istio_reading() -> (TrafficMetricSource, Result<TrafficReading, MetricsError>) {
    let workload = |name: &str| TrafficEnd::Workload(name.to_owned());
    reading(
        TrafficSourceKind::Istio,
        vec![
            flow(workload("ledger"), "payments-api", 35., 2.1),
            flow(workload("api"), "ledger", 12., 0.05),
            flow(
                TrafficEnd::Outside("unknown".to_owned()),
                "payments-api",
                4.,
                0.,
            ),
            flow(
                TrafficEnd::Outside("web/frontend".to_owned()),
                "payments-api",
                80.,
                0.,
            ),
        ],
    )
}

fn bytes_reading() -> (TrafficMetricSource, Result<TrafficReading, MetricsError>) {
    reading(
        TrafficSourceKind::PodNetwork,
        vec![
            pod_rate("api-7d9f-4xk2p", 1_200_000., 400_000.),
            pod_rate("api-7d9f-9qz7v", 300_000., 100_000.),
            pod_rate("api-7d9f-t5bcm", 0., 0.),
            pod_rate("ledger-0", 50_000., 20_000.),
            pod_rate("node-agent-x7k2p", 9_000_000., 3_000_000.),
        ],
    )
}

/// Istio on the fixture: `ledger` to `payments-api` 35 req/s with 6 % 5xx, `api` to `ledger`
/// 12 req/s, an `unknown` caller, and `web/frontend` from another namespace.
#[cfg(test)]
pub(crate) fn istio_sample() -> TrafficSample {
    sample(vec![istio_reading()])
}

/// Pod network bytes on the fixture: the three `api` pods at 1.2 MB/s, 300 KB/s, and 0, `ledger-0`
/// at 50 KB/s, and the host-network `node-agent-x7k2p` at 9 MB/s.
#[cfg(test)]
pub(crate) fn bytes_sample() -> TrafficSample {
    sample(vec![bytes_reading()])
}

/// Both sources at once, as a cluster with Istio and cAdvisor shows them.
pub(crate) fn traffic_fixture_sample() -> TrafficSample {
    sample(vec![istio_reading(), bytes_reading()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology_fixtures::traffic_namespace;

    #[test]
    fn the_hand_written_graph_equals_the_built_one() {
        let built = traffic_namespace().graph();
        assert_eq!(traffic_fixture_graph(), built);
    }

    #[test]
    fn the_hand_written_pods_match_the_built_ones() {
        let built = traffic_namespace().pods.expect("pods listed");
        let written = traffic_fixture_pods();
        assert_eq!(written.len(), built.len());
        for (written, built) in written.iter().zip(&built) {
            assert_eq!(written.name, built.name);
            assert_eq!(written.controller, built.controller);
            assert_eq!(written.host_network, built.host_network);
            assert_eq!(written.labels, built.labels);
        }
    }

    #[test]
    fn the_combined_sample_holds_both_readings() {
        let sample = traffic_fixture_sample();
        let kinds: Vec<_> = sample
            .readings
            .iter()
            .map(|(source, _)| source.kind())
            .collect();
        assert_eq!(
            kinds,
            [TrafficSourceKind::Istio, TrafficSourceKind::PodNetwork]
        );
    }
}
