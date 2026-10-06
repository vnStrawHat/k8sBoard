//! The port text on the `routes to` edges (spec 0050): the Service ports on a Service to Pod
//! edge, the backend port on an Ingress to Service edge. Only the edges of the focused node get a
//! chip, on the arc-length midpoint of the curve or beside it where a card is in the way, and only
//! at a zoom where cards show text. Pure screen math, no GPUI context.

use cluster::{IngressSummary, ServicePortSummary};

use crate::topology_graph::{Relation, TopologyGraph};
use crate::topology_layout::TopologyLayout;
use crate::topology_route::EdgeRoute;
use crate::topology_traffic::point_along;
use crate::topology_traffic_labels::{EdgeLabel, LABEL_HEIGHT, label_width, screen_box};
use crate::topology_viewport::{MIN_TEXT_ZOOM, Viewport};

/// More ports than this on one edge read `+N` for the rest.
const MAX_LISTED_PORTS: usize = 3;

/// Whether the chips are drawn at this zoom: below it the text would not be readable.
pub(crate) fn shows_port_labels(zoom: f32) -> bool {
    zoom >= MIN_TEXT_ZOOM
}

/// The ports of a Service as `80→8080` (the target port only when it differs from the port, else
/// `443`), joined with commas.
pub(crate) fn service_port_label(ports: &[ServicePortSummary]) -> String {
    let texts = ports.iter().map(|port| match &port.target_port {
        Some(target) if *target != port.port.to_string() => format!("{}→{target}", port.port),
        _ => port.port.to_string(),
    });
    join_ports(texts.collect())
}

/// The ports an ingress routes to `service`: the part after `name:` of each backend that names it,
/// without duplicates.
pub(crate) fn backend_port_label(ingress: &IngressSummary, service: &str) -> String {
    let rules = ingress
        .rules
        .iter()
        .filter(|rule| rule.service.as_deref() == Some(service))
        .map(|rule| &rule.backend);
    let default = ingress
        .default_service
        .as_deref()
        .filter(|name| *name == service)
        .and(ingress.default_backend.as_ref());
    let mut ports: Vec<String> = Vec::new();
    for backend in rules.chain(default) {
        let Some((_, port)) = backend.split_once(':') else {
            continue;
        };
        if !ports.iter().any(|known| known == port) {
            ports.push(port.to_owned());
        }
    }
    join_ports(ports)
}

/// Up to `MAX_LISTED_PORTS` ports joined with commas, then `+N` for the rest.
fn join_ports(mut ports: Vec<String>) -> String {
    let hidden = ports.len().saturating_sub(MAX_LISTED_PORTS);
    ports.truncate(MAX_LISTED_PORTS);
    let mut text = ports.join(",");
    if hidden > 0 {
        text.push_str(&format!(" +{hidden}"));
    }
    text
}

/// Where along a route a chip is tried, as shares of its arc length: the midpoint first, then
/// either side of it, so the chip leaves the cards that stand beside a narrow gutter.
const CHIP_SHARES: [f32; 7] = [0.5, 0.4, 0.6, 0.3, 0.7, 0.2, 0.8];

/// The chips of the `routes to` edges of the `focus` node (the hovered or selected one) on a
/// `canvas` of the given size; none at rest. Each sits on the first of `CHIP_SHARES` where it is
/// clear of every card, else on the midpoint, over the cards. Chips may overlap each other.
pub(crate) fn port_chips(
    graph: &TopologyGraph,
    layout: &TopologyLayout,
    viewport: Viewport,
    canvas: (f32, f32),
    focus: Option<usize>,
) -> Vec<EdgeLabel> {
    let Some(focus) = focus.filter(|_| shows_port_labels(viewport.zoom())) else {
        return Vec::new();
    };
    let cards: Vec<(f32, f32, f32, f32)> = layout
        .rects
        .iter()
        .map(|rect| screen_box(viewport, *rect))
        .collect();
    graph
        .edges
        .iter()
        .zip(&layout.routes)
        .filter(|(edge, _)| {
            edge.relation == Relation::RoutesTo && (edge.from == focus || edge.to == focus)
        })
        .filter_map(|(edge, route)| {
            let text = graph.port_labels.get(&(edge.from, edge.to))?;
            let chip = place_chip(text, route, viewport, &cards);
            let is_on_canvas = chip.left + chip.width > 0.
                && chip.left < canvas.0
                && chip.top + LABEL_HEIGHT > 0.
                && chip.top < canvas.1;
            is_on_canvas.then_some(chip)
        })
        .collect()
}

/// The chip of `text` on the first of `CHIP_SHARES` of `route` where it overlaps no card, else on
/// the midpoint.
fn place_chip(
    text: &str,
    route: &EdgeRoute,
    viewport: Viewport,
    cards: &[(f32, f32, f32, f32)],
) -> EdgeLabel {
    let width = label_width(text);
    let at = |share: f32| {
        let (x, y) = viewport.to_screen(point_along(route, share));
        EdgeLabel {
            text: text.to_owned().into(),
            left: x - width / 2.,
            top: y - LABEL_HEIGHT / 2.,
            width,
        }
    };
    CHIP_SHARES
        .iter()
        .map(|share| at(*share))
        .find(|chip| chip_fits(chip, cards))
        .unwrap_or_else(|| at(CHIP_SHARES[0]))
}

/// Whether `chip` overlaps none of the `cards` (`(left, top, right, bottom)` boxes).
fn chip_fits(chip: &EdgeLabel, cards: &[(f32, f32, f32, f32)]) -> bool {
    !cards.iter().any(|card| chip.overlaps(*card))
}

#[cfg(test)]
#[path = "topology_port_labels_tests.rs"]
mod topology_port_labels_tests;
