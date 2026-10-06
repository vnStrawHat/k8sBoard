//! One rendered log row: time, level tag, text with filter highlights, and the JSON block.

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Div, HighlightStyle, Hsla, IntoElement as _, ParentElement as _, Rems,
    SharedString, Styled as _, StyledText, div, prelude::FluentBuilder as _, rems,
};
use jiff::tz::TimeZone;

use crate::line_matcher::LineMatcher;
use crate::log_buffer::{BufferedLine, LineKind, format_log_time};
use crate::log_json::{JsonLine, json_line};
use crate::log_level::LogLevel;
use crate::status_tone::{StatusTone, tone_color};

/// 13 characters of the mono `text_xs` font (0.75 rem at about 0.6 em per character).
const TIME_COLUMN_WIDTH: Rems = rems(5.85);
/// `ERROR` plus a little air.
const LEVEL_COLUMN_WIDTH: Rems = rems(2.9);
/// `x2k4q/container` fits for the usual names; longer ones are cut.
const PREFIX_COLUMN_WIDTH: Rems = rems(9.);
const ERROR_TINT_OPACITY: f32 = 0.08;

/// The `{pod}/{container}` cell of a workload tab.
pub(crate) struct RowPrefix {
    pub(crate) text: SharedString,
    pub(crate) color: Hsla,
}

/// How every row of a tab is drawn.
pub(crate) struct RowStyle<'a> {
    pub(crate) shows_timestamps: bool,
    /// The zone the timestamp column reads in.
    pub(crate) time_zone: &'a TimeZone,
    pub(crate) wraps_lines: bool,
    pub(crate) shows_json: bool,
    pub(crate) matcher: Option<&'a LineMatcher>,
    pub(crate) prefix: Option<RowPrefix>,
}

pub(crate) fn log_row(line: &BufferedLine, style: &RowStyle, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let is_marker = line.kind == LineKind::Marker;
    // The column is the one that sorts, so a time the line itself starts with would show twice.
    let text = match (style.shows_timestamps, line.line.timestamp, is_marker) {
        (true, Some(_), false) => without_leading_timestamp(&line.line.text),
        _ => line.line.text.as_str(),
    };
    let json = (style.shows_json && !is_marker)
        .then(|| json_line(text))
        .flatten();
    let headline = match &json {
        Some(JsonLine {
            headline: Some(headline),
            ..
        }) => headline.as_str(),
        _ => text,
    };
    // An empty line still needs a line box, or the row would collapse to nothing.
    let shown = if headline.is_empty() { " " } else { headline };
    let highlights: Vec<_> = style
        .matcher
        .filter(|_| !is_marker)
        .map(|matcher| matcher.ranges(shown))
        .unwrap_or_default()
        .into_iter()
        .map(|range| {
            let highlight = HighlightStyle {
                background_color: Some(theme.selection),
                ..Default::default()
            };
            (range, highlight)
        })
        .collect();
    let time = line
        .line
        .timestamp
        .map(|timestamp| format_log_time(timestamp, style.time_zone))
        .unwrap_or_default();
    let text_column = v_flex()
        .flex_1()
        .min_w_0()
        .when(is_marker, |column| {
            column.text_color(theme.muted_foreground)
        })
        .child(
            no_wrap_unless(div(), style.wraps_lines)
                .child(StyledText::new(shown.to_owned()).with_highlights(highlights)),
        )
        .children(json.iter().flat_map(|json| &json.details).map(|detail| {
            no_wrap_unless(div(), style.wraps_lines)
                .text_color(theme.muted_foreground)
                .child(StyledText::new(detail.clone()))
        }));
    h_flex()
        .items_start()
        .gap_2()
        .font_family(theme.mono_font_family.clone())
        .text_xs()
        .when(line.level == Some(LogLevel::Error), |row| {
            row.bg(tone_color(StatusTone::Bad, cx).opacity(ERROR_TINT_OPACITY))
        })
        .when(style.shows_timestamps, |row| {
            row.child(
                div()
                    .w(TIME_COLUMN_WIDTH)
                    .flex_shrink_0()
                    .text_color(theme.muted_foreground)
                    .child(time),
            )
        })
        .children(style.prefix.as_ref().map(|prefix| {
            div()
                .w(PREFIX_COLUMN_WIDTH)
                .flex_shrink_0()
                .truncate()
                .text_color(prefix.color)
                .child(prefix.text.clone())
        }))
        .when(is_marker, |row| row.child(system_tag(cx)))
        .when(style.shows_json && !is_marker, |row| {
            row.child(level_tag(line.level, cx))
        })
        .child(text_column)
        .into_any_element()
}

