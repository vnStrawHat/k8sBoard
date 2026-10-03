# 0022b · Decisions

[Back to index](README.md). Architect defaults. The user's open questions are in the README.

## React Flow reference (what "matches React Flow" means here)

| Element | React Flow default | Source | 0022b |
|---|---|---|---|
| Node | `width 150px`, `padding 10px`, `border-radius 3px`, `1px solid #1a192b`, bg `#fff`; dark bg `#1e1e1e`, border `#3c3c3c` | `xyflow/packages/system/src/styles/style.css` | card 200 to 280 × 60, radius 8, 13 px name; theme `background` / `border` |
| Node hover / selected | `0 1px 4px 1px rgba(0,0,0,.08)` / `0 0 0 .5px #1a192b` | same, `--xy-node-boxshadow-*` | `shadow_sm` / 2 px kind border + `shadow_md` |
| Handle | 6 × 6 px, round, bg `#1a192b`, 1 px `#fff` border | style.css `.xy-flow__handle` | 6 × zoom dot, kind fill, `background` ring |
| Edge | bezier (`default` type), stroke `#b1b1b7`, width 1; selected `#555`; dark `#3e3e3e` | reactflow.dev/api-reference/types/edge, `init.css` | bezier kept; relation colors; 1.2–1.6 × zoom |
| Marker | `ArrowClosed`, 12.5 × 12.5 in `strokeWidth` units | `xyflow/packages/react/src/container/EdgeRenderer/MarkerDefinitions.tsx` | filled 12 × 11 arrow, feathered |
| Animated edge | `stroke-dasharray: 5`, `dashdraw 0.5s linear infinite` from offset 10 | `init.css` | the relation dash, 20 units/s |
| Background | Dots, gap 20, size 1; dots `#91919a`, dark `#555` | reactflow.dev/api-reference/components/background, theming page | gap 20, 1.5 px, `ring` |
| MiniMap | bottom right, `nodeColor` fn, radius 5, mask `rgba(240,240,240,.6)` | reactflow.dev/api-reference/components/minimap | kind colors, mask `background` × 0.55 |
| Controls | bottom left, vertical: zoom in, zoom out, fit, lock; 26 px buttons | reactflow.dev/api-reference/components/controls, `init.css` | +, −, Fit (no lock: nothing is editable) |

## Rendering

| # | Decision | Rationale |
|---|---|---|
| 1 | **Feathered ribbons through the existing path shader** (root-cause.md) | the only analytic coverage the platform exposes; no fork, no new crate, no CPU raster |
| 2 | Own flattening and dashing (`topology_stroke.rs`) instead of `PathBuilder` | `PathBuilder` cannot set `st`, and its `dash_array` has no phase, which the flow needs |
| 3 | `feather = 0.5 / scale` on Windows, `0` elsewhere, by one `cfg!(windows)` constant; the builders take it as a parameter | HLSL and Metal/WGSL disagree on the branch; tests run both values on every OS |
| 4 | Width floor `MIN_EDGE_WIDTH` 0.75 px, dashes solid below 0.55 | sub-pixel dashes are noise at overview zooms |
| 5 | Snap card origins and sizes, and dots, to device px | crisp 1 px borders; the dots no longer smear |

## Color and look

