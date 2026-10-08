use std::time::{Duration, Instant};

use super::*;

const TOLERANCE: f32 = 1e-3;

fn pt(x: f32, y: f32) -> Point<f32> {
    point(x, y)
}

fn xy(vertex: &gpui_kit::PathVertex<Pixels>) -> (f32, f32) {
    (
        f32::from(vertex.xy_position.x),
        f32::from(vertex.xy_position.y),
    )
}

fn line(from: (f32, f32), to: (f32, f32)) -> Vec<Point<f32>> {
    vec![pt(from.0, from.1), pt(to.0, to.1)]
}

/// The curves of the three edge shapes at zoom 2: forward, mounts, and backward.
fn curves() -> [[Point<f32>; 4]; 3] {
    [
        [pt(0., 0.), pt(200., 0.), pt(200., 140.), pt(400., 140.)],
        [pt(0., 0.), pt(0., 100.), pt(240., 100.), pt(240., 200.)],
        [pt(400., 108.), pt(400., 296.), pt(0., 296.), pt(0., 108.)],
    ]
}

fn distance_to_segment(p: Point<f32>, a: Point<f32>, b: Point<f32>) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / (dx * dx + dy * dy)).clamp(0., 1.);
    distance(p, pt(a.x + dx * t, a.y + dy * t))
}

#[test]
fn flattened_cubic_stays_within_a_fifth_of_a_pixel() {
    for [start, ctrl1, ctrl2, end] in curves() {
        let points = flatten_cubic(start, ctrl1, ctrl2, end, 0.2);
        for step in 0..=400 {
            let sample = cubic_at(start, ctrl1, ctrl2, end, step as f32 / 400.);
            let deviation = points
                .windows(2)
                .map(|pair| distance_to_segment(sample, pair[0], pair[1]))
                .fold(f32::INFINITY, f32::min);
            assert!(deviation <= 0.2, "{deviation} px at step {step}");
        }
        assert_eq!(points.first(), Some(&start));
        assert_eq!(points.last(), Some(&end));
    }
}

#[test]
fn shorter_curves_flatten_to_fewer_points() {
    let bent = |scale: f32| {
        flatten_cubic(
            pt(0., 0.),
            pt(scale, 0.),
            pt(scale, scale),
            pt(2. * scale, scale),
            0.2,
        )
    };
    assert!(bent(10.).len() < bent(100.).len());
    // Even a point has a segment to offset, and a huge curve stops at the cap.
    let point_curve = flatten_cubic(pt(1., 1.), pt(1., 1.), pt(1., 1.), pt(1., 1.), 0.2);
    assert_eq!(point_curve.len(), 3);
    assert_eq!(bent(1e5).len(), MAX_SEGMENTS + 1);
    // A straight line needs no more than the minimum, and a finer tolerance asks for more.
    let line = flatten_cubic(pt(0., 0.), pt(10., 0.), pt(20., 0.), pt(30., 0.), 0.2);
    assert_eq!(line.len(), 3);
    let coarse = flatten_cubic(pt(0., 0.), pt(60., 0.), pt(60., 60.), pt(120., 60.), 0.4);
    let fine = flatten_cubic(pt(0., 0.), pt(60., 0.), pt(60., 60.), pt(120., 60.), 0.05);
    assert!(coarse.len() < fine.len());
}

#[test]
fn zero_length_segments_are_skipped() {
    let clean = stroke_path(&[line((0., 0.), (10., 0.))], 2., 0.5).expect("a path");
    let repeated = vec![pt(0., 0.), pt(0., 0.), pt(10., 0.), pt(10., 0.)];
    let path = stroke_path(&[repeated], 2., 0.5).expect("a path");
    assert_eq!(path.vertices.len(), clean.vertices.len());
    for vertex in &path.vertices {
        assert!(xy(vertex).0.is_finite() && xy(vertex).1.is_finite());
    }
}

fn starts_and_ends(runs: &[Vec<Point<f32>>]) -> Vec<(f32, f32)> {
    runs.iter()
        .map(|run| (run[0].x, run[run.len() - 1].x))
        .collect()
}

#[test]
fn dash_runs_follow_the_pattern() {
    let dash = Dash {
        on: 5.,
        off: 3.,
        phase: 0.,
    };
    let runs = dash_runs(&line_points((0., 0.), (20., 0.)), dash);
    let spans = starts_and_ends(&runs);
    let expected = [(0., 5.), (8., 13.), (16., 20.)];
    assert_eq!(spans.len(), expected.len());
    for (span, wanted) in spans.iter().zip(expected) {
        assert!((span.0 - wanted.0).abs() < TOLERANCE && (span.1 - wanted.1).abs() < TOLERANCE);
    }
}

fn line_points(from: (f32, f32), to: (f32, f32)) -> Vec<Point<f32>> {
    line(from, to)
}

