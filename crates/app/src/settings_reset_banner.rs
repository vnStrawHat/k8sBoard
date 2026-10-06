//! The persistent banner under the title bar after the settings file was reset (spec 0030).

use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::{
    App, ClickEvent, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, Window,
    div, px,
};

/// `text` on a warning-tinted strip with a Dismiss button. The strip stays until `on_dismiss`
/// drops it; nothing about it is saved.
pub(crate) fn reset_banner(
    text: &str,
    cx: &App,
    on_dismiss: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let warning = cx.theme().warning;
    h_flex()
        .id("settings-reset-banner")
        .flex_none()
        .w_full()
        .gap_3()
        .px_3()
        .py_1()
        .items_center()
        .bg(warning.opacity(0.15))
        .border_b_1()
        .border_color(warning)
        .text_sm()
        .child(div().flex_1().min_w(px(0.)).child(text.to_owned()))
        .child(
            Button::new("dismiss-settings-reset")
                .label("Dismiss")
                .small()
                .outline()
                .on_click(on_dismiss),
        )
}
