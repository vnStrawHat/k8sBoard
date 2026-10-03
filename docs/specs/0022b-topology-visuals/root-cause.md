# 0022b · Why the edges are jagged

[Back to index](README.md). Read from the sources in `.cargo-home/registry/src/index.crates.io-*/` (gpui-pre 0.3.7, gpui-pre-windows 0.3.7, gpui-pre-apple 0.3.7, gpui-pre-wgpu 0.3.7, lyon_tessellation 1.0.22). The app runs on Windows, so the DirectX 11 renderer draws it.

## How Topology paints today

| Element | Primitive | Anti-aliasing |
|---|---|---|
| Edges | `PathBuilder::stroke(w × zoom.max(0.5))` + `dash_array`, `cubic_bezier_to`, `paint_path` (`topology_canvas.rs` `paint_edges`) | 4× MSAA only (below) |
| Arrows | `PathBuilder::fill()` triangle, `paint_path` | 4× MSAA only |
| Cards, bands, minimap | divs and `paint_quad` | analytic (quad SDF, `shaders.hlsl` `quad_fragment` l. 559, `antialias_threshold = 0.5` l. 585) |
| Dot grid | 1 × 1 px `paint_quad` at fractional positions | analytic, but blurred across 2 px |

## Root cause

1. **Paths get no analytic coverage.** `PathBuilder::build_path` emits every triangle with `st = (0, 1)` at all three vertices (`gpui-pre/src/path_builder.rs` l. 322–345). The path fragment shader turns `st` into coverage (Loop–Blinn: `f = s² − t`, `alpha = saturate(0.5 − f / |∇f|)`). With a constant `st`, the gradient is 0 and `alpha` is always 1.
2. **On Windows the curve branch is inverted.** HLSL tests `if (length(float2(dx.x, dy.x))) alpha = 1.0;` (`gpui-pre-windows/src/shaders.hlsl` l. 996–1012). Metal and WGSL test `length(..) < 0.001` (`gpui-pre-apple/src/shaders.metal` l. 772–792, `gpui-pre-wgpu/src/shaders.wgsl` l. 1080–1100). So on Windows even `Path::curve_to` triangles get no analytic edge.
3. **The only smoothing is 4× MSAA.** Paths are rasterized into an MSAA texture, `PATH_MULTISAMPLE_COUNT = 4` (`directx_renderer.rs` l. 28–29, texture l. 1440–1467, resolve l. 660–676). Four samples give five coverage levels, so a shallow line shows stair steps. Our layout is full of shallow lines: the bezier ends are horizontal (`edge_curve`, control points at `dx / 2`).
4. **Thin strokes make it worse.** Width is `1.2–1.6 × max(zoom, 0.5)`, so at the first-view zoom 0.56 (decision 38) a stroke is 0.7–0.9 px. Dashes `[5, 4]` and `[2, 3]` × zoom become 1–3 px pieces that 4× MSAA draws as broken blobs.
5. **Not the cause:** curve flattening. lyon flattens at `StrokeOptions::DEFAULT_TOLERANCE = 0.1` px (`lyon_tessellation/src/lib.rs` l. 367), and dashes are measured at 0.01 (`path_builder.rs` l. 281).
6. **Nothing to switch on.** The sample count is a private constant. Metal also uses 4 (`gpui-pre-apple/src/metal_renderer.rs` l. 40). wgpu picks the first of `[4, 2, 1]` that the format supports (`wgpu_renderer.rs` l. 2577). No public API sets it, and `D3D11_RASTERIZER_DESC.AntialiasedLineEnable` is `false` (l. 1482); line primitives are not used anyway.

Screenshots show exactly what the window shows: `render_to_image` calls the same `render()` (`directx_renderer.rs` l. 344–404, l. 413).

## The fix: feathered ribbons through the same shader

`Path::new` and `Path::push_triangle(xy, st)` are public (`gpui-pre/src/scene.rs` l. 803, 876). With `s ≡ 0` across a triangle, the HLSL else-branch reduces to:

```
gradient = −∇t,  f = −t,  alpha = saturate(0.5 + t / |∇t|)
```

If `t` is the **signed distance in logical px to the stroke edge** (positive inside), then `t / |∇t|` is that distance in device px: the derivatives are per device pixel. Coverage ramps over exactly **one device pixel**, centred on the edge, at any zoom and any scale factor. It is the same shader and the same MSAA pass, with no new pipeline and no new crate.

- **Stroke ribbon.** Flatten the cubic, offset each point by ±(`half + feather`) along the averaged normal, and emit two triangles per side and segment with `st = (0, half − offset)`. `feather = 0.5 / scale_factor`. The centre gets `t = half`, the outer edge `t = −feather`. See [stroke.md](stroke.md).
- **Filled arrow.** The inset polygon is solid (`st = (0, 1)`). A ring between inset and outset gets `t = ±feather`.
- **Other renderers.** Metal and WGSL treat `s ≡ 0` as solid, so a feather band would draw opaque. There `feather = 0`: the same geometry as a plain stroke, with MSAA as today. The choice is one `cfg!(windows)` constant, and tests run both values on every OS (decision 3).

## Rejected options

| Option | Why not |
|---|---|
| Raise the MSAA count | private const in `gpui-pre-windows`; needs a fork |
| Wider strokes only | still five coverage levels; React Flow uses 1 px |
| Rasterize edges with `resvg::tiny_skia` per frame | CPU raster plus a texture upload on every pan frame, and atlas churn; keep it as a fallback if a GPU misbehaves |
| Pixel-snapping the bezier | does not help diagonals |

Small extra fixes: dot-grid dots and card origins snap to device pixels (`(v × scale).round() / scale`). Dashes become solid below `MIN_TEXT_ZOOM`.
