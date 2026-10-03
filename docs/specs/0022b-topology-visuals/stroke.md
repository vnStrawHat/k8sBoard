# 0022b · Anti-aliased strokes (step 1)

[Back to index](README.md) · Module: `topology_stroke.rs` (new, pure, tests in `topology_stroke_tests.rs`). The routes the strokes follow are in [routing.md](routing.md). Root cause: [root-cause.md](root-cause.md). The export is not affected, because resvg already anti-aliases.

## API

All points are screen points in **logical px** (`Point<f32>`, relative to the window, like `screen_point`). The builders return a GPUI `Path<Pixels>` that `paint_path` scales by the scale factor.

```rust
pub(crate) struct Dash { pub(crate) on: f32, pub(crate) off: f32, pub(crate) phase: f32 } // logical px
pub(crate) enum PathCoverage { SignedDistance, MsaaOnly }          // how the renderer reads `t`
const PATH_COVERAGE: PathCoverage = if cfg!(windows) { SignedDistance } else { MsaaOnly };
/// The half width of the coverage ramp: 0.5 device px for `SignedDistance`, 0 for `MsaaOnly`.
pub(crate) fn feather(scale_factor: f32) -> f32;                    // feather_for(PATH_COVERAGE, scale)
fn feather_for(coverage: PathCoverage, scale_factor: f32) -> f32;   // pure; tested both ways on every OS
pub(crate) fn flatten_cubic(start, ctrl1, ctrl2, end: Point<f32>, tolerance: f32) -> Vec<Point<f32>>;
pub(crate) fn trim_end(points: &mut Vec<Point<f32>>, length: f32);  // cuts the last `length` px (arrow base)
pub(crate) fn stroke_line(points: &[Point<f32>], width: f32, feather: f32) -> Option<Path<Pixels>>;
pub(crate) fn stroke_dashed(points: &[Point<f32>], dash: Dash, width: f32, feather: f32) -> Option<Path<Pixels>>;
pub(crate) fn fill_convex(polygon: &[Point<f32>], feather: f32) -> Option<Path<Pixels>>;
// tests: dash_runs (the dashes as runs) and stroke_path (the ribbons of runs) build the same vertices
```

`None` means nothing to draw: no run with two distinct points, or a polygon with fewer than 3 points.

## Geometry rules

| Rule | Value |
|---|---|
| Flatten count | Wang's bound: `n = clamp(ceil(sqrt(3 × max(\|P0 − 2P1 + P2\|, \|P1 − 2P2 + P3\|) / (4 × tolerance))), 2, 128)` segments, uniform in the curve parameter. The deviation stays within `tolerance` (0.2 in the test, 0.1 graph units for the routes) |
| Zero-length segments | skipped before offsetting |
| Join normal | the normalized sum of the two segment normals. The offset is scaled by `1 / cos(θ/2)`, clamped to `MITER_LIMIT` = 2 |
| Caps | butt. Dash ends get no feather (see the README engineering note) |
| Dash walk | arc length along the polyline, one scratch buffer for the dash being built, and the ribbons written straight into one path. The pattern starts at `−phase mod (on + off)`, so a growing phase moves the dashes toward the end. A negative gap counts as none |
| Ribbon (per segment, per side) | the vertices are centre `c`, outer `o = c ± n × (half + feather)`. Triangles `(cᵢ, oᵢ, cᵢ₊₁)` and `(oᵢ, oᵢ₊₁, cᵢ₊₁)` |
| `st` of a ribbon vertex | `(0, half − offset)`: the centre is `(0, half)`, the outer edge `(0, −feather)` |
| Convex fill | the inset polygon (each vertex moved in by `feather / sin(α/2)` along its bisector), fanned with `st = (0, 1)`. A ring of quads (inset to outset) per side gets `t = +feather` inside and `−feather` outside |
| `feather == 0` | outer offset = `half`; the ring is omitted. This is the plain MSAA stroke |

A ribbon has no overlapping triangles. Paths blend premultiplied-over (`create_blend_state_for_path_rasterization`), so an overlap would darken the joins.

## Edge painting (replaces `paint_edges`)

Per visible edge (culling is unchanged):

1. `route = layout.routes[i]` (a polyline in graph units, computed once per layout, [routing.md](routing.md)) → screen points.
2. `emphasis = edge_emphasis(edge, focus)` (step 3; `Rest` until then).
3. Width = `max((relation width + focus extra) × zoom, MIN_EDGE_WIDTH)`, where `MIN_EDGE_WIDTH` = 0.75 logical px. With analytic coverage a 0.75 px line reads as a lighter 1 px line, not a broken one.
4. Dash: none below `MIN_TEXT_ZOOM`. An animated edge keeps the dash of its relation (a solid one gets long dashes), with a moving phase (step 4). Otherwise the relation dash × zoom, with phase 0.
5. If arrows show (zoom ≥ `MIN_BADGE_ZOOM`), `trim_end(points, (ARROW_LENGTH + ARROW_TIP_GAP) × zoom)`, so the translucent stroke does not overlap the arrow.
6. `stroke_dashed(..)` or `stroke_line(..)` with `feather(window.scale_factor())` → `paint_path(color)`.
7. Arrow: `arrow_head(route, ..)` with `ARROW_LENGTH` = 12 and `ARROW_HALF_WIDTH` = 5.5 (graph units × zoom), its tip `ARROW_TIP_GAP` = 1 short of the handle, along the last segment of the route → `fill_convex(.., feather)`, the relation color at full alpha (solid at rest). The joins are mitered: round joins would cost a curve fit for a 1 px gain.

The export draws the same arrow as a marker whose base is the end of the trimmed line.

## Dot grid (same step)

| Item | Before | After |
|---|---|---|
| Spacing | 16 graph units | `DOT_SPACING` 20 (React Flow `gap`) |
| Dot | 1 × 1 px square | `DOT_SIZE` 1.5 px round quad (`corner_radii` = half), a fixed screen size |
| Position | fractional | origin snapped: `(v × scale).round() / scale` |
| Skip below | 8 px spacing | `MIN_DOT_SPACING` 12 px (fewer quads when zoomed out) |
| Color | `border` | `ring` (light neutral-400 ≈ React Flow `#91919a`; dark neutral-500 ≈ `#555`) |

## Cost

- Vertices per segment: 12 (4 triangles) with a feather, against lyon's 6. 500 dashed edges of 300 px are about 276 k vertices.
- `edge_stroke_budget` (test-plan.md): 500 dashed edges at zoom 1, `flatten` + `stroke_dashed`. It asserts a deterministic vertex ceiling and prints the best of three times, which decisions.md "Measurements" records (release 2.2 ms against the 3 ms target).
- The curves are flattened once per layout (the routes), not per frame. A pan moves every point and builds the ribbons again; the measured release time makes a ribbon cache unnecessary.
