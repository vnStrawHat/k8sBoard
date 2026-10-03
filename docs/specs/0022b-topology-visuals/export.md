# 0022b · Export parity (step 5)

[Back to index](README.md) · Module: `topology_export.rs`. The SVG and PNG show the **rest state** of the new look: no hover, selection, flow, controls, or dot grid. resvg anti-aliases, so step 1 has no export change.

## One token table

`svg_style(cx)` is built from `CanvasColors::of(cx)` (palette.md), not from its own theme reads. `hex()` stays; it drops alpha, so translucency is written as `fill-opacity` / `stroke-opacity`.

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

| Element | Screen (palette.md, interaction.md) | SVG |
|---|---|---|
| Badge column | kind × `KIND_TINT` | `<rect fill="{kind}" fill-opacity="{KIND_TINT}">`, text `fill="{kind_text}"` |
| Card | `background`, 1 px `border`, tone borders | unchanged |
| Edge | relation color, `EDGE_REST_ALPHA` | `stroke="{relation}" stroke-opacity="0.75"` |
| Edge into a ghost | tone | unchanged (`warn`/`bad`, opacity 1) |
| Arrow marker | `fill_convex`, the edge color | one `<marker>` per color: `owns`, `routes`, `mounts`, `warn`, `bad` (replaces `muted`/`accent`); the path is `M0,0 L8,3.5 L0,7 z`, matching `ARROW_HALF_WIDTH` |
| Handles | `handle_points`, kind fill, `background` ring | `<circle r="3" fill="{kind}" stroke="{background}" stroke-width="1">` at both ends of every edge |
| Band | `muted` × 0.35 fill, dashed `border` | `fill="{band}" fill-opacity="0.35"` |
| Legend | relation colors | `style.relations[..]`, as on screen |

The edge is drawn up to the arrow base (step 1 `trim_end`). In SVG the marker's `refX` = 8 already puts the tip on the target. The stroke runs under the marker, and the opacity difference is too small to matter in a static document; the marker is drawn at full opacity.

## Unchanged

- Size, scale, the `MAX_EXPORT_SIDE` cap, the title strip, font fitting, the save flow, and file names (0022 export.md).
- Secret safety: names only. There is still no `tracing::` call in the topology modules.
