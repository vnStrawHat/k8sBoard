//! The Settings window in headless windows. Every run uses a config dir under the temp dir and
//! the test platform's clipboard: nothing here touches the real settings or clipboard.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::{Theme, WindowExt as _};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext, WeakEntity, WindowOptions};

use super::*;
use crate::cluster_registry::{ClusterRef, ClusterRegistry};
use crate::clusters_page::set_confirm;
use crate::color_theme::ColorTheme;
use crate::settings::{Settings, ThemePreference};
use crate::settings_store::{LoadedSettings, WriteMode};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("k8sboard-0025-win-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Installs the globals both windows share. `config_dir` `None` keeps writes off.
fn install(config_dir: Option<&Path>, chain: &[PathBuf], cx: &mut TestAppContext) {
    let chain = chain.to_vec();
    let writes = config_dir.map_or(WriteMode::Disabled, |dir| {
        WriteMode::Enabled(dir.to_path_buf())
    });
    cx.update(|cx| {
        gpui_kit::init(cx);
        // The dialog entrance animation would need frames to settle.
        cx.set_reduce_motion(true);
        crate::keymap::bind_keys(cx);
        AppSettings::install(
            LoadedSettings {
                settings: Settings {
                    registry: ClusterRegistry::default(),
                    ..Settings::default()
                },
                writes,
                notice: None,
            },
            cx,
        );
        CatalogHandle::install(chain, cx);
    });
    cx.run_until_parked();
}

fn open_window(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<SettingsWindow>) {
    open_window_with(WindowOptions::default(), cx)
}

fn open_window_sized(
    size: gpui_kit::Size<gpui_kit::Pixels>,
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<SettingsWindow>) {
    let bounds = gpui_kit::Bounds {
        origin: gpui_kit::Point::default(),
        size,
    };
    let options = WindowOptions {
        window_bounds: Some(gpui_kit::WindowBounds::Windowed(bounds)),
        ..Default::default()
    };
    open_window_with(options, cx)
}

fn open_window_with(
    options: WindowOptions,
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<SettingsWindow>) {
    cx.update(|cx| {
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| SettingsWindow::new(SettingsPage::Clusters, window, cx))
        })
        .expect("open the test window")
    })
}

fn render(window: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .expect("the window is open");
    cx.run_until_parked();
}

fn window_count(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| cx.windows().len())
}

fn close(window: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, _| window.remove_window())
        .expect("the window is open");
    cx.run_until_parked();
}

fn handle_of_settings(cx: &mut TestAppContext) -> Option<AnyWindowHandle> {
    cx.update(|cx| {
        cx.try_global::<SettingsWindowHandle>()
            .and_then(|handle| handle.0.as_ref().map(|open| open.window))
    })
}

/// A main-like window, so the close hook has a window that is not the Settings one.
fn open_main_like(cx: &mut TestAppContext) -> AnyWindowHandle {
    cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|_| gpui_kit::Empty)
        })
        .expect("open the test window")
        .0
    })
}

fn quit_flag(main: AnyWindowHandle, cx: &mut TestAppContext) -> Rc<Cell<bool>> {
    let flag = Rc::new(Cell::new(false));
    let hook_flag = Rc::clone(&flag);
    cx.update(|cx| {
        quit_when_main_window_closes(main.window_id(), move |_| hook_flag.set(true), cx);
    });
    flag
}

fn open_settings(cx: &mut TestAppContext) -> AnyWindowHandle {
    cx.update(|cx| open_settings_window(SettingsPage::Clusters, SettingsSize::Standard, cx))
        .expect("the Settings window opens")
}

// ---- Single instance and close rules ----

#[gpui_kit::test]
fn open_twice_keeps_one_window(cx: &mut TestAppContext) {
    install(None, &[], cx);
    let first = open_settings(cx);
    let second = open_settings(cx);
    assert_eq!(first.window_id(), second.window_id());
    assert_eq!(window_count(cx), 1);
}

#[gpui_kit::test]
fn open_after_close_opens_a_new_window(cx: &mut TestAppContext) {
    install(None, &[], cx);
    let main = open_main_like(cx);
    let _ = quit_flag(main, cx);
    let first = open_settings(cx);
    close(first, cx);
    let second = open_settings(cx);
    assert_ne!(first.window_id(), second.window_id());
    assert_eq!(window_count(cx), 2);
}

#[gpui_kit::test]
fn closing_settings_keeps_the_app(cx: &mut TestAppContext) {
    install(None, &[], cx);
    let main = open_main_like(cx);
    let quit = quit_flag(main, cx);
    let settings = open_settings(cx);
    close(settings, cx);
    assert!(!quit.get());
    assert_eq!(window_count(cx), 1);
}

