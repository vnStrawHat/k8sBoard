use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, ClickEvent, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::age::format_age;

pub(crate) const DRAWER_WIDTH: Pixels = px(420.);
pub(crate) const DRAWER_EXPANDED_WIDTH: Pixels = px(640.);
const LABEL_WIDTH: Pixels = px(104.);

/// The drawer is open exactly while a row is selected, so this holds only what the user
/// changes inside an open drawer. The tab and the expanded flag survive a change of
/// subject; the selected container does not.
pub(crate) struct DrawerState {
    pub(crate) tab: PodDrawerTab,
    pub(crate) is_expanded: bool,
    pub(crate) selected_container: Option<String>,
}

impl DrawerState {
    pub(crate) fn new() -> Self {
        Self {
            tab: PodDrawerTab::Overview,
            is_expanded: false,
            selected_container: None,
        }
    }

    pub(crate) fn width(&self) -> Pixels {
        if self.is_expanded {
            DRAWER_EXPANDED_WIDTH
        } else {
            DRAWER_WIDTH
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PodDrawerTab {
    Overview,
    Containers,
}

pub(crate) type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

pub(crate) struct ExpandToggle {
    pub(crate) is_expanded: bool,
    pub(crate) on_click: ClickHandler,
}

pub(crate) struct DrawerHeader {
    /// Two letters naming the kind, "Po" or "No".
    pub(crate) kind_badge: &'static str,
    pub(crate) name: SharedString,
    pub(crate) subtitle: AnyElement,
    /// The ⋯ button with its dropdown menu.
    pub(crate) menu: AnyElement,
    /// `None` for drawers that have only one column.
    pub(crate) expand: Option<ExpandToggle>,
    pub(crate) on_close: ClickHandler,
}

/// The shared frame: header, subtitle, optional tab bar, and a scrollable body. It is a
/// plain element laid over the workspace (the caller's container is `.relative()`), not
/// the kit `Sheet`: that one covers the sidebar and takes focus from the table.
pub(crate) fn drawer_frame(
    header: DrawerHeader,
    tabs: Option<AnyElement>,
    body: AnyElement,
    width: Pixels,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .w(width)
        .bg(theme.background)
        .border_l_1()
        .border_color(theme.border)
        .shadow_lg()
        // Clicks must not fall through to the table underneath.
        .occlude()
        .child(header_row(header, cx))
        .when_some(tabs, |this, tabs| this.child(tabs))
        .child(
            div()
                .id("drawer-body")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p_4()
                .child(body),
        )
}

fn header_row(header: DrawerHeader, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let on_close = header.on_close;
    v_flex()
        .flex_shrink_0()
        .gap_1()
        .px_4()
        .py_3()
        .border_b_1()
        .border_color(theme.border)
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .px_1p5()
                        .rounded(theme.radius)
                        .bg(theme.muted)
                        .text_color(theme.muted_foreground)
                        .text_xs()
                        .font_family(theme.mono_font_family.clone())
                        .child(header.kind_badge),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_semibold()
                        .font_family(theme.mono_font_family.clone())
                        .child(header.name),
                )
                .child(header.menu)
                .when_some(header.expand, |this, expand| {
                    let icon = if expand.is_expanded {
                        IconName::Minimize2
                    } else {
                        IconName::Maximize2
                    };
                    let on_click = expand.on_click;
                    this.child(
                        Button::new("drawer-expand")
                            .ghost()
                            .small()
                            .icon(Icon::new(icon))
                            .on_click(move |event, window, cx| on_click(event, window, cx)),
                    )
                })
                .child(
                    Button::new("drawer-close")
                        .ghost()
                        .small()
                        .icon(Icon::new(IconName::X))
                        .on_click(move |event, window, cx| on_close(event, window, cx)),
                ),
        )
        .child(header.subtitle)
}

/// The ⋯ button; the caller attaches the dropdown menu to it.
pub(crate) fn menu_button() -> Button {
    Button::new("drawer-menu")
        .ghost()
        .small()
        .icon(Icon::new(IconName::Ellipsis))
}

/// A muted section heading inside a drawer body.
pub(crate) fn section_title(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    div()
        .mt_4()
        .mb_2()
        .text_xs()
        .font_semibold()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// A label with its value, in the two-column layout shared by both drawers.
pub(crate) fn detail_row(
    label: &'static str,
    value: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    h_flex()
        .gap_3()
        .py_1()
        .items_start()
        .text_sm()
        .child(
            div()
                .w(LABEL_WIDTH)
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        // `overflow_hidden` keeps a long value inside the drawer; text values ellipsize.
        .child(div().flex_1().min_w_0().overflow_hidden().child(value))
}

/// A muted dash for a value the object does not have.
pub(crate) fn absent_text(cx: &App) -> impl IntoElement {
    div().text_color(cx.theme().muted_foreground).child("—")
}

/// The value as text, or the muted dash when it is missing.
pub(crate) fn value_or_absent(value: Option<&str>, cx: &App) -> AnyElement {
    match value {
        Some(text) => div().truncate().child(text.to_owned()).into_any_element(),
        None => absent_text(cx).into_any_element(),
    }
}

/// `created 3d ago` for a subtitle; `None` when the creation time is unknown.
pub(crate) fn created_text(
    created_at: Option<jiff::Timestamp>,
    now: jiff::Timestamp,
) -> Option<String> {
    created_at.map(|created_at| format!("created {} ago", format_age(Some(created_at), now)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn created_text_shows_age_or_nothing() {
        let now = jiff::Timestamp::from_second(7_200).expect("valid timestamp");
        let created = jiff::Timestamp::from_second(0).expect("valid timestamp");
        assert_eq!(
            created_text(Some(created), now).as_deref(),
            Some("created 2h ago")
        );
        assert_eq!(created_text(None, now), None);
    }

    #[test]
    fn drawer_is_wider_when_expanded() {
        let mut state = DrawerState::new();
        assert_eq!(state.width(), DRAWER_WIDTH);
        state.is_expanded = true;
        assert_eq!(state.width(), DRAWER_EXPANDED_WIDTH);
    }
}
