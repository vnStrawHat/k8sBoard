use std::collections::HashMap;

use cluster::PodSummary;

use super::*;
use crate::topology_fixtures::{Fixture, Ref, index_of, object, pod, pod_with, traffic_namespace};
use crate::topology_graph::GroupBy;
use crate::topology_layout::layout;
use crate::topology_traffic_fixture::{bytes_sample, istio_sample, pod_rate};

fn refs(pods: &[PodSummary]) -> Vec<&PodSummary> {
    pods.iter().collect()
}

/// The `traffic_namespace()` graph and the pods of its namespace.
fn namespace() -> (TopologyGraph, Vec<PodSummary>) {
    let fixture = traffic_namespace();
    (fixture.graph(), fixture.pods.clone().expect("pods listed"))
}

fn node(graph: &TopologyGraph, kind: TopologyKind, name: &str) -> usize {
    index_of(graph, &object(kind, name))
}

/// The overlay of `sample` on `traffic_namespace()`.
fn overlay_of(sample: &TrafficSample) -> (TopologyGraph, Vec<TopologyEdge>, TrafficOverlay) {
    let (graph, pods) = namespace();
    let calls = call_edges(&graph, &refs(&pods), sample);
    let overlay = traffic_overlay(&graph, &calls, &refs(&pods), sample);
    (graph, calls, overlay)
}

/// What the overlay says of the graph edge `from` to `to`.
fn edge_of<'a>(
    graph: &TopologyGraph,
    overlay: &'a TrafficOverlay,
    from: usize,
    to: usize,
) -> &'a EdgeTraffic {
    let index = graph
        .edges
        .iter()
        .position(|edge| edge.from == from && edge.to == to)
        .unwrap_or_else(|| panic!("no edge {from} -> {to}"));
    &overlay.edges[index]
}

fn flow_of(traffic: &EdgeTraffic) -> &EdgeFlow {
    match traffic {
        EdgeTraffic::Flow(flow) => flow,
        EdgeTraffic::Hidden | EdgeTraffic::Idle => panic!("expected a flow, got {traffic:?}"),
    }
}

/// Both samples as one.
fn both() -> TrafficSample {
    let mut sample = istio_sample();
    sample.readings.extend(bytes_sample().readings);
    sample
}

/// A node with nothing but its identity.
fn bare_node(kind: TopologyKind, name: &str) -> TopologyNode {
    TopologyNode {
        id: object(kind, name),
        kind,
        look: NodeLook::Plain,
        name: name.to_owned().into(),
        caption: "caption".into(),
        tone: None,
        group: None,
        objects: 1,
        key: None,
    }
}

fn hand_built(nodes: &[(TopologyKind, &str)], edges: &[(usize, usize, Relation)]) -> TopologyGraph {
    TopologyGraph {
        nodes: nodes
            .iter()
            .map(|(kind, name)| bare_node(*kind, name))
            .collect(),
        edges: edges
            .iter()
            .map(|(from, to, relation)| TopologyEdge {
                from: *from,
                to: *to,
                relation: *relation,
            })
            .collect(),
        checks: Vec::new(),
        resources: 0,
    }
}

#[test]
fn call_routes_leave_the_layout_alone() {
    let (graph, pods) = namespace();
    let calls = call_edges(&graph, &refs(&pods), &istio_sample());
    assert_eq!(calls.len(), 2);
    let laid_out = layout(&graph, GroupBy::Components, 1.6, &HashMap::new(), None);
    let rects = laid_out.rects.clone();
    let routes = laid_out.route_extra(&calls);
    assert_eq!(laid_out.rects, rects, "no card moved");
    assert_eq!(routes.len(), calls.len());
    for (edge, route) in calls.iter().zip(&routes) {
        let (source, target) = (laid_out.rects[edge.from], laid_out.rects[edge.to]);
        let on_side = |at: GraphPoint, card: crate::topology_layout::GraphRect| {
            let center = card.center();
            let on_row = (at.x == card.origin.x || at.x == card.right()) && at.y == center.y;
            let on_column = (at.y == card.origin.y || at.y == card.bottom()) && at.x == center.x;
            on_row || on_column
        };
        assert!(on_side(route.start(), source), "starts on the source");
        assert!(on_side(route.end(), target), "ends on the target");
    }
}

