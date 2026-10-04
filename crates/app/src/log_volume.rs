//! The log volume histogram of a zoomed tab: lines per time bucket, errors marked, and the brush
//! that picks a time window over it.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::chart::BarChart;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex};
use gpui_kit::{
    AnyElement, App, Bounds, DispatchPhase, InteractiveElement as _, IntoElement as _, MouseButton,
    MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, SharedString, Styled as _, canvas,
    div, px, relative,
};

use crate::log_buffer::TimeWindow;
use crate::log_level::LogLevel;
use crate::status_tone::{StatusTone, tone_color};

const MAX_BUCKETS: usize = 60;
const BUCKET_WIDTHS_SECS: [u64; 13] = [
    1, 5, 15, 30, 60, 300, 900, 1800, 3600, 10_800, 21_600, 43_200, 86_400,
];
const CHART_HEIGHT: f32 = 48.;
/// Pixels; a bucket with one or two lines would otherwise vanish next to a busy one.
const MIN_BAR_HEIGHT: f32 = 2.;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VolumeBucket {
    pub(crate) start: jiff::Timestamp,
    pub(crate) lines: u32,
    pub(crate) errors: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Volume {
    pub(crate) width: Duration,
    pub(crate) buckets: Vec<VolumeBucket>,
}

/// `None` without two distinct timestamps. Input: visible lines with a timestamp.
pub(crate) fn volume(
    lines: impl Iterator<Item = (jiff::Timestamp, Option<LogLevel>)>,
) -> Option<Volume> {
    // Lines arrive nearly sorted, so the span comes first and the counting second.
    let lines: Vec<_> = lines.collect();
    let first = lines.iter().map(|(time, _)| *time).min()?;
    let last = lines.iter().map(|(time, _)| *time).max()?;
    if first == last {
        return None;
    }
    let span = Duration::from_secs(u64::try_from(last.as_second() - first.as_second()).ok()?);
    let width = bucket_width(span);
    let width_secs = i64::try_from(width.as_secs()).ok()?;
    let align = |time: jiff::Timestamp| time.as_second().div_euclid(width_secs) * width_secs;
    let last_start = align(last);
    // A span beyond 60 days keeps the newest 60 days.
    let first_start = align(first).max(last_start - (MAX_BUCKETS as i64 - 1) * width_secs);
    let count = usize::try_from((last_start - first_start) / width_secs).ok()? + 1;
    let mut buckets = Vec::with_capacity(count);
    for index in 0..count {
        let start = first_start + i64::try_from(index).ok()? * width_secs;
        buckets.push(VolumeBucket {
            start: jiff::Timestamp::from_second(start).ok()?,
            lines: 0,
            errors: 0,
        });
    }
    for (time, level) in &lines {
        // A line older than the kept buckets has a negative offset and is skipped.
        let Ok(index) = usize::try_from((align(*time) - first_start) / width_secs) else {
            continue;
        };
        if let Some(bucket) = buckets.get_mut(index) {
            bucket.lines += 1;
            bucket.errors += u32::from(*level == Some(LogLevel::Error));
        }
    }
    Some(Volume { width, buckets })
}

/// The smallest width with `span / width < MAX_BUCKETS`; else one day.
fn bucket_width(span: Duration) -> Duration {
    let width = BUCKET_WIDTHS_SECS
        .into_iter()
        .find(|width| span.as_secs_f64() / (*width as f64) < MAX_BUCKETS as f64)
        .unwrap_or(86_400);
    Duration::from_secs(width)
}

/// `HH:MM:SS` under a minute, `HH:MM` under an hour, `MM-DD HH:MM` under a day, `MM-DD`
/// otherwise (UTC).
fn bucket_label(start: jiff::Timestamp, width: Duration) -> String {
    let format = match width.as_secs() {
        0..60 => "%H:%M:%S",
        60..3600 => "%H:%M",
        3600..86_400 => "%m-%d %H:%M",
        _ => "%m-%d",
    };
    start.strftime(format).to_string()
}

/// `1s`, `15s`, `5m`, `3h`, `1d`.
fn width_label(width: Duration) -> String {
    let secs = width.as_secs();
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86_400 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

/// The bucket under a fraction of the chart width: equal-width buckets, clamped to the range.
fn bucket_at(volume: &Volume, fraction: f32) -> usize {
    // ponytail: ignores the kit bar padding; an edge can be off by one bucket of 60.
    let count = volume.buckets.len();
    let index = (fraction.clamp(0., 1.) * count as f32) as usize;
    index.min(count.saturating_sub(1))
}

/// The end of a bucket that starts at `start`.
fn bucket_end(start: jiff::Timestamp, width: Duration) -> Option<jiff::Timestamp> {
    start.checked_add(width).ok()
}

/// The window a drag covers; `from` and `to` are fractions of the chart width, in any order.
pub(crate) fn brush_window(volume: &Volume, from: f32, to: f32) -> Option<TimeWindow> {
    let (a, b) = (bucket_at(volume, from), bucket_at(volume, to));
    let first = volume.buckets.get(a.min(b))?;
    let last = volume.buckets.get(a.max(b))?;
    Some(TimeWindow {
        start: first.start,
        end: bucket_end(last.start, volume.width)?,
    })
}

/// Where `window` sits on the chart, as fractions of its width; `None` when it misses every
/// bucket.
pub(crate) fn window_span(volume: &Volume, window: TimeWindow) -> Option<(f32, f32)> {
    let first = volume.buckets.iter().position(|bucket| {
        bucket_end(bucket.start, volume.width).is_some_and(|end| end > window.start)
    })?;
    let last = volume
        .buckets
        .iter()
        .rposition(|bucket| bucket.start < window.end)?;
    if last < first {
        return None;
    }
    let count = volume.buckets.len() as f32;
    Some((first as f32 / count, (last + 1) as f32 / count))
}

/// The pointer's place on the chart as a fraction of its width, clamped to the chart so a
/// pointer outside it still reads as the nearest edge; `None` for a chart without width.
pub(crate) fn brush_fraction(x: Pixels, bounds: Bounds<Pixels>) -> Option<f32> {
    let width = bounds.size.width;
    if width <= px(0.) {
        return None;
    }
    Some(((x - bounds.left()) / width).clamp(0., 1.))
}

/// A drag in progress across the chart: where it started and where the pointer is now, as
/// fractions of the chart width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BrushDrag {
    pub(crate) anchor: f32,
    pub(crate) current: f32,
}

/// A pointer handler: it gets a fraction of the chart width.
pub(crate) type FractionHandler = Rc<dyn Fn(f32, &mut App)>;

/// What the tab does with the pointer.
#[derive(Clone)]
pub(crate) struct BrushHandlers {
    pub(crate) press: FractionHandler,
    pub(crate) moved: FractionHandler,
    pub(crate) release: FractionHandler,
    pub(crate) clear: Rc<dyn Fn(&mut App)>,
}

/// The brush as the tab holds it: the committed window, the live drag, and the handlers.
pub(crate) struct BrushView {
    pub(crate) window: Option<TimeWindow>,
    pub(crate) drag: Option<BrushDrag>,
    /// The chart's bounds from the last prepaint; the kit `BarChart` has no hit test.
    pub(crate) bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    pub(crate) handlers: BrushHandlers,
}

/// The caption, the bars with the brush over them. A bar with an error line is drawn in the
/// `Bad` tone.
pub(crate) fn volume_chart(volume: &Rc<Volume>, brush: BrushView, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (normal, bad) = (theme.chart_1, tone_color(StatusTone::Bad, cx));
    let width = volume.width;
    let chart = BarChart::new(volume.buckets.iter().cloned().enumerate())
        .id("log-volume")
        // The band must be unique; the axis that would show it is hidden.
        .band(|(index, _)| index.to_string())
        .value(|(_, bucket)| f64::from(bucket.lines))
        // A bucket without lines stays invisible; one with a line or two gets the minimum height.
        .min_length(MIN_BAR_HEIGHT)
        .fill(
            move |(_, bucket), _, _, _| match (bucket.lines, bucket.errors) {
                (0, _) => normal.opacity(0.),
                (_, 0) => normal,
                _ => bad,
            },
        )
        .label_axis(false)
        .value_axis(false)
        .grid(false)
        .tooltip_title(move |(_, bucket)| SharedString::from(bucket_label(bucket.start, width)))
        .tooltip_value(|(_, bucket), lines| {
            SharedString::from(format!("{lines} lines · {} errors", bucket.errors))
        });
    // The live drag is shaded as it moves; the committed window sits where its time falls.
    let shade = match (brush.drag, brush.window) {
        (Some(drag), _) => Some((drag.anchor.min(drag.current), drag.anchor.max(drag.current))),
        (None, Some(window)) => window_span(volume, window),
        (None, None) => None,
    };
    let BrushView {
        window,
        drag,
        bounds,
        handlers,
    } = brush;
    let cell = div()
        .id("log-volume-brush")
        .debug_selector(|| "log-volume-brush".into())
        .relative()
        .flex_1()
        .h(px(CHART_HEIGHT))
        .py_1()
        .cursor_crosshair()
        .on_mouse_down(MouseButton::Left, {
            let (bounds, press) = (Rc::clone(&bounds), Rc::clone(&handlers.press));
            move |event, _, cx| {
                let fraction = bounds
                    .get()
                    .and_then(|bounds| brush_fraction(event.position.x, bounds));
                if let Some(fraction) = fraction {
                    press(fraction, cx);
                }
            }
        })
        .child(chart)
        .children(shade.map(|(from, to)| {
            div()
                .absolute()
                .top_0()
                .h_full()
                .left(relative(from))
                .w(relative(to - from))
                .bg(theme.selection)
        }))
        .child(bounds_canvas(drag.is_some(), bounds, handlers.clone()));
    h_flex()
        .flex_shrink_0()
        .items_center()
        .gap_3()
        .px_3()
        .py_1()
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(format!("Lines per {}", width_label(width))),
        )
        .children(window.map(|window| window_chip(window, width, handlers.clear, cx)))
        .child(cell)
        .into_any_element()
}

