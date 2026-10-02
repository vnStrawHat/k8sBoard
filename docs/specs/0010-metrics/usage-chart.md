# 0010 · App: usage chart

[Back to index](README.md) · Step 3 · Module: `usage_chart.rs` (new) + `usage_chart_tests.rs`. Prototype it first in the step (roadmap R3).

## Why not the kit `LineChart`

| Need (W4c) | `LineChart` 0.7 | `UsageChart` |
|---|---|---|
| Gaps for "not running" and missed polls | joins across `None` (`shape::Line` drops `None` points and strokes one path through the rest; checked in gpui-base 0.7 `line.rs`) | one `Line` per segment |
| Request (dim) and limit (red) lines with labels | `reference_line` is one muted color, no label | own dashed lines and labels |
| OOMKilled dots under the axis | none | marker dots |
| Time-based x (`-15m` … `now`) | point index | timestamp mapped linearly |
| Hover crosshair and tooltip | yes | yes, through the same kit `Plot` trait |

`UsageChart` implements `gpui_kit::component::plot::Plot` and paints with kit primitives (`shape::Line`, `shape::Area`, `Grid`, `PlotLabel`/`label::Text`, `tooltip::{Tooltip, CrossLine, Dot}`). `#[derive(IntoPlot)]` (`gpui_kit::component::plot::IntoPlot`) supplies `IntoElement`.

## Model

```rust
// The unit is `usage_format::Measure` (Cpu, Bytes; 0011 adds a rate).
pub(crate) struct ChartSeries { pub(crate) name: SharedString,
    pub(crate) points: Vec<(jiff::Timestamp, Option<f64>)> }   // cores or bytes, oldest first
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReferenceKind { Request, Limit, Allocatable }
pub(crate) struct ReferenceLine { pub(crate) kind: ReferenceKind, pub(crate) label: SharedString, pub(crate) value: f64 }
pub(crate) struct UsageChartModel {
    pub(crate) id: SharedString,                 // "monitor-cpu"; unique among sibling charts
    pub(crate) title: SharedString,              // "CPU", "Memory"
    pub(crate) unit: Measure,
    pub(crate) step: Duration,                   // 15 s or 5 min, from `UsageSeries.step`
    pub(crate) start: jiff::Timestamp, pub(crate) end: jiff::Timestamp,
    pub(crate) series: Vec<ChartSeries>,         // 1 in 0010
    pub(crate) references: Vec<ReferenceLine>,
    pub(crate) markers: Vec<jiff::Timestamp>,    // OOMKilled
}
/// The `Plot`; holds the memoized model (monitor-tab.md), so a hover repaint copies nothing.
#[derive(IntoPlot)] pub(crate) struct UsageChart { model: Rc<UsageChartModel> }
pub(crate) fn usage_chart_card(model: Rc<UsageChartModel>, height: Pixels, cx: &App) -> impl IntoElement;
```

## Pure geometry (unit-tested)

```rust
pub(crate) fn nice_max(value: f64, unit: Measure) -> f64;    // CPU 1, 2, 5 × 10^k (halves stay whole); bytes a power of two of the value's binary unit (900Mi → 1Gi); floor 10m CPU, 1 Mi bytes
fn y_max(model: &UsageChartModel) -> f64;                     // nice_max(max(values, references) × 1.06)
fn x_at(at: jiff::Timestamp, start: jiff::Timestamp, end: jiff::Timestamp, width: f32) -> f32;
fn segments(points: &[(jiff::Timestamp, Option<f64>)], start: jiff::Timestamp, max_gap: Duration)
    -> Vec<Vec<(jiff::Timestamp, f64)>>;                      // split at None and at gaps > max_gap
fn nearest_tick(points: &[(jiff::Timestamp, Option<f64>)], at: jiff::Timestamp) -> Option<usize>;
```

`max_gap` = 2.5 × `model.step` (decision 12). Points before `start` are dropped.

## Paint (inside `bounds`)

| Layer | Rule | Token |
|---|---|---|
| Gutter | left 52 px for y labels, bottom 16 px for x labels, top 6 px | — |
| Grid | solid 1 px lines at 0, ½, and 1 × `y_max`; labels `unit.format`, right-aligned in the gutter, `text_xs` | `chart_grid`, labels `muted_foreground` |
| X labels | `-15m`, `-1h`, `-6h`, or `-24h` at the left edge, `now` right-aligned at the right edge | `muted_foreground` |
| Area | one series only: per segment, from the baseline | `chart_1.opacity(0.1)` |
| Line | per segment, linear, 2 px | series `i` → `chart_1`, `chart_2` |
| Last dot | at the newest non-`None` point of each series, 8 px, 2 px ring | series color, ring `background` |
| References | dashed (4, 3) 1 px full-width line; label `{label} {value}` right-aligned just above it (below it near the top), 14 px left of the right edge so the newest value's dot never covers it; a request equal to the limit within 0.5% is one Limit line labelled `request = limit` | Request, Allocatable → `muted_foreground`; Limit → `tone_color(Bad)` |
| Markers | 8 px dot centered on the baseline at `x_at(marker)`, 2 px ring | `tone_color(Bad)`, ring `background` |

Order: grid, area, lines, references, markers, last dots. A reference above `y_max` cannot happen (`y_max` includes it).

## Hover

- `id()` → `Some(model.id)`; ids are `monitor-cpu` and `monitor-memory` because both charts are built at one call site (kit rule).
- `tooltip_state`: below the plot or in the gutter → `None`; else `nearest_tick` of the first series at the cursor's time → `TooltipState::new(index, point(x, cursor.y), dots)` with one dot per series that has a value there.
- `tooltip`: kit `Tooltip::new(cursor, bounds.size).gap(px(8.))`, a `CrossLine` limited to the plot height, `Dot`s in the series colors; title `format_offset(end − tick)`; one row per series: name and the formatted value, or `not running`; plus a row `OOMKilled` toned Bad when a marker is within half a `step` of the point.

## Card (`usage_chart_card`)

- Border `theme.border`, `theme.radius`, `p_2`, `gap_1`: a header row, then the chart at `height`, full width.
- Header: title (`text_sm`, semibold); right side, muted `text_xs`: one series → `now {value}` or `not running` (newest point `None`); two or more → legend swatches (0011); markers present → a Bad dot + `OOMKilled` legend item.
- An empty window (no point in range) still draws grid, axes, references, and markers.