#[test]
fn missing_pair_becomes_a_calls_edge() {
    let (graph, calls, _) = overlay_of(&istio_sample());
    let ledger = node(&graph, TopologyKind::StatefulSet, "ledger");
    let api = node(&graph, TopologyKind::Deployment, "api");
    let payments = node(&graph, TopologyKind::Service, "payments-api");
    let ledger_service = node(&graph, TopologyKind::Service, "ledger");
    let mut found: Vec<(usize, usize, Relation)> = calls
        .iter()
        .map(|edge| (edge.from, edge.to, edge.relation))
        .collect();
    found.sort();
    let mut expected = vec![
        (ledger, payments, Relation::Calls),
        (api, ledger_service, Relation::Calls),
    ];
    expected.sort();
    assert_eq!(found, expected);
}

#[test]
fn existing_edge_takes_the_flow() {
    let (graph, calls, overlay) = overlay_of(&bytes_sample());
    assert!(calls.is_empty(), "bytes make no Calls edge");
    let service = node(&graph, TopologyKind::Service, "payments-api");
    let busy = node(&graph, TopologyKind::Pod, "api-7d9f-4xk2p");
    let flow = flow_of(edge_of(&graph, &overlay, service, busy));
    assert_eq!(flow.unit, TrafficUnit::Bytes);
    assert_eq!(flow.rate, 1_200_000.);
    assert_eq!(flow.label.as_deref(), Some("1.2 MB/s"));
    let idle = node(&graph, TopologyKind::Pod, "api-7d9f-t5bcm");
    assert_eq!(edge_of(&graph, &overlay, service, idle), &EdgeTraffic::Idle);
}

#[test]
fn istio_wins_over_bytes_on_an_edge() {
    // Deployment `api` to Service `ledger` is an edge of the graph, and the Service reaches a pod.
    let graph = hand_built(
        &[
            (TopologyKind::Deployment, "api"),
            (TopologyKind::Service, "ledger"),
            (TopologyKind::Pod, "ledger-0"),
        ],
        &[(0, 1, Relation::RoutesTo), (1, 2, Relation::RoutesTo)],
    );
    let pods = vec![pod("ledger-0", &[], None)];
    let mut sample = istio_sample();
    sample.readings.extend(
        crate::topology_traffic_fixture::bytes_sample()
            .readings
            .into_iter()
            .take(1),
    );
    let calls = call_edges(&graph, &refs(&pods), &sample);
    assert!(
        calls.is_empty(),
        "the edge exists, so no Calls edge is made"
    );
    let overlay = traffic_overlay(&graph, &calls, &refs(&pods), &sample);
    let istio = flow_of(&overlay.edges[0]);
    assert_eq!(istio.unit, TrafficUnit::Requests);
    assert_eq!(istio.rate, 12.);
    let bytes = flow_of(&overlay.edges[1]);
    assert_eq!(bytes.unit, TrafficUnit::Bytes);
    assert_eq!(bytes.rate, 50_000.);
}

#[test]
fn unresolved_ends_count_as_outside() {
    let (_, _, overlay) = overlay_of(&istio_sample());
    assert_eq!(overlay.outside, ["unknown", "web/frontend"]);
    let mut sample = istio_sample();
    sample.readings.push((
        sample.readings[0].0,
        Ok(TrafficReading {
            rates: vec![TrafficRate {
                from: Some(TrafficEnd::Workload("ledger".to_owned())),
                to: TrafficEnd::Service("gone".to_owned()),
                requests: Some(1.),
                errors: Some(0.),
                receive: None,
                transmit: None,
            }],
            was_cut: false,
        }),
    ));
    let (_, calls, overlay) = overlay_of(&sample);
    assert_eq!(calls.len(), 2, "an unknown Service makes no edge");
    assert!(
        !overlay.outside.contains(&"gone".to_owned()),
        "a Service of the namespace that is not drawn is not an outside peer"
    );
}

#[test]
fn outside_peers_are_deduplicated_and_capped() {
    let (graph, pods) = namespace();
    let rates = (0..30)
        .flat_map(|index| {
            let name = format!("other/caller-{}", index % 25);
            [name.clone(), name]
        })
        .map(|name| TrafficRate {
            from: Some(TrafficEnd::Outside(name)),
            to: TrafficEnd::Service("payments-api".to_owned()),
            requests: Some(1.),
            errors: None,
            receive: None,
            transmit: None,
        })
        .collect();
    let mut sample = istio_sample();
    sample.readings[0].1 = Ok(TrafficReading {
        rates,
        was_cut: false,
    });
    let overlay = traffic_overlay(&graph, &[], &refs(&pods), &sample);
    assert_eq!(overlay.outside.len(), OUTSIDE_LIMIT);
    let unique: BTreeSet<_> = overlay.outside.iter().collect();
    assert_eq!(unique.len(), OUTSIDE_LIMIT);
}

