//! Anti-aliased strokes for the Topology canvas (0022b step 1). GPUI paths get only 4x MSAA on
//! Windows, so a stroke here is a ribbon whose vertices carry the signed distance to the stroke
//! edge in the `t` of the path shader. The shader turns that distance into one device pixel of
//! coverage. Pure: no GPUI context, only the `Path` value that `paint_path` takes.

use gpui_kit::{Path, Pixels, Point, point, px};

const MAX_SEGMENTS: usize = 128;
/// A stroke join never reaches further than this many half widths from its point.
const MITER_LIMIT: f32 = 2.;
/// A filled corner (the arrow tip is sharp) may reach further than a stroke join.
const FILL_MITER_LIMIT: f32 = 4.;
/// A segment shorter than this (px) has no direction.
const MIN_SEGMENT: f32 = 1e-3;
/// The `t` of a vertex that is plainly inside: coverage 1 with no ramp.
const SOLID: f32 = 1.;

/// A dash pattern in logical px. The pattern starts at `-phase` along the line, so a growing phase
/// moves the dashes toward the end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Dash {
    pub(crate) on: f32,
    pub(crate) off: f32,
    pub(crate) phase: f32,
}

/// How the path shader of the renderer turns a triangle into coverage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PathCoverage {
    /// Windows (gpui-pre-windows 0.3.8): `t` is read as the signed distance to the edge, so a
    /// feathered ribbon gets one device pixel of analytic coverage.
    SignedDistance,
    /// Metal and WGSL read `s == 0` as solid, so a feather band would draw opaque: the ribbon is a
    /// plain stroke and 4x MSAA does the smoothing.
    MsaaOnly,
}

/// The coverage of the renderer this build runs on. If a gpui update fixes the Windows branch of
/// `path_rasterization_fragment`, set `MsaaOnly` here (root-cause.md).
const PATH_COVERAGE: PathCoverage = if cfg!(windows) {
    PathCoverage::SignedDistance
} else {
    PathCoverage::MsaaOnly
};

/// The half width of the coverage ramp on this platform, in logical px.
pub(crate) fn feather(scale_factor: f32) -> f32 {
    feather_for(PATH_COVERAGE, scale_factor)
}

/// The half width of the coverage ramp for `coverage`: half a device px, or none.
fn feather_for(coverage: PathCoverage, scale_factor: f32) -> f32 {
    match coverage {
        PathCoverage::SignedDistance => 0.5 / scale_factor,
        PathCoverage::MsaaOnly => 0.,
    }
}

/// The cubic as a polyline, uniform in the curve parameter, within `tolerance` of the curve.
/// The segment count is Wang's bound: `sqrt(3 * max second difference / (4 * tolerance))`.
pub(crate) fn flatten_cubic(
    start: Point<f32>,
    ctrl1: Point<f32>,
    ctrl2: Point<f32>,
    end: Point<f32>,
    tolerance: f32,
) -> Vec<Point<f32>> {
    let second_difference = |a: Point<f32>, b: Point<f32>, c: Point<f32>| {
        (a.x - 2. * b.x + c.x).hypot(a.y - 2. * b.y + c.y)
    };
    let bend = second_difference(start, ctrl1, ctrl2).max(second_difference(ctrl1, ctrl2, end));
    let count = ((3. * bend / (4. * tolerance)).sqrt().ceil() as usize).clamp(2, MAX_SEGMENTS);
    (0..=count)
        .map(|step| cubic_at(start, ctrl1, ctrl2, end, step as f32 / count as f32))
        .collect()
}

fn cubic_at(
    start: Point<f32>,
    ctrl1: Point<f32>,
    ctrl2: Point<f32>,
    end: Point<f32>,
    t: f32,
) -> Point<f32> {
    let u = 1. - t;
    let (w0, w1, w2, w3) = (u * u * u, 3. * u * u * t, 3. * u * t * t, t * t * t);
    point(
        w0 * start.x + w1 * ctrl1.x + w2 * ctrl2.x + w3 * end.x,
        w0 * start.y + w1 * ctrl1.y + w2 * ctrl2.y + w3 * end.y,
    )
}

/// Cuts the last `length` px off the polyline (the base of an arrow). A cut longer than the line
/// leaves its first point alone.
pub(crate) fn trim_end(points: &mut Vec<Point<f32>>, length: f32) {
    let mut remaining = length;
    while points.len() > 1 {
        let last = points[points.len() - 1];
        let before = points[points.len() - 2];
        let segment = distance(before, last);
        if segment > remaining {
            let keep = (segment - remaining) / segment;
            let cut = lerp(before, last, keep);
            if let Some(slot) = points.last_mut() {
                *slot = cut;
            }
            return;
        }
        remaining -= segment;
        points.pop();
    }
}

