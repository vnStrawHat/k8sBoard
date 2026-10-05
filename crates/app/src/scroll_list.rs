//! A scrolling list whose scrollbar is always drawn. The kit scrollbar fades out when idle, so a
//! result list in a dialog gave no sign that more rows hide below the fold.

use gpui_kit::component::scroll::{Scrollbar, ScrollbarMode};
use gpui_kit::{
    AnyElement, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, ScrollHandle,
    SharedString, StatefulInteractiveElement as _, Styled as _, div,
};

/// `content` in a box of at most `max_height` that scrolls, with the scrollbar drawn from the
/// start. `scroll` must live as long as the view that shows the list, so the position survives a
/// re-render.
pub(crate) fn scroll_list(
    id: &'static str,
    scroll: &ScrollHandle,
    max_height: Pixels,
    content: impl IntoElement,
) -> AnyElement {
    div()
        .relative()
        .child(
            div()
                .id(id)
                .max_h(max_height)
                .overflow_y_scroll()
                .track_scroll(scroll)
                .child(content),
        )
        .child(
            div().absolute().inset_0().child(
                Scrollbar::vertical(scroll)
                    .id(SharedString::from(format!("{id}-scrollbar")))
                    .mode(ScrollbarMode::Always)
                    .viewport_from_layout(),
            ),
        )
        .into_any_element()
}
