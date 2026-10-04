//! One rendered log row: time, level tag, text with filter highlights, and the JSON block.

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Div, HighlightStyle, Hsla, IntoElement as _, ParentElement as _, Rems,
    SharedString, Styled as _, StyledText, div, prelude::FluentBuilder as _, rems,
};

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
    pub(crate) wraps_lines: bool,
    pub(crate) shows_json: bool,
    pub(crate) matcher: Option<&'a LineMatcher>,
    pub(crate) prefix: Option<RowPrefix>,
}

pub(crate) fn log_row(line: &BufferedLine, style: &RowStyle, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let text = line.line.text.as_str();
    let is_marker = line.kind == LineKind::Marker;
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
    let time = line.line.timestamp.map(format_log_time).unwrap_or_default();
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
