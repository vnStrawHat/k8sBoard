use std::collections::HashMap;

use super::*;
use crate::topology_canvas::{ARROW_HALF_WIDTH, ARROW_LENGTH, ARROW_TIP_GAP, arrow_head};
use crate::topology_fixtures::{Fixture, Ref, ingress, pod, pod_with};
use crate::topology_graph::Relation;
use crate::topology_graph::{GroupBy, TopologyGraph};
use crate::topology_layout::{NODE_HEIGHT, TopologyLayout, layout};

fn app_pod(app: &str, n: usize, refs: &[Ref]) -> cluster::PodSummary {
    let labels = format!("app={app}");
    let owner = format!("{app}-rs");
    pod_with(
        pod(
            &format!("{app}-{n}"),
            &[labels.as_str()],
            Some(("ReplicaSet", owner.as_str())),
        ),
        refs,
    )
}

/// One app: a Service and a Deployment, a ReplicaSet, and pods that mount a Secret.
fn with_app(fixture: Fixture, app: &str, pods: usize, refs: &[Ref]) -> Fixture {
    let selector = format!("app={app}");
    let mut fixture = fixture
        .with_service(app, &[selector.as_str()])
        .with_deployment(app, 1, 1)
        .with_replica_set(&format!("{app}-rs"), Some(app), 1, 1);
    for n in 0..pods {
        fixture = fixture.with_pod(app_pod(app, n, refs));
    }
    fixture
}

/// Three apps that share one Secret, like the keda namespace.
fn keda() -> TopologyGraph {
    let mut fixture = Fixture::default().with_secret("kedaorg-certs", "Opaque");
    for app in [
        "keda-admission-webhooks",
        "keda-operator",
        "keda-metrics-apiserver",
    ] {
        fixture = with_app(fixture, app, 1, &[Ref::MountSecret("kedaorg-certs")]);
    }
    fixture.graph()
}

/// Many apps, ingresses, statefulsets, and config shared across apps, like the monitoring
/// namespace: long edges cross bands, and columns have different heights.
fn monitoring() -> TopologyGraph {
    let shared = [
        Ref::MountSecret("monitoring-tls"),
        Ref::EnvConfigMap("common"),
    ];
    let mut fixture = Fixture::default()
        .with_secret("monitoring-tls", "kubernetes.io/tls")
        .with_config_map("common");
    for (index, app) in [
        "grafana",
        "vmalert",
        "vmagent",
        "vmquery",
        "vminsert",
        "alertmanager",
        "node-exporter",
    ]
    .into_iter()
    .enumerate()
    {
        let own_config = format!("{app}-config");
        fixture = fixture.with_config_map(&own_config);
        let refs = [shared[0], shared[1], Ref::EnvConfigMap(&own_config)];
        fixture = with_app(fixture, app, 2 + index % 3, &refs);
        if index % 2 == 0 {
            fixture = fixture.with_ingress(ingress(
                &format!("{app}-ingress"),
                &[("/", app)],
                None,
                None,
            ));
        }
    }
    for app in ["vmselect", "vmstorage"] {
        let selector = format!("app={app}");
        fixture = fixture
            .with_service(app, &[selector.as_str()])
            .with_stateful_set(app, 3, 3);
        for n in 0..3 {
            let name = format!("{app}-{n}");
            fixture = fixture.with_pod(pod_with(
                pod(&name, &[selector.as_str()], Some(("StatefulSet", app))),
                &[Ref::MountSecret("monitoring-tls")],
            ));
        }
    }
    fixture.graph()
}

fn laid_out(graph: &TopologyGraph, group_by: GroupBy, aspect: f32) -> TopologyLayout {
    layout(graph, group_by, aspect, &HashMap::new(), None)
}