#[test]
fn pod_in_a_group_resolves_to_the_group() {
    let mut fixture = Fixture::default()
        .with_service("web", &["app=web"])
        .with_deployment("web", 8, 8)
        .with_replica_set("web-rs", Some("web"), 8, 8);
    for index in 0..8 {
        fixture = fixture.with_pod(pod(
            &format!("web-rs-{index}"),
            &["app=web"],
            Some(("ReplicaSet", "web-rs")),
        ));
    }
    let graph = fixture.graph();
    let pods = fixture.pods.clone().expect("pods listed");
    let group = index_of(
        &graph,
        &NodeId::PodGroup {
            owner_kind: TopologyKind::ReplicaSet,
            owner: "web-rs".to_owned(),
        },
    );
    let mut bytes = bytes_sample();
    bytes.readings[0].1 = Ok(TrafficReading {
        rates: vec![
            pod_rate("web-rs-0", 100., 10.),
            pod_rate("web-rs-5", 250., 20.),
        ],
        was_cut: false,
    });
    let overlay = traffic_overlay(&graph, &[], &refs(&pods), &bytes);
    let at_group = overlay.nodes[group]
        .as_ref()
        .expect("the group has traffic");
    assert_eq!(at_group.receive, Some(350.));
    assert_eq!(at_group.transmit, Some(30.));
    let service = node(&graph, TopologyKind::Service, "web");
    let flow = flow_of(edge_of(&graph, &overlay, service, group));
    assert_eq!(flow.rate, 350.);
}

#[test]
fn host_network_pods_add_no_bytes() {
    let (graph, _, overlay) = overlay_of(&bytes_sample());
    let agent_pod = node(&graph, TopologyKind::Pod, "node-agent-x7k2p");
    let daemon_set = node(&graph, TopologyKind::DaemonSet, "node-agent");
    assert_eq!(overlay.nodes[agent_pod], None);
    assert_eq!(overlay.nodes[daemon_set], None);
    assert_eq!(
        edge_of(&graph, &overlay, daemon_set, agent_pod),
        &EdgeTraffic::Idle
    );
    assert!(overlay.outside.is_empty(), "{:?}", overlay.outside);
}

#[test]
fn bytes_sum_once_per_pod() {
    let (graph, _, overlay) = overlay_of(&bytes_sample());
    let received = |kind, name| {
        let index = node(&graph, kind, name);
        overlay.nodes[index].as_ref().expect("has traffic").receive
    };
    // 1.2 MB/s + 300 KB/s + 0 over the three `api` pods, reached through the Service and through
    // the ReplicaSet alike.
    assert_eq!(
        received(TopologyKind::Service, "payments-api"),
        Some(1_500_000.)
    );
    assert_eq!(
        received(TopologyKind::Ingress, "shop-api"),
        Some(1_500_000.)
    );
    assert_eq!(
        received(TopologyKind::ReplicaSet, "api-7d9f"),
        Some(1_500_000.)
    );
    assert_eq!(received(TopologyKind::Deployment, "api"), Some(1_500_000.));
    assert_eq!(received(TopologyKind::StatefulSet, "ledger"), Some(50_000.));
}

#[test]
fn a_pod_behind_two_paths_counts_once() {
    // A reaches the pod through B and through C.
    let graph = hand_built(
        &[
            (TopologyKind::Service, "a"),
            (TopologyKind::ReplicaSet, "b"),
            (TopologyKind::StatefulSet, "c"),
            (TopologyKind::Pod, "p"),
        ],
        &[
            (0, 1, Relation::RoutesTo),
            (0, 2, Relation::RoutesTo),
            (1, 3, Relation::Owns),
            (2, 3, Relation::Owns),
        ],
    );
    let pods = vec![pod("p", &[], None)];
    let mut sample = bytes_sample();
    sample.readings[0].1 = Ok(TrafficReading {
        rates: vec![pod_rate("p", 700., 70.)],
        was_cut: false,
    });
    let overlay = traffic_overlay(&graph, &[], &refs(&pods), &sample);
    let top = overlay.nodes[0].as_ref().expect("has traffic");
    assert_eq!((top.receive, top.transmit), (Some(700.), Some(70.)));
}

