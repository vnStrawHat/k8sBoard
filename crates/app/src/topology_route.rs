//! The routes of the edges (0022b polish, 0050): every edge is one smooth cubic curve between an
//! anchor of its source card and an anchor of its target card. A card has four anchors, the
//! middles of its sides. The pair is chosen from the geometry (`facing_sides`); when a card is in
//! the way, the other pairs are tried before the curve is accepted as it is. Pure, in graph
//! units, and computed once per layout, not per frame.

use gpui_kit::{Point, point};

use crate::topology_graph::TopologyEdge;
use crate::topology_layout::{GraphPoint, GraphRect};
use crate::topology_stroke::flatten_cubic;

/// Cards are avoided with this much room.
const CLEARANCE: f32 = 4.;
/// How far a curve may stray from its true shape, in graph units (0.2 px at the highest zoom).
const FLATTEN_TOLERANCE: f32 = 0.1;
/// The straight run into the target card, perpendicular to its side. It is the arrow: the canvas
/// trims exactly `ARROW_LENGTH + ARROW_TIP_GAP` off the end, so the stroke stops on the true
/// tangent and the arrow points straight along it (a test pins the sum, because this module must
/// not import the canvas).
const ARRIVAL_STUB: f32 = 13.;
/// The least a curve bulges out of a card when its two sides do not face each other.
const LOOP_REACH: f32 = 24.;
/// How far into its own cards a curve may dip before the pair of anchors is rejected.
const OWN_INSET: f32 = 1.;

/// A side of a card, and so the anchor at its middle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    const ALL: [Self; 4] = [Self::Left, Self::Right, Self::Top, Self::Bottom];

    /// The unit vector pointing out of the card; a curve leaves and enters along it.
    fn normal(self) -> (f32, f32) {
        match self {
            Self::Left => (-1., 0.),
            Self::Right => (1., 0.),
            Self::Top => (0., -1.),
            Self::Bottom => (0., 1.),
        }
    }

    fn anchor(self, rect: GraphRect) -> GraphPoint {
        let center = rect.center();
        match self {
            Self::Left => GraphPoint {
                x: rect.origin.x,
                y: center.y,
            },
            Self::Right => GraphPoint {
                x: rect.right(),
                y: center.y,
            },
            Self::Top => GraphPoint {
                x: center.x,
                y: rect.origin.y,
            },
            Self::Bottom => GraphPoint {
                x: center.x,
                y: rect.bottom(),
            },
        }
    }
}

/// An edge as a polyline in graph units.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EdgeRoute {
    pub(crate) points: Vec<GraphPoint>,
}

impl EdgeRoute {
    pub(crate) fn start(&self) -> GraphPoint {
        self.points[0]
    }

    pub(crate) fn end(&self) -> GraphPoint {
        self.points[self.points.len() - 1]
    }

    /// The unit direction the edge arrives in: along its last segment that has a length.
    pub(crate) fn end_direction(&self) -> (f32, f32) {
        let end = self.end();
        self.points
            .iter()
            .rev()
            .find(|at| distance(**at, end) > 1e-3)
            .map_or((1., 0.), |before| {
                let length = distance(*before, end);
                ((end.x - before.x) / length, (end.y - before.y) / length)
            })
    }

    /// The box of the points: `(left, top, right, bottom)`.
    pub(crate) fn bounds(&self) -> (f32, f32, f32, f32) {
        bounds_of(&self.points)
    }
}

/// The route of every edge of `edges`, in edge order: the graph's own, or the 0049 `Calls` edges
/// routed over the same cards.
#[cfg_attr(feature = "hotpath-profiling", hotpath::measure)]
pub(crate) fn route_edges(edges: &[TopologyEdge], rects: &[GraphRect]) -> Vec<EdgeRoute> {
    edges
        .iter()
        .map(|edge| route_edge(edge.from, edge.to, rects))
        .collect()
}

/// The anchors of an edge: where it leaves the source and where it enters the target.
#[derive(Clone, Copy)]
struct Anchors {
    start: GraphPoint,
    start_side: Side,
    end: GraphPoint,
    end_side: Side,
    /// A curve that turns back or bends round a corner bulges out by a share of its span; a tight
    /// one only by `LOOP_REACH`, so it fits in a narrow gutter between rows.
    is_tight: bool,
}