fn fixtures() -> [(&'static str, TopologyGraph); 2] {
    [("keda", keda()), ("monitoring", monitoring())]
}

const ARRANGEMENTS: [(GroupBy, f32); 3] = [
    (GroupBy::App, 0.1),
    (GroupBy::App, 1.7),
    (GroupBy::Components, 1.),
];

/// Every segment of every route, against every card that is not one of its ends.
fn crossings(graph: &TopologyGraph, layout: &TopologyLayout) -> Vec<String> {
    let mut found = Vec::new();
    for (index, edge) in graph.edges.iter().enumerate() {
        let route = &layout.routes[index];
        for (card, rect) in layout.rects.iter().enumerate() {
            if card == edge.from || card == edge.to {
                continue;
            }
            for pair in route.points.windows(2) {
                if crosses(pair[0], pair[1], *rect, 0.) {
                    found.push(format!(
                        "{:?} -> {:?} crosses {:?}",
                        graph.nodes[edge.from].id, graph.nodes[edge.to].id, graph.nodes[card].id
                    ));
                    break;
                }
            }
        }
    }
    found
}

fn card(x: f32, y: f32) -> GraphRect {
    GraphRect {
        origin: GraphPoint { x, y },
        width: 160.,
        height: NODE_HEIGHT,
    }
}

/// The curve of the default sides of `source` and `target`.
fn curve(source: GraphRect, target: GraphRect) -> Vec<GraphPoint> {
    let (start_side, end_side) = facing_sides(source, target);
    bezier(Anchors::between(source, start_side, target, end_side))
}

// ---- the anchor rule ----

#[test]
fn a_card_has_four_anchors_at_the_middles_of_its_sides() {
    let rect = card(100., 40.);
    let at = |side: Side| side.anchor(rect);
    assert_eq!(at(Side::Left), GraphPoint { x: 100., y: 70. });
    assert_eq!(at(Side::Right), GraphPoint { x: 260., y: 70. });
    assert_eq!(at(Side::Top), GraphPoint { x: 180., y: 40. });
    assert_eq!(at(Side::Bottom), GraphPoint { x: 180., y: 100. });
}

#[test]
fn the_default_sides_follow_the_larger_gap() {
    let source = card(0., 0.);
    let cases = [
        // Beside it, level or a little off: right to left, or left to right.
        (card(300., 0.), Side::Right, Side::Left),
        (card(300., 90.), Side::Right, Side::Left),
        (card(-300., 20.), Side::Left, Side::Right),
        // Stacked in one column: bottom to top, or top to bottom.
        (card(0., 200.), Side::Bottom, Side::Top),
        (card(30., -200.), Side::Top, Side::Bottom),
        // Offset both ways: the larger gap wins.
        (card(200., 400.), Side::Bottom, Side::Top),
        (card(400., 200.), Side::Right, Side::Left),
        // A tie is horizontal: gaps of 200 on both axes.
        (card(360., 260.), Side::Right, Side::Left),
    ];
    for (target, start_side, end_side) in cases {
        assert_eq!(
            facing_sides(source, target),
            (start_side, end_side),
            "{target:?}"
        );
    }
}

#[test]
fn overlapping_cards_are_joined_across_the_axis_they_overlap_less_on() {
    // Both gaps are negative (-60 across, -40 down): the vertical one is nearer to a real gap.
    assert_eq!(
        facing_sides(card(0., 0.), card(100., 20.)),
        (Side::Bottom, Side::Top)
    );
}

// ---- the curve ----

#[test]
fn a_horizontal_curve_leaves_flat_and_ends_in_a_horizontal_stub() {
    let points = curve(card(0., 0.), card(400., 300.));
    let last = points.len() - 1;
    assert!((points[1].y - points[0].y).abs() < 0.5);
    let stub = points[last].x - points[last - 1].x;
    assert!((stub - ARRIVAL_STUB).abs() < 1e-3, "{stub}");
    assert!((points[last].y - points[last - 1].y).abs() < 1e-3);
}

#[test]
fn a_vertical_curve_leaves_and_enters_vertically() {
    let (source, target) = (card(0., 0.), card(40., 300.));
    let points = curve(source, target);
    let last = points.len() - 1;
    assert_eq!(points[0], Side::Bottom.anchor(source));
    assert_eq!(points[last], Side::Top.anchor(target));
    assert!((points[1].x - points[0].x).abs() < 0.5);
    let stub = points[last].y - points[last - 1].y;
    assert!((stub - ARRIVAL_STUB).abs() < 1e-3, "{stub}");
    assert!((points[last].x - points[last - 1].x).abs() < 1e-3);
}

#[test]
fn a_same_column_edge_runs_straight_down_from_bottom_to_top() {
    let (source, target) = (card(0., 0.), card(0., 200.));
    let points = curve(source, target);
    // No loop into the gutter: it stays on the middle line and only goes down.
    for at in &points {
        assert!((at.x - source.center().x).abs() < 1e-3, "{at:?}");
    }
    for pair in points.windows(2) {
        assert!(pair[1].y >= pair[0].y - 1e-3, "{pair:?}");
    }
}

#[test]
fn a_backward_edge_is_one_s_curve() {
    let points = curve(card(400., 0.), card(0., 200.));
    // Leftward and downward only: no loop and no wiggle.
    for pair in points.windows(2) {
        assert!(pair[1].x <= pair[0].x + 1e-3, "{pair:?}");
        assert!(pair[1].y >= pair[0].y - 1e-3, "{pair:?}");
    }
    assert_eq!(points[0].x, 400.);
    assert_eq!(points[points.len() - 1].x, 160.);
}

#[test]
fn a_close_neighbour_curve_never_runs_backwards() {
    let graph = Fixture::default()
        .with_deployment("web", 1, 1)
        .with_replica_set("web-rs", Some("web"), 1, 1)
        .graph();
    let owns = graph
        .edges
        .iter()
        .position(|edge| edge.relation == Relation::Owns)
        .expect("a deployment owns the replica set");
    let (from, to) = (graph.edges[owns].from, graph.edges[owns].to);
    let first = laid_out(&graph, GroupBy::Components, 1.);
    // A 25-unit gap, as after a drag; a 20-37 gap is where a floored reach would turn back.
    let source = first.rects[from];
    let pins = HashMap::from([
        (graph.nodes[from].id.clone(), GraphPoint { x: 0., y: 0. }),
        (
            graph.nodes[to].id.clone(),
            GraphPoint {
                x: source.width + 25.,
                y: 50.,
            },
        ),
    ]);
    let arranged = layout(&graph, GroupBy::Components, 1., &pins, None);
    for pair in arranged.routes[owns].points.windows(2) {
        assert!(pair[1].x >= pair[0].x - 1e-3, "{pair:?}");
    }
}

#[test]
fn the_arrow_of_a_curve_points_along_its_end_tangent() {
    // The arrow arrives perpendicular to the side it enters: horizontal into a left or right
    // side, vertical into a top or bottom side.
    let cases = [
        (card(0., 0.), card(400., 300.), (1., 0.)),
        (card(400., 0.), card(0., 200.), (-1., 0.)),
        (card(0., 0.), card(0., 200.), (0., 1.)),
        (card(0., 300.), card(40., 0.), (0., -1.)),
    ];
    for (source, target, expected) in cases {
        let route = EdgeRoute {
            points: curve(source, target),
        };
        let [tip, left, right] = arrow_head(&route, ARROW_LENGTH, ARROW_HALF_WIDTH);
        let end = route.end();
        let (dx, dy) = route.end_direction();
        assert!((dx - expected.0).abs() < 1e-3 && (dy - expected.1).abs() < 1e-3);
        assert!((tip.x - (end.x - dx * ARROW_TIP_GAP)).abs() < 1e-3);
        assert!((tip.y - (end.y - dy * ARROW_TIP_GAP)).abs() < 1e-3);
        // The base corners sit across the axis, centered on it.
        let back = ARROW_TIP_GAP + ARROW_LENGTH;
        assert!((left.x + right.x - 2. * (end.x - dx * back)).abs() < 1e-3);
        assert!((left.y + right.y - 2. * (end.y - dy * back)).abs() < 1e-3);
    }
}

#[test]
fn arrival_stub_is_the_arrow_length_and_gap() {
    assert_eq!(ARRIVAL_STUB, ARROW_LENGTH + ARROW_TIP_GAP);
}

#[test]
fn a_blocked_edge_takes_another_pair_of_anchors() {
    // A card sits right between the two on their row: the straight S would run behind it.
    let (source, blocker, target) = (card(0., 0.), card(280., 0.), card(560., 0.));
    let rects = [source, target, blocker];
    let route = route_edge(0, 1, &rects);
    let (start_side, end_side) = facing_sides(source, target);
    let preferred = bezier(Anchors::between(source, start_side, target, end_side));
    assert_ne!(route.points, preferred);
    for pair in route.points.windows(2) {
        assert!(!crosses(pair[0], pair[1], blocker, 0.));
    }
    let start = route.start();
    assert!(Side::ALL.iter().any(|side| side.anchor(source) == start));
}

// ---- every edge is one curve ----

#[test]
fn every_route_is_a_curve_between_two_anchors() {
    // Regression: a blocked edge used to fall back to a right-angle lane route with rounded
    // corners, so some edges stayed elbows. Every route is now the curve of some pair of anchors.
    for (name, graph) in fixtures() {
        for (group_by, aspect) in ARRANGEMENTS {
            let arranged = laid_out(&graph, group_by, aspect);
            for (index, edge) in graph.edges.iter().enumerate() {
                let (source, target) = (arranged.rects[edge.from], arranged.rects[edge.to]);
                let route = &arranged.routes[index];
                let is_curve = Side::ALL.into_iter().any(|a| {
                    Side::ALL.into_iter().any(|b| {
                        let anchors = Anchors::between(source, a, target, b);
                        [anchors, anchors.tight()]
                            .iter()
                            .any(|anchors| bezier(*anchors) == route.points)
                    })
                });
                assert!(is_curve, "{name} {group_by:?} {aspect}: edge {index}");
            }
        }
    }
}

#[test]
fn routes_start_and_end_at_anchors_in_edge_order() {
    for (name, graph) in fixtures() {
        let arranged = laid_out(&graph, GroupBy::App, 1.7);
        assert_eq!(arranged.routes.len(), graph.edges.len());
        for (route, edge) in arranged.routes.iter().zip(&graph.edges) {
            for (at, card) in [(route.start(), edge.from), (route.end(), edge.to)] {
                let rect = arranged.rects[card];
                let is_anchor = Side::ALL.iter().any(|side| side.anchor(rect) == at);
                assert!(is_anchor, "{name} {at:?}");
            }
        }
    }
}

#[test]
fn curves_keep_clear_of_most_cards_they_do_not_join() {
    // A long edge across packed columns passes behind cards when no pair of anchors is clear
    // (under half of the edge and card pairs in the big fixture); the bound catches a regression
    // in the anchor search.
    for (name, graph) in fixtures() {
        for (group_by, aspect) in ARRANGEMENTS {
            let arranged = laid_out(&graph, group_by, aspect);
            let found = crossings(&graph, &arranged);
            assert!(
                found.len() * 2 <= graph.edges.len(),
                "{name} {group_by:?} {aspect}: {} of {} edges, e.g. {:?}",
                found.len(),
                graph.edges.len(),
                found.first()
            );
        }
    }
}

#[test]
fn a_route_ends_with_the_direction_it_arrives_in() {
    let route = EdgeRoute {
        points: vec![
            GraphPoint { x: 0., y: 0. },
            GraphPoint { x: 10., y: 0. },
            GraphPoint { x: 10., y: 0. },
        ],
    };
    // A repeated last point has no direction of its own.
    assert_eq!(route.end_direction(), (1., 0.));
    assert_eq!(route.bounds(), (0., 0., 10., 0.));
    let single = EdgeRoute {
        points: vec![GraphPoint { x: 3., y: 4. }],
    };
    assert_eq!(single.end_direction(), (1., 0.));
}

#[test]
fn crosses_tells_a_segment_through_a_rect_from_one_beside_it() {
    let rect = GraphRect {
        origin: GraphPoint { x: 10., y: 10. },
        width: 20.,
        height: 20.,
    };
    let at = |x: f32, y: f32| GraphPoint { x, y };
    assert!(crosses(at(0., 20.), at(40., 20.), rect, 0.));
    assert!(crosses(at(20., 0.), at(20., 40.), rect, 0.));
    assert!(crosses(at(12., 12.), at(14., 14.), rect, 0.));
    assert!(!crosses(at(0., 5.), at(40., 5.), rect, 0.));
    assert!(!crosses(at(5., 0.), at(5., 40.), rect, 0.));
    // The clearance grows the rect.
    assert!(crosses(at(0., 8.), at(40., 8.), rect, 4.));
    assert!(!crosses(at(0., 8.), at(40., 8.), rect, 1.));
    // A negative margin shrinks it: a segment along the edge no longer touches.
    assert!(crosses(at(0., 10.), at(40., 10.), rect, 0.));
    assert!(!crosses(at(0., 10.), at(40., 10.), rect, -1.));
    // A segment that stops short of the rect does not cross it.
    assert!(!crosses(at(0., 20.), at(5., 20.), rect, 0.));
}

#[test]
fn route_edges_routes_a_slice() {
    let graph = monitoring();
    let layout = laid_out(&graph, GroupBy::Components, 1.6);
    let routes = route_edges(&graph.edges, &layout.rects);
    assert_eq!(routes, layout.routes);
    let half = graph.edges.len() / 2;
    let tail = route_edges(&graph.edges[half..], &layout.rects);
    assert_eq!(tail, layout.routes[half..], "a slice routes alone");
}
