//! Copy buttons and click-to-copy text of the drawers: object names, images, addresses, hosts, and
//! label sets. The values are plain object data and go to the clipboard as they are. A Secret's
//! values never pass through here: they keep the masked path of `secret_clipboard`.

use gpui_kit::component::clipboard::Clipboard;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    AnyElement, App, ClipboardItem, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, Styled as _, div,
};

use crate::drawer::truncated_text;

/// The text a whole set of terms copies: one term per line.
pub(crate) fn joined_terms(terms: &[SharedString]) -> String {
    terms
        .iter()
        .map(SharedString::as_ref)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Puts `text` on the clipboard.
pub(crate) fn copy_text(text: &str, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
}

/// A small ghost icon button that copies `text`; the icon turns into a check for a moment after
/// the copy. The `id` must be unique among the elements that can be on screen together.
pub(crate) fn copy_button(id: impl Into<ElementId>, text: impl Into<SharedString>) -> AnyElement {
    let id = id.into();
    let selector = id.to_string();
    div()
        .flex_shrink_0()
        .debug_selector(move || selector)
        .child(Clipboard::new(id).value(text).tooltip("Copy"))
        .into_any_element()
}

/// Mono text that truncates with its full value as a tooltip, followed by a copy button. The text
/// takes `id`; the button takes a child id of it.
pub(crate) fn copyable_mono(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    cx: &App,
) -> AnyElement {
    let id = id.into();
    let text = text.into();
    h_flex()
        .gap_1()
        .items_center()
        .child(
            truncated_text(id.clone(), text.clone())
                .min_w_0()
                .font_family(cx.theme().mono_font_family.clone()),
        )
        .child(copy_button((id, "copy"), text))
        .into_any_element()
}

#[cfg(test)]
#[path = "clipboard_copy_tests.rs"]
mod clipboard_copy_tests;