fn no_wrap_unless(cell: Div, wraps_lines: bool) -> Div {
    cell.when(!wraps_lines, |cell| cell.whitespace_nowrap().truncate())
}

/// The `SYS` tag of a marker row, shown in either mode.
fn system_tag(cx: &App) -> Div {
    div()
        .w(LEVEL_COLUMN_WIDTH)
        .flex_shrink_0()
        .text_color(tone_color(StatusTone::Warn, cx))
        .child("SYS")
}

/// The level word in JSON mode; the column stays so rows without a level line up.
fn level_tag(level: Option<LogLevel>, cx: &App) -> Div {
    let theme = cx.theme();
    let tag = div().w(LEVEL_COLUMN_WIDTH).flex_shrink_0();
    let Some(level) = level else {
        return tag;
    };
    let color: Hsla = match level {
        LogLevel::Error => tone_color(StatusTone::Bad, cx),
        LogLevel::Warn => tone_color(StatusTone::Warn, cx),
        LogLevel::Info | LogLevel::Debug => theme.muted_foreground,
    };
    tag.text_color(color)
        .child(SharedString::from(level.label()))
}

/// `text` without a leading ISO 8601 time (`2026-10-05T11:09:25+07:00`, fractions and `Z` allowed)
/// and the blanks after it; the text of a line that does not start with one.
fn without_leading_timestamp(text: &str) -> &str {
    let bytes = text.as_bytes();
    let digits = |from: usize, count: usize| {
        bytes
            .get(from..from + count)
            .is_some_and(|part| part.iter().all(u8::is_ascii_digit))
    };
    let at = |index: usize, wanted: u8| bytes.get(index) == Some(&wanted);
    let is_date_and_time = digits(0, 4)
        && at(4, b'-')
        && digits(5, 2)
        && at(7, b'-')
        && digits(8, 2)
        && at(10, b'T')
        && digits(11, 2)
        && at(13, b':')
        && digits(14, 2)
        && at(16, b':')
        && digits(17, 2);
    if !is_date_and_time {
        return text;
    }
    let mut end = 19;
    if at(end, b'.') {
        end += 1;
        while digits(end, 1) {
            end += 1;
        }
    }
    if at(end, b'Z') {
        end += 1;
    } else if (at(end, b'+') || at(end, b'-')) && digits(end + 1, 2) {
        end += 3;
        // The minutes of the offset, with or without a colon.
        let colon = usize::from(at(end, b':'));
        if digits(end + colon, 2) {
            end += colon + 2;
        }
    }
    text[end..].trim_start()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leading_iso_time_is_removed_with_its_blanks() {
        for (text, rest) in [
            ("2026-10-05T11:09:25+07:00\tinfo started", "info started"),
            ("2026-10-05T11:09:25Z started", "started"),
            ("2026-10-05T11:09:25.123456789Z started", "started"),
            ("2026-10-05T11:09:25.5-0530 started", "started"),
            ("2026-10-05T11:09:25+07:00info started", "info started"),
            ("2026-10-05T11:09:25 started", "started"),
        ] {
            assert_eq!(without_leading_timestamp(text), rest, "{text}");
        }
    }

    #[test]
    fn other_lines_keep_their_text() {
        for text in [
            "time=\"2026-10-05T02:16:23+07:00\" level=error",
            "2026/10/05 02:21:42 http: TLS handshake error",
            "2026-10-05 11:09:25 started",
            "2026-10-05T11:09 started",
            "{\"time\":\"2026-10-05T11:09:25Z\"}",
            "",
        ] {
            assert_eq!(without_leading_timestamp(text), text, "{text}");
        }
    }
}
