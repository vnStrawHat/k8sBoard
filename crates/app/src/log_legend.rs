//! The pod legend of a workload log tab: one chip per pod name, in the color of its prefix.

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    AnyElement, App, Hsla, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
};

use crate::status_tone::{StatusTone, readable_chart_color, tone_color};

/// `chart_1..chart_5`; a sixth pod reuses the first color.
const POD_COLOR_SLOTS: usize = 5;

pub(crate) struct LegendChip {
    /// The short pod name, as in the row prefix.
    pub(crate) short_name: SharedString,
    pub(crate) color_slot: usize,
    /// The dot after the name: what the pod's streams add up to.
    pub(crate) tone: StatusTone,
    /// Every stream of the pod left the list.
    pub(crate) is_deleted: bool,
    /// The first stream error, shown as a tooltip.
    pub(crate) failure: Option<SharedString>,
}

/// The prefix and legend color of a pod, readable as text on either theme.
pub(crate) fn pod_color(slot: usize, cx: &App) -> Hsla {
    let theme = cx.theme();
    let chart = match slot % POD_COLOR_SLOTS {
        0 => theme.chart_1,
        1 => theme.chart_2,
        2 => theme.chart_3,
        3 => theme.chart_4,
        _ => theme.chart_5,
    };
    readable_chart_color(chart, cx)
}

pub(crate) fn legend_row(chips: Vec<LegendChip>, is_frozen: bool, cx: &App) -> AnyElement {
    let theme = cx.theme();
    h_flex()
        .flex_shrink_0()
        .flex_wrap()
        .items_center()
        .gap_x_3()
        .gap_y_1()
        .px_3()
        .py_1()
        .border_b_1()
        .border_color(theme.border)
        .text_xs()
        .children(chips.into_iter().enumerate().map(|(index, chip)| {
            h_flex()
                .id(("log-legend-chip", index))
                .items_center()
                .gap_1()
                .font_family(theme.mono_font_family.clone())
                .child(
                    div()
                        .size_2()
                        .rounded_full()
                        .bg(pod_color(chip.color_slot, cx)),
                )
                .child(chip.short_name.clone())
                .child(
                    div()
                        .size_1p5()
                        .rounded_full()
                        .bg(tone_color(chip.tone, cx)),
                )
                .when(chip.is_deleted, |item| {
                    item.child(div().text_color(theme.muted_foreground).child("deleted"))
                })
                .when_some(chip.failure, |item, message| {
                    item.tooltip(move |window, cx| Tooltip::new(message.clone()).build(window, cx))
                })
        }))
        .when(is_frozen, |row| {
            row.child(
                div()
                    .text_color(theme.muted_foreground)
                    .child("Pod changes are not followed outside the namespace filter"),
            )
        })
        .into_any_element()
}
