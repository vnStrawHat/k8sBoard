//! The log volume histogram of a zoomed tab: lines per time bucket, errors marked.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::chart::BarChart;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    AnyElement, App, IntoElement as _, ParentElement as _, SharedString, Styled as _, div, px,
};

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

/// The caption and the bars. A bar with an error line is drawn in the `Bad` tone.
pub(crate) fn volume_chart(volume: &Rc<Volume>, cx: &App) -> AnyElement {
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
        .child(div().flex_1().h(px(CHART_HEIGHT)).py_1().child(chart))
        .into_any_element()
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn width_label_names_the_unit() {
        assert_eq!(width_label(secs(15)), "15s");
        assert_eq!(width_label(secs(300)), "5m");
        assert_eq!(width_label(secs(10_800)), "3h");
        assert_eq!(width_label(secs(86_400)), "1d");
    }
}