#[test]
fn edges_without_requests_carry_target_bytes() {
    let (graph, _, overlay) = overlay_of(&bytes_sample());
    let ingress = node(&graph, TopologyKind::Ingress, "shop-api");
    let service = node(&graph, TopologyKind::Service, "payments-api");
    let deployment = node(&graph, TopologyKind::Deployment, "api");
    let replica_set = node(&graph, TopologyKind::ReplicaSet, "api-7d9f");
    let routes_to = flow_of(edge_of(&graph, &overlay, ingress, service));
    assert_eq!(routes_to.rate, 1_500_000.);
    assert_eq!(routes_to.label.as_deref(), Some("1.5 MB/s"));
    let owns = flow_of(edge_of(&graph, &overlay, deployment, replica_set));
    assert_eq!(owns.unit, TrafficUnit::Bytes);
    assert_eq!(owns.rate, 1_500_000.);
    assert_eq!(owns.label, None, "an Owns edge has no bytes label");
    assert_eq!(owns.tone, None, "bytes have no tone");
}

#[test]
fn scalers_are_not_in_the_data_path() {
    let graph = hand_built(
        &[
            (TopologyKind::HorizontalPodAutoscaler, "api"),
            (TopologyKind::Deployment, "api"),
            (TopologyKind::Pod, "p"),
        ],
        &[(0, 1, Relation::Owns), (1, 2, Relation::Owns)],
    );
    let pods = vec![pod("p", &[], None)];
    let mut sample = bytes_sample();
    sample.readings[0].1 = Ok(TrafficReading {
        rates: vec![pod_rate("p", 700., 70.)],
        was_cut: false,
    });
    let overlay = traffic_overlay(&graph, &[], &refs(&pods), &sample);
    assert_eq!(overlay.edges[0], EdgeTraffic::Idle);
    assert_eq!(overlay.nodes[0], None);
    assert!(matches!(overlay.edges[1], EdgeTraffic::Flow(_)));
}

#[test]
fn mounts_and_access_are_hidden() {
    let pod = pod_with(
        pod("api-0", &[], Some(("ReplicaSet", "api-rs"))),
        &[Ref::MountSecret("tls")],
    );
    let fixture = Fixture::default()
        .with_rbac()
        .with_deployment("api", 1, 1)
        .with_replica_set("api-rs", Some("api"), 1, 1)
        .with_secret("tls", "Opaque")
        .with_service_account(crate::topology_fixtures::service_account("default"))
        .with_pod(pod);
    let graph = fixture.graph();
    let pods = fixture.pods.clone().expect("pods listed");
    let overlay = traffic_overlay(&graph, &[], &refs(&pods), &bytes_sample());
    let mut hidden = 0;
    for (edge, traffic) in graph.edges.iter().zip(&overlay.edges) {
        match edge.relation {
            Relation::Mounts | Relation::Access => {
                assert_eq!(traffic, &EdgeTraffic::Hidden, "{edge:?}");
                hidden += 1;
            }
            Relation::Owns | Relation::RoutesTo | Relation::Calls => {
                assert_ne!(traffic, &EdgeTraffic::Hidden, "{edge:?}");
            }
        }
    }
    assert!(hidden >= 2, "the fixture has a Mounts and an Access edge");
}

#[test]
fn width_scales_by_square_root_per_unit() {
    assert_eq!(flow_width(100., 100.), 6.);
    assert_eq!(flow_width(25., 100.), 3.75);
    assert_eq!(flow_width(0., 100.), 1.5);
    let (graph, calls, overlay) = overlay_of(&both());
    let calls_start = graph.edges.len();
    let widths: Vec<f32> = overlay
        .edges
        .iter()
        .filter_map(|edge| match edge {
            EdgeTraffic::Flow(flow) => Some(flow.width),
            EdgeTraffic::Hidden | EdgeTraffic::Idle => None,
        })
        .collect();
    assert!(widths.iter().all(|width| (1.5..=6.).contains(width)));
    // Each unit has its own busiest flow at 6: the 35 req/s call, and the 1.5 MB/s edges.
    let busiest_call = flow_of(&overlay.edges[calls_start]);
    let other_call = flow_of(&overlay.edges[calls_start + 1]);
    assert_eq!(calls.len(), 2);
    let (big, small) = if busiest_call.rate > other_call.rate {
        (busiest_call, other_call)
    } else {
        (other_call, busiest_call)
    };
    assert_eq!((big.unit, big.width), (TrafficUnit::Requests, 6.));
    assert!(small.width < 6. && small.width > 1.5);
    let bytes_max = overlay
        .edges
        .iter()
        .filter_map(|edge| match edge {
            EdgeTraffic::Flow(flow) if flow.unit == TrafficUnit::Bytes => Some(flow.width),
            _ => None,
        })
        .fold(0., f32::max);
    assert_eq!(bytes_max, 6.);
}

