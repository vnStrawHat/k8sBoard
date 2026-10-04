use std::collections::HashMap;

use super::*;
use crate::topology_canvas::{ARROW_HALF_WIDTH, ARROW_LENGTH, ARROW_TIP_GAP, arrow_head};
use crate::topology_fixtures::{Fixture, Ref, ingress, pod, pod_with};
use crate::topology_graph::TopologyKind;
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
    laid_out_as(graph, group_by, aspect, EdgeShape::Elbows)
}

fn laid_out_as(
    graph: &TopologyGraph,
    group_by: GroupBy,
    aspect: f32,
    shape: EdgeShape,
) -> TopologyLayout {
    layout(graph, group_by, aspect, &HashMap::new(), None, shape)
}

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

#[test]
fn no_edge_runs_through_a_card_it_does_not_join() {
    for (name, graph) in [("keda", keda()), ("monitoring", monitoring())] {
        for (group_by, aspect) in [
            (GroupBy::App, 0.1),
            (GroupBy::App, 1.7),
            (GroupBy::Components, 1.),
        ] {
            let arranged = laid_out(&graph, group_by, aspect);
            let found = crossings(&graph, &arranged);
            assert!(
                found.is_empty(),
                "{name} {group_by:?} {aspect}: {} crossings, e.g. {:?}",
                found.len(),
                found.first()
            );
        }
    }
}

#[test]
fn the_fixtures_are_big_enough_to_test_the_routing() {
    let graph = monitoring();
    assert!(graph.nodes.len() >= 60, "{}", graph.nodes.len());
    assert!(graph.edges.len() >= 80, "{}", graph.edges.len());
    let arranged = laid_out(&graph, GroupBy::App, 1.7);
    // Some edge skips a column or crosses bands, so some route has more than a curve's worth of
    // bends.
    assert!(arranged.routes.iter().any(|route| route.points.len() > 30));
    assert!(!keda().edges.is_empty());
}

#[test]
fn a_route_leaves_and_enters_by_the_side_of_its_cards() {
    for graph in [keda(), monitoring()] {
        let arranged = laid_out(&graph, GroupBy::App, 1.7);
        for (index, edge) in graph.edges.iter().enumerate() {
            let route = &arranged.routes[index];
            for (at, card) in [(route.start(), edge.from), (route.end(), edge.to)] {
                let rect = arranged.rects[card];
                let on_side =
                    (at.x - rect.origin.x).abs() < 1e-3 || (at.x - rect.right()).abs() < 1e-3;
                assert!(on_side, "{:?}", graph.nodes[card].id);
                assert!((at.y - rect.center().y).abs() < 1e-3);
            }
        }
    }
}

#[test]
fn neighbouring_columns_are_joined_by_one_smooth_curve() {
    let graph = Fixture::default()
        .with_service("web", &["app=web"])
        .with_deployment("web", 1, 1)
        .with_replica_set("web-rs", Some("web"), 1, 1)
        .with_pod(app_pod("web", 0, &[]))
        .graph();
    let arranged = laid_out(&graph, GroupBy::Components, 1.);
    let owns = graph
        .edges
        .iter()
        .position(|edge| {
            edge.relation == Relation::Owns
                && graph.nodes[edge.from].kind == crate::topology_graph::TopologyKind::Deployment
        })
        .expect("a deployment owns the replica set");
    let route = &arranged.routes[owns];
    // A curve runs monotonically to the right, flat at both ends.
    assert!(
        route
            .points
            .windows(2)
            .all(|pair| pair[1].x >= pair[0].x - 1e-3)
    );
    let flat = |a: GraphPoint, b: GraphPoint| (a.y - b.y).abs() < 0.5;
    assert!(flat(route.points[0], route.points[1]));
    let last = route.points.len() - 1;
    assert!(flat(route.points[last - 1], route.points[last]));
}

#[test]
fn a_mounts_edge_leaves_by_the_side_and_runs_in_a_gutter() {
    let graph = keda();
    let arranged = laid_out(&graph, GroupBy::App, 0.1);
    let mounts: Vec<usize> = graph
        .edges
        .iter()
        .enumerate()
        .filter(|(_, edge)| edge.relation == Relation::Mounts)
        .map(|(index, _)| index)
        .collect();
    assert!(!mounts.is_empty());
    for index in mounts {
        let route = &arranged.routes[index];
        let source = arranged.rects[graph.edges[index].from];
        // The first segment goes sideways, out of the card, into the gutter.
        let (a, b) = (route.points[0], route.points[1]);
        assert!((a.y - b.y).abs() < 1e-3 && (b.x - a.x).abs() > 1.);
        let lane = (source.right() + LANE_OFFSET, source.origin.x - LANE_OFFSET);
        let reaches_lane = route
            .points
            .iter()
            .any(|at| (at.x - lane.0).abs() < 1e-3 || (at.x - lane.1).abs() < 1e-3);
        assert!(reaches_lane);
    }
}

