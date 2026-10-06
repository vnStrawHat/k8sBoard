use std::collections::HashMap;

use super::*;
use crate::topology_fixtures::{Fixture, index_of, ingress, object, pod, service};
use crate::topology_graph::{GroupBy, TopologyKind};
use crate::topology_layout::layout;

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

#[test]
fn chips_sit_on_the_midpoints_of_the_edges_with_ports() {
    let (graph, arranged) = routed();
    let chips = port_chips(&graph, &arranged, Viewport::default(), (5000., 5000.), None);
    assert_eq!(chips.len(), graph.port_labels.len());
    assert!(chips.iter().all(|chip| !chip.is_focused));
}

#[test]
fn no_chips_below_the_zoom_gate() {
    let (graph, arranged) = routed();
    let far = Viewport::default().zoom_at(0., 0., -20);
    assert!(!shows_port_labels(far.zoom()));
    assert!(port_chips(&graph, &arranged, far, (5000., 5000.), None).is_empty());
}

#[test]
fn chips_off_the_canvas_are_dropped() {
    let (graph, arranged) = routed();
    assert!(port_chips(&graph, &arranged, Viewport::default(), (0., 0.), None).is_empty());
}

#[test]
fn the_chips_of_the_focused_node_come_last() {
    let (graph, arranged) = routed();
    let db = index_of(&graph, &object(TopologyKind::Service, "db"));
    let chips = port_chips(
        &graph,
        &arranged,
        Viewport::default(),
        (5000., 5000.),
        Some(db),
    );
    let focused: Vec<bool> = chips.iter().map(|chip| chip.is_focused).collect();
    assert_eq!(focused, [false, false, true]);
}
