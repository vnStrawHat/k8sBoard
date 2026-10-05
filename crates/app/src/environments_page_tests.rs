//! The Environments page in a headless window. Settings live in memory only
//! (`WriteMode::Disabled`): nothing here touches a real settings file.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dialog::Confirm;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, TestAppContext, WindowOptions};

use super::*;
use crate::cluster_registry::{ClusterEntry, ClusterRef, ClusterRegistry};
use crate::environment::EnvironmentKey;
use crate::settings::Settings;
use crate::settings_store::{LoadedSettings, WriteMode};

fn custom(name: &str, tier: EnvironmentTier) -> CustomEnvironment {
    CustomEnvironment {
        name: name.to_owned(),
        color: EnvironmentColor::Teal,
        tier,
    }
}

fn entry_on(name: &str) -> ClusterEntry {
    ClusterEntry {
        cluster: ClusterRef {
            kubeconfig: "a.yaml".into(),
            context: "ctx".to_owned(),
        },
        display_name: None,
        environment: Some(EnvironmentKey::Custom(name.to_owned())),
        read_only: None,
        confirm: None,
        default_namespace: None,
        allow_node_shell: None,
        debug_image: None,
        node_shell_namespace: None,
        proxy: None,
        metrics: None,
    }
}

fn open(
    registry: ClusterRegistry,
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<EnvironmentsPage>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        AppSettings::install(
            LoadedSettings {
                settings: Settings {
                    registry,
                    ..Settings::default()
                },
                writes: WriteMode::Disabled,
                notice: None,
            },
            cx,
        );
    });
    let opened = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| EnvironmentsPage::new(window, cx))
        })
        .expect("open the test window")
    });
    render(opened.0, cx);
    opened
}

fn render(window: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .expect("the window is open");
    cx.run_until_parked();
}

fn stored(cx: &TestAppContext) -> ClusterRegistry {
    cx.read(|cx| AppSettings::get(cx).registry.clone())
}

