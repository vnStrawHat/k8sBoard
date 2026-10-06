//! The usage chart of the Monitor tab: a line per series on a time axis, with request and limit
//! lines and OOM markers. The kit `LineChart` cannot do this (it joins across gaps and has no
//! labelled reference lines or markers), so this is a `Plot` of its own on the kit primitives.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::plot::label::Text;
use gpui_kit::component::plot::shape::{Area, Line};
use gpui_kit::component::plot::tooltip::{CrossLine, Dot, Tooltip, TooltipState};
use gpui_kit::component::plot::{Curve, Grid, IntoPlot, Plot, PlotLabel};
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, BorderStyle, Bounds, ElementId, Hsla, IntoElement, ParentElement as _, Pixels,
    Point, SharedString, Styled as _, TextAlign, Window, div, point, px, quad, relative, size,
};

use crate::drawer::truncated_text;
use crate::status_tone::{StatusTone, tone_color};
use crate::usage_format::{Measure, format_offset};

const SECONDS_PER_DAY: u64 = 86_400;
const GUTTER_LEFT: f32 = 52.;
const GUTTER_BOTTOM: f32 = 16.;
const GUTTER_TOP: f32 = 6.;
const LABEL_SIZE: f32 = 10.;
const LINE_WIDTH: f32 = 2.;
const DOT_SIZE: f32 = 8.;
const DOT_RING: f32 = 2.;
const DASH: [f32; 2] = [4., 3.];
/// A line breaks where neighbours are further apart than this many steps (decision 12).
const MAX_GAP_STEPS: f64 = 2.5;
/// Headroom above the highest value, so a line never touches the top.
const Y_HEADROOM: f64 = 1.06;
const MIN_CPU_MAX: f64 = 0.01;
const MIN_BYTES_MAX: f64 = (1u64 << 20) as f64;
/// 1 KB/s: a quiet line is not stretched to the top of the chart.
const MIN_RATE_MAX: f64 = 1_000.;
/// Decimal steps whose halves stay whole at every power of ten: 2.5 would put a `12.5m` midline
/// that reads as `13m`.
const NICE_STEPS: [f64; 4] = [1., 2., 5., 10.];
/// Rates use 1, 2, 4, and 10 instead: the midline of a 5 KB/s top would read `3 KB/s` in whole
/// units, while the halves of these are whole at every power of ten.
const RATE_STEPS: [f64; 4] = [1., 2., 4., 10.];
const THREE_QUARTERS: f64 = 0.75;
/// Below this power of two, three quarters of it halves into a fraction the axis text would round.
const MIN_THREE_QUARTER_POWER: f64 = 8.;
/// A reference label keeps this far left of the right edge, clear of the newest value's dot.
const LABEL_INSET: f32 = 14.;

