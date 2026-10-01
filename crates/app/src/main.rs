//! k8sBoard desktop application entry point.

mod age;
mod app_shell;
mod batch_rows;
mod cluster_runtime;
mod cluster_session;
mod config_map_rows;
mod container_detail;
mod drawer;
mod event_rows;
mod kind_drawer;
mod kind_row;
mod kind_table;
mod launch_options;
mod log_buffer;
mod log_dock;
mod log_tab;
mod namespace_rows;
mod navigation;
mod network_rows;
mod node_drawer;
mod node_table;
mod object_events;
mod pod_diagnosis;
mod pod_drawer;
mod pod_table;
mod related_pods;
mod resource_actions;
mod resource_kind;
mod screenshot;
mod status_bar;
mod status_tone;
mod table_layout;
mod table_selection;
mod title_bar;
mod workload_rows;
mod yaml_view;

use std::process::ExitCode;
use std::time::Duration;

use gpui_kit::component::{Theme, ThemeMode, TitleBar};
use gpui_kit::{App, AppContext as _, Bounds, WindowBounds, WindowOptions, px, size};
use tracing_subscriber::EnvFilter;

use crate::app_shell::AppShell;
use crate::cluster_runtime::ClusterRuntime;
use crate::launch_options::{LaunchOptions, LaunchRequest, ThemeChoice, USAGE};

const WINDOW_WIDTH: f32 = 1320.;
const WINDOW_HEIGHT: f32 = 900.;
/// The API client needs few threads: every request waits on the network.
const RUNTIME_WORKER_THREADS: usize = 2;
const RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let options = match launch_options::parse_launch_options(std::env::args().skip(1)) {
        Ok(LaunchRequest::Run(options)) => options,
        Ok(LaunchRequest::Help) => {
            eprint!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("error: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if options.screenshot.is_some() && !cfg!(feature = "screenshot") {
        eprintln!("error: --screenshot is a dev-only flag: rebuild with --features screenshot");
        return ExitCode::from(2);
    }
    match run(options) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(options: LaunchOptions) -> anyhow::Result<ExitCode> {
    // The runtime lives on this stack frame until the UI loop ends; GPUI only gets a handle.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(RUNTIME_WORKER_THREADS)
        .thread_name("k8sboard-kube")
        .enable_all()
        .build()?;
    let handle = runtime.handle().clone();

    #[cfg(feature = "screenshot")]
    let outcome = std::rc::Rc::new(std::cell::Cell::new(screenshot::ScreenshotOutcome::Failed));
    #[cfg(feature = "screenshot")]
    let hook_outcome = std::rc::Rc::clone(&outcome);

    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            apply_theme(options.theme, cx);
            cx.set_global(ClusterRuntime::new(handle));
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            #[cfg(feature = "screenshot")]
            let screenshot_request =
                options
                    .screenshot
                    .clone()
                    .map(|path| screenshot::ScreenshotRequest {
                        path,
                        screen: options.screen,
                    });
            let window_options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)),
                    cx,
                ))),
                ..TitleBar::window_options()
            };
            let opened = gpui_kit::open_window(window_options, cx, |window, cx| {
                cx.new(|cx| AppShell::new(options, window, cx))
            });
            let (window, shell) = match opened {
                Ok(opened) => opened,
                Err(error) => {
                    tracing::error!(%error, "failed to open the main window");
                    cx.quit();
                    return;
                }
            };
            #[cfg(feature = "screenshot")]
            if let Some(request) = screenshot_request {
                screenshot::capture(window, shell, request, hook_outcome, cx);
            }
            #[cfg(not(feature = "screenshot"))]
            let _ = (window, shell);
        });
    // A stuck credential plugin or watch must not keep the process alive after the window closes.
    runtime.shutdown_timeout(RUNTIME_SHUTDOWN_TIMEOUT);

    #[cfg(feature = "screenshot")]
    return Ok(ExitCode::from(outcome.get().exit_code()));
    #[cfg(not(feature = "screenshot"))]
    Ok(ExitCode::SUCCESS)
}

fn apply_theme(choice: Option<ThemeChoice>, cx: &mut App) {
    match choice {
        Some(ThemeChoice::Light) => Theme::change(ThemeMode::Light, None, cx),
        Some(ThemeChoice::Dark) => Theme::change(ThemeMode::Dark, None, cx),
        None => Theme::sync_system_appearance(None, cx),
    }
    // The kit highlights the selected row with a faint tint; the theme's selection colour
    // makes the open drawer's row easy to find in both modes.
    Theme::update(cx, |theme| theme.table_active = theme.selection);
}
