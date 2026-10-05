//! Where the labels of the flow edges go (spec 0049): centered on the arc-length midpoint of the
//! route, so every curve carries them, only at a zoom where cards show text, and never
//! over a card or another label. Pure screen math, no GPUI context.

use gpui_kit::SharedString;

use crate::topology_canvas::drawn_edges;
use crate::topology_graph::TopologyGraph;
use crate::topology_layout::{GraphRect, TopologyLayout};
use crate::topology_traffic::{EdgeTraffic, TrafficLayer, label_anchor, point_along};
use crate::topology_viewport::{MIN_TEXT_ZOOM, Viewport};

/// The font size of a label, in pixels, and the height and side padding of its box.
pub(crate) const LABEL_TEXT_SIZE: f32 = 11.;
pub(crate) const LABEL_HEIGHT: f32 = 16.;
const LABEL_PADDING: f32 = 6.;
/// Mono fonts are about this many em wide.
const MONO_EM_WIDTH: f32 = 0.6;

/// A label box on the canvas, in screen pixels.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EdgeLabel {
    pub(crate) text: SharedString,
    pub(crate) left: f32,
    pub(crate) top: f32,
    pub(crate) width: f32,
}

impl EdgeLabel {
    /// `(left, top, right, bottom)`.
    fn bounds(&self) -> (f32, f32, f32, f32) {
        (
            self.left,
            self.top,
            self.left + self.width,
            self.top + LABEL_HEIGHT,
        )
    }

    fn overlaps(&self, other: (f32, f32, f32, f32)) -> bool {
        let (left, top, right, bottom) = other;
        self.left < right
            && self.left + self.width > left
            && self.top < bottom
            && self.top + LABEL_HEIGHT > top
    }
}

/// The width of a label box for `text`.
pub(crate) fn label_width(text: &str) -> f32 {
    text.chars().count() as f32 * LABEL_TEXT_SIZE * MONO_EM_WIDTH + 2. * LABEL_PADDING
}

/// Where along a route a label is tried after its midpoint (`label_anchor`), as shares of its arc length:
/// either side of it. A gutter is narrower than a label, so the midpoint is often beside a card.
const LABEL_SHARES: [f32; 6] = [0.58, 0.42, 0.66, 0.34, 0.74, 0.26];

/// The labels to draw on a `canvas` of the given size: one per flow edge that has a text, at the
/// first of `LABEL_SHARES` that is on the canvas, clear of every card, and clear of the labels
/// placed before it; an edge with none gets no label.
pub(crate) fn edge_labels(
    graph: &TopologyGraph,
    layout: &TopologyLayout,
    layer: &TrafficLayer,
    viewport: Viewport,
    canvas: (f32, f32),
) -> Vec<EdgeLabel> {
    let zoom = viewport.zoom();
    if zoom < MIN_TEXT_ZOOM {
        return Vec::new();
    }
    let cards: Vec<(f32, f32, f32, f32)> = layout
        .rects
        .iter()
        .map(|rect| screen_box(viewport, *rect))
        .collect();
    let mut placed: Vec<EdgeLabel> = Vec::new();
    for (_, route, traffic) in drawn_edges(graph, &layout.routes, Some(layer)) {
        let Some(EdgeTraffic::Flow(flow)) = traffic else {
            continue;
        };
        let Some(text) = &flow.label else {
            continue;
        };
        let width = label_width(text);
        let candidates = std::iter::once(label_anchor(route))
            .chain(LABEL_SHARES.iter().map(|share| point_along(route, *share)));
        let first_clear = candidates.into_iter().find_map(|at| {
            let (x, y) = viewport.to_screen(at);
            let label = EdgeLabel {
                text: text.clone(),
                left: x - width / 2.,
                top: y - LABEL_HEIGHT / 2.,
                width,
            };
            let is_on_canvas = label.left + width > 0.
                && label.left < canvas.0
                && label.top + LABEL_HEIGHT > 0.
                && label.top < canvas.1;
            let is_clear = !cards.iter().any(|card| label.overlaps(*card))
                && !placed.iter().any(|other| label.overlaps(other.bounds()));
            (is_on_canvas && is_clear).then_some(label)
        });
        placed.extend(first_clear);
    }
    placed
}

/// `(left, top, right, bottom)` of a card on the canvas.
fn screen_box(viewport: Viewport, rect: GraphRect) -> (f32, f32, f32, f32) {
    let (left, top) = viewport.to_screen(rect.origin);
    let zoom = viewport.zoom();
    (
        left,
        top,
        left + rect.width * zoom,
        top + rect.height * zoom,
    )
}

#[cfg(test)]
#[path = "topology_traffic_labels_tests.rs"]
mod topology_traffic_labels_tests;
