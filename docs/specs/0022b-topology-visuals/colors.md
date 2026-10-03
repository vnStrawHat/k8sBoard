# 0022b · Color, cards, minimap, level of detail (step 2)

[Back to index](README.md) · Module: `topology_colors.rs` (new; `CanvasColors` moves here from `topology_canvas.rs`, tests in module). Cards: `topology_canvas.rs` `node_card`. Decision 30 of 0022 is replaced by this table.

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
| ServiceAccount, RoleBinding, ClusterRoleBinding, Role, ClusterRole ([0022 RBAC layer](../0022-topology/rbac-layer.md)) | `Access` | `cyan_light` |

- `kind_text` = `readable_chart_color(kind color)` (`status_tone.rs`; it already pulls a fill hue toward the foreground for text).
- `CanvasColors::of(cx)` resolves every token once per frame. It holds `kinds: [Hsla; 7]`, `kind_texts: [Hsla; 7]`, the relation colors, and the existing fields.

## Relation → edge color

| Relation | Token | Rest alpha |
|---|---|---|
| Owns | `blue` | `EDGE_REST_ALPHA` 0.8 |
| RoutesTo | `cyan` | 0.8 |
| Mounts | `green` | 0.8 |
| Access ([0022 RBAC layer](../0022-topology/rbac-layer.md)) | `cyan_light` | 0.8 |
| Into a ghost | `tone_color` of its check (unchanged) | 1.0 |

`relation_stroke`: 1.5 px for every relation; routes dash (7, 4), mounts dash (4, 3), access dash (2, 3), owns solid. The legend swatches are drawn with the same stroke code and the relation color.

## Cards (`node_card`, at `Text` detail)

| Part | Before | After |
|---|---|---|
| Position and size | fractional | snapped to device px (`snap`, a `scale_factor` parameter) |
| Chip and bar | `muted` badge column | a solid 30 px kind-color chip (radius 7) with the two letters in bold mono 11 px, and a 3 px kind-color bar on the left edge. The chip text is whichever of `background` and `foreground` contrasts more (WCAG, at least 4.5 : 1 for every kind in both themes), `kind_text` |
| Surface | `background` | light: `background` (white) plus a trace of the kind (5 %); dark: `background` raised 85 % toward `muted` plus 10 % of the kind. `card_border` is `border`, pulled 45 % toward `muted_foreground` in dark |
| Text | 9 / 10.8 px mono | caption 11 px mono uppercase `muted_foreground`; name 13 px in the theme UI font, `foreground`; 10 px padding. Card 200 to 280 wide by 60 (the longest name decides, [routing.md](routing.md)), gutter 50, row pitch 80. Every card has a tooltip: the full name, or the check text of a ghost |
| Resting shadow | none | `shadow_xs`; hover `shadow_md`; selected: a kind-color glow, per theme in `CanvasColors.glow` (light alpha 0.45, blur 12, spread 2; dark alpha 0.6, blur 16, spread 3) |
| Border at rest | 1 px `border` | unchanged |
| Hover (step 3) | none | 1 px kind color + `shadow_md()`. A Bad or Warn border keeps its tone; only the shadow is added |
| Selected | 2 px `ring` | 2 px kind color + the kind glow; selection still wins over tones |
| Radius, captions, names, ghosts, unchecked, 0.85 config opacity | W11 | unchanged |

`shadow_xs`/`shadow_md` are GPUI style helpers (`gpui-pre-macros` `styles.rs`). They draw one shadow primitive each (blurred SDF). The glow is one `BoxShadow` in the kind color (a theme token, so no color literal). The polish round added the faint resting shadow.

## Level of detail (decision 39, restyled)

| Zoom | Card | Edges | Arrows | Handles (step 3) |
|---|---|---|---|---|
| ≥ 0.55 | text, solid chip and bar | relation dash, feathered | yes | yes |
| 0.3 – 0.55 | a solid chip with the badge, centered (the tone color when Bad/Warn) | solid, feathered | yes | no |
| < 0.3 | box filled with kind color × `KIND_BOX_TINT` (0.4); Bad/Warn border kept | solid, `MIN_EDGE_WIDTH` floor | no | no |

At an overview zoom the graph reads by color, like the minimap.

## Minimap (`paint_minimap`)

- Node rects take the kind color (tone when Bad/Warn), with a 1 px corner radius. This is React Flow's `nodeColor` function.
- Mask: four quads outside the viewport rect, `background` at 0.55 (React Flow `maskColor`). The 1.5 px `ring` outline stays.

## Bands

Fill `muted` at `band_alpha` (light 0.6, dark 0.5), and the 1 px dashed `card_border` stays. The title is a pill (`card` fill, `card_border`, mono 11.5 px, 20 px high) 12 px in and 7 px down from the band corner; below zoom 0.55 it keeps a fixed 11 px size on the band edge. The first view never goes below zoom 0.8 (the name is then about 10 px); Fit still goes further out.

## Color-literal rule

`crates/app/src` stays free of hex, `rgb(`, and `hsla(` (0003 AC4). Tints are `token.opacity(a)`; the shadows come from the GPUI helpers.
