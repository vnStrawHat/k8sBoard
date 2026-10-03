# 0022b · Anti-aliased strokes (step 1)

[Back to index](README.md) · Module: `topology_stroke.rs` (new, pure, tests in `topology_stroke_tests.rs`). Root cause: [root-cause.md](root-cause.md). The export is not affected, because resvg already anti-aliases.

## API

All points are screen points in **logical px** (`Point<f32>`, relative to the window, like `screen_point`). The builders return a GPUI `Path<Pixels>` that `paint_path` scales by the scale factor.

```rust
pub(crate) struct Dash { pub(crate) on: f32, pub(crate) off: f32, pub(crate) phase: f32 } // logical px
/// The half width of the coverage ramp: 0.5 device px on Windows, 0 elsewhere (root-cause.md).
pub(crate) fn feather(scale_factor: f32) -> f32;                    // if cfg!(windows) { 0.5 / scale } else { 0. }
pub(crate) fn flatten_cubic(start: Point<f32>, ctrl1: Point<f32>, ctrl2: Point<f32>, end: Point<f32>) -> Vec<Point<f32>>;
pub(crate) fn trim_end(points: &mut Vec<Point<f32>>, length: f32);  // cuts the last `length` px (arrow base)
pub(crate) fn dash_runs(points: &[Point<f32>], dash: Dash) -> Vec<Vec<Point<f32>>>;
pub(crate) fn stroke_path(runs: &[Vec<Point<f32>>], width: f32, feather: f32) -> Option<Path<Pixels>>;
pub(crate) fn fill_convex(polygon: &[Point<f32>], feather: f32) -> Option<Path<Pixels>>;
```

`None` means nothing to draw: no run with two distinct points, or a polygon with fewer than 3 points.

## Geometry rules

| Rule | Value |
|---|---|
| Flatten count | `n = clamp(ceil(control polygon length / FLATTEN_STEP), 2, 128)` segments, uniform in the curve parameter. `FLATTEN_STEP` = 4 px. The deviation stays ≤ 0.2 px on the test curves |
| Zero-length segments | skipped before offsetting |
| Join normal | the normalized sum of the two segment normals. The offset is scaled by `1 / cos(θ/2)`, clamped to `MITER_LIMIT` = 2 |
| Caps | butt. Dash ends get no feather (see the README engineering note) |
| Dash walk | arc length along the polyline. The pattern starts at `−phase mod (on + off)`, so a growing phase moves the dashes toward the end |
| Ribbon (per segment, per side) | the vertices are centre `c`, outer `o = c ± n × (half + feather)`. Triangles `(cᵢ, oᵢ, cᵢ₊₁)` and `(oᵢ, oᵢ₊₁, cᵢ₊₁)` |
| `st` of a ribbon vertex | `(0, half − offset)`: the centre is `(0, half)`, the outer edge `(0, −feather)` |
| Convex fill | the inset polygon (each vertex moved in by `feather / sin(α/2)` along its bisector), fanned with `st = (0, 1)`. A ring of quads (inset to outset) per side gets `t = +feather` inside and `−feather` outside |
| `feather == 0` | outer offset = `half`; the ring is omitted. This is the plain MSAA stroke |

A ribbon has no overlapping triangles. Paths blend premultiplied-over (`create_blend_state_for_path_rasterization`), so an overlap would darken the joins.

## Edge painting (replaces `paint_edges`)

Per visible edge (culling is unchanged):

1. `curve = edge_curve(..)` → screen points; `points = flatten_cubic(..)`.
2. `emphasis = edge_emphasis(edge, focus)` (step 3; `Rest` until then).
3. Width = `max((relation width + focus extra) × zoom, MIN_EDGE_WIDTH)`, where `MIN_EDGE_WIDTH` = 0.75 logical px. With analytic coverage a 0.75 px line reads as a lighter 1 px line, not a broken one.
4. Dash: none below `MIN_TEXT_ZOOM`. An animated edge uses `FLOW_DASH` (step 4). Otherwise the relation dash × zoom, with phase 0.
5. If arrows show (zoom ≥ `MIN_BADGE_ZOOM`), `trim_end(points, ARROW_LENGTH × zoom)`, so the translucent stroke does not overlap the arrow.
6. `stroke_path(dash_runs(..) or [points], width, feather(window.scale_factor()))` → `paint_path(color)`.
7. Arrow: `arrow_head` with `ARROW_LENGTH` = 8 and `ARROW_HALF_WIDTH` = 3.5 (graph units × zoom) → `fill_convex(.., feather)`, the same color.

`arrow_head(curve, size)` gains a half-width argument; the export keeps its marker.

## Dot grid (same step)

| Item | Before | After |
|---|---|---|
| Spacing | 16 graph units | `DOT_SPACING` 20 (React Flow `gap`) |
| Dot | 1 × 1 px square | `DOT_SIZE` 1.5 px round quad (`corner_radii` = half), a fixed screen size |
| Position | fractional | origin snapped: `(v × scale).round() / scale` |
| Skip below | 8 px spacing | `MIN_DOT_SPACING` 12 px (fewer quads when zoomed out) |
| Color | `border` | `ring` (light neutral-400 ≈ React Flow `#91919a`; dark neutral-500 ≈ `#555`) |

## Cost

- Vertices per segment: 12 (4 triangles) with a feather, against lyon's 6. With the flatten cap and culling, 500 visible edges stay under about 250 k vertices.
- `edge_stroke_budget` (test-plan.md): 500 dashed edges at zoom 1, `flatten` + `dash_runs` + `stroke_path`, ≤ 30 ms in the debug gate. The release time goes in decisions.md "Measurements" (target ≤ 3 ms).
- Nothing is cached between frames. A pan moves every point; add a cache only if a profile shows the need.
