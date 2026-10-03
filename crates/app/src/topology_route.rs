//! The routes of the edges (0022b polish): an edge never runs behind a card it does not join, so
//! it can never look like a link it is not. An edge between neighbouring columns is one smooth
//! curve through the gutter. Any other edge leaves its card by the side, bends along the vertical
//! lane in the gutter, crosses over in a horizontal corridor that is free of cards, and enters the
//! other card by the side. Pure, in graph units, and computed once per layout, not per frame.

use gpui_kit::{Point, point};

use crate::topology_graph::{Relation, TopologyGraph};
use crate::topology_layout::{GraphPoint, GraphRect};
use crate::topology_stroke::flatten_cubic;

/// How far a lane runs from the card side: the middle of the gutter between two columns.
const LANE_OFFSET: f32 = 25.;
const CORNER_RADIUS: f32 = 12.;
const CORNER_STEPS: usize = 6;
/// Cards are avoided with this much room.
const CLEARANCE: f32 = 4.;
/// A curve between columns needs at least this much room between the cards.
const DIRECT_GAP: f32 = 20.;
/// How far a curve may stray from its true shape, in graph units (0.2 px at the highest zoom).
const FLATTEN_TOLERANCE: f32 = 0.1;
/// The corridor above and below everything: always free.
const MARGIN_CORRIDOR: f32 = 12.;
/// Corridors tried for one edge before the margin one.
const MAX_CORRIDORS: usize = 24;

/// An edge as a polyline in graph units, with its corners rounded.
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
        let fold = |value: fn(&GraphPoint) -> f32, init: f32, pick: fn(f32, f32) -> f32| {
            self.points.iter().map(value).fold(init, pick)
        };
        (
            fold(|p| p.x, f32::INFINITY, f32::min),
            fold(|p| p.y, f32::INFINITY, f32::min),
            fold(|p| p.x, f32::NEG_INFINITY, f32::max),
            fold(|p| p.y, f32::NEG_INFINITY, f32::max),
        )
    }
}

/// The route of every edge of `graph`, in edge order. `bands` are the frames, which only decide
/// where the corridors between rows of bands are.
pub(crate) fn route_edges(
    graph: &TopologyGraph,
    rects: &[GraphRect],
    bands: &[GraphRect],
) -> Vec<EdgeRoute> {
    let space = Space::new(rects, bands);
    graph
        .edges
        .iter()
        .map(|edge| route_edge(edge.from, edge.to, edge.relation, &space))
        .collect()
}

/// The cards, and the horizontal corridors between them.
struct Space<'a> {
    rects: &'a [GraphRect],
    /// The y of every corridor candidate, sorted.
    corridors: Vec<f32>,
    top: f32,
    bottom: f32,
}

impl<'a> Space<'a> {
    fn new(rects: &'a [GraphRect], bands: &[GraphRect]) -> Self {
        let mut corridors = Vec::new();
        for rect in rects {
            corridors.push(rect.origin.y - CLEARANCE * 2.5);
            corridors.push(rect.bottom() + CLEARANCE * 2.5);
        }
        for band in bands {
            corridors.push(band.origin.y - 22.);
            corridors.push(band.bottom() + 22.);
        }
        corridors.sort_by(f32::total_cmp);
        corridors.dedup_by(|a, b| (*a - *b).abs() < 1.);
        let all = || rects.iter().chain(bands);
        let top = all()
            .map(|rect| rect.origin.y)
            .fold(f32::INFINITY, f32::min);
        let bottom = all()
            .map(GraphRect::bottom)
            .fold(f32::NEG_INFINITY, f32::max);
        Self {
            rects,
            corridors,
            top: top - MARGIN_CORRIDOR,
            bottom: bottom + MARGIN_CORRIDOR,
        }
    }

    /// Whether a polyline keeps clear of every card but `from` and `to`.
    fn is_free(&self, points: &[GraphPoint], from: usize, to: usize) -> bool {
        points.windows(2).all(|pair| {
            let (left, right) = (pair[0].x.min(pair[1].x), pair[0].x.max(pair[1].x));
            let (top, bottom) = (pair[0].y.min(pair[1].y), pair[0].y.max(pair[1].y));
            self.rects.iter().enumerate().all(|(index, rect)| {
                if index == from || index == to {
                    return true;
                }
                let outside = rect.right() + CLEARANCE < left
                    || rect.origin.x - CLEARANCE > right
                    || rect.bottom() + CLEARANCE < top
                    || rect.origin.y - CLEARANCE > bottom;
                outside || !crosses(pair[0], pair[1], *rect, CLEARANCE)
            })
        })
    }
}

fn route_edge(from: usize, to: usize, relation: Relation, space: &Space) -> EdgeRoute {
    let (source, target) = (space.rects[from], space.rects[to]);
    // A mount runs from the side lane to its config card; an access edge is a curve only along a
    // row of the access layer, and takes the lanes when it comes down from a workload.
    let is_level = (source.origin.y - target.origin.y).abs() < 1.;
    let wants_curve = match relation {
        Relation::Mounts => false,
        Relation::Access => is_level,
        Relation::Owns | Relation::RoutesTo => true,
    };
    if wants_curve && target.origin.x >= source.right() + DIRECT_GAP {
        let curve = between_columns(source, target);
        if space.is_free(&curve, from, to) {
            return EdgeRoute { points: curve };
        }
    }
    EdgeRoute {
        points: round_corners(&along_lanes(from, to, space)),
    }
}