pub(crate) struct ChartSeries {
    pub(crate) name: SharedString,
    /// Cores or bytes, oldest first.
    pub(crate) points: Vec<(jiff::Timestamp, Option<f64>)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReferenceKind {
    Request,
    Limit,
    Allocatable,
}

pub(crate) struct ReferenceLine {
    pub(crate) kind: ReferenceKind,
    pub(crate) label: SharedString,
    pub(crate) value: f64,
}

pub(crate) struct UsageChartModel {
    /// Unique among sibling charts: the kit keys hover state on it.
    pub(crate) id: SharedString,
    pub(crate) title: SharedString,
    pub(crate) unit: Measure,
    /// The spacing of the points: 15 s or 5 min.
    pub(crate) step: Duration,
    pub(crate) start: jiff::Timestamp,
    pub(crate) end: jiff::Timestamp,
    pub(crate) series: Vec<ChartSeries>,
    pub(crate) references: Vec<ReferenceLine>,
    /// OOM kills.
    pub(crate) markers: Vec<jiff::Timestamp>,
    /// Why the chart has less than it should: `Collecting…`, a failed node, no disk series.
    pub(crate) notice: Option<SharedString>,
}

/// The `Plot`. It holds the memoized model, so a hover repaint copies nothing.
#[derive(IntoPlot)]
pub(crate) struct UsageChart {
    model: Rc<UsageChartModel>,
}

// ---- pure geometry ----

/// The top of a y axis that holds `value`, at least the floor of the unit (10m for CPU, 1Mi for
/// memory), chosen so that the midline is a round number too. CPU takes 1, 2, or 5 times a power
/// of ten. Memory takes a power of two of its own binary unit, or three quarters of one from 6
/// up (6Gi, not 8Gi, for 4.8Gi), so 900Mi gives a 1Gi top and a
/// 512Mi midline. A rate takes 1, 2, 4, or 10 times a power of ten, so its midline is whole in
/// its unit.
pub(crate) fn nice_max(value: f64, unit: Measure) -> f64 {
    match unit {
        Measure::Cpu => nice_decimal(value.max(MIN_CPU_MAX), &NICE_STEPS),
        Measure::Rate => nice_decimal(value.max(MIN_RATE_MAX), &RATE_STEPS),
        Measure::Bytes => {
            let value = value.max(MIN_BYTES_MAX);
            let mut base = 1.;
            while value / base >= 1024. {
                base *= 1024.;
            }
            let mut power = 1.;
            while power * base < value {
                power *= 2.;
            }
            // Three quarters of a power sits between two powers (6Gi between 4Gi and 8Gi), so a
            // 4.8Gi pod does not get an 8Gi chart. Its half is whole from a power of 8 up.
            let three_quarters = power * THREE_QUARTERS;
            if power >= MIN_THREE_QUARTER_POWER && three_quarters * base >= value {
                return three_quarters * base;
            }
            power * base
        }
    }
}

fn nice_decimal(value: f64, steps: &[f64]) -> f64 {
    let power = 10f64.powf(value.log10().floor());
    let mantissa = value / power;
    let step = steps
        .iter()
        .copied()
        .find(|step| mantissa <= *step * (1. + 1e-9))
        .unwrap_or(10.);
    step * power
}

/// The top of the y axis: the highest value or reference line, with headroom, rounded up.
fn y_max(model: &UsageChartModel) -> f64 {
    let values = model
        .series
        .iter()
        .flat_map(|series| series.points.iter().filter_map(|(_, value)| *value));
    let references = model.references.iter().map(|reference| reference.value);
    let highest = values.chain(references).fold(0., f64::max);
    nice_max(highest * Y_HEADROOM, model.unit)
}

/// Where `at` falls on a `width` wide axis from `start` to `end`, clamped to the axis.
fn x_at(at: jiff::Timestamp, start: jiff::Timestamp, end: jiff::Timestamp, width: f32) -> f32 {
    let span = end.as_millisecond() - start.as_millisecond();
    if span <= 0 {
        return width;
    }
    let offset = at.as_millisecond() - start.as_millisecond();
    (offset as f64 / span as f64).clamp(0., 1.) as f32 * width
}

/// The time at `x` on the axis; the inverse of `x_at`.
fn time_at(x: f32, start: jiff::Timestamp, end: jiff::Timestamp, width: f32) -> jiff::Timestamp {
    if width <= 0. {
        return end;
    }
    let span = (end.as_millisecond() - start.as_millisecond()) as f64;
    let fraction = f64::from((x / width).clamp(0., 1.));
    let millis = start.as_millisecond() + (span * fraction) as i64;
    jiff::Timestamp::from_millisecond(millis).unwrap_or(end)
}

/// The runs of consecutive values: a `None`, or neighbours further apart than `max_gap`, ends a
/// run. Points before `start` are dropped.
fn segments(
    points: &[(jiff::Timestamp, Option<f64>)],
    start: jiff::Timestamp,
    max_gap: Duration,
) -> Vec<Vec<(jiff::Timestamp, f64)>> {
    let mut runs: Vec<Vec<(jiff::Timestamp, f64)>> = Vec::new();
    let mut previous: Option<jiff::Timestamp> = None;
    for (at, value) in points.iter().filter(|(at, _)| *at >= start) {
        let Some(value) = value else {
            previous = None;
            runs.push(Vec::new());
            continue;
        };
        let is_gap =
            previous.is_some_and(|before| at.duration_since(before).unsigned_abs() > max_gap);
        if previous.is_none() || is_gap {
            runs.push(Vec::new());
        }
        if let Some(run) = runs.last_mut() {
            run.push((*at, *value));
        }
        previous = Some(*at);
    }
    runs.retain(|run| !run.is_empty());
    runs
}

/// The index of the entry nearest in time to `at`; entries without a value count.
fn nearest_tick(points: &[(jiff::Timestamp, Option<f64>)], at: jiff::Timestamp) -> Option<usize> {
    let after = points.partition_point(|(time, _)| *time < at);
    let distance = |index: usize| {
        points
            .get(index)
            .map(|(time, _)| time.duration_since(at).unsigned_abs())
    };
    match (after.checked_sub(1), distance(after)) {
        (Some(before), Some(next)) => {
            let previous = distance(before)?;
            Some(if previous <= next { before } else { after })
        }
        (Some(before), None) => Some(before),
        (None, Some(_)) => Some(after),
        (None, None) => None,
    }
}

/// The top of a reference line's label: above the line, or below it when the line is too near the
/// top of the chart for the text to fit.
fn reference_label_y(line_y: f32) -> f32 {
    let above = line_y - LABEL_SIZE - 3.;
    if above >= 0. { above } else { line_y + 3. }
}

/// Where the chart is drawn inside its bounds.
#[derive(Clone, Copy)]
struct Plane {
    width: f32,
    height: f32,
    y_max: f64,
    start: jiff::Timestamp,
    end: jiff::Timestamp,
}

impl Plane {
    fn of(model: &UsageChartModel, bounds: Size) -> Self {
        Self {
            width: (bounds.0 - GUTTER_LEFT).max(0.),
            height: (bounds.1 - GUTTER_TOP - GUTTER_BOTTOM).max(0.),
            y_max: y_max(model),
            start: model.start,
            end: model.end,
        }
    }

