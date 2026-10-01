//! k8sBoard desktop application entry point.

use gpui_kit::*;
use tracing_subscriber::EnvFilter;

const APP_TITLE: &str = "k8sBoard";

/// Root view of the application window.
struct AppShell;

impl Render for AppShell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(APP_TITLE)
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some(APP_TITLE.into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        if let Err(error) = gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| AppShell)) {
            tracing::error!(%error, "failed to open the main window");
            cx.quit();
        }
    });
    Ok(())
}