/// Calls `emit` with each dash of the polyline, in order. `run` is the scratch the dash is built in.
/// The walk follows the arc length. A dash pattern with nothing on, or of no length, emits nothing.
fn walk_dashes(
    points: &[Point<f32>],
    dash: Dash,
    run: &mut Vec<Point<f32>>,
    mut emit: impl FnMut(&[Point<f32>]),
) {
    let period = dash.on + dash.off.max(0.);
    if points.len() < 2 || dash.on <= 0. || period <= 0. {
        return;
    }
    let off = period - dash.on;
    let position = (-dash.phase).rem_euclid(period);
    let mut is_on = position < dash.on;
    // What is left of the current dash or gap.
    let mut left = if is_on {
        dash.on - position
    } else {
        period - position
    };
    run.clear();
    if is_on {
        run.push(points[0]);
    }
    for pair in points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let length = distance(from, to);
        let mut walked = 0.;
        while length - walked > left {
            walked += left;
            let at = lerp(from, to, walked / length);
            if is_on {
                run.push(at);
                emit(run);
                run.clear();
                left = off;
            } else {
                run.clear();
                run.push(at);
                left = dash.on;
            }
            is_on = !is_on;
        }
        left -= length - walked;
        if is_on {
            run.push(to);
        }
    }
    if run.len() > 1 {
        emit(run);
    }
}

/// The dashes of a polyline, each as its own run.
#[cfg(test)]
pub(crate) fn dash_runs(points: &[Point<f32>], dash: Dash) -> Vec<Vec<Point<f32>>> {
    let mut runs = Vec::new();
    walk_dashes(points, dash, &mut Vec::new(), |run| runs.push(run.to_vec()));
    runs
}

/// A ribbon along each run, `width` px wide, with `feather` px of coverage ramp on both sides.
/// `None` when no run has two distinct points.
#[cfg(test)]
pub(crate) fn stroke_path(
    runs: &[Vec<Point<f32>>],
    width: f32,
    feather: f32,
) -> Option<Path<Pixels>> {
    let mut ribbons = Ribbons::new(width, feather);
    for run in runs {
        ribbons.push_run(run);
    }
    ribbons.finish()
}

/// One ribbon along the polyline, which is solid.
pub(crate) fn stroke_line(points: &[Point<f32>], width: f32, feather: f32) -> Option<Path<Pixels>> {
    let mut ribbons = Ribbons::new(width, feather);
    ribbons.push_run(points);
    ribbons.finish()
}

/// A ribbon along each dash of the polyline: the dash walk writes its ribbons straight into one
/// path, reusing its buffers, so a dashed edge costs no allocation per dash.
pub(crate) fn stroke_dashed(
    points: &[Point<f32>],
    dash: Dash,
    width: f32,
    feather: f32,
) -> Option<Path<Pixels>> {
    let mut ribbons = Ribbons::new(width, feather);
    let mut run = Vec::new();
    walk_dashes(points, dash, &mut run, |dash_points| {
        ribbons.push_run(dash_points);
    });
    ribbons.finish()
}

/// The ribbons of one stroke, and the buffers they are built from.
struct Ribbons {
    path: Option<Path<Pixels>>,
    half: f32,
    feather: f32,
    distinct: Vec<Point<f32>>,
    normals: Vec<(f32, f32)>,
    sides: Vec<[Point<f32>; 2]>,
}

impl Ribbons {
    fn new(width: f32, feather: f32) -> Self {
        Self {
            path: None,
            half: width / 2.,
            feather,
            distinct: Vec::new(),
            normals: Vec::new(),
            sides: Vec::new(),
        }
    }

    fn finish(self) -> Option<Path<Pixels>> {
        self.path
    }

    /// Appends the ribbon of a run: per segment and side, the triangles `(c0, o0, c1)` and
    /// `(o0, o1, c1)`. Neighbouring segments share their offset vertices, so nothing overlaps
    /// (paths blend over each other). A run without two distinct points adds nothing.
    fn push_run(&mut self, run: &[Point<f32>]) {
        self.distinct.clear();
        for at in run {
            if self
                .distinct
                .last()
                .is_none_or(|last| distance(*last, *at) >= MIN_SEGMENT)
            {
                self.distinct.push(*at);
            }
        }
        if self.distinct.len() < 2 {
            return;
        }
        self.offsets();
        let (half, feather) = (self.half, self.feather);
        let path = self
            .path
            .get_or_insert_with(|| Path::new(screen(self.distinct[0])));
        // Four triangles per segment.
        path.vertices.reserve((self.distinct.len() - 1) * 12);
        // `t` at distance `offset` from the centre; without a feather the ribbon is a plain solid.
        let t_at = |offset: f32| if feather > 0. { half - offset } else { SOLID };
        let (t_centre, t_outer) = (t_at(0.), t_at(half + feather));
        for index in 0..self.distinct.len() - 1 {
            let next = index + 1;
            for (o0, o1) in self.sides[index].into_iter().zip(self.sides[next]) {
                let (c0, c1) = (self.distinct[index], self.distinct[next]);
                push(path, [(c0, t_centre), (o0, t_outer), (c1, t_centre)]);
                push(path, [(o0, t_outer), (o1, t_outer), (c1, t_centre)]);
            }
        }
    }