    /// Relative to the chart's own origin.
    fn x(&self, at: jiff::Timestamp) -> f32 {
        GUTTER_LEFT + x_at(at, self.start, self.end, self.width)
    }

    fn y(&self, value: f64) -> f32 {
        let fraction = (value / self.y_max).clamp(0., 1.) as f32;
        GUTTER_TOP + self.height * (1. - fraction)
    }

    fn baseline(&self) -> f32 {
        GUTTER_TOP + self.height
    }
}

/// Width and height in pixels.
type Size = (f32, f32);

fn bounds_size(bounds: Bounds<Pixels>) -> Size {
    (bounds.size.width.as_f32(), bounds.size.height.as_f32())
}

// ---- paint ----

impl UsageChart {
    fn new(model: Rc<UsageChartModel>) -> Self {
        Self { model }
    }

    fn series_color(index: usize, cx: &App) -> Hsla {
        let theme = cx.theme();
        match index {
            0 => theme.chart_1,
            // Every chart token is a shade of blue, so a second series takes the green one: two
            // blues of any lightness read as one on one of the themes.
            _ => theme.chart_bullish,
        }
    }

    fn paint_dot(center: Point<Pixels>, fill: Hsla, ring: Hsla, window: &mut Window) {
        let diameter = px(DOT_SIZE);
        window.paint_quad(quad(
            Bounds::centered_at(center, size(diameter, diameter)),
            diameter / 2.,
            fill,
            px(DOT_RING),
            ring,
            BorderStyle::default(),
        ));
    }
}

impl Plot for UsageChart {
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let model = Rc::clone(&self.model);
        let plane = Plane::of(&model, bounds_size(bounds));
        let (grid, muted, background) = {
            let theme = cx.theme();
            (theme.chart_grid, theme.muted_foreground, theme.background)
        };
        let bad = tone_color(StatusTone::Bad, cx);
        let origin = bounds.origin;
        let plot = Bounds::new(
            origin + point(px(GUTTER_LEFT), px(GUTTER_TOP)),
            size(px(plane.width), px(plane.height)),
        );