fn type_into(
    window: AnyWindowHandle,
    input: &Entity<InputState>,
    text: &str,
    cx: &mut TestAppContext,
) {
    cx.update_window(window, |_, window, cx| {
        input.update(cx, |state, cx| state.focus(window, cx));
        window.input(text, cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
}

fn row_input(
    page: &Entity<EnvironmentsPage>,
    at: usize,
    cx: &TestAppContext,
) -> Entity<InputState> {
    page.read_with(cx, |page, _| page.rows[at].input.clone())
}

#[gpui_kit::test]
fn add_saves_and_clears_the_input(cx: &mut TestAppContext) {
    let (window, page) = open(ClusterRegistry::default(), cx);
    let new_name = page.read_with(cx, |page, _| page.new_name.clone());
    type_into(window, &new_name, "QA", cx);
    cx.update_window(window, |_, window, cx| {
        page.update(cx, |page, cx| page.add(window, cx));
    })
    .expect("the window is open");
    render(window, cx);
    let saved = stored(cx).environments;
    assert_eq!(
        saved,
        [CustomEnvironment {
            name: "QA".to_owned(),
            color: EnvironmentColor::Purple,
            tier: EnvironmentTier::Production,
        }]
    );
    assert_eq!(
        new_name.read_with(cx, |state, _| state.value().to_string()),
        ""
    );
    assert!(page.read_with(cx, |page, _| page.new_error.is_none()));
    assert_eq!(page.read_with(cx, |page, _| page.rows.len()), 1);
}

#[gpui_kit::test]
fn invalid_rename_keeps_the_stored_name(cx: &mut TestAppContext) {
    let (window, page) = open(
        ClusterRegistry {
            environments: vec![custom("PRO", EnvironmentTier::Staging)],
            ..ClusterRegistry::default()
        },
        cx,
    );
    type_into(window, &row_input(&page, 0, cx), "D", cx);
    let message = page.read_with(cx, |page, _| page.rows[0].error.clone());
    assert_eq!(
        message.map(|error| error.0.to_string()).as_deref(),
        Some("'PROD' is used by a built-in environment.")
    );
    assert_eq!(stored(cx).environments[0].name, "PRO");
}

#[gpui_kit::test]
fn valid_rename_follows_in_entries(cx: &mut TestAppContext) {
    let (window, page) = open(
        ClusterRegistry {
            environments: vec![custom("QA", EnvironmentTier::Staging)],
            clusters: vec![entry_on("QA")],
            ..ClusterRegistry::default()
        },
        cx,
    );
    type_into(window, &row_input(&page, 0, cx), "2", cx);
    let saved = stored(cx);
    assert_eq!(saved.environments[0].name, "QA2");
    assert_eq!(
        saved.clusters[0].environment,
        Some(EnvironmentKey::Custom("QA2".to_owned()))
    );
    assert!(page.read_with(cx, |page, _| page.rows[0].error.is_none()));
}

#[gpui_kit::test]
fn swatch_and_stronger_tier_save(cx: &mut TestAppContext) {
    let (window, page) = open(
        ClusterRegistry {
            environments: vec![custom("QA", EnvironmentTier::Staging)],
            clusters: vec![entry_on("QA")],
            ..ClusterRegistry::default()
        },
        cx,
    );
    cx.update(|cx| set_color(0, EnvironmentColor::Green, cx));
    cx.update_window(window, |_, window, cx| {
        page.update(cx, |page, cx| {
            page.pick_tier(0, EnvironmentTier::Production, window, cx);
        });
    })
    .expect("the window is open");
    cx.run_until_parked();
    let saved = stored(cx).environments;
    assert_eq!(saved[0].color, EnvironmentColor::Green);
    assert_eq!(saved[0].tier, EnvironmentTier::Production);
    let has_dialog = cx
        .update_window(window, |_, window, cx| window.has_active_dialog(cx))
        .expect("the window is open");
    assert!(!has_dialog);
}

#[gpui_kit::test]
fn weaker_tier_on_used_environment_asks(cx: &mut TestAppContext) {
    let (window, page) = open(
        ClusterRegistry {
            environments: vec![custom("DR", EnvironmentTier::Production)],
            clusters: vec![entry_on("DR")],
            ..ClusterRegistry::default()
        },
        cx,
    );
    cx.update_window(window, |_, window, cx| {
        page.update(cx, |page, cx| {
            page.pick_tier(0, EnvironmentTier::Staging, window, cx);
        });
    })
    .expect("the window is open");
    cx.run_until_parked();
    let has_dialog = cx
        .update_window(window, |_, window, cx| window.has_active_dialog(cx))
        .expect("the window is open");
    assert!(has_dialog);
    assert_eq!(stored(cx).environments[0].tier, EnvironmentTier::Production);
    cx.update_window(window, |_, window, cx| {
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
    assert_eq!(stored(cx).environments[0].tier, EnvironmentTier::Staging);
}

#[gpui_kit::test]
fn weaker_tier_on_unused_environment_saves(cx: &mut TestAppContext) {
    let (window, page) = open(
        ClusterRegistry {
            environments: vec![custom("DR", EnvironmentTier::Production)],
            ..ClusterRegistry::default()
        },
        cx,
    );
    cx.update_window(window, |_, window, cx| {
        page.update(cx, |page, cx| {
            page.pick_tier(0, EnvironmentTier::Local, window, cx);
        });
    })
    .expect("the window is open");
    cx.run_until_parked();
    assert_eq!(stored(cx).environments[0].tier, EnvironmentTier::Local);
}

#[gpui_kit::test]
fn delete_rebuilds_the_rows_and_moves_clusters(cx: &mut TestAppContext) {
    let (window, page) = open(
        ClusterRegistry {
            environments: vec![custom("QA", EnvironmentTier::Staging)],
            clusters: vec![entry_on("QA")],
            ..ClusterRegistry::default()
        },
        cx,
    );
    cx.update_window(window, |_, window, cx| {
        page.update(cx, |page, cx| page.confirm_delete(0, window, cx));
    })
    .expect("the window is open");
    cx.run_until_parked();
    assert_eq!(stored(cx).environments.len(), 1, "waits for the confirm");
    cx.update_window(window, |_, window, cx| {
        window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
    let saved = stored(cx);
    assert!(saved.environments.is_empty());
    assert_eq!(
        saved.clusters[0].environment,
        Some(EnvironmentKey::BuiltIn(EnvironmentTier::Staging))
    );
    assert!(page.read_with(cx, |page, _| page.rows.is_empty()));
}

#[gpui_kit::test]
fn skipped_row_shows_its_error(cx: &mut TestAppContext) {
    let seeded = ClusterRegistry {
        environments: vec![custom("Prod", EnvironmentTier::Local)],
        ..ClusterRegistry::default()
    };
    let (_window, page) = open(seeded.clone(), cx);
    let message = page.read_with(cx, |page, _| page.row_error(0, &seeded.environments));
    assert_eq!(
        message.map(|error| error.0.to_string()).as_deref(),
        Some("'Prod' is used by a built-in environment.")
    );
}