/// `HH:MM:SS – HH:MM:SS` and the ✕ that shows every line again.
fn window_chip(
    window: TimeWindow,
    width: Duration,
    clear: Rc<dyn Fn(&mut App)>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let label = format!(
        "{} – {}",
        bucket_label(window.start, width),
        bucket_label(window.end, width)
    );
    h_flex()
        .flex_shrink_0()
        .items_center()
        .gap_0p5()
        .child(
            div()
                .px_1p5()
                .rounded_sm()
                .bg(theme.muted)
                .font_family(theme.mono_font_family.clone())
                .text_xs()
                .child(label),
        )
        .child(
            Button::new("log-volume-clear")
                .ghost()
                .xsmall()
                .icon(Icon::new(IconName::X))
                .tooltip("Show all lines")
                .on_click(move |_, _, cx| clear(cx)),
        )
        .into_any_element()
}

/// Fills the chart's box: stores its bounds at prepaint, and, while a drag runs, listens on the
/// window so a pointer that left the chart keeps dragging and a release anywhere ends the drag.
fn bounds_canvas(
    is_dragging: bool,
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    handlers: BrushHandlers,
) -> AnyElement {
    let stored = Rc::clone(&bounds);
    canvas(
        move |chart_bounds, _, _| stored.set(Some(chart_bounds)),
        move |_, (), window, _| {
            if !is_dragging {
                return;
            }
            window.on_mouse_event({
                let (bounds, moved) = (Rc::clone(&bounds), Rc::clone(&handlers.moved));
                move |event: &MouseMoveEvent, phase, _, cx| {
                    let fraction = bounds
                        .get()
                        .and_then(|bounds| brush_fraction(event.position.x, bounds));
                    if let (DispatchPhase::Bubble, Some(fraction)) = (phase, fraction) {
                        moved(fraction, cx);
                    }
                }
            });
            window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                let fraction = bounds
                    .get()
                    .and_then(|bounds| brush_fraction(event.position.x, bounds));
                if let (DispatchPhase::Bubble, Some(fraction)) = (phase, fraction) {
                    (handlers.release)(fraction, cx);
                }
            });
        },
    )
    .absolute()
    .size_full()
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui_kit::{point, size};

    use super::*;

    fn at(text: &str) -> jiff::Timestamp {
        text.parse().expect("valid time")
    }

    fn secs(count: u64) -> Duration {
        Duration::from_secs(count)
    }

    fn info(time: &str) -> (jiff::Timestamp, Option<LogLevel>) {
        (at(time), None)
    }

    #[test]
    fn bucket_width_picks_smallest_fitting_width() {
        assert_eq!(bucket_width(secs(50)), secs(1));
        assert_eq!(bucket_width(secs(10 * 60)), secs(15));
        assert_eq!(bucket_width(secs(3 * 3600)), secs(300));
        assert_eq!(bucket_width(secs(90 * 86_400)), secs(86_400));
    }

    #[test]
    fn volume_counts_lines_and_errors_per_aligned_bucket() {
        let lines = [
            info("2024-05-01T10:00:00.4Z"),
            (at("2024-05-01T10:00:00.9Z"), Some(LogLevel::Error)),
            (at("2024-05-01T10:00:01Z"), Some(LogLevel::Warn)),
            info("2024-05-01T10:00:03Z"),
        ];
        let volume = volume(lines.into_iter()).expect("a volume");
        assert_eq!(volume.width, secs(1));
        let counts: Vec<_> = volume
            .buckets
            .iter()
            .map(|bucket| (bucket.start, bucket.lines, bucket.errors))
            .collect();
        assert_eq!(
            counts,
            [
                (at("2024-05-01T10:00:00Z"), 2, 1),
                (at("2024-05-01T10:00:01Z"), 1, 0),
                (at("2024-05-01T10:00:02Z"), 0, 0),
                (at("2024-05-01T10:00:03Z"), 1, 0),
            ]
        );
    }

    #[test]
    fn volume_fills_empty_buckets_between_first_and_last() {
        let lines = [info("2024-05-01T10:00:00Z"), info("2024-05-01T10:00:30Z")];
        let volume = volume(lines.into_iter()).expect("a volume");
        assert_eq!(volume.buckets.len(), 31);
        assert_eq!(
            volume
                .buckets
                .iter()
                .map(|bucket| bucket.lines)
                .sum::<u32>(),
            2
        );
    }

    #[test]
    fn volume_keeps_newest_buckets_beyond_limit() {
        let lines = [info("2024-01-01T00:00:00Z"), info("2024-05-01T00:00:00Z")];
        let volume = volume(lines.into_iter()).expect("a volume");
        assert_eq!(volume.width, secs(86_400));
        assert_eq!(volume.buckets.len(), MAX_BUCKETS);
        let newest = volume.buckets.last().expect("a bucket");
        assert_eq!(newest.start, at("2024-05-01T00:00:00Z"));
        assert_eq!(newest.lines, 1);
        // The oldest line fell out with its bucket.
        assert_eq!(
            volume
                .buckets
                .iter()
                .map(|bucket| bucket.lines)
                .sum::<u32>(),
            1
        );
    }

    #[test]
    fn volume_without_two_timestamps_is_none() {
        assert_eq!(volume(std::iter::empty()), None);
        assert_eq!(volume([info("2024-05-01T10:00:00Z")].into_iter()), None);
        let same = [info("2024-05-01T10:00:00Z"), info("2024-05-01T10:00:00Z")];
        assert_eq!(volume(same.into_iter()), None);
    }

    #[test]
    fn bucket_label_formats_by_width() {
        let start = at("2024-05-01T10:47:58Z");
        assert_eq!(bucket_label(start, secs(5)), "10:47:58");
        assert_eq!(bucket_label(start, secs(300)), "10:47");
        assert_eq!(bucket_label(start, secs(10_800)), "05-01 10:47");
        assert_eq!(bucket_label(start, secs(86_400)), "05-01");
    }

    /// Ten buckets of 5 s from 10:00:00.
    fn ten_buckets() -> Volume {
        let first = at("2024-05-01T10:00:00Z");
        Volume {
            width: secs(5),
            buckets: (0..10)
                .map(|index| VolumeBucket {
                    start: first + Duration::from_secs(5 * index),
                    lines: 1,
                    errors: 0,
                })
                .collect(),
        }
    }

    fn window_of_buckets(first: u64, last: u64) -> TimeWindow {
        let start = at("2024-05-01T10:00:00Z");
        TimeWindow {
            start: start + Duration::from_secs(5 * first),
            end: start + Duration::from_secs(5 * (last + 1)),
        }
    }

    #[test]
    fn brush_window_covers_the_dragged_buckets_in_any_order() {
        let volume = ten_buckets();
        let expected = Some(window_of_buckets(2, 5));
        assert_eq!(brush_window(&volume, 0.25, 0.55), expected);
        assert_eq!(brush_window(&volume, 0.55, 0.25), expected);
    }

    #[test]
    fn brush_window_clamps_to_the_chart() {
        let volume = ten_buckets();
        assert_eq!(
            brush_window(&volume, -0.2, 1.4),
            Some(window_of_buckets(0, 9))
        );
    }

    #[test]
    fn window_span_places_the_shade() {
        let volume = ten_buckets();
        let (from, to) = window_span(&volume, window_of_buckets(2, 5)).expect("on the chart");
        assert!(
            (from - 0.2).abs() < 1e-6 && (to - 0.6).abs() < 1e-6,
            "{from} {to}"
        );
        let outside = TimeWindow {
            start: at("2024-05-01T11:00:00Z"),
            end: at("2024-05-01T11:00:05Z"),
        };
        assert_eq!(window_span(&volume, outside), None);
    }

    #[test]
    fn brush_fraction_clamps_outside_the_chart() {
        let bounds = Bounds::new(point(px(100.), px(0.)), size(px(200.), px(48.)));
        assert_eq!(brush_fraction(px(150.), bounds), Some(0.25));
        assert_eq!(brush_fraction(px(40.), bounds), Some(0.));
        assert_eq!(brush_fraction(px(900.), bounds), Some(1.));
        let empty = Bounds::new(point(px(100.), px(0.)), size(px(0.), px(48.)));
        assert_eq!(brush_fraction(px(150.), empty), None);
    }
    #[test]
    fn width_label_names_the_unit() {
        assert_eq!(width_label(secs(15)), "15s");
        assert_eq!(width_label(secs(300)), "5m");
        assert_eq!(width_label(secs(10_800)), "3h");
        assert_eq!(width_label(secs(86_400)), "1d");
    }
}