/// The curve from the right side of `source` to the left side of `target`, flat where it leaves
/// and where it arrives.
fn between_columns(source: GraphRect, target: GraphRect) -> Vec<GraphPoint> {
    let (start, end) = (side_port(source, true), side_port(target, false));
    let half = (end.x - start.x) / 2.;
    let corner = |x: f32, y: f32| point(x, y);
    flatten_cubic(
        corner(start.x, start.y),
        corner(start.x + half, start.y),
        corner(end.x - half, end.y),
        corner(end.x, end.y),
        FLATTEN_TOLERANCE,
    )
    .into_iter()
    .map(graph_point)
    .collect()
}

/// The middle of the right or left side of a card.
fn side_port(rect: GraphRect, is_right: bool) -> GraphPoint {
    GraphPoint {
        x: if is_right {
            rect.right()
        } else {
            rect.origin.x
        },
        y: rect.center().y,
    }
}

/// Out of the side of `from` that faces `to`, along the lane of the gutter, across in a free
/// corridor, and into the side of `to` that faces that lane.
fn along_lanes(from: usize, to: usize, space: &Space) -> Vec<GraphPoint> {
    let (source, target) = (space.rects[from], space.rects[to]);
    let leaves_right = target.center().x >= source.center().x;
    let start = side_port(source, leaves_right);
    let lane_out = start.x
        + if leaves_right {
            LANE_OFFSET
        } else {
            -LANE_OFFSET
        };
    let arrives_right = lane_out > target.center().x;
    let end = side_port(target, arrives_right);
    let lane_in = end.x
        + if arrives_right {
            LANE_OFFSET
        } else {
            -LANE_OFFSET
        };
    let through = |corridor: f32| {
        let mut points = vec![
            start,
            GraphPoint {
                x: lane_out,
                y: start.y,
            },
            GraphPoint {
                x: lane_out,
                y: corridor,
            },
        ];
        if (lane_out - lane_in).abs() > 1. {
            points.push(GraphPoint {
                x: lane_in,
                y: corridor,
            });
        }
        points.push(GraphPoint {
            x: lane_in,
            y: end.y,
        });
        points.push(end);
        simplified(points)
    };
    // The same lane up and down needs no corridor at all.
    if (lane_out - lane_in).abs() <= 1. {
        let points = through(start.y);
        if space.is_free(&points, from, to) {
            return points;
        }
    }
    let wanted = (start.y + end.y) / 2.;
    let mut candidates: Vec<f32> = space
        .corridors
        .iter()
        .copied()
        .chain([start.y, end.y])
        .collect();
    candidates.sort_by(|a, b| {
        let cost = |y: f32| (y - start.y).abs() + (y - end.y).abs() + (y - wanted).abs() * 0.01;
        cost(*a).total_cmp(&cost(*b))
    });
    for corridor in candidates.into_iter().take(MAX_CORRIDORS) {
        let points = through(corridor);
        if space.is_free(&points, from, to) {
            return points;
        }
    }
    // Above or below everything nothing is in the way.
    let corridor = if (start.y - space.top).abs() + (end.y - space.top).abs()
        <= (start.y - space.bottom).abs() + (end.y - space.bottom).abs()
    {
        space.top
    } else {
        space.bottom
    };
    through(corridor)
}

/// The points without a repeat, and without a middle point of a straight run.
fn simplified(points: Vec<GraphPoint>) -> Vec<GraphPoint> {
    let mut kept: Vec<GraphPoint> = Vec::with_capacity(points.len());
    for at in points {
        if kept.last().is_some_and(|last| distance(*last, at) < 1e-3) {
            continue;
        }
        while kept.len() >= 2 {
            let (a, b) = (kept[kept.len() - 2], kept[kept.len() - 1]);
            let cross = (b.x - a.x) * (at.y - b.y) - (b.y - a.y) * (at.x - b.x);
            if cross.abs() > 1e-3 {
                break;
            }
            kept.pop();
        }
        kept.push(at);
    }
    kept
}

/// The polyline with each corner replaced by a quadratic curve through it, as wide as the two
/// segments allow, at most `CORNER_RADIUS`.
fn round_corners(points: &[GraphPoint]) -> Vec<GraphPoint> {
    let mut rounded = vec![points[0]];
    for corner in points.windows(3) {
        let (before, at, after) = (corner[0], corner[1], corner[2]);
        let (to_before, to_after) = (distance(before, at), distance(at, after));
        let radius = CORNER_RADIUS.min(to_before / 2.).min(to_after / 2.);
        let enter = lerp(at, before, radius / to_before);
        let leave = lerp(at, after, radius / to_after);
        rounded.push(enter);
        for step in 1..CORNER_STEPS {
            let t = step as f32 / CORNER_STEPS as f32;
            rounded.push(lerp(lerp(enter, at, t), lerp(at, leave, t), t));
        }
        rounded.push(leave);
    }
    rounded.push(points[points.len() - 1]);
    rounded
}

/// Whether the segment `a`-`b` meets `rect` grown by `margin` (Liang-Barsky).
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

fn lerp(a: GraphPoint, b: GraphPoint, t: f32) -> GraphPoint {
    GraphPoint {
        x: a.x + (b.x - a.x) * t,
        y: a.y + (b.y - a.y) * t,
    }
}

fn distance(a: GraphPoint, b: GraphPoint) -> f32 {
    (b.x - a.x).hypot(b.y - a.y)
}

#[cfg(test)]
#[path = "topology_route_tests.rs"]
mod topology_route_tests;