#[test]
fn tone_thresholds() {
    assert_eq!(error_tone(0.009), None);
    assert_eq!(error_tone(0.01), Some(StatusTone::Warn));
    assert_eq!(error_tone(0.049), Some(StatusTone::Warn));
    assert_eq!(error_tone(0.05), Some(StatusTone::Bad));
    let (graph, calls, overlay) = overlay_of(&both());
    let flow_for = |kind, name, service| {
        let from = node(&graph, kind, name);
        let to = node(&graph, TopologyKind::Service, service);
        let index = calls
            .iter()
            .position(|edge| edge.from == from && edge.to == to)
            .expect("a call edge");
        flow_of(&overlay.edges[graph.edges.len() + index]).clone()
    };
    let ledger = flow_for(TopologyKind::StatefulSet, "ledger", "payments-api");
    assert_eq!(ledger.tone, Some(StatusTone::Bad), "6 % is Bad");
    let api = flow_for(TopologyKind::Deployment, "api", "ledger");
    assert_eq!(api.tone, None, "0.4 % is below Warn");
    for edge in &overlay.edges {
        if let EdgeTraffic::Flow(flow) = edge
            && flow.unit == TrafficUnit::Bytes
        {
            assert_eq!(flow.tone, None);
        }
    }
}

#[test]
fn labels_per_unit_and_relation() {
    assert_eq!(request_text(35., Some(0.06)), "35 req/s \u{b7} 6% 5xx");
    assert_eq!(request_text(4., Some(0.)), "4.0 req/s");
    assert_eq!(
        request_text(12., Some(0.05 / 12.)),
        "12 req/s \u{b7} 0.4% 5xx"
    );
    assert_eq!(
        request_text(12., Some(0.0005)),
        "12 req/s",
        "below 0.1 % it is left out"
    );
    assert_eq!(request_text(12., None), "12 req/s");
    let (graph, calls, overlay) = overlay_of(&istio_sample());
    let from = node(&graph, TopologyKind::StatefulSet, "ledger");
    let to = node(&graph, TopologyKind::Service, "payments-api");
    let index = calls
        .iter()
        .position(|edge| (edge.from, edge.to) == (from, to))
        .expect("a call edge");
    let flow = flow_of(&overlay.edges[graph.edges.len() + index]);
    assert_eq!(flow.label.as_deref(), Some("35 req/s \u{b7} 6% 5xx"));
    assert_eq!(
        flow_label(1_000., TrafficUnit::Bytes, None, Relation::Owns),
        None
    );
    assert_eq!(
        flow_label(1_000., TrafficUnit::Bytes, None, Relation::RoutesTo).as_deref(),
        Some("1 KB/s")
    );
}

#[test]
fn label_anchor_is_the_arc_length_midpoint() {
    let at = |x, y| GraphPoint { x, y };
    // An L: 10 across and 10 down; the middle is the corner.
    let l_shape = EdgeRoute {
        points: vec![at(0., 0.), at(10., 0.), at(10., 10.)],
    };
    assert_eq!(label_anchor(&l_shape), at(10., 0.));
    // Unequal legs: 2 across and 10 down, the middle is 4 down the second leg.
    let uneven = EdgeRoute {
        points: vec![at(0., 0.), at(2., 0.), at(2., 10.)],
    };
    assert_eq!(label_anchor(&uneven), at(2., 4.));
    // A flattened S-curve is symmetric about the middle of its box, so that is where its arc
    // length halves.
    let curve = crate::topology_stroke::flatten_cubic(
        gpui_kit::point(0., 0.),
        gpui_kit::point(50., 0.),
        gpui_kit::point(50., 50.),
        gpui_kit::point(100., 50.),
        0.1,
    );
    let curved = EdgeRoute {
        points: curve.into_iter().map(|p| at(p.x, p.y)).collect(),
    };
    let middle = label_anchor(&curved);
    assert!(
        (middle.x - 50.).abs() < 0.3 && (middle.y - 25.).abs() < 0.3,
        "{middle:?}"
    );
}