#[test]
fn dashes_continue_around_a_corner() {
    let corner = vec![pt(0., 0.), pt(6., 0.), pt(6., 10.)];
    let dash = Dash {
        on: 5.,
        off: 3.,
        phase: 0.,
    };
    let runs = dash_runs(&corner, dash);
    // The second dash starts at arc length 8, which is 2 px down the second segment.
    assert_eq!(runs.len(), 2);
    let second = &runs[1];
    assert!((second[0].y - 2.).abs() < TOLERANCE && (second[0].x - 6.).abs() < TOLERANCE);
}

#[test]
fn a_growing_phase_moves_dashes_toward_the_end() {
    let points = line_points((0., 0.), (40., 0.));
    let first_dash = |phase: f32| {
        let dash = Dash {
            on: 5.,
            off: 5.,
            phase,
        };
        let runs = dash_runs(&points, dash);
        (runs[0][0].x, runs[0][runs[0].len() - 1].x)
    };
    assert_eq!(first_dash(0.), (0., 5.));
    // The pattern shifted by 2: the first dash starts 2 px in.
    let (start, end) = first_dash(2.);
    assert!((start - 2.).abs() < TOLERANCE && (end - 7.).abs() < TOLERANCE);
    // Shifted by 7, the dash that began at -3 is cut at the start of the line, ending at 2.
    let (start, end) = first_dash(7.);
    assert!(start.abs() < TOLERANCE && (end - 2.).abs() < TOLERANCE);
}

#[test]
fn trim_end_shortens_by_the_arrow_length() {
    let mut points = vec![pt(0., 0.), pt(10., 0.), pt(10., 10.)];
    trim_end(&mut points, 4.);
    assert_eq!(points, vec![pt(0., 0.), pt(10., 0.), pt(10., 6.)]);
    trim_end(&mut points, 12.);
    assert_eq!(points.len(), 2);
    assert!((points[1].x - 4.).abs() < TOLERANCE && points[1].y == 0.);
    trim_end(&mut points, 100.);
    assert_eq!(points, vec![pt(0., 0.)]);
}

#[test]
fn ribbon_vertices_carry_the_signed_edge_distance() {
    let (half, feather) = (1., 0.5);
    let path = stroke_path(&[line((0., 10.), (30., 10.))], 2. * half, feather).expect("a path");
    assert!(!path.vertices.is_empty());
    let mut outer = 0;
    for vertex in &path.vertices {
        let offset = (xy(vertex).1 - 10.).abs();
        assert_eq!(vertex.st_position.x, 0.);
        assert!((vertex.st_position.y - (half - offset)).abs() < TOLERANCE);
        if offset > half {
            assert!((offset - (half + feather)).abs() < TOLERANCE);
            assert!((vertex.st_position.y + feather).abs() < TOLERANCE);
            outer += 1;
        }
    }
    assert!(outer > 0);
}

#[test]
fn zero_feather_is_the_plain_stroke() {
    let path = stroke_path(&[line((0., 10.), (30., 10.))], 2., 0.).expect("a path");
    for vertex in &path.vertices {
        assert_eq!(vertex.st_position, point(0., 1.));
        assert!((xy(vertex).1 - 10.).abs() <= 1. + TOLERANCE);
    }
    let fill = fill_convex(&[pt(0., 0.), pt(8., 3.5), pt(0., 7.)], 0.).expect("a path");
    assert_eq!(fill.vertices.len(), 3);
    assert!(fill.vertices.iter().all(|v| v.st_position == point(0., 1.)));
}

#[test]
fn consecutive_segments_share_offset_vertices() {
    let points = vec![pt(0., 0.), pt(10., 0.), pt(20., 6.)];
    let path = stroke_path(&[points], 2., 0.5).expect("a path");
    // Per segment and side, 6 vertices: (c0, o0, c1) and (o0, o1, c1).
    let at = |segment: usize, side: usize, slot: usize| {
        xy(&path.vertices[(segment * 2 + side) * 6 + slot])
    };
    for side in 0..2 {
        let end_of_first = at(0, side, 4);
        let start_of_second = at(1, side, 1);
        assert!((end_of_first.0 - start_of_second.0).abs() < TOLERANCE);
        assert!((end_of_first.1 - start_of_second.1).abs() < TOLERANCE);
    }
}

#[test]
fn a_sharp_join_clamps_its_miter() {
    let (half, feather) = (1., 0.5);
    let hairpin = vec![pt(0., 0.), pt(10., 0.), pt(0., 1.)];
    let path = stroke_path(&[hairpin], 2. * half, feather).expect("a path");
    let limit = MITER_LIMIT * (half + feather) + TOLERANCE;
    for vertex in &path.vertices {
        let (x, y) = xy(vertex);
        assert!(distance(pt(x, y), pt(10., 0.)) <= limit.max(10.2));
        let reach = distance(pt(x, y), pt(10., 0.));
        if (reach - 0.).abs() > f32::EPSILON && x > 9. {
            assert!(reach <= limit, "{reach} beyond {limit}");
        }
    }
}