#[gpui_kit::test]
fn closing_settings_clears_the_handle(cx: &mut TestAppContext) {
    install(None, &[], cx);
    let main = open_main_like(cx);
    let _ = quit_flag(main, cx);
    let settings = open_settings(cx);
    assert_eq!(handle_of_settings(cx), Some(settings));
    close(settings, cx);
    assert_eq!(handle_of_settings(cx), None);
}

#[gpui_kit::test]
fn closing_main_window_quits(cx: &mut TestAppContext) {
    install(None, &[], cx);
    let main = open_main_like(cx);
    let quit = quit_flag(main, cx);
    let _settings = open_settings(cx);
    close(main, cx);
    // Settings is still open, so only the main window's own rule can have fired.
    assert!(quit.get());
}

#[test]
fn pages_follow_w2_order() {
    let titles: Vec<&str> = PAGES.iter().map(|page| page.title()).collect();
    assert_eq!(
        titles,
        [
            "General",
            "Clusters",
            "Appearance",
            "Keyboard Shortcuts",
            "Safety",
            "Terminal & Shell",
            "Logs",
            "Metrics",
            "About"
        ]
    );
}

#[test]
fn default_page_is_clusters() {
    assert_eq!(PAGES[1], SettingsPage::Clusters);
    assert_eq!(SettingsPage::Clusters.index(), 1);
    assert_eq!(SettingsPage::About.index(), 8);
}

#[gpui_kit::test]
fn dialog_opens_in_settings_window(cx: &mut TestAppContext) {
    let dir = temp_dir("dialog");
    install(Some(&dir), &[], cx);
    let (window, view) = open_window(cx);
    render(window, cx);
    render(window, cx);
    let probe = cx
        .update_window(window, |_, window, _| {
            window
                .try_find("environment")
                .map(|button| (button.bounds(), button.visible()))
        })
        .expect("the window is open");
    eprintln!("PROBE {probe:?}");
    render(window, cx);
    let page = view.read_with(cx, |view, _| view.clusters.clone());
    cx.update_window(window, |_, window, cx| {
        page.update(cx, |page, cx| page.start_paste(window, cx));
    })
    .expect("the window is open");
    render(window, cx);
    let has_dialog = cx
        .update_window(window, |_, window, cx| window.has_active_dialog(cx))
        .expect("the window is open");
    assert!(has_dialog);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn appearance_change_updates_settings_and_theme(cx: &mut TestAppContext) {
    install(None, &[], cx);
    cx.update(|cx| {
        change_theme("Dark", cx);
        assert_eq!(AppSettings::get(cx).theme, ThemePreference::Dark);
        assert!(Theme::global(cx).is_dark());
        change_theme("Light", cx);
        assert_eq!(AppSettings::get(cx).theme, ThemePreference::Light);
        assert!(!Theme::global(cx).is_dark());
    });
}

#[test]
fn color_theme_labels_round_trip() {
    for colors in [ColorTheme::Default, ColorTheme::ZedOne] {
        let label = COLOR_THEME_OPTIONS.label(colors, || unreachable!("every value is listed"));
        assert_eq!(COLOR_THEME_OPTIONS.value(&label), colors);
    }
    assert_eq!(COLOR_THEME_OPTIONS.value("Solarized"), ColorTheme::ZedOne);
}

#[gpui_kit::test]
fn changing_color_theme_saves_and_applies(cx: &mut TestAppContext) {
    install(None, &[], cx);
    cx.update(|cx| {
        change_theme("Dark", cx);
        change_color_theme("Zed One", cx);
        assert_eq!(
            AppSettings::get(cx).appearance.color_theme,
            ColorTheme::ZedOne
        );
        assert_eq!(Theme::global(cx).theme_name(), "One Dark");
        change_theme("Light", cx);
        assert_eq!(Theme::global(cx).theme_name(), "One Light");
        change_color_theme("Default", cx);
        assert_eq!(
            AppSettings::get(cx).appearance.color_theme,
            ColorTheme::Default
        );
        assert_eq!(Theme::global(cx).theme_name(), "Default Light");
    });
}

#[gpui_kit::test]
fn settings_window_is_released_on_close(cx: &mut TestAppContext) {
    install(None, &[], cx);
    let (window, view) = open_window(cx);
    render(window, cx);
    let weak: WeakEntity<SettingsWindow> = view.downgrade();
    drop(view);
    close(window, cx);
    assert!(weak.upgrade().is_none());
}

#[gpui_kit::test]
fn manage_clusters_switches_an_open_window_to_the_clusters_page(cx: &mut TestAppContext) {
    install(None, &[], cx);
    open_settings_window_on(SettingsPage::Appearance, cx);
    let state = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let open = open_window_of(cx).expect("the window is open");
            open.view
                .read_with(cx, |view, _| (view.first_page, view.page_generation))
                .expect("the view is alive")
        })
    };
    assert_eq!(state(cx), (SettingsPage::Appearance, 0));
    // `Ctrl ,` only brings the window forward: the page the user is on stays.
    open_settings_window_on(SettingsPage::Clusters, cx);
    assert_eq!(state(cx), (SettingsPage::Appearance, 0));
    cx.update(manage_clusters);
    assert_eq!(state(cx), (SettingsPage::Clusters, 1));
    assert_eq!(window_count(cx), 1);
}

