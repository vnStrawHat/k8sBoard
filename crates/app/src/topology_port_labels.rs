//! The port text on the `routes to` edges (spec 0050): the Service ports on a Service to Pod
//! edge, the backend port on an Ingress to Service edge. A chip sits on the arc-length midpoint of
//! the curve, only at a zoom where cards show text. Pure screen math, no GPUI context.

use cluster::{IngressSummary, ServicePortSummary};

use crate::topology_graph::{Relation, TopologyGraph};
use crate::topology_layout::TopologyLayout;
use crate::topology_traffic::label_anchor;
use crate::topology_traffic_labels::{EdgeLabel, LABEL_HEIGHT, label_width};
use crate::topology_viewport::{MIN_TEXT_ZOOM, Viewport};

/// More ports than this on one edge read `+N` for the rest.
const MAX_LISTED_PORTS: usize = 3;

/// A port chip and whether its edge touches the focused node.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PortChip {
    pub(crate) label: EdgeLabel,
    pub(crate) is_focused: bool,
}

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

/// The chips to draw on a `canvas` of the given size, the ones on the focused node's edges last so
/// they stay on top. Labels may overlap where edges bundle: no collision solver.
pub(crate) fn port_chips(
    graph: &TopologyGraph,
    layout: &TopologyLayout,
    viewport: Viewport,
    canvas: (f32, f32),
    focus: Option<usize>,
) -> Vec<PortChip> {
    if !shows_port_labels(viewport.zoom()) {
        return Vec::new();
    }
    let mut chips: Vec<PortChip> = graph
        .edges
        .iter()
        .zip(&layout.routes)
        .filter(|(edge, _)| edge.relation == Relation::RoutesTo)
        .filter_map(|(edge, route)| {
            let text = graph.port_labels.get(&(edge.from, edge.to))?;
            let (x, y) = viewport.to_screen(label_anchor(route));
            let width = label_width(text);
            let label = EdgeLabel {
                text: text.clone().into(),
                left: x - width / 2.,
                top: y - LABEL_HEIGHT / 2.,
                width,
            };
            let is_on_canvas = label.left + width > 0.
                && label.left < canvas.0
                && label.top + LABEL_HEIGHT > 0.
                && label.top < canvas.1;
            is_on_canvas.then_some(PortChip {
                label,
                is_focused: focus.is_some_and(|node| node == edge.from || node == edge.to),
            })
        })
        .collect();
    chips.sort_by_key(|chip| chip.is_focused);
    chips
}

#[cfg(test)]
#[path = "topology_port_labels_tests.rs"]
mod topology_port_labels_tests;
