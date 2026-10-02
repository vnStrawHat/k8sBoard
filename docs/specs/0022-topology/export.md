# 0022 · Export PNG / SVG (step 3)

[Back to index](README.md) · Module: `topology_export.rs` (new, tests in module). It uses `file_export.rs` (`ExportState`, `export_file_name`, which sanitizes per the 0021 amendment) from 0021 step 4. C9 is user-approved (0019 decision 22). Decisions 32–35.

## Feasibility (checked at `1ea46f4`)

| Option | Verdict |
|---|---|
| `Window::render_to_image` | **Rejected.** It is `#[cfg(any(test, feature = "test-support"))]` in gpui-pre 0.3.7 (the app enables it only in the dev `screenshot` feature), and it captures the window, not the whole graph |
| `resvg 0.46` | **Chosen.** It is a non-optional dependency of `gpui-pre 0.3.7` (features `text`, `system-fonts`, `memmap-fonts`, `raster-images`), and it is already in `Cargo.lock` with `usvg 0.46`, `tiny-skia 0.11.4` (depends on `png`), and `fontdb`. A direct dependency adds **no package** |

## Cargo change (the only one in 0022)

```toml
# root Cargo.toml [workspace.dependencies]
# Rasterizes the Topology SVG export; the same version gpui-pre already builds (no new package).
resvg = { version = "0.46", default-features = false, features = ["text", "system-fonts"] }
# crates/app/Cargo.toml [dependencies]
resvg.workspace = true
```

AC: `git diff Cargo.lock` adds exactly one line, `"resvg 0.46.0",`, to the `k8sboard` package's dependencies. Use `resvg::usvg` and `resvg::tiny_skia`; add no other direct dependency.

## SVG builder (pure)

```rust
pub(crate) struct SvgStyle { pub(crate) background: String, pub(crate) border: String, pub(crate) text: String,
    pub(crate) muted: String, pub(crate) accent: String, pub(crate) warn: String, pub(crate) bad: String,
    pub(crate) font_family: String }                          // colors as #rrggbb
pub(crate) fn svg_style(cx: &App) -> SvgStyle;                // theme tokens + tone_color through `hex`
/// `#rrggbb` by hand from `Rgba::from(hsla)`; channels rounded from 0–1 to 0–255. Alpha is dropped:
/// the export background is opaque, and translucent fills set `fill-opacity` themselves.
fn hex(color: Hsla) -> String;
pub(crate) fn topology_svg(graph: &TopologyGraph, layout: &TopologyLayout, title: &str, style: &SvgStyle) -> String;
```

- The view box is `layout.extent`, plus a 32-unit title strip on top. Order:
  1. the background rect;
  2. the `title`: `Topology · {context} · ns: {ns} · {n} resources · {RFC 3339 now}`;
  3. the bands;
  4. the edges: `<path d="M… C…">` with the canvas.md curves, widths, and dashes, and `marker-end` from three `<marker>`s (muted, accent, bad);
  5. the nodes: `<rect rx="8">`, badge, caption, name. Ghost and unchecked looks use `stroke-dasharray`;
  6. the legend at the bottom right, in the same glyph text as on screen.
- Text uses the canvas styles. Names are cut by `fit_chars(width, font_size) = floor(width / (font_size × 0.6))` with `…`, because mono fonts are about 0.6 em wide. Every string is XML-escaped (`&`, `<`, `>`, `"`, `'`).
- It holds the same data as the screen: names, captions, and tones only, never a Secret value (AC 8).

## Rasterizing (background executor)

```rust
#[derive(Debug)]
pub(crate) enum ExportError { Render(String) /* usvg parse */, EmptyImage, Encode(String), Write(std::io::Error) }
impl std::fmt::Display for ExportError { /* "could not render the topology: {e}", "the image is empty", … */ }
pub(crate) struct Png { pub(crate) bytes: Vec<u8>, pub(crate) scale: f32 }
/// SVG text → PNG. Loads the system fonts (100–500 ms), so it never runs on the main thread.
pub(crate) fn render_png(svg: &str) -> Result<Png, ExportError>;
pub(crate) fn export_scale(width: f32, height: f32) -> f32;    // min(EXPORT_SCALE 2.0, MAX_EXPORT_SIDE 4096 / longest side)
```

1. `let mut options = usvg::Options::default(); options.fontdb_mut().load_system_fonts();`
2. `usvg::Tree::from_str(svg, &options)`. An error becomes `Render`.
3. `scale = export_scale(..)`, then `tiny_skia::Pixmap::new(w, h)`. `None` becomes `EmptyImage`.
4. `resvg::render(&tree, Transform::from_scale(scale, scale), &mut pixmap.as_mut())`, then `pixmap.encode_png()`. An error becomes `Encode`.

## Save flow (the 0019/0021 steps)

| # | Step |
|---|---|
| 1 | `Export PNG` is a ghost small header button with `IconName::Download`. It is disabled without a graph and while `Choosing`/`Saving`. Click → `Choosing`; `name = export_file_name("topology-{context}-{ns}", "png", now)` (0021 sanitizes `@`, `:`, `/`, …); `cx.prompt_for_new_path(&home_dir, Some(&name))` |
| 2 | Confirm → `Saving`. Build `svg` **now** from the current `Rc` graph and layout (main thread, string only) |
| 3 | `background_spawn`: a path ending in `.svg` (ASCII case-insensitive) → `std::fs::write(path, svg)`; else `render_png(&svg)`, then `std::fs::write`. I/O errors become `Write` |
| 4 | Ok → `Saved { file_name }`. The view keeps `export_scale: Option<f32>` as its own detail text: muted `Saved to {file_name}`, plus ` · exported at {pct}%` when the scale is < 1 |
| 5 | Error → `Failed { message: error.to_string() }`, shown as an error `Alert` under the header: `Could not export the topology: {message}` |
| 6 | Cancel → `Idle`; nothing is written. Paths and file names are never traced |

Export takes the whole graph, regardless of the viewport and the drawer. It keeps the current filters, grouping, and pins: the user exports what they arranged.