#[test]
fn bad_node_keeps_its_caption() {
    let (graph, _, overlay) = overlay_of(&bytes_sample());
    let crashing = node(&graph, TopologyKind::Pod, "api-7d9f-t5bcm");
    let healthy = node(&graph, TopologyKind::Pod, "ledger-0");
    let bad = &graph.nodes[crashing];
    assert!(matches!(bad.tone, Some(StatusTone::Bad | StatusTone::Warn)));
    let traffic = overlay.nodes[crashing].as_ref();
    assert_eq!(shown_caption(bad, traffic), bad.caption);
    assert!(
        tooltip_with_traffic(&bad.name, traffic).ends_with("\u{2193} 0 B/s  \u{2191} 0 B/s"),
        "the tooltip still carries the traffic"
    );
    let ok = &graph.nodes[healthy];
    let text = overlay.nodes[healthy].as_ref().expect("traffic");
    assert_eq!(text.text, "\u{2193} 50 KB/s  \u{2191} 20 KB/s");
    assert_eq!(shown_caption(ok, Some(text)), text.text);
    assert_eq!(shown_caption(ok, None), ok.caption);
}

#[test]
fn node_text_prefers_requests() {
    let (graph, _, overlay) = overlay_of(&both());
    let service = node(&graph, TopologyKind::Service, "payments-api");
    let traffic = overlay.nodes[service].as_ref().expect("traffic");
    // 35 + 4 + 80 req/s reach the Service from the workload, an unknown caller, and a peer
    // outside the namespace; 2.1 of them fail.
    assert_eq!(traffic.requests, Some(119.));
    assert_eq!(traffic.text, "119 req/s \u{b7} 1.8% 5xx");
    assert_eq!(traffic.receive, Some(1_500_000.));
}

#[test]
fn failed_and_cut_readings_are_noted() {
    let mut sample = both();
    sample.readings[0].1 = Err(MetricsError::TimedOut);
    sample.readings[1].1 = Ok(TrafficReading {
        rates: Vec::new(),
        was_cut: true,
    });
    let (_, _, overlay) = overlay_of(&sample);
    assert_eq!(overlay.sources, [TrafficSourceKind::PodNetwork]);
    assert_eq!(overlay.notes.len(), 2);
    assert!(
        overlay.notes[0].starts_with("Istio: "),
        "{:?}",
        overlay.notes
    );
    assert!(overlay.notes[1].starts_with("pod network bytes: more than 2,000"));
}

#[test]
fn sources_follow_the_answers() {
    let (_, _, overlay) = overlay_of(&both());
    assert_eq!(
        overlay.sources,
        [TrafficSourceKind::Istio, TrafficSourceKind::PodNetwork]
    );
    assert!(overlay.notes.is_empty());
}

#[test]
fn a_pod_gone_from_the_list_is_not_an_outside_peer() {
    let (graph, mut pods) = namespace();
    // The pod ended after the sample was read: it is no longer in the list.
    pods.retain(|pod| pod.name != "ledger-0");
    let sample = bytes_sample();
    let overlay = traffic_overlay(&graph, &[], &refs(&pods), &sample);
    assert!(overlay.outside.is_empty(), "{:?}", overlay.outside);
    let ledger = node(&graph, TopologyKind::StatefulSet, "ledger");
    assert_eq!(overlay.nodes[ledger], None, "its bytes are not counted");
    // A pod the graph does not draw is skipped the same way.
    let hidden = Fixture::default().with_pod(pod("p", &[], None)).graph();
    let only_pods = [pod("p", &[], None)];
    let rates = refs(&only_pods);
    let mut only_p = bytes_sample();
    only_p.readings[0].1 = Ok(TrafficReading {
        rates: vec![pod_rate("p", 5., 5.)],
        was_cut: false,
    });
    let overlay = traffic_overlay(&hidden, &[], &rates, &only_p);
    assert!(overlay.outside.is_empty());
}
