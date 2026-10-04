//! The window of a popped-out log tab: the title bar, then the tab in its Full layout.

use gpui_kit::component::{ActiveTheme as _, TitleBar, v_flex};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Window, WindowBounds, WindowOptions,
    div, px, size,
};

use crate::log_tab::{LogLayout, LogTab};

const WINDOW_WIDTH: f32 = 1000.;
const WINDOW_HEIGHT: f32 = 640.;

/// The root of a pop-out window. It owns the tab: closing the window drops it, and a tab that is
/// dropped ends its streams.
pub(crate) struct LogWindow {
    tab: Entity<LogTab>,
    title: SharedString,
}

/// Opens a window for `tab`; the tab moves its window-bound parts there and uses the Full layout.
/// The window is titled `title` (taskbar, Alt Tab) with no cluster suffix. `None`, with a log
/// line, when the window could not open.
pub(crate) fn open_log_window(
    tab: Entity<LogTab>,
    title: String,
    cx: &mut App,
) -> Option<AnyWindowHandle> {
    let options = log_window_options(&title, cx);
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        tab.update(cx, |tab, cx| {
            tab.move_to_window(window, cx);
            tab.set_layout(LogLayout::Full, cx);
        });
        cx.new(|_| LogWindow {
            tab,
            title: title.into(),
        })
    });
    match opened {
        Ok((window, _)) => Some(window),
        Err(error) => {
            tracing::error!(%error, "failed to open a log window");
            None
        }
    }
}

/// Centered, with the OS window (taskbar, Alt Tab) named after the tab.
fn log_window_options(title: &str, cx: &App) -> WindowOptions {
    let mut options = TitleBar::window_options();
    options.window_bounds = Some(WindowBounds::Windowed(Bounds::centered(
        None,
        size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)),
        cx,
    )));
    if let Some(titlebar) = &mut options.titlebar {
        titlebar.title = Some(title.to_owned().into());
    }
    options
}

#[cfg(test)]
impl LogWindow {
    pub(crate) fn title(&self) -> &str {
        &self.title
    }
}

impl Render for LogWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                TitleBar::new().child(
                    div()
                        .truncate()
                        .font_family(theme.mono_font_family.clone())
                        .child(self.title.clone()),
                ),
            )
            .child(div().flex_1().min_h_0().child(self.tab.clone()))
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;

    use super::*;

    #[gpui_kit::test]
    fn the_os_window_is_named_after_the_tab(cx: &mut TestAppContext) {
        let options = cx.update(|cx| log_window_options("deploy/payments-api", cx));
        let title = options.titlebar.and_then(|titlebar| titlebar.title);
        assert_eq!(title.as_deref(), Some("deploy/payments-api"));
    }
}