#[test]
fn convex_fill_feathers_every_side() {
    let (feather, triangle) = (0.5, [pt(0., 0.), pt(8., 3.5), pt(0., 7.)]);
    let path = fill_convex(&triangle, feather).expect("a path");
    // The solid core, then a ring of two triangles per side.
    assert_eq!(path.vertices.len(), 3 + 3 * 6);
    let line_distance = |p: Point<f32>, a: Point<f32>, b: Point<f32>| {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        ((p.x - a.x) * dy - (p.y - a.y) * dx).abs() / dx.hypot(dy)
    };
    for vertex in &path.vertices[3..] {
        let (x, y) = xy(vertex);
        let nearest = (0..3)
            .map(|i| line_distance(pt(x, y), triangle[i], triangle[(i + 1) % 3]))
            .fold(f32::INFINITY, f32::min);
        // Inner ring points sit `feather` inside (t = +feather), outer ones `feather` outside.
        assert!((nearest - feather).abs() < 0.01, "{nearest}");
        assert!((vertex.st_position.y.abs() - feather).abs() < TOLERANCE);
    }
    assert!(path.vertices[..3].iter().all(|v| v.st_position.y == SOLID));
}

#[test]
fn degenerate_input_builds_no_path() {
    assert!(stroke_path(&[], 2., 0.5).is_none());
    assert!(stroke_path(&[vec![pt(1., 1.)]], 2., 0.5).is_none());
    assert!(stroke_path(&[vec![pt(1., 1.), pt(1., 1.)]], 2., 0.5).is_none());
    assert!(fill_convex(&[pt(0., 0.), pt(1., 1.)], 0.5).is_none());
    assert!(
        dash_runs(
            &[pt(0., 0.)],
            Dash {
                on: 1.,
                off: 1.,
                phase: 0.
            }
        )
        .is_empty()
    );
    assert!(
        dash_runs(
            &line((0., 0.), (5., 0.)),
            Dash {
                on: 0.,
                off: 1.,
                phase: 0.
            }
        )
        .is_empty()
    );
}

#[test]
fn feather_is_half_a_device_pixel_on_windows_only() {
    for scale in [1., 1.5, 2.] {
        // Both coverages are checked on every OS; the build picks one of them.
        assert!((feather_for(PathCoverage::SignedDistance, scale) - 0.5 / scale).abs() < TOLERANCE);
        assert_eq!(feather_for(PathCoverage::MsaaOnly, scale), 0.);
        let expected = if cfg!(windows) { 0.5 / scale } else { 0. };
        assert!((feather(scale) - expected).abs() < TOLERANCE);
    }
}

#[test]
fn the_renderer_version_the_coverage_relies_on_is_pinned() {
    // `PathCoverage::SignedDistance` reads `t` as a distance because of how gpui-pre-windows
    // 0.3.8 evaluates `path_rasterization_fragment`. A newer version may fix its inverted curve
    // branch, which would change what a feathered ribbon draws.
    let lock = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock")).replace(
        "
", "
",
    );
    assert!(
        lock.contains(
            "name = \"gpui-pre-windows\"
version = \"0.3.8\"
"
        ),
        "re-check shaders.hlsl path_rasterization_fragment; if the branch is fixed set PathCoverage to MsaaOnly (root-cause.md)"
    );
}

#[test]
fn a_negative_gap_is_a_solid_dash() {
    let dash = Dash {
        on: 5.,
        off: -2.,
        phase: 0.,
    };
    let runs = dash_runs(&line_points((0., 0.), (20., 0.)), dash);
    // With no gap the dashes touch: the line is covered end to end.
    let covered: f32 = runs.iter().map(|run| run[run.len() - 1].x - run[0].x).sum();
    assert!((covered - 20.).abs() < 0.01, "{covered}");
}

#[test]
fn a_dashed_stroke_equals_the_ribbons_of_its_runs() {
    let points = flatten_cubic(pt(0., 0.), pt(60., 0.), pt(60., 40.), pt(120., 40.), 0.2);
    let dash = Dash {
        on: 6.,
        off: 4.,
        phase: 3.,
    };
    let direct = stroke_dashed(&points, dash, 1.5, 0.5).expect("a path");
    let runs = stroke_path(&dash_runs(&points, dash), 1.5, 0.5).expect("a path");
    assert_eq!(direct.vertices.len(), runs.vertices.len());
    for (a, b) in direct.vertices.iter().zip(&runs.vertices) {
        assert_eq!(xy(a), xy(b));
        assert_eq!(a.st_position, b.st_position);
    }
}