| # | Decision | Rationale |
|---|---|---|
| 6 | Kinds use `magenta`, `cyan`, `blue`, `green` and their `_light` variants; `red`/`yellow` stay for tones | Bad and Warn must stay unambiguous; `chart_*` are all blue in the default theme |
| 7 | Edges are colored by **relation** (owns `blue`, routes `cyan`, mounts `green`) at alpha 0.75 | keeps the legend's meaning; the color matches the kinds each relation usually links |
| 8 | Card color goes in the **badge column** (tint plus text), not the whole card | the text stays on a neutral fill in both themes; tones still own the border |
| 9 | A controls panel (+, −, Fit) replaces 0022 decision 25's "no zoom buttons"; the header Fit stays | React Flow parity; W11 still shows header Fit |
| 10 | Flow animation runs only on the selected node's edges, via `request_animation_frame`, with a zero-cost idle state; a 30 fps timer is the fallback | the request asks for React Flow quality; a full-graph animation would repaint forever |
| 11 | Hover dims other edges, but not cards | edges are cheap (per-frame paint); dimming cards would rebuild every card style on each hover |
| 12 | The export is the rest state with the same token table | an incident document must not depend on what was hovered |
| 13 | A faint resting `shadow_xs`; stronger shadows on hover and selection (the polish round replaced "no resting shadow") | the light cards need to lift off the canvas; one blurred primitive per visible card |
| 14 | Polish round: cards 200 to 280 × 60 (gutter 50, row pitch 80), name 13 px UI font, caption 11 px mono, 10 px padding; the first view never goes below zoom 0.8; LOD text still from zoom 0.55 (name about 7 px) | the 0022 sizes were 8 px at the first view; React Flow nodes use about 12 px text |
| 15 | Kind color is a solid 30 px chip (bold mono letters, text by WCAG contrast of `background`/`foreground`), a 3 px left bar, and a kind-tinted surface (light 5 %, dark 10 %). No kit icon fits the kinds, so the letters stay | the badge column tint was barely visible, in dark especially |
| 16 | Dark cards are raised 85 % toward `muted` with a border pulled 45 % toward `muted_foreground`; bands fill `muted` at 0.6 (light) / 0.5 (dark) | cards must separate from both the canvas and the band |
| 17 | Edges: 1.5 px, routes dash (7, 4), mounts dash (4, 3), rest alpha 0.8, dimmed alpha 0.1, focus +1 px, arrow 12 × 11 with its tip 1 px short of the handle and solid at rest, handle dots at least 6 px | mounts dots were nearly invisible; focused and dimmed edges looked alike in dark |
| 18 | Band titles are pills (card surface, card border, 11.5 px mono); the selected card gets a kind-color glow (`BoxShadow`), stronger and wider in dark | the app name must read, and selection needs more than a border |
| 19 | Edges follow routes computed once per layout: a curve between neighbouring columns, else lanes in the gutters and a free corridor ([routing.md](routing.md)) | review: an edge behind a card looked like a link to it; no cached flatten per frame |
| 20 | `PathCoverage` (`SignedDistance` on Windows, `MsaaOnly` elsewhere) with a Cargo.lock pin test and a one-line fallback (root-cause.md) | the fix relies on an inverted shader branch of one gpui version |
| 21 | The flow repaints with a 33 ms `cx.spawn` timer behind `needs_flow_frame`, restarted by the window activation; no `request_animation_frame` | the display-rate request repainted the whole window every frame |
| 22 | Columns no node uses are dropped; the card width follows the longest name (cap 280); every card has a tooltip | argocd opened cut off, keda had an empty strip, and long names were cut with no way to read them |
| 23 | Each relation keeps its dash while it flows; solid owns edges flow in long dashes (16, 4) | routes and mounts must stay distinguishable from owns when selected |
| 24 | The legend swatches use the stroke code of the edges; the legend and the minimap move left of the drawer while it is open; the graph keeps 60 px clear of the zoom panel | overlays must not cover content |
| 25 | `edge_stroke_budget` asserts a vertex ceiling, not a time | a wall-clock assertion fails on a loaded machine |

## Measurements (filled by the coder)

| Item | Value |
|---|---|
| `edge_stroke_budget` release timing (target ≤ 3 ms) | 2.2 ms best of 3 for 500 dashed edges (276 000 vertices; release with LTO off, 16 codegen units), under the 3 ms target, so no ribbon cache. The curves are flattened once per layout. Debug: 9 ms. The gate asserts the vertex ceiling (500 000), not the time. Layout with routes: 6 ms release for the 340-node budget graph (56 ms debug against its 80 ms limit) |
| Frame time while the flow runs (`monitoring`, drawer open, release) | The 30 fps timer ships (decision 21), so a frame is at most one repaint per 33 ms. Not measured on a display (the capture hook renders once); the CPU part is the stroke build above, about 0.5 ms for the visible edges of `monitoring` |
| Edge pixel census, v61 → v67 → v68 → v69 (light, dark) | `monitoring`, three solid shallow edges, 20 columns × 7 rows around each (`.tmp/census.py`). Light: 6, 4, 4 → 29, 32, 28. Dark: 6, 4, 4 → 31, 34, 30. v61 is above the spec's expected ≤ 3 because the dot grid and the arrow neighbourhood add a few colors to the window; v67 is far above 8. After the polish round (v68, `.tmp/census68.py`, three long dashed mounts edges, 20 columns × 7 rows): light 12, 17, 21 and dark 13, 17, 22. The edges are dashed now, so there are fewer edge pixels per window, and smoothness holds | v69 (`.tmp/census69.py`, the Deployment-to-ReplicaSet S-curves of `keda`, 36 × 36 px windows, blue-ish colors only; the windows also hold a handle dot and the accent bar): light 75, 75, 73; dark 70, 69, 69. The curves are smooth (checked at 8x).