#[gpui_kit::test]
fn manage_clusters_opens_the_window_when_closed(cx: &mut TestAppContext) {
    install(None, &[], cx);
    cx.update(manage_clusters);
    assert_eq!(window_count(cx), 1);
    assert!(handle_of_settings(cx).is_some());
}

#[gpui_kit::test]
fn the_clusters_footer_scrolls_into_view_in_a_short_window(cx: &mut TestAppContext) {
    let dir = temp_dir("scroll");
    let file = dir.join("kubeconfig.yaml");
    std::fs::write(
        &file,
        "apiVersion: v1\nkind: Config\nclusters:\n  - name: c\n    cluster: { server: 'https://127.0.0.1:1' }\ncontexts:\n  - name: ctx\n    context: { cluster: c }\n",
    )
    .expect("write fixture");
    install(None, std::slice::from_ref(&file), cx);
    let (window, _view) =
        open_window_sized(gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(420.)), cx);
    render(window, cx);
    let footer_bottom = |cx: &mut TestAppContext| {
        cx.update_window(window, |_, window, _| {
            window
                .try_find("remove-cluster")
                .map(|button| button.bounds().bottom())
        })
        .expect("the window is open")
    };
    let viewport = cx
        .update_window(window, |_, window, _| window.viewport_size().height)
        .expect("the window is open");
    let before = footer_bottom(cx).expect("the footer is laid out");
    assert!(
        before > viewport,
        "the footer starts below the fold: {before:?}"
    );
    // One wheel turn on a control that is in view; the list clamps it at the end.
    cx.update_window(window, |_, window, cx| {
        window.scroll(
            "environment",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-2000.))),
            cx,
        );
    })
    .expect("the window is open");
    render(window, cx);
    let after = footer_bottom(cx).expect("the footer is laid out");
    assert!(
        after <= viewport,
        "the footer scrolled into view: {after:?} in {viewport:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn open_settings_window_on(page: SettingsPage, cx: &mut TestAppContext) {
    cx.update(|cx| open_settings_window(page, SettingsSize::Standard, cx))
        .expect("the Settings window opens");
}

// ---- Safety page and the confirm select ----

fn target_cluster() -> ClusterRef {
    ClusterRef {
        kubeconfig: PathBuf::from("a.yaml"),
        context: "ctx".to_owned(),
    }
}

fn stored_confirm(cx: &mut TestAppContext) -> Option<ConfirmMode> {
    cx.update(|cx| {
        AppSettings::get(cx)
            .registry
            .clusters
            .iter()
            .find(|entry| entry.cluster == target_cluster())
            .and_then(|entry| entry.confirm)
    })
}

#[gpui_kit::test]
fn confirm_select_stores_the_mode(cx: &mut TestAppContext) {
    install(None, &[], cx);
    cx.update(|cx| set_confirm(&target_cluster(), Some(ConfirmMode::TypeName), cx));
    assert_eq!(stored_confirm(cx), Some(ConfirmMode::TypeName));
    cx.update(|cx| set_confirm(&target_cluster(), Some(ConfirmMode::Click), cx));
    assert_eq!(stored_confirm(cx), Some(ConfirmMode::Click));
}

#[gpui_kit::test]
fn confirm_auto_clears_the_value(cx: &mut TestAppContext) {
    install(None, &[], cx);
    cx.update(|cx| set_confirm(&target_cluster(), Some(ConfirmMode::TypeName), cx));
    cx.update(|cx| set_confirm(&target_cluster(), None, cx));
    assert_eq!(stored_confirm(cx), None);
    // An entry left with no override is dropped again.
    let entries = cx.update(|cx| AppSettings::get(cx).registry.clusters.len());
    assert_eq!(entries, 0);
}

#[test]
fn the_tier_table_groups_environments_by_tier() {
    let rows = tier_rows();
    assert_eq!(
        rows,
        [
            TierRow {
                environments: "Production".to_owned(),
                change: "Type the cluster name".to_owned(),
                destructive: "Type the cluster name, danger button".to_owned(),
                privileged: "Type the node name, danger button".to_owned(),
            },
            TierRow {
                environments: "Staging, Development, Local".to_owned(),
                change: "Click Confirm".to_owned(),
                destructive: "Click Confirm, danger button".to_owned(),
                privileged: "Type the node name, danger button".to_owned(),
            },
        ]
    );
}

