//! k8sBoard desktop application entry point.

mod access_bindings;
mod access_query;
mod access_rows;
mod active_session;
mod age;
mod app_shell;
mod audit_log;
mod batch_rows;
mod bundled_fonts;
mod cell_truncation;
mod certificate_expiry;
mod clipboard_copy;
mod cluster_capacity;
mod cluster_catalog;
mod cluster_form;
mod cluster_health;
mod cluster_metrics;
mod cluster_metrics_section;
mod cluster_registry;
mod cluster_runtime;
mod cluster_session;
mod cluster_switcher;
mod cluster_switcher_rows;
mod clusters_page;
mod color_theme;
mod command_palette;
mod config_map_rows;
mod confirm_dialog;
mod container_detail;
mod counted_text;
mod crd_rows;
mod custom_kind;
mod custom_rows;
mod debug_dialogs;
mod dock;
mod drain_placement;
mod drain_plan;
mod drain_run;
mod drain_tab;
mod drain_writes;
mod drawer;
mod edit_error_line;
mod edit_quota;
mod environment;
mod environment_form;
mod environments_page;
mod event_rows;
mod file_export;
mod filter_bar;
mod fresh_enter;
mod fuzzy_score;
mod helm_release_view;
mod helm_rows;
mod history_rings;
mod ingress_backends;
mod issue;
mod issue_board;
mod issue_feeds;
mod issue_kind_rules;
mod issue_rules;
mod issue_table;
mod keymap;
mod kind_access;
mod kind_diagnosis;
mod kind_drawer;
mod kind_join;
mod kind_row;
mod kind_table;
mod kubeconfig_folder;
mod kubeconfig_import;
mod kubelet_history;
mod kubelet_metrics;
mod last_log;
mod launch_options;
mod line_matcher;
#[cfg(feature = "hotpath-profiling-alloc")]
mod live_heap;
mod live_sections;
mod log_buffer;
mod log_filter;
#[cfg(test)]
mod log_fixtures;
mod log_json;
mod log_legend;
mod log_level;
mod log_rows;
mod log_selection;
mod log_since;
mod log_tab;
mod log_target;
mod log_volume;
mod log_window;
mod log_workload;
mod metadata_edits;
mod metrics_history;
mod monitor_data;
mod monitor_notices;
mod monitor_source;
mod monitor_tab;
mod name_index;
mod namespace_compare_rows;
mod namespace_compare_view;
mod namespace_picker;
mod namespace_rows;
mod navigation;
mod navigation_history;
mod network_policy_rows;
mod network_rows;
mod node_drawer;
mod node_edits;
mod node_heatmap;
mod node_summary;
mod node_table;
mod node_usage;
mod object_create_view;
mod object_events;
mod object_templates;
mod overview;
mod overview_report;
mod palette_search;
mod permission_table;
mod permissions_view;
mod pod_diagnosis;
mod pod_drawer;
mod pod_scheduling;
mod pod_table;
mod policy_rows;
mod port_forward_menu;
mod port_forwards;
mod process_memory;
mod process_usage;
mod quota_room;
mod recent_changes;
mod related_objects;
mod related_pods;
mod resource_actions;
mod resource_edits;
mod resource_kind;
mod revision_diff;
mod revision_history;
mod row_context;
mod row_selection;
mod scale_effects;
mod scheduler_summary;
mod screenshot;
#[cfg(any(test, feature = "screenshot"))]
mod screenshot_script;
mod scroll_list;
mod secret_clipboard;
mod secret_forms;
mod secret_rows;
mod secret_values;
mod settings;
mod settings_reset_banner;
mod settings_store;
mod settings_window;
mod shell_tab;
mod shortcut_sheet;
mod status_bar;
mod status_tone;
mod status_tooltip;
mod storage_rows;
mod table_filter;
mod table_layout;
mod table_selection;
mod table_sort;
mod table_view;
mod terminal_element;
mod terminal_input;
mod terminal_session;
mod title_bar;
mod topology_access;
mod topology_canvas;
mod topology_card;
mod topology_checks;
mod topology_colors;
mod topology_export;
mod topology_feeds;
#[cfg(test)]
mod topology_fixtures;
mod topology_graph;
mod topology_layout;
mod topology_port_labels;
mod topology_route;
mod topology_stroke;
mod topology_traffic;
#[cfg(any(test, feature = "screenshot"))]
mod topology_traffic_fixture;
mod topology_traffic_labels;
mod topology_view;
mod topology_viewport;
mod traffic_test_view;
mod usage_bar;
mod usage_chart;
mod usage_format;
mod value_popover;
mod values_edit;
mod volume_edits;
mod watched_kinds;
mod who_can_view;
mod workload_actions;
mod workload_rows;
mod write_guard;
mod yaml_diff;
mod yaml_edit;
mod yaml_view;

use std::process::ExitCode;
use std::time::Duration;

use gpui_kit::component::TitleBar;
use gpui_kit::{AppContext as _, Bounds, WindowBounds, WindowOptions, px, size};

use crate::app_shell::AppShell;
use crate::cluster_catalog::CatalogHandle;
use crate::cluster_runtime::ClusterRuntime;
use crate::launch_options::{LaunchOptions, LaunchRequest, USAGE, kubeconfig_chain};
use crate::settings::AppSettings;
use crate::settings_store::{
    CONFIG_DIR_ENV, LoadedSettings, config_dir, default_config_dir, load_settings,
};
use crate::settings_window::{
    ManageClusters, OpenSettings, SettingsPage, SettingsSize, manage_clusters, open_settings_window,
};

