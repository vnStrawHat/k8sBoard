# 0022b · Export parity (step 5)

[Back to index](README.md) · Module: `topology_export.rs`. The SVG and PNG show the **rest state** of the new look: no hover, selection, flow, controls, or dot grid. resvg anti-aliases, so step 1 has no export change.

## One token table

`svg_style(cx)` is built from `CanvasColors::of(cx)` (colors.md), not from its own theme reads. `hex()` stays; it drops alpha, so translucency is written as `fill-opacity` / `stroke-opacity`.

```rust
pub(crate) struct SvgStyle {
    // existing: background, border, text, muted, warn, bad, font_family
    // `badge` (theme.muted) is renamed `band`; `accent` is removed
    pub(crate) band: String,
    pub(crate) kinds: [String; 7],       // KindHue order
    pub(crate) kind_texts: [String; 7],
    pub(crate) relations: [String; 3],   // Owns, RoutesTo, Mounts
}
```

## Mapping

| Element | Screen (colors.md, interaction.md) | SVG |
|---|---|---|
| Chip, bar, surface | solid kind chip, 3 px kind bar, kind-tinted card surface (colors.md) | `<rect rx="7" fill="{kind}">`, text `fill="{kind_text}"`, bar clipped by a per-card `clipPath`, card `fill="{card_fill}"` |
| Card | `background`, 1 px `border`, tone borders | unchanged |
| Edge | relation color, `EDGE_REST_ALPHA` | `stroke="{relation}" stroke-opacity="0.8"` |
| Edge into a ghost | tone | unchanged (`warn`/`bad`, opacity 1) |
| Arrow marker | `fill_convex`, the edge color | one `<marker>` per color: `owns`, `routes`, `mounts`, `warn`, `bad` (replaces `muted`/`accent`); the path is `M0,0 L12,5.5 L0,11 z`, from `ARROW_LENGTH` and `ARROW_HALF_WIDTH`, with `refX` 0: the line is trimmed to the arrow base, as on screen, and the marker starts there |
| Handles | `handle_points`, kind fill, `background` ring | `<circle r="3" fill="{kind}" stroke="{background}" stroke-width="1">` at both ends of every edge |
| Band | `muted` × `band_alpha` fill, dashed card border, title pill | `fill="{band}" fill-opacity="{band_alpha}"`, a pill `<rect>` and the title `<text>` |
| Edge path | the route of the layout | `M x y L x y ..`, the rounded polyline, so the export bends where the screen does |
| Legend | swatches of the relations | a dashed `<path>` with a `<polygon>` arrow and the label, as on screen |

The edge is drawn up to the arrow base (step 1 `trim_end`). In SVG the marker's `refX` = 8 already puts the tip on the target. The stroke runs under the marker, and the opacity difference is too small to matter in a static document; the marker is drawn at full opacity.

## Unchanged

- Size, scale, the `MAX_EXPORT_SIDE` cap, the title strip, font fitting, the save flow, and file names (0022 export.md).
- Secret safety: names only. There is still no `tracing::` call in the topology modules.