impl Anchors {
    fn between(source: GraphRect, start_side: Side, target: GraphRect, end_side: Side) -> Self {
        Self {
            start: start_side.anchor(source),
            start_side,
            end: end_side.anchor(target),
            end_side,
            is_tight: false,
        }
    }

    fn tight(self) -> Self {
        Self {
            is_tight: true,
            ..self
        }
    }
}

/// The sides an edge uses by default: when the horizontal gap between the cards is at least the
/// vertical one, out of the right side into the left side (or the reverse for a target on the
/// left); otherwise out of the bottom into the top (or the reverse for a target above). Cards
/// that overlap on an axis have a negative gap there, so two cards in one column are joined
/// vertically.
fn facing_sides(source: GraphRect, target: GraphRect) -> (Side, Side) {
    let gap_x = (target.origin.x - source.right()).max(source.origin.x - target.right());
    let gap_y = (target.origin.y - source.bottom()).max(source.origin.y - target.bottom());
    if gap_x >= gap_y {
        if target.center().x >= source.center().x {
            (Side::Right, Side::Left)
        } else {
            (Side::Left, Side::Right)
        }
    } else if target.center().y >= source.center().y {
        (Side::Bottom, Side::Top)
    } else {
        (Side::Top, Side::Bottom)
    }
}

fn route_edge(from: usize, to: usize, rects: &[GraphRect]) -> EdgeRoute {
    let (source, target) = (rects[from], rects[to]);
    let (start_side, end_side) = facing_sides(source, target);
    // The default pair first; then, if a card is in the way, the other pairs (the nearest first,
    // each also as a tight curve) may get round it.
    let preferred = Anchors::between(source, start_side, target, end_side);
    let mut others: Vec<Anchors> = Side::ALL
        .into_iter()
        .flat_map(|a| Side::ALL.into_iter().map(move |b| (a, b)))
        .filter(|pair| *pair != (start_side, end_side))
        .map(|(a, b)| Anchors::between(source, a, target, b))
        .collect();
    others.sort_by(|a, b| distance(a.start, a.end).total_cmp(&distance(b.start, b.end)));
    let candidates = std::iter::once(preferred).chain(
        others
            .into_iter()
            .flat_map(|anchors| [anchors, anchors.tight()]),
    );
    // ponytail: when every pair is blocked (a long edge across packed columns) the curve with the
    // fewest cards in the way is kept and passes behind them; a waypoint search could avoid that.
    let mut best: Option<Vec<GraphPoint>> = None;
    let mut fewest = usize::MAX;
    for anchors in candidates {
        let curve = bezier(anchors);
        let Some(blocked) = cards_in_the_way(anchors, &curve, from, to, rects, fewest) else {
            continue;
        };
        if blocked == 0 {
            return EdgeRoute { points: curve };
        }
        if blocked < fewest {
            fewest = blocked;
            best = Some(curve);
        }
    }
    EdgeRoute {
        points: best.unwrap_or_else(|| bezier(preferred)),
    }
}

/// The four points of the cubic: the start, two control points, and the base of the stub.
fn controls(anchors: Anchors) -> [GraphPoint; 4] {
    let (out_x, out_y) = anchors.start_side.normal();
    let (in_x, in_y) = anchors.end_side.normal();
    let base = GraphPoint {
        x: anchors.end.x + in_x * ARRIVAL_STUB,
        y: anchors.end.y + in_y * ARRIVAL_STUB,
    };
    let forward = (base.x - anchors.start.x) * out_x + (base.y - anchors.start.y) * out_y;
    let is_facing = (in_x, in_y) == (-out_x, -out_y);
    // The reach is half the way for an S between facing sides. Elsewhere (a turn back, or a
    // corner) it grows with the span, so the curve clears the card it leaves; the floor keeps a
    // short one round.
    let reach = if is_facing && forward > 0. {
        forward / 2.
    } else if anchors.is_tight {
        LOOP_REACH
    } else {
        (distance(anchors.start, base) / 4.).max(LOOP_REACH)
    };
    [
        anchors.start,
        GraphPoint {
            x: anchors.start.x + out_x * reach,
            y: anchors.start.y + out_y * reach,
        },
        GraphPoint {
            x: base.x + in_x * reach,
            y: base.y + in_y * reach,
        },
        base,
    ]
}

