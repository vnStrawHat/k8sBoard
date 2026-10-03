# 0022b · Color, cards, minimap, level of detail (step 2)

[Back to index](README.md) · Module: `topology_palette.rs` (new; `CanvasColors` moves here from `topology_canvas.rs`, tests in module). Cards: `topology_canvas.rs` `node_card`. Decision 30 of 0022 is replaced by this table.

## Tokens available (gpui-component 0.7 `ThemeColor`, `theme/theme_color.rs`)

| Token | Default Light | Default Dark | Already used for |
|---|---|---|---|
| `red`, `yellow` (+ `_light`) | red/yellow-600 (400) | red/yellow-400 (300) | nothing, but they read as Bad and Warn (`danger`, `warning`) |
| `blue`, `cyan`, `magenta`, `green` (+ `_light`) | -600 (light -400) | -400 (light -300) | free |
| `chart_1..5` | all blues (`#93c5fd`…`#1e40af`) | same | charts; too alike for kinds |
| `ring` | neutral-400 | neutral-500 | the dot grid (step 1) |

Values are from `theme/default-theme.json`. Red and yellow stay reserved for tones, so the kinds share four hues.

## Kind → token (the only table; `KindHue`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KindHue { Ingress, Service, Workload, Pod, ConfigMap, Secret, Claim }
pub(crate) fn kind_hue(kind: TopologyKind) -> KindHue;     // exhaustive match
impl CanvasColors { pub(crate) fn kind(&self, hue: KindHue) -> Hsla; pub(crate) fn kind_text(&self, hue: KindHue) -> Hsla; }
```

| Kinds | Hue | Token |
|---|---|---|
| Ingress | `Ingress` | `magenta` |
| Service | `Service` | `cyan` |
| Deployment, StatefulSet, DaemonSet, ReplicaSet, HPA (Jobs are not nodes, 0022 non-goal) | `Workload` | `blue` |
| Pod (and pod groups) | `Pod` | `blue_light` |
| ConfigMap | `ConfigMap` | `green` |
| Secret | `Secret` | `green_light` |
| PVC | `Claim` | `magenta_light` |

- `kind_text` = `readable_chart_color(kind color)` (`status_tone.rs`; it already pulls a fill hue toward the foreground for text).
- `CanvasColors::of(cx)` resolves every token once per frame. It holds `kinds: [Hsla; 7]`, `kind_texts: [Hsla; 7]`, the relation colors, and the existing fields.

## Relation → edge color

| Relation | Token | Rest alpha |
|---|---|---|
| Owns | `blue` | `EDGE_REST_ALPHA` 0.75 |
| RoutesTo | `cyan` | 0.75 |
| Mounts | `green` | 0.75 |
| Into a ghost | `tone_color` of its check (unchanged) | 1.0 |

The widths and dashes of `relation_stroke` are unchanged. The legend glyphs take the relation color (`legend_color`), as before.

## Cards (`node_card`, at `Text` detail)

| Part | Before | After |
|---|---|---|
| Position and size | fractional | snapped to device px (`snap`, a `scale_factor` parameter) |
| Badge column | `muted` fill, `muted_foreground` text | kind color × `KIND_TINT` (light 0.14, dark 0.22) over the opaque card; text `kind_text` |
| Border at rest | 1 px `border` | unchanged |
| Hover (step 3) | none | 1 px kind color + `shadow_sm()`. A Bad or Warn border keeps its tone; only the shadow is added |
| Selected | 2 px `ring` | 2 px kind color + `shadow_md()`; selection still wins over tones |
| Radius, captions, names, ghosts, unchecked, 0.85 config opacity | W11 | unchanged |

`shadow_sm`/`shadow_md` are GPUI style helpers (`gpui-pre-macros` `styles.rs` l. 428, 441). They draw one shadow primitive each (blurred SDF). The app already uses `shadow_md` (`log_dock.rs`, `row_selection.rs`). There is no resting shadow, matching React Flow's default node.

## Level of detail (decision 39, restyled)

| Zoom | Card | Edges | Arrows | Handles (step 3) |
|---|---|---|---|---|
| ≥ 0.55 | text, tinted badge column | relation dash, feathered | yes | yes |
| 0.3 – 0.55 | badge alone: kind tint fill, `kind_text` (tone when Bad/Warn) | solid, feathered | yes | no |
| < 0.3 | box filled with kind color × `KIND_BOX_TINT` (0.35); Bad/Warn border kept | solid, `MIN_EDGE_WIDTH` floor | no | no |

At an overview zoom the graph reads by color, like the minimap.

## Minimap (`paint_minimap`)

- Node rects take the kind color (tone when Bad/Warn), with a 1 px corner radius. This is React Flow's `nodeColor` function.
- Mask: four quads outside the viewport rect, `background` at 0.55 (React Flow `maskColor`). The 1.5 px `ring` outline stays.

## Bands

Fill `muted` at 0.35 alpha (React Flow group node `rgba(240,240,240,0.25)`), and the 1 px dashed `border` stays. The title is unchanged.

## Color-literal rule

`crates/app/src` stays free of hex, `rgb(`, and `hsla(` (0003 AC4). Tints are `token.opacity(a)`; the shadows come from the GPUI helpers.