        // Grid at 0, half, and the top, with the value of each on the left.
        Grid::new()
            .y([0., plane.height / 2., plane.height])
            .stroke(grid)
            .paint(&plot, window);
        let mut labels = Vec::new();
        for fraction in [0., 0.5, 1.] {
            let value = plane.y_max * fraction;
            let y = plane.y(value) - LABEL_SIZE / 2. - 1.;
            labels.push(
                Text::new(
                    model.unit.format(value),
                    point(px(GUTTER_LEFT - 6.), px(y)),
                    muted,
                )
                .font_size(px(LABEL_SIZE))
                .align(TextAlign::Right),
            );
        }
        let x_labels_y = plane.baseline() + 3.;
        labels.push(
            Text::new(
                range_label(&model),
                point(px(GUTTER_LEFT), px(x_labels_y)),
                muted,
            )
            .font_size(px(LABEL_SIZE)),
        );
        labels.push(
            Text::new(
                "now",
                point(px(GUTTER_LEFT + plane.width), px(x_labels_y)),
                muted,
            )
            .font_size(px(LABEL_SIZE))
            .align(TextAlign::Right),
        );
        PlotLabel::new(labels).paint(&bounds, window, cx);

        // The area (one series only), then the lines.
        let max_gap = model.step.mul_f64(MAX_GAP_STEPS);
        for (index, series) in model.series.iter().enumerate() {
            let color = Self::series_color(index, cx);
            for run in segments(&series.points, model.start, max_gap) {
                if model.series.len() == 1 && run.len() > 1 {
                    Area::new()
                        .data(run.clone())
                        .x(move |(at, _)| Some(plane.x(*at)))
                        .y0(plane.baseline())
                        .y1(move |(_, value)| Some(plane.y(*value)))
                        .fill(color.opacity(0.1))
                        .curve(Curve::Linear)
                        .paint(&bounds, window);
                }
                let mut line = Line::new()
                    .data(run.iter().copied())
                    .x(move |(at, _)| Some(plane.x(*at)))
                    .y(move |(_, value)| Some(plane.y(*value)))
                    .stroke(color)
                    .stroke_width(px(LINE_WIDTH))
                    .curve(Curve::Linear);
                if run.len() == 1 {
                    // A lone sample has no line to draw, so it shows as a dot.
                    line = line.dot().dot_size(px(DOT_SIZE / 2.)).dot_fill(color);
                }
                line.paint(&bounds, window);
            }
        }

        // Request, allocatable, and limit lines, labelled just above.
        for reference in &model.references {
            let color = match reference.kind {
                ReferenceKind::Limit => bad,
                ReferenceKind::Request | ReferenceKind::Allocatable => muted,
            };
            let y = plane.y(reference.value);
            let label_y = reference_label_y(y);
            Grid::new()
                .y([y - GUTTER_TOP])
                .stroke(color)
                .dash_array(&[px(DASH[0]), px(DASH[1])])
                .paint(&plot, window);
            PlotLabel::new(vec![
                Text::new(
                    format!("{} {}", reference.label, model.unit.format(reference.value)),
                    point(px(GUTTER_LEFT + plane.width - LABEL_INSET), px(label_y)),
                    color,
                )
                .font_size(px(LABEL_SIZE))
                .align(TextAlign::Right),
            ])
            .paint(&bounds, window, cx);
        }

        // OOM kills sit on the baseline.
        for marker in &model.markers {
            if *marker < model.start || *marker > model.end {
                continue;
            }
            let center = origin + point(px(plane.x(*marker)), px(plane.baseline()));
            Self::paint_dot(center, bad, background, window);
        }