/// One cubic, perpendicular to both sides (React Flow's default edge), from the start to the
/// base of a straight stub into the end.
fn bezier(anchors: Anchors) -> Vec<GraphPoint> {
    let [start, c1, c2, base] = controls(anchors);
    let at = |p: GraphPoint| point(p.x, p.y);
    let mut points: Vec<GraphPoint> =
        flatten_cubic(at(start), at(c1), at(c2), at(base), FLATTEN_TOLERANCE)
            .into_iter()
            .map(graph_point)
            .collect();
    points.push(anchors.end);
    points
}

/// How many other cards the curve runs behind, or `None` when it enters one of its own two cards
/// (which no pair of anchors may do).
fn cards_in_the_way(
    anchors: Anchors,
    curve: &[GraphPoint],
    from: usize,
    to: usize,
    rects: &[GraphRect],
    cap: usize,
) -> Option<usize> {
    let enters_own = [from, to].iter().any(|card| {
        curve
            .windows(2)
            .any(|pair| crosses(pair[0], pair[1], rects[*card], -OWN_INSET))
    });
    if enters_own {
        return None;
    }
    // The curve lies inside the box of its control points: only the cards that meet that box need
    // the per-segment test.
    let [start, c1, c2, base] = controls(anchors);
    let (left, top, right, bottom) = bounds_of(&[start, c1, c2, base, anchors.end]);
    let in_the_way = rects
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != from && *index != to)
        .filter(|(_, rect)| {
            rect.right() + CLEARANCE >= left
                && rect.origin.x - CLEARANCE <= right
                && rect.bottom() + CLEARANCE >= top
                && rect.origin.y - CLEARANCE <= bottom
        })
        .filter(|(_, rect)| {
            curve
                .windows(2)
                .any(|pair| crosses(pair[0], pair[1], **rect, CLEARANCE))
        })
        .take(cap)
        .count();
    Some(in_the_way)
}

fn bounds_of(points: &[GraphPoint]) -> (f32, f32, f32, f32) {
    let fold = |value: fn(&GraphPoint) -> f32, init: f32, pick: fn(f32, f32) -> f32| {
        points.iter().map(value).fold(init, pick)
    };
    (
        fold(|p| p.x, f32::INFINITY, f32::min),
        fold(|p| p.y, f32::INFINITY, f32::min),
        fold(|p| p.x, f32::NEG_INFINITY, f32::max),
        fold(|p| p.y, f32::NEG_INFINITY, f32::max),
    )
}

/// Whether the segment `a`-`b` meets `rect` grown by `margin` (Liang-Barsky); a negative margin
/// shrinks the rect.
fn crosses(a: GraphPoint, b: GraphPoint, rect: GraphRect, margin: f32) -> bool {
    let (left, top) = (rect.origin.x - margin, rect.origin.y - margin);
    let (right, bottom) = (rect.right() + margin, rect.bottom() + margin);
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let (mut enter, mut leave) = (0_f32, 1_f32);
    for (p, q) in [
        (-dx, a.x - left),
        (dx, right - a.x),
        (-dy, a.y - top),
        (dy, bottom - a.y),
    ] {
        if p == 0. {
            if q < 0. {
                return false;
            }
            continue;
        }
        let t = q / p;
        if p < 0. {
            enter = enter.max(t);
        } else {
            leave = leave.min(t);
        }
        if enter > leave {
            return false;
        }
    }
    true
}

fn graph_point(at: Point<f32>) -> GraphPoint {
    GraphPoint { x: at.x, y: at.y }
}

fn distance(a: GraphPoint, b: GraphPoint) -> f32 {
    (b.x - a.x).hypot(b.y - a.y)
}

#[cfg(test)]
#[path = "topology_route_tests.rs"]
mod topology_route_tests;