/// `path_rasterization_fragment` of `shaders.hlsl` (gpui-pre-windows 0.3.8, l. 1004-1012), line for
/// line. `ds` and `dt` are the screen derivatives of `s` and `t`, per device pixel.
fn hlsl_path_alpha(s: f32, t: f32, ds: (f32, f32), dt: (f32, f32)) -> f32 {
    let (dx, dy) = ((ds.0, dt.0), (ds.1, dt.1));
    if dx.0.hypot(dy.0) != 0. {
        return 1.;
    }
    let gradient = (2. * s * dx.0 - dx.1, 2. * s * dy.0 - dy.1);
    let f = s * s - t;
    let distance = f / gradient.0.hypot(gradient.1);
    (0.5 - distance).clamp(0., 1.)
}

/// The `t` and its gradient (per logical px) at `(x, y)`, from the triangle that holds it.
fn locate(path: &Path<Pixels>, x: f32, y: f32) -> Option<(f32, (f32, f32))> {
    path.vertices.chunks(3).find_map(|triangle| {
        let [a, b, c] = [&triangle[0], &triangle[1], &triangle[2]];
        let ((ax, ay), (bx, by), (cx, cy)) = (xy(a), xy(b), xy(c));
        let det = (by - cy) * (ax - cx) + (cx - bx) * (ay - cy);
        if det.abs() < 1e-9 {
            return None;
        }
        let w0 = ((by - cy) * (x - cx) + (cx - bx) * (y - cy)) / det;
        let w1 = ((cy - ay) * (x - cx) + (ax - cx) * (y - cy)) / det;
        let w2 = 1. - w0 - w1;
        if w0 < -1e-6 || w1 < -1e-6 || w2 < -1e-6 {
            return None;
        }
        let (ta, tb, tc) = (a.st_position.y, b.st_position.y, c.st_position.y);
        let gx = (ta * (by - cy) + tb * (cy - ay) + tc * (ay - by)) / det;
        let gy = (ta * (cx - bx) + tb * (ax - cx) + tc * (bx - ax)) / det;
        Some((w0 * ta + w1 * tb + w2 * tc, (gx, gy)))
    })
}

#[test]
fn ribbon_alpha_ramps_over_one_device_pixel() {
    let width = 1.5;
    let half = width / 2.;
    for scale in [1., 1.5, 2.] {
        let feather = 0.5 / scale;
        let path = stroke_path(&[line((0., 10.), (50., 10.))], width, feather).expect("a path");
        let edge = half * scale;
        let mut previous = 1.;
        let mut step = 0;
        loop {
            let distance = step as f32 * 0.1;
            step += 1;
            if distance > edge + 1.5 {
                break;
            }
            let alpha = locate(&path, 25., 10. + distance / scale).map_or(0., |(t, gradient)| {
                let per_pixel = (gradient.0 / scale, gradient.1 / scale);
                hlsl_path_alpha(0., t, (0., 0.), per_pixel)
            });
            if distance <= edge - 0.5 {
                assert!(alpha >= 0.99, "scale {scale}: {alpha} at {distance}");
            }
            if distance >= edge + 0.5 {
                assert!(alpha <= 0.01, "scale {scale}: {alpha} at {distance}");
            }
            if (distance - edge).abs() < 0.05 {
                assert!(
                    (alpha - 0.5).abs() <= 0.05 + 1e-3,
                    "scale {scale}: {alpha} at the edge"
                );
            }
            assert!(
                alpha <= previous + 1e-4,
                "scale {scale}: rises at {distance}"
            );
            previous = alpha;
        }
    }
}

#[test]
fn edge_stroke_budget() {
    let dash = Dash {
        on: 5.,
        off: 4.,
        phase: 0.,
    };
    // The best of three runs is printed: the other tests of the binary share the machine, so the
    // time is a measurement and the vertex count is the assertion (a deterministic ceiling).
    let (mut vertices, mut best) = (0, Duration::MAX);
    for _ in 0..3 {
        let started = Instant::now();
        vertices = 0;
        for index in 0..500 {
            let shift = index as f32 * 0.37;
            let points = flatten_cubic(
                pt(shift, 0.),
                pt(shift + 120., 0.),
                pt(shift + 120., 80.),
                pt(shift + 240., 80.),
                0.2,
            );
            vertices +=
                stroke_dashed(&points, dash, 1.4, 0.5).map_or(0, |path| path.vertices.len());
        }
        best = best.min(started.elapsed());
    }
    eprintln!("500 dashed edges: {vertices} vertices in {best:?}");
    assert!(vertices > 0);
    assert!(vertices <= MAX_VERTICES_PER_500_DASHED_EDGES, "{vertices}");
}

/// A ceiling for the vertices of 500 dashed edges of 300 px: about 12 vertices per segment.
const MAX_VERTICES_PER_500_DASHED_EDGES: usize = 500_000;