const WINDOW_WIDTH: f32 = 1320.;
const WINDOW_HEIGHT: f32 = 900.;
/// The API client needs few threads: every request waits on the network.
const RUNTIME_WORKER_THREADS: usize = 2;
const RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(log_filter::log_filter())
        .init();

    let options = match launch_options::parse_launch_options(std::env::args().skip(1)) {
        Ok(LaunchRequest::Run(options)) => *options,
        Ok(LaunchRequest::Help) => {
            eprint!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("error: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if (options.screenshot.is_some() || options.script.is_some()) && !cfg!(feature = "screenshot") {
        eprintln!(
            "error: --screenshot and --script are dev-only flags: rebuild with --features screenshot"
        );
        return ExitCode::from(2);
    }
    #[cfg(feature = "screenshot")]
    let screenshot_request = match screenshot::ScreenshotRequest::from_options(&options) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(2);
        }
    };
    match run(
        options,
        #[cfg(feature = "screenshot")]
        screenshot_request,
    ) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Under `hotpath-profiling` (spec 0055) this is the profiler's scope: the report is written when
/// `run` returns, or after `HOTPATH_SHUTDOWN_MS`. Under `hotpath-profiling-alloc` the attribute
/// also declares the global allocator: hotpath's counting allocator around the live-heap tracker.
#[cfg_attr(
    feature = "hotpath-profiling",
    hotpath::main(allocator = live_heap::LiveHeapAllocator, percentiles = [50, 95, 99], limit = 60)
)]
fn run(
    options: LaunchOptions,
    #[cfg(feature = "screenshot")] screenshot_request: Option<screenshot::ScreenshotRequest>,
) -> anyhow::Result<ExitCode> {
    #[cfg(feature = "hotpath-profiling-alloc")]
    live_heap::report_every(Duration::from_secs(5));
    // The runtime lives on this stack frame until the UI loop ends; GPUI only gets a handle.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(RUNTIME_WORKER_THREADS)
        .thread_name("k8sboard-kube")
        .enable_all()
        .build()?;
    let handle = runtime.handle().clone();
    // A small local file, read before the first frame so the saved theme applies at once.
    let loaded_settings = load_launch_settings(&options);

    #[cfg(feature = "screenshot")]
    let outcome = std::rc::Rc::new(std::cell::Cell::new(screenshot::ScreenshotOutcome::Failed));
    #[cfg(feature = "screenshot")]
    let hook_outcome = std::rc::Rc::clone(&outcome);

    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            bundled_fonts::init_with_bundled_fonts(cx);
            AppSettings::install(loaded_settings, cx);
            let saved = AppSettings::get(cx);
            let theme = options.theme.unwrap_or(saved.theme);
            let colors = options.color_theme.unwrap_or(saved.appearance.color_theme);
            theme.apply(colors, cx);
            cx.set_global(ClusterRuntime::new(handle));
            let chain = kubeconfig_chain(
                options.kubeconfig.clone(),
                std::env::var_os("KUBECONFIG"),
                std::env::home_dir(),
            );
            CatalogHandle::install(chain, cx);
            keymap::bind_keys(cx);
            cluster_switcher::bind_keys(cx);
            cx.on_action(|_: &OpenSettings, cx| {
                open_settings_window(SettingsPage::Clusters, SettingsSize::Standard, cx);
            });
            cx.on_action(|_: &ManageClusters, cx| manage_clusters(cx));
            let settings_screen = options.screen.settings_screen();
            let opens_cluster_metrics = options.screen.opens_cluster_metrics();
            #[cfg(feature = "screenshot")]
            if options.screen.opens_cluster_metrics() {
                cx.set_global(cluster_metrics_section::ClusterMetricsFixture);
            }

            let window_width = options.window_width.map_or(WINDOW_WIDTH, f32::from);
            let window_options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(window_width), px(WINDOW_HEIGHT)),
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
            // A close waits while a node shell pod still has to be deleted (spec 0037).
            // Weak: the callback lives as long as the window, and a strong handle would outlive the app.
            let closing = shell.downgrade();
            let hooked = window.update(cx, |_, window, cx| {
                window.on_window_should_close(cx, move |_, cx| {
                    closing
                        .update(cx, |shell, cx| shell.main_window_may_close(cx))
                        .unwrap_or(true)
                });
            });
            if hooked.is_err() {
                tracing::warn!("could not hook the close of the main window");
            }
            settings_window::quit_when_main_window_closes(window.window_id(), |cx| cx.quit(), cx);
            // A settings screen shows the Settings window next to the main one; a screenshot
            // captures the Settings window.
            let window = match settings_screen {
                Some((page, size)) => open_settings_window(page, size, cx).unwrap_or(window),
                None => window,
            };
            if opens_cluster_metrics {
                settings_window::show_cluster_metrics(cx);
            }
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

fn load_launch_settings(options: &LaunchOptions) -> LoadedSettings {
    let dir = config_dir(
        options.config_dir.clone(),
        std::env::var_os(CONFIG_DIR_ENV),
        default_config_dir(),
    );
    let Some(dir) = dir else {
        return LoadedSettings::without_config_dir();
    };
    let loaded = load_settings(&dir);
    if options.screenshot.is_some() {
        return loaded.for_screenshot(cfg!(feature = "lab-writes"));
    }
    loaded
}
