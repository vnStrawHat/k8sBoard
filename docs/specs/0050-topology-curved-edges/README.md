# 0050 — Topology curved edges

Status: approved with changes (advisor, 2026-10-04). **Amended 2026-10-05: see [as-built.md](as-built.md); Elbows and the `Edges:` option were removed, so edges are always curves with four anchors per node.** Follows 0022 / 0022b. Crate: `crates/app` only, no dependency change. User request (2026-10-04): an option to draw Topology edges as smooth curves instead of right-angle elbows. W11 already draws cubic `C` edges. Rules: theme tokens only, names only, English.

## Goal

- `EdgeShape { Elbows (default), Curves }`: **geometry only**. Color, dash, width, emphasis, flow, and 0049 traffic styling stay per edge and ignore the shape.
- A toolbar dropdown `Edges: Elbows | Curves`, remembered as `topology.edges` (`"elbows"` / `"curves"`).
- The canvas, handles, culling, and PNG/SVG export follow the shape through `TopologyLayout.routes`. The minimap draws no edges.

## Non-goals

- Edge hit-testing, hover, and labels: none exist (hover and selection are per card). None is added.
- Bundling, `smoothstep`, a per-edge shape, a Settings › Appearance entry (Group by is not there either), and self edges (0022 builds none).

## Geometry (`topology_route.rs`, pure, graph units)

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum EdgeShape { #[default] Elbows, Curves }
pub(crate) fn route_edges(graph, rects, bands, shape: EdgeShape) -> Vec<EdgeRoute>;
fn ports(source: GraphRect, target: GraphRect) -> Ports; // start, out (±1), end, into (±1): the first lines of `along_lanes`
fn bezier(ports: Ports) -> Vec<GraphPoint>;              // replaces `between_columns`
const ARRIVAL_STUB: f32 = 13.; // test: == ARROW_LENGTH + ARROW_TIP_GAP (no route -> canvas import)
const LOOP_REACH: f32 = 24.;
```

**Ports.** They are today's lane-route ports: leave by the side facing the target, enter by the side facing the lane. `along_lanes` calls `ports`, so both shapes share endpoints.

**`bezier`.** One cubic with horizontal tangents (React Flow's default edge):

- `base = end − into × ARRIVAL_STUB` and `forward = (base.x − start.x) × out`.
- `reach = if out == into && forward > 0. { forward / 2. } else { LOOP_REACH }`. The floor applies only to U-turns. A floor on a forward S would make it run backwards for gaps of 20–37 after a drag.
- `c1 = start + (out × reach, 0)` and `c2 = base − (into × reach, 0)`.
- Points: `flatten_cubic(start, c1, c2, base, FLATTEN_TOLERANCE)`, then `end`, which is a straight horizontal stub.

Why the stub: `trim_end` removes exactly it, so the stroke ends on the true tangent at the arrow base, and `end_direction` reads the stub, so the arrow is exactly horizontal. Without the stub, loops and steep S-curves tilt the arrow by 6–20°.

**Free check.** The curve lies inside the box of its 4 control points plus `end`. If no other card (grown by `CLEARANCE`) meets that box, the curve is free. Otherwise run `space.is_free(..)` per segment.

**Elbows** works as today. The neighbour curve now uses `bezier`, so it gains the stub (a small visible change that fixes the arrow).

**Curves**, for every edge of any relation: `bezier(ports)` if free, else `round_corners(&along_lanes(..))` directly. It does not call the Elbows route, so a blocked neighbour edge is not flattened and checked twice. No edge runs behind a card it does not join (the 0022b invariant).

| Case | Curves result |
|---|---|
| Neighbour columns, forward | S-curve, the same as Elbows |
| Same column (out = +1, into = −1) | C-loop in the right gutter, peak ≈ `right + 25.8` |
| Backward, or a skipped column with a free path | one S-curve |
| Path blocked by a card | the lane route with rounded corners (decided) |
| Overlapping or touching pinned cards (forward ≤ 0, out == into) | `LOOP_REACH`; may pass over its own two cards, as lane routes do today |
| Several edges on one port | the curves fan out (no bus); overlaps are accepted, and hover dimming separates them |

## Wiring

- `topology_layout::layout(.., previous, edges: EdgeShape)` passes `edges` to `route_edges`. Card positions do not depend on it.
- `TopologyView.edge_shape` is read from `AppSettings::get(cx).topology.edges` in `new`, and `install` and `relayout` pass it.
- `set_edge_shape(shape, cx)`: no-op if equal; otherwise set it, `relayout` (seeded by the previous layout, as on a drag), and `cx.notify()`. The toolbar handler then calls `AppSettings::update(cx, |s| s.topology.edges = shape)`.
- Toolbar: `Button::new("topology-edges")`, outline, small, label `Edges: Elbows|Curves`, a caret, the tooltip "How edges are drawn", and a 2-item checked `dropdown_menu` (the Group by pattern), after Group by. Do not touch the Resources/Traffic segment (0049).
- Settings: 0050 **creates** the `topology` section (`group_by` and pins stay memory-only). `Settings.topology: TopologySettings { edges: EdgeShape }`, `#[serde(default)]`, `skip_serializing_if = "is_default"`.
- Export (`topology_export.rs`): delete `markers()` (:225) and its call (:178). `edge_svg` writes the edge `<path class="edge" ..>` without `marker-end`, then a `<polygon>` from `arrow_head(route, ARROW_LENGTH, ARROW_HALF_WIDTH)` in the edge color at opacity 1. This is the screen's triangle; a marker's `orient="auto"` follows the last flattened segment, which tilts on curves.
- `--screen topology-curves`: add `LaunchScreen::TopologyCurves` (parse, USAGE, `shows_topology`, screen mapping). `app_shell` calls `set_edge_shape(Curves)` in memory only and writes nothing.

## Performance

Routes are still built once per layout, which includes every drag move, on the main thread. Only the active shape is built. Curves costs one flatten (≤ 128 segments) plus the box pre-check per edge. A blocked edge also pays `is_free` and then the lane search, so the worst case is about 2× Elbows. Painting is unchanged.

## Steps for the coder (one step)

1. `topology_route.rs`: `EdgeShape`, `ports`, `bezier`, the box pre-check, and the shape parameter. Delete `between_columns`.
2. The `layout()` parameter: a mechanical `EdgeShape::Elbows` at existing call sites (layout, route, export, and viewport tests).
3. The settings section, view field, toolbar, export polygons, and launch screen.

## Acceptance criteria

- [ ] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`, no `Cargo.toml` change.
- [ ] 2. Geometry tests call `bezier(ports(..))` on hand-built rects, with span D ≤ 1,000 and flatness tolerance 0.5. Past D ≈ 2,700 the first and last chord y-step (≈ 3D/count²) exceeds 0.5.
  - `a_curve_leaves_flat_and_ends_in_a_horizontal_stub`: the stub length is ≈ `ARRIVAL_STUB` (± 1e-3).
  - `a_same_column_edge_is_a_loop_in_the_gutter`: x in the closed range `[right, right + LANE_OFFSET + 1]` (peak ≈ 25.8, so this is tight).
  - `a_backward_edge_is_one_s_curve`.
  - `a_close_neighbour_curve_never_runs_backwards`: a 25-unit gap via pins; x is non-decreasing in both shapes.
  - `the_arrow_of_a_curve_points_along_its_end_tangent`: the `arrow_head` axis is horizontal and the tip is at `end − ARROW_TIP_GAP`.
  - `arrival_stub_is_the_arrow_length_and_gap`.
- [ ] 3. Invariant tests on the `keda`/`monitoring` fixtures:
  - `curves_keep_the_ports_of_elbows`
  - `no_curved_edge_runs_through_a_card_it_does_not_join`
  - `a_blocked_curve_keeps_the_lane_route`
  - `the_edge_shape_does_not_move_cards` (`rects` are equal)
- [ ] 4. Settings: `topology_edges_round_trip_and_default_is_omitted`. `settings_keys_are_the_allow_list` gains `topology` and `topology.edges` (`collect_keys` records parents). `full_settings()` sets `edges: Curves`, because `skip_serializing_if` hides the default. Launch: `topology_curves_screen_parses`.
- [ ] 5. Export tests:
  - `edge_paths` (tests :60) matches `class="edge"`, not `marker-end`. This affects `svg_dash_per_relation`.
  - `svg_has_a_marker_for_every_edge_color` is replaced by `svg_arrows_are_polygons_at_the_route_end` (one polygon per edge with the `arrow_head` points; no `<marker`).
  - The other export tests pass unchanged.
- [ ] 6. All existing route and layout tests pass, changed only by the `EdgeShape::Elbows` argument. `topology_budget` gains a Curves run that prints its time, with the counts unchanged.
- [ ] 7. The 0003 color-literal grep is clean. No `tracing::` in the topology modules. No new Kubernetes request.
- [ ] 8. ui-verifier, light and dark, `monitoring`:
  - `topology-curves`: curves with the arrow on the tangent, no edge behind a card, the label `Edges: Curves`, and a PNG export that matches the screen.
  - `topology` (default): elbows as before.
  - The toolbar at a 1024 px window: no overflow or clipping.

## Files the coder touches

`topology_route.rs`, `topology_route_tests.rs`, `topology_layout.rs`, `topology_layout_tests.rs`, `topology_view.rs` (field, `install`/`relayout`, `render_toolbar`, `set_edge_shape`), `topology_viewport.rs` (one test call), `topology_export.rs`, `topology_export_tests.rs`, `settings.rs`, `settings_tests.rs`, `launch_options.rs`, `launch_options_tests.rs`, `app_shell.rs` (launch wiring), and `screenshot.rs` (only if the settle match needs the variant). `topology_canvas.rs` is not touched.

## Decisions (2026-10-04)

1. Blocked curves fall back to the lane route; no edge passes behind a card.
2. The default stays Elbows (today's look).