        // The newest value of each series.
        for (index, series) in model.series.iter().enumerate() {
            let newest = series
                .points
                .iter()
                .rev()
                .find_map(|(at, value)| value.map(|value| (*at, value)));
            if let Some((at, value)) = newest.filter(|(at, _)| *at >= model.start) {
                let center = origin + point(px(plane.x(at)), px(plane.y(value)));
                Self::paint_dot(center, Self::series_color(index, cx), background, window);
            }
        }
    }

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::from(self.model.id.clone()))
    }

    fn tooltip_state(
        &self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        _cx: &App,
    ) -> Option<TooltipState> {
        let model = &self.model;
        let plane = Plane::of(model, bounds_size(bounds));
        let (x, y) = (position.x.as_f32(), position.y.as_f32());
        if x < GUTTER_LEFT || y > plane.baseline() {
            return None;
        }
        let at = time_at(x - GUTTER_LEFT, model.start, model.end, plane.width);
        let first = model.series.first()?;
        let index = nearest_tick(&first.points, at)?;
        let tick = first.points.get(index)?.0;
        let dots = model
            .series
            .iter()
            .filter_map(|series| series.points.get(index)?.1)
            .map(|value| point(px(plane.x(tick)), px(plane.y(value))))
            .collect();
        Some(TooltipState::new(
            index,
            point(px(plane.x(tick)), position.y),
            dots,
        ))
    }

    fn tooltip(
        &self,
        state: &TooltipState,
        cursor: Point<Pixels>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let model = &self.model;
        let plane = Plane::of(model, bounds_size(bounds));
        let tick = model.series.first()?.points.get(state.index)?.0;
        let background = cx.theme().background;
        let bad = tone_color(StatusTone::Bad, cx);
        let ago = model.end.duration_since(tick).as_secs().max(0) as u64;
        let colors: Vec<(usize, Hsla)> = model
            .series
            .iter()
            .enumerate()
            .filter(|(_, series)| {
                series
                    .points
                    .get(state.index)
                    .is_some_and(|(_, value)| value.is_some())
            })
            .map(|(index, _)| (index, Self::series_color(index, cx)))
            .collect();
        let dots = state
            .dots
            .iter()
            .zip(&colors)
            .map(|(dot, (_, color))| {
                Dot::new(*dot)
                    .size(DOT_SIZE)
                    .halo(DOT_SIZE + 4.)
                    .stroke(background)
                    .fill(*color)
            })
            .collect::<Vec<_>>();
        let mut tooltip = Tooltip::new(cursor, bounds.size)
            .gap(px(8.))
            .cross_line(CrossLine::new(state.cross_line).span(GUTTER_TOP, plane.height))
            .dots(dots)
            .title(format_offset(ago));
        for (index, series) in model.series.iter().enumerate() {
            let value = series.points.get(state.index).and_then(|(_, value)| *value);
            let text = value.map_or_else(
                || "not running".to_owned(),
                |value| model.unit.format(value),
            );
            tooltip = tooltip.row(Self::series_color(index, cx), series.name.clone(), text);
        }
        let half_step = model.step / 2;
        let is_oom = model
            .markers
            .iter()
            .any(|marker| marker.duration_since(tick).unsigned_abs() <= half_step);
        if is_oom {
            tooltip = tooltip.plain_row("", "OOMKilled").value_color(bad);
        }
        Some(tooltip.into_any_element())
    }
}

/// The label at the left edge of the axis: how far back the chart reaches.
fn range_label(model: &UsageChartModel) -> String {
    let seconds = model.end.duration_since(model.start).as_secs().max(0) as u64;
    // A source range of whole days reads `-7d` and `-30d`, not `-168h`.
    if seconds >= SECONDS_PER_DAY * 7 && seconds.is_multiple_of(SECONDS_PER_DAY) {
        return format!("-{}d", seconds / SECONDS_PER_DAY);
    }
    format_offset(seconds)
}

/// Whether any series has a value inside the chart's window.
fn has_points_in_range(model: &UsageChartModel) -> bool {
    model.series.iter().any(|series| {
        series
            .points
            .iter()
            .any(|(at, value)| *at >= model.start && value.is_some())
    })
}

/// How much of the chart's range the data may fill before the chart stops saying it is still
/// collecting: a line a quarter of the way across is a stub, not a trend.
const COLLECTING_SHARE: f64 = 0.25;
/// The share of the plot width the collecting text may take, clear of the data on the right.
const COLLECTING_OVERLAY_WIDTH: f32 = 0.5;
/// Charts of this step or finer are fed by the app's own polling, which keeps 24 hours. The coarser
/// ones come from a metrics source with its own retention, so they never say "collecting".
const POLLED_STEP_LIMIT: Duration = Duration::from_secs(60);

