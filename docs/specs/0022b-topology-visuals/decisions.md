# 0022b · Decisions

[Back to index](README.md). Architect defaults. The user's open questions are in the README.

## React Flow reference (what "matches React Flow" means here)

| Element | React Flow default | Source | 0022b |
|---|---|---|---|
| Node | `width 150px`, `padding 10px`, `border-radius 3px`, `1px solid #1a192b`, bg `#fff`; dark bg `#1e1e1e`, border `#3c3c3c` | `xyflow/packages/system/src/styles/style.css` | the 0022 card (170 × 54, radius 8) is kept; theme `background` / `border` |
| Node hover / selected | `0 1px 4px 1px rgba(0,0,0,.08)` / `0 0 0 .5px #1a192b` | same, `--xy-node-boxshadow-*` | `shadow_sm` / 2 px kind border + `shadow_md` |
| Handle | 6 × 6 px, round, bg `#1a192b`, 1 px `#fff` border | style.css `.xy-flow__handle` | 6 × zoom dot, kind fill, `background` ring |
| Edge | bezier (`default` type), stroke `#b1b1b7`, width 1; selected `#555`; dark `#3e3e3e` | reactflow.dev/api-reference/types/edge, `init.css` | bezier kept; relation colors; 1.2–1.6 × zoom |
| Marker | `ArrowClosed`, 12.5 × 12.5 in `strokeWidth` units | `xyflow/packages/react/src/container/EdgeRenderer/MarkerDefinitions.tsx` | filled 8 × 7 arrow, feathered |
| Animated edge | `stroke-dasharray: 5`, `dashdraw 0.5s linear infinite` from offset 10 | `init.css` | `FLOW_DASH` (5, 5), 20 units/s |
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
| 13 | No resting card shadow; shadows only on hover and selection | React Flow default; one fewer primitive per card |

## Measurements (filled by the coder)

| Item | Value |
|---|---|
| `edge_stroke_budget` release timing (target ≤ 3 ms) | |
| Frame time while the flow runs (`monitoring`, drawer open, release) | |
| Edge pixel census, v61 → after (light, dark) | |