    /// Fills `sides` with both offset points of every distinct point: the unit join direction,
    /// longer at a bend (`1 / cos(half the turn)`, capped at `MITER_LIMIT`) so the ribbon keeps its
    /// width there.
    fn offsets(&mut self) {
        self.normals.clear();
        for pair in self.distinct.windows(2) {
            let (dx, dy) = unit(pair[1].x - pair[0].x, pair[1].y - pair[0].y);
            self.normals.push((-dy, dx));
        }
        let reach = self.half + self.feather;
        self.sides.clear();
        for index in 0..self.distinct.len() {
            let before = self.normals[index.saturating_sub(1)];
            let after = self.normals[index.min(self.normals.len() - 1)];
            let sum = (before.0 + after.0, before.1 + after.1);
            let join = if sum.0.hypot(sum.1) < MIN_SEGMENT {
                // A full reversal has no miter; the next segment's normal is as good as any.
                after
            } else {
                let join = unit(sum.0, sum.1);
                let cos_half = (join.0 * before.0 + join.1 * before.1).max(f32::EPSILON);
                let scale = (1. / cos_half).min(MITER_LIMIT);
                (join.0 * scale, join.1 * scale)
            };
            let at = self.distinct[index];
            let (dx, dy) = (join.0 * reach, join.1 * reach);
            self.sides
                .push([point(at.x + dx, at.y + dy), point(at.x - dx, at.y - dy)]);
        }
    }
}

/// A convex polygon filled solid, with `feather` px of coverage ramp across its outline. `None`
/// for fewer than three points.
pub(crate) fn fill_convex(polygon: &[Point<f32>], feather: f32) -> Option<Path<Pixels>> {
    if polygon.len() < 3 {
        return None;
    }
    let mut path = Path::new(screen(polygon[0]));
    if feather <= 0. {
        fan(&mut path, polygon);
        return Some(path);
    }
    let inner = offset_polygon(polygon, -feather);
    let outer = offset_polygon(polygon, feather);
    fan(&mut path, &inner);
    for index in 0..polygon.len() {
        let next = (index + 1) % polygon.len();
        let (a, b) = (inner[index], inner[next]);
        let (c, d) = (outer[next], outer[index]);
        push(&mut path, [(a, feather), (d, -feather), (b, feather)]);
        push(&mut path, [(d, -feather), (c, -feather), (b, feather)]);
    }
    Some(path)
}

/// Solid triangles from the first point of `polygon` to every other side.
fn fan(path: &mut Path<Pixels>, polygon: &[Point<f32>]) {
    for pair in polygon[1..].windows(2) {
        push(
            path,
            [(polygon[0], SOLID), (pair[0], SOLID), (pair[1], SOLID)],
        );
    }
}

/// The polygon moved outward by `by` px (inward when negative), each corner along its miter.
fn offset_polygon(polygon: &[Point<f32>], by: f32) -> Vec<Point<f32>> {
    let count = polygon.len();
    let area: f32 = (0..count)
        .map(|index| {
            let (a, b) = (polygon[index], polygon[(index + 1) % count]);
            a.x * b.y - b.x * a.y
        })
        .sum();
    let outward = if area >= 0. { 1. } else { -1. };
    let normal = |from: Point<f32>, to: Point<f32>| {
        let (dx, dy) = unit(to.x - from.x, to.y - from.y);
        (dy * outward, -dx * outward)
    };
    (0..count)
        .map(|index| {
            let (previous, here, next) = (
                polygon[(index + count - 1) % count],
                polygon[index],
                polygon[(index + 1) % count],
            );
            let (n1, n2) = (normal(previous, here), normal(here, next));
            // The miter vector reaches `by` px perpendicular to both sides.
            let scale = 1. / (1. + n1.0 * n2.0 + n1.1 * n2.1).max(f32::EPSILON);
            let (mx, my) = ((n1.0 + n2.0) * scale, (n1.1 + n2.1) * scale);
            let cap = (FILL_MITER_LIMIT / mx.hypot(my).max(f32::EPSILON)).min(1.);
            point(here.x + by * mx * cap, here.y + by * my * cap)
        })
        .collect()
}

fn push(path: &mut Path<Pixels>, corners: [(Point<f32>, f32); 3]) {
    let [(a, ta), (b, tb), (c, tc)] = corners;
    path.push_triangle(
        (screen(a), screen(b), screen(c)),
        (point(0., ta), point(0., tb), point(0., tc)),
    );
}

fn screen(at: Point<f32>) -> Point<Pixels> {
    point(px(at.x), px(at.y))
}

fn distance(a: Point<f32>, b: Point<f32>) -> f32 {
    (b.x - a.x).hypot(b.y - a.y)
}

fn lerp(a: Point<f32>, b: Point<f32>, t: f32) -> Point<f32> {
    point(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

fn unit(dx: f32, dy: f32) -> (f32, f32) {
    let length = dx.hypot(dy).max(f32::EPSILON);
    (dx / length, dy / length)
}

#[cfg(test)]
#[path = "topology_stroke_tests.rs"]
mod topology_stroke_tests;