/// `Collecting · 2 min of data (kept 24 h)` while the polled history fills less than a quarter of
/// the range, so the near-empty plot says why; `None` once it fills more, or for a source chart.
fn collecting_text(model: &UsageChartModel) -> Option<String> {
    if model.step > POLLED_STEP_LIMIT {
        return None;
    }
    let times = || {
        model.series.iter().flat_map(|series| {
            series
                .points
                .iter()
                .filter(|(at, value)| *at >= model.start && value.is_some())
                .map(|(at, _)| *at)
        })
    };
    let span = times().max()?.duration_since(times().min()?).as_secs_f64();
    let range = model.end.duration_since(model.start).as_secs_f64();
    if span >= range * COLLECTING_SHARE {
        return None;
    }
    Some(format!(
        "Collecting · {} of data (kept 24 h)",
        span_text(span as u64)
    ))
}

/// `under 1 min`, `2 min`, `1 h 20 min`.
fn span_text(seconds: u64) -> String {
    let minutes = seconds / 60;
    match (minutes / 60, minutes % 60) {
        (0, 0) => "under 1 min".to_owned(),
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} h"),
        (hours, minutes) => format!("{hours} h {minutes} min"),
    }
}

// ---- card ----

/// The chart in a bordered card, with its title above it and, on the right, its legend (two
/// series) or its newest value (one). A notice reads under the header while there are points,
/// and fills the empty plot when there are none.
pub(crate) fn usage_chart_card(
    model: Rc<UsageChartModel>,
    height: Pixels,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let has_oom = !model.markers.is_empty();
    let bad = tone_color(StatusTone::Bad, cx);
    let right = if model.series.len() > 1 {
        let swatches = model.series.iter().enumerate().map(|(index, series)| {
            h_flex()
                .gap_1()
                .items_center()
                .child(
                    div()
                        .size(px(DOT_SIZE))
                        .rounded_full()
                        .bg(UsageChart::series_color(index, cx)),
                )
                .child(series.name.clone())
        });
        Some(
            h_flex()
                .gap_2()
                .text_xs()
                .text_color(muted)
                .children(swatches)
                .into_any_element(),
        )
    } else {
        model
            .series
            .first()
            .and_then(|series| {
                let (_, value) = series.points.last()?;
                Some(match value {
                    Some(value) => format!("now {}", model.unit.format(*value)),
                    None => "not running".to_owned(),
                })
            })
            .map(|text| {
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(text)
                    .into_any_element()
            })
    };
    let header = h_flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .flex_1()
                .text_sm()
                .font_semibold()
                .child(model.title.clone()),
        )
        .children(has_oom.then(|| {
            h_flex()
                .gap_1()
                .items_center()
                .text_xs()
                .text_color(muted)
                .child(div().size(px(DOT_SIZE)).rounded_full().bg(bad))
                .child("OOMKilled")
        }))
        .children(right);
    let has_points = has_points_in_range(&model);
    let notice_line = model.notice.clone().filter(|_| has_points).map(|text| {
        let id = SharedString::from(format!("{}-notice", model.id));
        truncated_text(id, text).text_xs().text_color(muted)
    });
    let notice_overlay = model.notice.clone().filter(|_| !has_points).map(|text| {
        div()
            .absolute()
            // Clear of the axis labels on the left.
            .top_0()
            .bottom_0()
            .right_0()
            .left(px(GUTTER_LEFT))
            .flex()
            .items_center()
            .justify_center()
            .px_4()
            .child(
                div()
                    .max_w_full()
                    .text_xs()
                    .text_color(muted)
                    .text_center()
                    .child(text),
            )
    });
    // The data sits against the right edge, so the text takes the empty left part of the plot.
    let collecting_overlay = collecting_text(&model).filter(|_| has_points).map(|text| {
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(GUTTER_LEFT))
            .w(relative(COLLECTING_OVERLAY_WIDTH))
            .flex()
            .items_center()
            .justify_center()
            .px_2()
            .text_xs()
            .text_color(muted)
            .text_center()
            .child(div().max_w_full().child(text))
    });
    v_flex()
        .w_full()
        .gap_1()
        .p_2()
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius)
        .child(header)
        .children(notice_line)
        .child(
            div()
                .relative()
                .w_full()
                .h(height)
                .child(UsageChart::new(model))
                .children(notice_overlay)
                .children(collecting_overlay),
        )
}

#[cfg(test)]
#[path = "usage_chart_tests.rs"]
mod usage_chart_tests;
