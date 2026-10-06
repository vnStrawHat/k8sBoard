//! One rendered log row: time, level tag, text with filter highlights, and the JSON block.

use std::borrow::Cow;

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Div, HighlightStyle, Hsla, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Rems, SharedString, StatefulInteractiveElement as _, Styled as _,
    StyledText, div, prelude::FluentBuilder as _, rems,
};
use jiff::tz::TimeZone;

use crate::line_matcher::LineMatcher;
use crate::log_buffer::{BufferedLine, LineKind, format_log_time, log_date_prefix};
use crate::log_json::{JsonLine, json_line};
use crate::log_level::LogLevel;
use crate::status_tone::{StatusTone, tone_color};

/// 13 characters of the mono `text_xs` font (0.75 rem at about 0.6 em per character).
const TIME_COLUMN_WIDTH: Rems = rems(5.85);
/// `ERROR` plus a little air.
/// `MM-DD ` is six characters of the same font.
const DATE_PREFIX_WIDTH: Rems = rems(2.7);
const LEVEL_COLUMN_WIDTH: Rems = rems(2.9);
/// `x2k4q/container` fits for the usual names; longer ones are cut.
const PREFIX_COLUMN_WIDTH: Rems = rems(9.);
/// A line this long may be cut by the row, so it gets a tooltip with the whole text. The width of
/// the dock is not known here, so shorter lines never get one.
const TOOLTIP_MIN_CHARS: usize = 80;
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
    /// The date in `time_zone`; a line from another day carries its `MM-DD`.
    pub(crate) today: jiff::civil::Date,
    /// Whether the time column has room for the date, so the times of every row stay in line.
    pub(crate) reserves_date: bool,
    pub(crate) wraps_lines: bool,
    pub(crate) shows_json: bool,
    pub(crate) matcher: Option<&'a LineMatcher>,
    pub(crate) prefix: Option<RowPrefix>,
}

pub(crate) fn log_row(index: usize, line: &BufferedLine, style: &RowStyle, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let is_marker = line.kind == LineKind::Marker;
    // The column is the one that sorts, so a time the line itself starts with would show twice.
    let text = match (style.shows_timestamps, line.line.timestamp, is_marker) {
        (true, Some(_), false) => without_leading_time(&line.line.text),
        _ => Cow::Borrowed(line.line.text.as_str()),
    };
    let json = (style.shows_json && !is_marker)
        .then(|| json_line(&text))
        .flatten();
    let headline = match &json {
        Some(JsonLine {
            headline: Some(headline),
            ..
        }) => headline.as_str(),
        _ => text.as_ref(),
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
        .map(|timestamp| {
            let date = log_date_prefix(timestamp, style.time_zone, style.today);
            let clock = format_log_time(timestamp, style.time_zone);
            format!("{}{clock}", date.unwrap_or_default())
        })
        .unwrap_or_default();
    let time_width = if style.reserves_date {
        TIME_COLUMN_WIDTH + DATE_PREFIX_WIDTH
    } else {
        TIME_COLUMN_WIDTH
    };
    let text_column = v_flex()
        .flex_1()
        .min_w_0()
        .when(is_marker, |column| {
            column.text_color(theme.muted_foreground)
        })
        .child(
            no_wrap_unless(div(), style.wraps_lines)
                .id(("log-line", index))
                .child(StyledText::new(shown.to_owned()).with_highlights(highlights))
                .when(
                    !style.wraps_lines && shown.chars().count() > TOOLTIP_MIN_CHARS,
                    |cell| {
                        let full = SharedString::from(shown.to_owned());
                        cell.tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
                    },
                ),
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
                    .w(time_width)
                    .flex_shrink_0()
                    .text_right()
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

/// `text` without the time it starts with, which the Timestamps column already shows: an ISO time,
/// a logfmt `time=`/`ts=` field, or a JSON `"time":`/`"ts":` first member. The rest of the line is
/// kept as it was.
fn without_leading_time(text: &str) -> Cow<'_, str> {
    let iso = without_leading_timestamp(text);
    if iso.len() != text.len() {
        return Cow::Borrowed(iso);
    }
    if let Some(rest) = without_logfmt_time(text) {
        return Cow::Borrowed(rest);
    }
    match without_json_time(text) {
        Some(rest) => Cow::Owned(rest),
        None => Cow::Borrowed(text),
    }
}

/// The text after a `time=` or `ts=` field, quoted or bare, whose value starts like a time (a
/// digit).
fn without_logfmt_time(text: &str) -> Option<&str> {
    let value = ["time=", "ts="]
        .iter()
        .find_map(|key| text.strip_prefix(key))?;
    let end = match value.strip_prefix('"') {
        Some(quoted) => 1 + quoted.find('"')? + 1,
        None => value.find(char::is_whitespace).unwrap_or(value.len()),
    };
    let first = value.trim_start_matches('"').chars().next()?;
    first.is_ascii_digit().then(|| value[end..].trim_start())
}

/// `{rest}` for an object whose first member is `"time":"…"` or `"ts":"…"` followed by a comma.
fn without_json_time(text: &str) -> Option<String> {
    let members = text.trim_start().strip_prefix('{')?.trim_start();
    let value = ["\"time\"", "\"ts\""]
        .iter()
        .find_map(|key| members.strip_prefix(key))?
        .trim_start()
        .strip_prefix(':')?
        .trim_start()
        .strip_prefix('"')?;
    let (time, rest) = value.split_once('"')?;
    if !time.starts_with(|first: char| first.is_ascii_digit()) {
        return None;
    }
    let remaining = rest.trim_start().strip_prefix(',')?.trim_start();
    Some(format!("{{{remaining}"))
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
    fn a_leading_logfmt_time_field_is_removed() {
        for (text, rest) in [
            (
                "time=\"2026-10-05T02:16:23+07:00\" level=error msg=boom",
                "level=error msg=boom",
            ),
            ("time=2026-10-05T02:16:23Z level=info", "level=info"),
            (
                "ts=\"2026-10-05T02:16:23.5Z\" caller=main.go:7",
                "caller=main.go:7",
            ),
            ("ts=1759648583.5 level=warn", "level=warn"),
        ] {
            assert_eq!(without_leading_time(text), rest, "{text}");
        }
    }

    #[test]
    fn a_leading_json_time_member_is_removed_and_the_rest_stays_valid() {
        for (text, rest) in [
            (
                r#"{"ts":"2026-09-29T09:14:52.541Z","level":"info","msg":"hi"}"#,
                r#"{"level":"info","msg":"hi"}"#,
            ),
            (
                r#"{ "time": "2026-09-29T09:14:52Z" , "msg": "hi" }"#,
                r#"{"msg": "hi" }"#,
            ),
        ] {
            assert_eq!(without_leading_time(text), rest, "{text}");
            assert!(
                serde_json::from_str::<serde_json::Value>(rest).is_ok(),
                "{rest}"
            );
        }
    }

    #[test]
    fn other_lines_keep_their_text() {
        for text in [
            "2026/10/05 02:21:42 http: TLS handshake error",
            "2026-10-05 11:09:25 started",
            "2026-10-05T11:09 started",
            "level=error time=\"2026-10-05T02:16:23+07:00\"",
            "time=soon level=error",
            "ts=",
            "time=\"2026-10-05",
            r#"{"time":"2026-10-05T11:09:25Z"}"#,
            r#"{"msg":"hi","ts":"2026-10-05T11:09:25Z"}"#,
            r#"{"ts":"later","msg":"hi"}"#,
            "",
        ] {
            assert_eq!(without_leading_time(text), text, "{text}");
        }
    }
}