#[test]
fn an_edge_that_skips_a_column_bends_around_the_cards_between() {
    // Service (column 1) to pod (column 3) with a ReplicaSet in column 2 between them.
    let graph = with_app(Fixture::default(), "web", 1, &[]).graph();
    let arranged = laid_out(&graph, GroupBy::Components, 1.);
    let skip = graph
        .edges
        .iter()
        .position(|edge| {
            edge.relation == Relation::RoutesTo
                && graph.nodes[edge.from].kind == crate::topology_graph::TopologyKind::Service
        })
        .expect("a service routes to the pod");
    let route = &arranged.routes[skip];
    let (source, target) = (
        arranged.rects[graph.edges[skip].from],
        arranged.rects[graph.edges[skip].to],
    );
    assert!(target.origin.x > source.right() + 200.);
    assert!(crossings(&graph, &arranged).is_empty());
    // It leaves the gutter vertically: some point is above or below the source card's row.
    assert!(
        route
            .points
            .iter()
            .any(|at| (at.y - source.center().y).abs() > 20.)
    );
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
fn rounded_corners_keep_the_ends_and_cut_the_corner() {
    let corner = [
        GraphPoint { x: 0., y: 0. },
        GraphPoint { x: 50., y: 0. },
        GraphPoint { x: 50., y: 50. },
    ];
    let rounded = round_corners(&corner);
    assert_eq!(rounded[0], corner[0]);
    assert_eq!(rounded[rounded.len() - 1], corner[2]);
    // No point of the curve is the sharp corner, and all stay inside its box.
    assert!(rounded.iter().all(|at| *at != corner[1]));
    assert!(
        rounded
            .iter()
            .all(|at| at.x <= 50. + 1e-3 && at.y >= -1e-3 && at.y <= 50. + 1e-3)
    );
    let nearest = rounded
        .iter()
        .map(|at| distance(*at, corner[1]))
        .fold(f32::INFINITY, f32::min);
    assert!(nearest > 1.);
}

#[test]
fn a_short_segment_limits_the_corner_radius() {
    let corner = [
        GraphPoint { x: 0., y: 0. },
        GraphPoint { x: 6., y: 0. },
        GraphPoint { x: 6., y: 40. },
    ];
    let rounded = round_corners(&corner);
    // Half the short segment: the curve never starts before the middle of it.
    assert!(rounded.iter().all(|at| at.x >= 0. - 1e-3));
    assert!(rounded[1].x >= 3. - 1e-3);
}

#[test]
fn simplified_drops_repeats_and_straight_runs() {
    let points = vec![
        GraphPoint { x: 0., y: 0. },
        GraphPoint { x: 0., y: 0. },
        GraphPoint { x: 5., y: 0. },
        GraphPoint { x: 10., y: 0. },
        GraphPoint { x: 10., y: 7. },
    ];
    assert_eq!(
        simplified(points),
        vec![
            GraphPoint { x: 0., y: 0. },
            GraphPoint { x: 10., y: 0. },
            GraphPoint { x: 10., y: 7. },
        ]
    );
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
    // A segment that stops short of the rect does not cross it.
    assert!(!crosses(at(0., 20.), at(5., 20.), rect, 0.));
}

#[test]
fn routes_are_computed_for_every_edge_in_edge_order() {
    let graph = keda();
    let arranged = laid_out(&graph, GroupBy::App, 1.);
    assert_eq!(arranged.routes.len(), graph.edges.len());
    assert!(arranged.routes.iter().all(|route| route.points.len() >= 2));
}

// ---- 0050: curved edges ----

fn card(x: f32, y: f32) -> GraphRect {
    GraphRect {
        origin: GraphPoint { x, y },
        width: 160.,
        height: NODE_HEIGHT,
    }
}

fn curve(source: GraphRect, target: GraphRect) -> Vec<GraphPoint> {
    bezier(ports(source, target))
}

#[test]
fn a_curve_leaves_flat_and_ends_in_a_horizontal_stub() {
    let points = curve(card(0., 0.), card(400., 300.));
    let last = points.len() - 1;
    // Flat where it leaves, within the flatness tolerance of the first chord.
    assert!((points[1].y - points[0].y).abs() < 0.5);
    let stub = points[last].x - points[last - 1].x;
    assert!((stub - ARRIVAL_STUB).abs() < 1e-3, "{stub}");
    assert!((points[last].y - points[last - 1].y).abs() < 1e-3);
}

#[test]
fn a_same_column_edge_is_a_loop_in_the_gutter() {
    let source = card(0., 0.);
    let points = curve(source, card(0., 200.));
    // Out of the right side, round the gutter, and back in by the right side.
    for at in &points {
        assert!(at.x >= source.right() - 1e-3, "{at:?}");
        assert!(at.x <= source.right() + LANE_OFFSET + 1., "{at:?}");
    }
    let peak = points.iter().map(|at| at.x).fold(f32::MIN, f32::max);
    assert!(peak > source.right() + 20., "{peak}");
    assert_eq!(points[points.len() - 1].x, source.right());
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
    for shape in [EdgeShape::Elbows, EdgeShape::Curves] {
        let arranged = layout(&graph, GroupBy::Components, 1., &pins, None, shape);
        for pair in arranged.routes[owns].points.windows(2) {
            assert!(pair[1].x >= pair[0].x - 1e-3, "{shape:?} {pair:?}");
        }
    }
}

#[test]
fn the_arrow_of_a_curve_points_along_its_end_tangent() {
    let cases = [
        (card(0., 0.), card(400., 300.)),
        (card(0., 0.), card(0., 200.)),
        (card(400., 0.), card(0., 200.)),
    ];
    for (source, target) in cases {
        let route = EdgeRoute {
            points: curve(source, target),
        };
        let [tip, left, right] = arrow_head(&route, ARROW_LENGTH, ARROW_HALF_WIDTH);
        let end = route.end();
        let (dx, dy) = route.end_direction();
        assert!(dx.abs() == 1. && dy == 0., "{dx} {dy}");
        assert!((tip.x - (end.x - dx * ARROW_TIP_GAP)).abs() < 1e-3);
        assert!((tip.y - end.y).abs() < 1e-3);
        // A horizontal axis: the base corners share x and sit above and below the tip.
        assert!((left.x - right.x).abs() < 1e-3);
        assert!((left.y + right.y - 2. * end.y).abs() < 1e-3);
    }
}

#[test]
fn arrival_stub_is_the_arrow_length_and_gap() {
    assert_eq!(ARRIVAL_STUB, ARROW_LENGTH + ARROW_TIP_GAP);
}

fn fixtures() -> [(&'static str, TopologyGraph); 2] {
    [("keda", keda()), ("monitoring", monitoring())]
}

#[test]
fn curves_keep_the_ports_of_elbows() {
    for (name, graph) in fixtures() {
        let elbows = laid_out_as(&graph, GroupBy::App, 1.7, EdgeShape::Elbows);
        let curves = laid_out_as(&graph, GroupBy::App, 1.7, EdgeShape::Curves);
        for (index, (a, b)) in elbows.routes.iter().zip(&curves.routes).enumerate() {
            assert!(distance(a.start(), b.start()) < 1e-3, "{name} {index}");
            assert!(distance(a.end(), b.end()) < 1e-3, "{name} {index}");
        }
    }
}

#[test]
fn no_curved_edge_runs_through_a_card_it_does_not_join() {
    for (name, graph) in fixtures() {
        for (group_by, aspect) in [
            (GroupBy::App, 0.1),
            (GroupBy::App, 1.7),
            (GroupBy::Components, 1.),
        ] {
            let arranged = laid_out_as(&graph, group_by, aspect, EdgeShape::Curves);
            let found = crossings(&graph, &arranged);
            assert!(
                found.is_empty(),
                "{name} {group_by:?} {aspect}: {} crossings, e.g. {:?}",
                found.len(),
                found.first()
            );
        }
    }
}

#[test]
fn a_blocked_curve_keeps_the_lane_route() {
    // The ReplicaSet sits between the Service and the pod: the straight curve would run behind it.
    let graph = with_app(Fixture::default(), "web", 1, &[]).graph();
    let curves = laid_out_as(&graph, GroupBy::Components, 1., EdgeShape::Curves);
    let elbows = laid_out_as(&graph, GroupBy::Components, 1., EdgeShape::Elbows);
    let skip = graph
        .edges
        .iter()
        .position(|edge| {
            edge.relation == Relation::RoutesTo
                && graph.nodes[edge.from].kind == TopologyKind::Service
        })
        .expect("a service routes to the pod");
    let (source, target) = (
        curves.rects[graph.edges[skip].from],
        curves.rects[graph.edges[skip].to],
    );
    assert_ne!(curves.routes[skip].points, curve(source, target));
    assert_eq!(curves.routes[skip], elbows.routes[skip]);
    assert!(crossings(&graph, &curves).is_empty());
}

#[test]
fn the_edge_shape_does_not_move_cards() {
    for (name, graph) in fixtures() {
        let elbows = laid_out_as(&graph, GroupBy::App, 1.7, EdgeShape::Elbows);
        let curves = laid_out_as(&graph, GroupBy::App, 1.7, EdgeShape::Curves);
        assert_eq!(elbows.rects, curves.rects, "{name}");
        assert_eq!(elbows.extent, curves.extent, "{name}");
    }
}

#[test]
fn route_edges_routes_a_slice() {
    let graph = monitoring();
    for shape in [EdgeShape::Elbows, EdgeShape::Curves] {
        let layout = laid_out_as(&graph, GroupBy::Components, 1.6, shape);
        let frames: Vec<GraphRect> = layout.bands.iter().map(|band| band.rect).collect();
        let routes = route_edges(&graph.edges, &layout.rects, &frames, shape);
        assert_eq!(routes, layout.routes, "{shape:?}");
        let half = graph.edges.len() / 2;
        let tail = route_edges(&graph.edges[half..], &layout.rects, &frames, shape);
        assert_eq!(
            tail,
            layout.routes[half..],
            "{shape:?}: a slice routes alone"
        );
    }
}
