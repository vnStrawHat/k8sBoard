//! The thin bar that shows usage against a total: node table cells and the drawers.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    App, DefiniteLength, IntoElement, ParentElement as _, Styled as _, div, px, relative,
};

use crate::status_tone::{StatusTone, tone_color};
use crate::usage_format::usage_tone;

const TRACK_HEIGHT: f32 = 6.;
const MARKER_WIDTH: f32 = 2.;
const MARKER_HEIGHT: f32 = 12.;

#[derive(Debug, PartialEq)]
pub(crate) struct UsageBar {
    /// Clamped to 0..=1.
    pub(crate) fill: f32,
    /// A request tick, clamped to 0..=1; `None` hides it.
    pub(crate) marker: Option<f32>,
    pub(crate) tone: Option<StatusTone>,
}

impl UsageBar {
    /// A bar filled to `ratio` (usage over its total), toned like the numbers beside it. A ratio
    /// past 1 fills the bar but keeps its tone.
    pub(crate) fn of_ratio(ratio: f64, marker: Option<f64>) -> Self {
        Self {
            fill: clamp_unit(ratio),
            marker: marker.map(clamp_unit),
            tone: usage_tone(ratio),
        }
    }
}

fn clamp_unit(ratio: f64) -> f32 {
    if ratio.is_nan() {
        return 0.;
    }
    ratio.clamp(0., 1.) as f32
}

/// A `TRACK_HEIGHT` track with a fill and an optional request tick, `width` wide.
pub(crate) fn usage_bar(
    bar: UsageBar,
    width: impl Into<DefiniteLength>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let fill_color = bar
        .tone
        .map_or(theme.muted_foreground, |tone| tone_color(tone, cx));
    let track = div()
        .absolute()
        .top(px((MARKER_HEIGHT - TRACK_HEIGHT) / 2.))
        .left_0()
        .right_0()
        .h(px(TRACK_HEIGHT))
        .rounded(theme.radius)
        .overflow_hidden()
        .bg(theme.muted)
        .child(div().h_full().w(relative(bar.fill)).bg(fill_color));
    let marker = bar.marker.map(|position| {
        div()
            .absolute()
            .top_0()
            .left(relative(position))
            .ml(px(-MARKER_WIDTH / 2.))
            .w(px(MARKER_WIDTH))
            .h(px(MARKER_HEIGHT))
            .bg(theme.foreground)
    });
    div()
        .relative()
        .flex_shrink_0()
        .w(width.into())
        .h(px(MARKER_HEIGHT))
        .child(track)
        .children(marker)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_clamps_fill_and_marker() {
        let bar = UsageBar::of_ratio(1.4, Some(-0.2));
        assert_eq!(bar.fill, 1.);
        assert_eq!(bar.marker, Some(0.));
        assert_eq!(UsageBar::of_ratio(f64::NAN, None).fill, 0.);
        assert_eq!(UsageBar::of_ratio(0.25, Some(2.)).marker, Some(1.));
    }

    #[test]
    fn bar_tone_follows_the_unclamped_ratio() {
        assert_eq!(UsageBar::of_ratio(0.5, None).tone, None);
        assert_eq!(UsageBar::of_ratio(0.82, None).tone, Some(StatusTone::Warn));
        assert_eq!(UsageBar::of_ratio(1.4, None).tone, Some(StatusTone::Bad));
    }
}
