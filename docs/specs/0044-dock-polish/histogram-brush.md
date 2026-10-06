# 0044 · Step 3: histogram brush

[Back to index](README.md) · Modules: `log_buffer.rs` (+ tests), `log_volume.rs` (+ tests), `log_tab.rs`. Decisions 12–15. Wireframe: W8b density chart with its window (`.histo .hwin`), note 4. Supersedes the ceiling of 0019 decision 21.

## Behavior

- Full layout only, where the histogram is drawn (zoomed dock, pop-out window).
- Press on the chart, drag, release (decision 12: W8b draws the window but not its effect, so filtering is an interpretation): the buckets under the drag become the window `[first bucket start, last bucket end)`. Only lines whose timestamp falls in it show; lines without a timestamp hide while a window is set (decision 13).
- The bars keep every bucket: the histogram ignores the window. The window is shaded over its buckets with `theme.selection` (the log match highlight token).
- Caption, after `Lines per 5s` and its key (a red swatch, "has errors", and a tooltip saying each bar is one bucket and a red bar holds an error line): a chip `10:47:58 – 10:48:06` (local zone, the `bucket_label` format of the bucket width) and ✕ (ghost xsmall `IconName::X`, tooltip `Show all lines`).
- Cleared by ✕, by a release without movement, and by `restart_stream` (Reconnect, container switch, Previous).
- New lines keep arriving; those outside the window stay hidden and the view does not jump. As the span grows the bucket width can change; the window stays in time, so its shade moves.

## Model

```rust
/// `start` inclusive, `end` exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TimeWindow { pub(crate) start: jiff::Timestamp, pub(crate) end: jiff::Timestamp }
pub(crate) struct LineView { /* … */ pub(crate) window: Option<TimeWindow> }        // log_buffer.rs
impl LogBuffer {
    /// What the histogram counts: lines the view shows without its window; markers excluded.
    pub(crate) fn volume_lines(&self) -> impl Iterator<Item = &BufferedLine>;
}
/// The window a drag covers; `from` and `to` are fractions of the chart width, in any order.
pub(crate) fn brush_window(volume: &Volume, from: f32, to: f32) -> Option<TimeWindow>;  // log_volume.rs
/// Where `window` sits on the chart, as fractions; `None` when it misses every bucket.
pub(crate) fn window_span(volume: &Volume, window: TimeWindow) -> Option<(f32, f32)>;
```

- `LineView::shows` adds the window test after the marker rule; `is_filtering` counts a window.
- `volume_lines`: without a window, `visible_lines()` minus markers; with one, every line through the view minus its window. `// ponytail: rescans up to 10,000 lines per buffer revision while a window is set (Full layout only); keep a second index if traces show the cost.`
- `current_volume` reads `volume_lines` instead of `visible_lines`.
- Mapping: equal-width buckets over the chart width, `index = floor(fraction × buckets)`, clamped to the range. `// ponytail: ignores the kit bar padding; an edge can be off by one bucket of 60.`

## Interaction (`log_volume.rs`, `log_tab.rs`)

- `volume_chart` also takes the window, the live drag, and the handlers from the tab. The chart cell gets `.id("log-volume-brush")` and `on_mouse_down(Left)` (starts `brush`), plus a `canvas` child that stores its bounds at prepaint in an `Rc<Cell<Option<Bounds<Pixels>>>>` held by `LogTab` (the kit `BarChart` has no hit test). While `brush` is `Some`, the canvas paint registers window-level listeners (`window.on_mouse_event`, as the kit resizable group does) for `MouseMoveEvent` (updates `current`) and `MouseUpEvent` (ends the drag), so a pointer that leaves the chart keeps dragging and a release anywhere ends it.
- Fractions are `((x - bounds.left()) / bounds.width()).clamp(0., 1.)`; a zero-width chart starts no drag.
- `LogTab` keeps `brush: Option<BrushDrag { anchor: f32, current: f32 }>` while dragging; the drag is shaded live with the same token.
- Release: `brush_window` → `LineView.window` → `refresh_view` (the scroller resets to the visible count, as a filter change does). A release without movement clears the window.
- The kit tooltips on the bars stay.
- The chip and ✕ live in the caption row of `volume_chart`; ✕ → `window = None` → `refresh_view`.
