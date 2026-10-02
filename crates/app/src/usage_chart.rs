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
    Point, SharedString, Styled as _, TextAlign, Window, div, point, px, quad, size,
};

use crate::status_tone::{StatusTone, tone_color};
use crate::usage_format::{Measure, format_offset};

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
/// Decimal steps whose halves stay whole at every power of ten: 2.5 would put a `12.5m` midline
/// that reads as `13m`.
const NICE_STEPS: [f64; 4] = [1., 2., 5., 10.];
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
}

/// The `Plot`. It holds the memoized model, so a hover repaint copies nothing.
#[derive(IntoPlot)]
pub(crate) struct UsageChart {
    model: Rc<UsageChartModel>,
}

// ---- pure geometry ----

/// The top of a y axis that holds `value`, at least the floor of the unit (10m for CPU, 1Mi for
/// memory), chosen so that the midline is a round number too. CPU takes 1, 2, or 5 times a power
/// of ten. Memory takes a power of two of its own binary unit, so 900Mi gives a 1Gi top and a
/// 512Mi midline.
pub(crate) fn nice_max(value: f64, unit: Measure) -> f64 {
    match unit {
        Measure::Cpu => nice_decimal(value.max(MIN_CPU_MAX)),
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
            power * base
        }
    }
}

fn nice_decimal(value: f64) -> f64 {
    let power = 10f64.powf(value.log10().floor());
    let mantissa = value / power;
    let step = NICE_STEPS
        .into_iter()
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
            _ => theme.chart_2,
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
    format_offset(seconds)
}

// ---- card ----

/// The chart in a bordered card, with its title and newest value above it.
pub(crate) fn usage_chart_card(
    model: Rc<UsageChartModel>,
    height: Pixels,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let newest = model.series.first().and_then(|series| {
        let (_, value) = series.points.last()?;
        Some(match value {
            Some(value) => format!("now {}", model.unit.format(*value)),
            None => "not running".to_owned(),
        })
    });
    let has_oom = !model.markers.is_empty();
    let bad = tone_color(StatusTone::Bad, cx);
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
                .text_color(theme.muted_foreground)
                .child(div().size(px(DOT_SIZE)).rounded_full().bg(bad))
                .child("OOMKilled")
        }))
        .children(newest.map(|text| {
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(text)
        }));
    v_flex()
        .w_full()
        .gap_1()
        .p_2()
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius)
        .child(header)
        .child(div().w_full().h(height).child(UsageChart::new(model)))
}

#[cfg(test)]
#[path = "usage_chart_tests.rs"]
mod usage_chart_tests;
