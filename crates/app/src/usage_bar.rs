//! The thin bar that shows usage against a total: node table cells and the drawers.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    App, DefiniteLength, Hsla, IntoElement, ParentElement as _, Styled as _, div, px, relative,
};

use crate::cluster_capacity::CapacityRow;
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

const CAPACITY_BAR_HEIGHT: f32 = 10.;
/// Requested layer of a capacity bar: the foreground, faint, under the used layer.
const REQUESTED_OPACITY: f32 = 0.28;
/// The notch that keeps the requested position visible when the used layer covers it.
const REQUESTED_MARKER_WIDTH: f32 = 2.;

/// The layers of a capacity row's bar, as shares of its total.
#[derive(Debug, PartialEq)]
pub(crate) struct CapacityBar {
    /// Clamped to 0..=1; `None` draws no used layer.
    pub(crate) used: Option<f32>,
    /// Clamped to 0..=1; `None` draws no requested layer.
    pub(crate) requested: Option<f32>,
}

impl CapacityBar {
    pub(crate) fn of_row(row: &CapacityRow) -> Self {
        let (used, requested) = row.ratios();
        Self {
            used: used.map(clamp_unit),
            requested: requested.map(clamp_unit),
        }
    }
}

/// A full-width track with the requested layer under the used one, and a background-colored notch
/// at the requested position. The layers are neutral, like the wireframe; the figures beside the
/// bar carry the tone.
pub(crate) fn capacity_bar(bar: CapacityBar, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let layer = |share: f32, color: Hsla| {
        div()
            .absolute()
            .top_0()
            .left_0()
            .h_full()
            .w(relative(share))
            .bg(color)
    };
    div()
        .relative()
        .w_full()
        .h(px(CAPACITY_BAR_HEIGHT))
        .rounded(theme.radius)
        .overflow_hidden()
        .bg(theme.muted)
        .children(
            bar.requested
                .map(|share| layer(share, theme.foreground.opacity(REQUESTED_OPACITY))),
        )
        .children(bar.used.map(|share| layer(share, theme.foreground)))
        .children(
            bar.requested
                .filter(|share| *share > 0. && *share < 1.)
                .map(|share| {
                    div()
                        .absolute()
                        .top_0()
                        .left(relative(share))
                        .ml(px(-REQUESTED_MARKER_WIDTH / 2.))
                        .w(px(REQUESTED_MARKER_WIDTH))
                        .h_full()
                        .bg(theme.background)
                }),
        )
}

#[cfg(test)]
mod tests {
    use crate::cluster_capacity::FromPods;

    use super::*;

    #[test]
    fn bar_clamps_fill_and_marker() {
        let bar = UsageBar::of_ratio(1.4, Some(-0.2));
        assert_eq!(bar.fill, 1.);
        assert_eq!(bar.marker, Some(0.));
        assert_eq!(UsageBar::of_ratio(f64::NAN, None).fill, 0.);
        assert_eq!(UsageBar::of_ratio(0.25, Some(2.)).marker, Some(1.));
    }

    fn compute_row(used: Option<f64>, requested: Option<f64>) -> CapacityRow {
        CapacityRow::Cpu(crate::cluster_capacity::Layers {
            used,
            requested: requested.map_or(FromPods::NeedsAllNamespaces, FromPods::Known),
            allocatable: 10.,
            nodes: 1,
            unsampled_nodes: 0,
        })
    }

    #[test]
    fn capacity_bar_clamps_layers() {
        let bar = CapacityBar::of_row(&compute_row(Some(25.), Some(5.)));
        assert_eq!(bar.used, Some(1.));
        assert_eq!(bar.requested, Some(0.5));
    }

    #[test]
    fn capacity_bar_without_usage_has_no_used_layer() {
        let bar = CapacityBar::of_row(&compute_row(None, Some(5.)));
        assert_eq!(bar.used, None);
        assert_eq!(
            CapacityBar::of_row(&compute_row(Some(2.), None)).requested,
            None
        );
    }

    #[test]
    fn bar_tone_follows_the_unclamped_ratio() {
        assert_eq!(UsageBar::of_ratio(0.5, None).tone, None);
        assert_eq!(UsageBar::of_ratio(0.82, None).tone, Some(StatusTone::Warn));
        assert_eq!(UsageBar::of_ratio(1.4, None).tone, Some(StatusTone::Bad));
    }
}