#[gpui_kit::test]
fn the_safety_page_renders(cx: &mut TestAppContext) {
    install(None, &[], cx);
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| SettingsWindow::new(SettingsPage::Safety, window, cx))
        })
        .expect("open the test window")
    });
    render(window, cx);
    render(window, cx);
    assert_eq!(
        view.read_with(cx, |view, _| view.first_page),
        SettingsPage::Safety
    );
}

#[test]
fn the_about_page_carries_the_terminal_engine_notice() {
    // The Apache License asks a redistributor to keep the notice of what it links.
    assert!(ONETERM_NOTICE.contains("oneterm-vt"));
    assert!(ONETERM_NOTICE.contains("Apache-2.0"));
    assert!(ONETERM_NOTICE.contains("The OneTerm authors"));
}

#[gpui_kit::test]
fn every_page_renders_with_non_default_values(cx: &mut TestAppContext) {
    install(None, &[], cx);
    cx.update(|cx| {
        AppSettings::update(cx, |settings| {
            settings.general.export_dir = Some(PathBuf::from("exports"));
            settings.general.watch_tls_secrets = false;
            settings.logs.tail_lines = 50;
            settings.terminal.scrollback_lines = 1_234;
            settings.terminal.font_size = Some(16);
        });
    });
    for page in PAGES {
        let window = cx
            .update(|cx| open_settings_window(page, SettingsSize::Standard, cx))
            .expect("the Settings window opens");
        render(window, cx);
        render(window, cx);
        close(window, cx);
    }
}

#[test]
fn settings_page_icons_are_distinct() {
    let mut seen = std::collections::HashSet::new();
    for page in PAGES {
        assert!(
            seen.insert(format!("{:?}", page.icon())),
            "{page:?} repeats an icon"
        );
    }
}

// ---- The Metrics page ----

#[gpui_kit::test]
fn a_window_on_another_page_lists_no_service_until_the_metrics_page_shows(cx: &mut TestAppContext) {
    install(None, &[], cx);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("a tokio runtime");
    cx.executor().allow_parking();
    let (connection, api) = {
        let _guard = runtime.enter();
        cluster::fake_api::FakeApi::connection(cluster::WritePolicy::Blocked, |_| {
            (
                200,
                r#"{"apiVersion":"v1","kind":"ServiceList","metadata":{},"items":[]}"#.to_owned(),
            )
        })
    };
    cx.update(|cx| {
        cx.set_global(crate::cluster_runtime::ClusterRuntime::new(
            runtime.handle().clone(),
        ));
        cx.set_global(crate::active_session::ActiveConnection {
            cluster: target_cluster(),
            label: "ctx".to_owned(),
            connection,
            session: WeakEntity::new_invalid(),
            generation: 1,
        });
    });
    let window = open_settings(cx);
    render(window, cx);
    render(window, cx);
    std::thread::sleep(std::time::Duration::from_millis(100));
    cx.run_until_parked();
    assert!(
        api.requests().is_empty(),
        "opening Settings on Clusters lists nothing: {:?}",
        api.requests()
    );
    cx.update(show_metrics_page);
    render(window, cx);
    for _ in 0..500 {
        cx.run_until_parked();
        if !api.requests().is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(api.requests().len(), 1);
    assert_eq!(api.requests()[0].path, "/api/v1/services");
    close(window, cx);
}

// ---- Add cluster in the Settings window ----

fn press_confirm(window: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, cx| {
        window.click("ok", cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
    render(window, cx);
}

fn has_cluster_form(window: AnyWindowHandle, cx: &mut TestAppContext) -> bool {
    cx.update_window(window, |_, window, _| {
        window.try_find("remove-cluster").is_some()
    })
    .expect("the window is open")
}

const ADDED_KUBECONFIG: &str = "apiVersion: v1\nkind: Config\nclusters:\n  - name: c\n    cluster: { server: 'https://127.0.0.1:1' }\nusers:\n  - name: u\n    user: { token: fixture-token-value }\ncontexts:\n  - name: added-ctx\n    context: { cluster: c, user: u }\n";

#[gpui_kit::test]
fn pasted_kubeconfig_appears_in_the_open_settings_window(cx: &mut TestAppContext) {
    let dir = temp_dir("paste-flow");
    install(Some(&dir), &[], cx);
    let (window, view) = open_window(cx);
    render(window, cx);
    assert!(!has_cluster_form(window, cx));
    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
        ADDED_KUBECONFIG.to_owned(),
    ));
    let page = view.read_with(cx, |view, _| view.clusters.clone());
    cx.update_window(window, |_, window, cx| {
        page.update(cx, |page, cx| page.start_paste(window, cx));
    })
    .expect("the window is open");
    render(window, cx);
    press_confirm(window, cx);
    press_confirm(window, cx);
    render(window, cx);
    assert!(has_cluster_form(window, cx));
    let _ = std::fs::remove_dir_all(&dir);
}
