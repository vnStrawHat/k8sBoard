use std::collections::HashMap;
use std::rc::Rc;

use super::*;
use crate::topology_fixtures::traffic_namespace;
use crate::topology_graph::GroupBy;
use crate::topology_layout::layout;
use crate::topology_route::EdgeShape;
use crate::topology_traffic::TrafficSample;
use crate::topology_traffic_fixture::{bytes_sample, istio_sample};

const CANVAS: (f32, f32) = (4_000., 4_000.);

struct Scene {
    graph: TopologyGraph,
    layout: TopologyLayout,
    layer: TrafficLayer,
}

fn scene(sample: TrafficSample, shape: EdgeShape) -> Scene {
    let fixture = traffic_namespace();
    let graph = fixture.graph();
    let pods = fixture.pods.clone().expect("pods listed");
    let arranged = layout(
        &graph,
        GroupBy::Components,
        1.6,
        &HashMap::new(),
        None,
        shape,
    );
    let refs: Vec<&cluster::PodSummary> = pods.iter().collect();
    let layer = TrafficLayer::build(&graph, &arranged, shape, &refs, Rc::new(sample));
    Scene {
        graph,
        layout: arranged,
        layer,
    }
}

fn zoom_one() -> Viewport {
    Viewport::default()
}

#[test]
fn flow_edges_get_a_label_that_clears_every_card() {
    for shape in [EdgeShape::Elbows, EdgeShape::Curves] {
        let scene = scene(istio_sample(), shape);
        let labels = edge_labels(
            &scene.graph,
            &scene.layout,
            &scene.layer,
            zoom_one(),
            CANVAS,
        );
        assert!(!labels.is_empty(), "{shape:?}");
        for label in &labels {
            for rect in &scene.layout.rects {
                let card = screen_box(zoom_one(), *rect);
                assert!(!label.overlaps(card), "{shape:?}: {label:?} over a card");
            }
        }
        for (index, label) in labels.iter().enumerate() {
            for other in &labels[index + 1..] {
                let area = (
                    other.left,
                    other.top,
                    other.left + other.width,
                    other.top + LABEL_HEIGHT,
                );
                assert!(!label.overlaps(area), "{shape:?}: two labels overlap");
            }
        }
    }
}

#[test]
fn bytes_labels_sit_on_routes_to_edges_only() {
    let scene = scene(bytes_sample(), EdgeShape::Elbows);
    let labels = edge_labels(
        &scene.graph,
        &scene.layout,
        &scene.layer,
        zoom_one(),
        CANVAS,
    );
    assert!(
        labels.iter().all(|label| label.text.ends_with("/s")),
        "{labels:?}"
    );
    let flows_with_label = scene
        .layer
        .overlay
        .edges
        .iter()
        .filter(|edge| matches!(edge, EdgeTraffic::Flow(flow) if flow.label.is_some()))
        .count();
    assert!(labels.len() <= flows_with_label);
}

#[test]
fn no_label_below_the_text_zoom() {
    let scene = scene(istio_sample(), EdgeShape::Elbows);
    let far = Viewport::default().zoom_at(0., 0., -12);
    assert!(far.zoom() < MIN_TEXT_ZOOM);
    assert!(edge_labels(&scene.graph, &scene.layout, &scene.layer, far, CANVAS).is_empty());
}

#[test]
fn labels_off_the_canvas_are_dropped() {
    let scene = scene(istio_sample(), EdgeShape::Elbows);
    assert!(
        edge_labels(
            &scene.graph,
            &scene.layout,
            &scene.layer,
            zoom_one(),
            (1., 1.)
        )
        .len()
            <= 1
    );
}

#[test]
fn a_label_box_fits_its_text() {
    assert!(label_width("35 req/s") < label_width("35 req/s \u{b7} 6% 5xx"));
    assert!(label_width("") >= 2. * LABEL_PADDING);
}

#[test]
fn zz_debug() {
    let scene = scene(istio_sample(), EdgeShape::Elbows);
    for (edge, route, traffic) in
        drawn_edges(&scene.graph, &scene.layout.routes, Some(&scene.layer))
    {
        let a = crate::topology_traffic::label_anchor(route);
        eprintln!(
            "{:?} {:?}->{:?} anchor {:?} n={} traffic {:?}",
            edge.relation,
            edge.from,
            edge.to,
            a,
            route.points.len(),
            traffic.map(|t| matches!(t, EdgeTraffic::Flow(_)))
        );
    }
    for (i, r) in scene.layout.rects.iter().enumerate() {
        eprintln!(
            "rect {i} {:?} {} {:?}",
            r.origin, r.width, scene.graph.nodes[i].name
        );
    }
}
