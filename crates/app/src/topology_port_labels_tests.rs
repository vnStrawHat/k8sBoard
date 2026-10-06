use std::collections::HashMap;

use super::*;
use crate::topology_fixtures::{Fixture, index_of, ingress, object, pod, service};
use crate::topology_graph::{GroupBy, TopologyKind};
use crate::topology_layout::{GraphPoint, layout};

fn port(port: u16, target: Option<&str>) -> ServicePortSummary {
    ServicePortSummary {
        name: None,
        port,
        target_port: target.map(str::to_owned),
        node_port: None,
        protocol: "TCP".to_owned(),
    }
}

#[test]
fn a_target_port_that_differs_follows_the_port() {
    assert_eq!(service_port_label(&[port(80, Some("8080"))]), "80→8080");
    assert_eq!(service_port_label(&[port(80, Some("http"))]), "80→http");
}

#[test]
fn a_target_port_that_matches_or_is_absent_shows_the_port_alone() {
    assert_eq!(service_port_label(&[port(443, Some("443"))]), "443");
    assert_eq!(service_port_label(&[port(443, None)]), "443");
}

#[test]
fn up_to_three_ports_are_joined_and_the_rest_counts() {
    let ports: Vec<_> = [80, 81, 82, 83, 84]
        .into_iter()
        .map(|number| port(number, None))
        .collect();
    assert_eq!(service_port_label(&ports[..3]), "80,81,82");
    assert_eq!(service_port_label(&ports), "80,81,82 +2");
    assert_eq!(service_port_label(&[]), "");
}

#[test]
fn an_ingress_shows_the_backend_ports_of_one_service() {
    let mut routes = ingress(
        "web",
        &[("/a", "api"), ("/b", "api"), ("/c", "other")],
        None,
        None,
    );
    routes.rules[1].backend = "api:8443".to_owned();
    routes.rules[2].backend = "other:9000".to_owned();
    assert_eq!(backend_port_label(&routes, "api"), "80,8443");
    assert_eq!(backend_port_label(&routes, "other"), "9000");
    routes.rules[0].backend = "api".to_owned();
    assert_eq!(backend_port_label(&routes, "api"), "8443");
}

#[test]
fn the_default_backend_port_counts() {
    let mut routes = ingress("web", &[], Some("api"), None);
    routes.default_backend = Some("api:http".to_owned());
    assert_eq!(backend_port_label(&routes, "api"), "http");
}

#[test]
fn the_zoom_gate_matches_the_card_text() {
    assert!(shows_port_labels(MIN_TEXT_ZOOM));
    assert!(!shows_port_labels(MIN_TEXT_ZOOM - 0.01));
}

fn routed() -> (TopologyGraph, TopologyLayout) {
    let mut api = service("api", &["app=api"]);
    api.ports = vec![port(80, Some("8080")), port(443, None)];
    let graph = Fixture::default()
        .with_service_summary(api)
        .with_service("db", &["app=db"])
        .with_ingress(ingress("web", &[("/", "api")], None, None))
        .with_pod(pod("api-1", &["app=api"], None))
        .with_pod(pod("db-1", &["app=db"], None))
        .graph();
    let arranged = layout(&graph, GroupBy::Components, 1., &HashMap::new(), None);
    (graph, arranged)
}

#[test]
fn the_graph_carries_the_ports_of_each_routes_to_edge() {
    let (graph, _) = routed();
    let at = |kind, name| index_of(&graph, &object(kind, name));
    let (web, api, db) = (
        at(TopologyKind::Ingress, "web"),
        at(TopologyKind::Service, "api"),
        at(TopologyKind::Service, "db"),
    );
    let api_pod = at(TopologyKind::Pod, "api-1");
    assert_eq!(
        graph.port_labels.get(&(web, api)).map(String::as_str),
        Some("80")
    );
    assert_eq!(
        graph.port_labels.get(&(api, api_pod)).map(String::as_str),
        Some("80→8080,443")
    );
    assert_eq!(
        graph
            .port_labels
            .get(&(db, at(TopologyKind::Pod, "db-1")))
            .map(String::as_str),
        Some("80")
    );
}

const BIG: (f32, f32) = (5000., 5000.);

fn chips_of(graph: &TopologyGraph, arranged: &TopologyLayout, focus: Option<usize>) -> usize {
    port_chips(graph, arranged, Viewport::default(), BIG, focus).len()
}

#[test]
fn nothing_is_drawn_at_rest() {
    let (graph, arranged) = routed();
    assert_eq!(chips_of(&graph, &arranged, None), 0);
}

#[test]
fn only_the_edges_of_the_focused_node_get_a_chip() {
    let (graph, arranged) = routed();
    let at = |kind, name| index_of(&graph, &object(kind, name));
    assert_eq!(
        chips_of(&graph, &arranged, Some(at(TopologyKind::Service, "api"))),
        2
    );
    assert_eq!(
        chips_of(&graph, &arranged, Some(at(TopologyKind::Service, "db"))),
        1
    );
    assert_eq!(
        chips_of(&graph, &arranged, Some(at(TopologyKind::Ingress, "web"))),
        1
    );
}

#[test]
fn no_chips_below_the_zoom_gate() {
    let (graph, arranged) = routed();
    let api = index_of(&graph, &object(TopologyKind::Service, "api"));
    let far = Viewport::default().zoom_at(0., 0., -20);
    assert!(!shows_port_labels(far.zoom()));
    assert!(port_chips(&graph, &arranged, far, BIG, Some(api)).is_empty());
}

#[test]
fn chips_off_the_canvas_are_dropped() {
    let (graph, arranged) = routed();
    let api = index_of(&graph, &object(TopologyKind::Service, "api"));
    assert!(port_chips(&graph, &arranged, Viewport::default(), (0., 0.), Some(api)).is_empty());
}

fn straight() -> EdgeRoute {
    EdgeRoute {
        points: vec![
            GraphPoint { x: 0., y: 100. },
            GraphPoint { x: 1000., y: 100. },
        ],
    }
}

/// The left edge of the chip of `80` on the straight route, with these cards in the way.
fn chip_left(cards: &[(f32, f32, f32, f32)]) -> f32 {
    place_chip("80", &straight(), Viewport::default(), cards).left
}

#[test]
fn a_clear_midpoint_keeps_the_chip_there() {
    let middle = chip_left(&[]);
    let width = label_width("80");
    assert!((middle + width / 2. - 500. * Viewport::default().zoom()).abs() < 0.01);
}

#[test]
fn a_card_on_the_midpoint_moves_the_chip_to_the_next_share() {
    let middle = chip_left(&[]);
    let blocked = [(middle - 1., 0., middle + 20., 300.)];
    let moved = chip_left(&blocked);
    assert!(moved < middle, "0.4 comes before 0.6");
    let zoom = Viewport::default().zoom();
    assert!((moved + label_width("80") / 2. - 400. * zoom).abs() < 0.01);
}

#[test]
fn with_every_share_blocked_the_chip_stays_on_the_midpoint() {
    let everywhere = [(-1000., -1000., 5000., 5000.)];
    assert_eq!(chip_left(&everywhere), chip_left(&[]));
}

#[test]
fn a_chip_beside_a_card_fits_and_one_on_it_does_not() {
    let chip = place_chip("80", &straight(), Viewport::default(), &[]);
    assert!(chip_fits(
        &chip,
        &[(chip.left + chip.width, 0., 9000., 9000.)]
    ));
    assert!(!chip_fits(&chip, &[(chip.left + 1., 0., 9000., 9000.)]));
}
