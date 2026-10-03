# 0022b · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own, and every new item has a production user in its step. There is no `Cargo.toml` or `Cargo.lock` change, and `crates/cluster` is not touched. Prerequisite: 0022 merged (it is, at `0350adb`).

## Code (`crates/app/src`)

| S | File | Change |
|---|---|---|
| 1 | `topology_stroke.rs` (new), `topology_stroke_tests.rs` (new) | `Dash`, `feather`, `flatten_cubic`, `trim_end`, `dash_runs`, `stroke_path`, `fill_convex` (stroke.md) |
| 1 | `topology_canvas.rs` | `paint_edges` uses the stroke module (no more `PathBuilder`); `MIN_EDGE_WIDTH`; solid below `MIN_TEXT_ZOOM`; `ARROW_LENGTH`, `ARROW_HALF_WIDTH`, and `arrow_head(curve, length, half_width)`; dot grid (`DOT_SPACING` 20, `DOT_SIZE`, `MIN_DOT_SPACING` 12, `ring`, snapped); `snap` |
| 1 | `main.rs` | `mod topology_stroke;` |
| 2 | `topology_colors.rs` (new) | `CanvasColors` (moved from `topology_canvas.rs`, extended), `KindHue`, `kind_hue`, relation colors, `EDGE_REST_ALPHA`, `KIND_TINT`, `KIND_BOX_TINT` |
| 2 | `topology_canvas.rs` | `node_card` takes `scale_factor` (snap), tints the badge column, kind selection border + `shadow_md`, box-tint LOD; `paint_minimap` uses kind colors and draws the mask; band fill; legend colors come from the palette |
| 2 | `topology_view.rs` | `render_canvas(.., window)` passes `window.scale_factor()`; imports from `topology_colors` |
| 2 | `main.rs` | `mod topology_colors;` |
| 3 | `topology_canvas.rs` | `Emphasis`, `edge_emphasis`, `focus` in `CanvasPaint`, alphas and widths; `handle_points`, `handle_canvas`; `CardState.is_hovered` (kind border + `shadow_sm`) |
| 3 | `topology_view.rs` | `hovered: Option<NodeId>`, card `on_hover`, clears (namespace, rebuild, Escape); controls panel (`Plus`, `Minus`, `Maximize`), `zoom_by_button` |
| 3 | `topology_viewport.rs` | `ZOOM_BUTTON_STEPS`; a button zoom uses `zoom_at` at the view centre (a helper only if the test needs one) |
| 3 | `launch_options.rs`, `screenshot.rs` (+ tests) | `--screen topology-selected`: select the first Deployment node; set `reduce_motion` for that capture |
| 4 | `topology_canvas.rs` | `FLOW_DASH`, `FLOW_SPEED`, `flow_phase`, `needs_flow_frame`; animated dashes; `request_animation_frame` in the paint closure |
| 4 | `topology_view.rs` | `created: Instant` (the flow clock); passes the elapsed time into `CanvasPaint` |
| 5 | `topology_export.rs`, `topology_export_tests.rs` | `svg_style` from `CanvasColors`; `kinds`, `kind_texts`, `relations`, `band`; markers per color; handles; band fill; badge tint |

Polish round: `topology_route.rs` (+ tests) is new (routes per layout), `topology_card.rs` (+ tests) takes the card code out of `topology_canvas.rs`, whose tests move to `topology_canvas_tests.rs`; `topology_layout.rs` drops empty columns, sizes the cards by their names, and computes the routes; `topology_graph.rs` gets `ghost_tone`; `topology_viewport.rs` gets `snap`, `reveal`, and the first-view rules. The split of `topology_view.rs` waits for 0027.

## Docs (updated by the coder in the step that completes the row)

- `docs/specs/0022-topology/canvas.md`:
  - "Painting": the edge rows point to 0022b stroke.md; the dot grid row is updated (step 1).
  - Colors and cards point to 0022b colors.md (step 2).
  - "Interaction": add hover, the panel, and the flow (steps 3–4).
- `docs/specs/0022-topology/decisions.md`: decisions 25, 30, and 31 get "replaced by 0022b" notes (steps 2–3).
- `docs/specs/0022-topology/README.md` non-goals: "zoom buttons … animation" → "see 0022b" (step 3).
- [decisions.md](decisions.md) "Measurements": the budget (step 1), the census (step 1), and the frame time (step 4).
- `docs/roadmap/inventory-screens.md`: the W11 rows note "visuals 0022b" (step 5).
