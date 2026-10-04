use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gpui_kit::TestAppContext;
use gpui_kit::component::Theme;
use serde_json::{Value, json};

use super::*;
use crate::cluster_registry::{ClusterEntry, ClusterRef};
use crate::environment::Environment;
use crate::port_forwards::{ForwardPreset, ForwardSpec, LocalPortSpec, TargetSpec};
use crate::settings_store::settings_path;
use crate::table_sort::SortDirection;
use crate::write_guard::ConfirmMode;

#[test]
fn default_settings_have_the_current_version() {
    assert_eq!(Settings::default().version, SETTINGS_VERSION);
}

#[test]
fn default_settings_round_trip() {
    let bytes = serialize_settings(&Settings::default()).expect("serializes");
    let back: Settings = serde_json::from_slice(&bytes).expect("parses");
    assert_eq!(back, Settings::default());
}

/// Every key set, so the serialized value shows the whole allow-list.
fn full_settings() -> Settings {
    let cluster = ClusterRef {
        kubeconfig: PathBuf::from("a.yaml"),
        context: "ctx".to_owned(),
    };
    Settings {
        theme: ThemePreference::Dark,
        registry: ClusterRegistry {
            kubeconfigs: vec![PathBuf::from("extra.yaml")],
            clusters: vec![ClusterEntry {
                cluster: cluster.clone(),
                display_name: Some("name".to_owned()),
                environment: Some(Environment::Production),
                read_only: Some(true),
                confirm: Some(ConfirmMode::TypeName),
                default_namespace: Some("ns".to_owned()),
                allow_node_shell: Some(true),
                debug_image: Some("registry.local/busybox:1".to_owned()),
                node_shell_namespace: Some("debug".to_owned()),
            }],
            last_used: Some(cluster),
        },
        port_forward: PortForwardSettings {
            presets: vec![ForwardPreset {
                cluster: ClusterRef {
                    kubeconfig: PathBuf::from("a.yaml"),
                    context: "ctx".to_owned(),
                },
                spec: ForwardSpec {
                    namespace: "shop".to_owned(),
                    target: TargetSpec::pod("api-0"),
                    remote_port: 8080,
                    local_port: LocalPortSpec::Exact(18080),
                },
            }],
        },
        dock: DockSettings { height: Some(402.) },
        tables: BTreeMap::from([(
            "pods".to_owned(),
            TablePrefs {
                sort: Some(SavedSort {
                    column: "Age".to_owned(),
                    direction: SortDirection::Ascending,
                }),
                hidden: vec!["Node".to_owned()],
            },
        )]),
        ..Settings::default()
    }
}

#[test]
fn dock_height_round_trips_and_is_omitted_when_unset() {
    let settings = Settings {
        dock: DockSettings { height: Some(402.) },
        ..Settings::default()
    };
    let bytes = serialize_settings(&settings).expect("serializes");
    let back: Settings = serde_json::from_slice(&bytes).expect("parses");
    assert_eq!(back.dock.height, Some(402.));
    let value = serde_json::to_value(Settings::default()).expect("serializes");
    assert!(value.get("dock").is_none(), "{value}");
}

#[test]
fn full_settings_round_trip() {
    let settings = full_settings();
    let bytes = serialize_settings(&settings).expect("serializes");
    let back: Settings = serde_json::from_slice(&bytes).expect("parses");
    assert_eq!(back, settings);
}

#[test]
fn unknown_fields_are_ignored() {
    let value = json!({ "version": 1, "theme": "light", "later": { "a": [1, 2] } });
    let settings: Settings = serde_json::from_value(value).expect("parses");
    assert_eq!(settings.theme, ThemePreference::Light);
}

#[test]
fn missing_fields_take_defaults() {
    let settings: Settings = serde_json::from_value(json!({ "version": 1 })).expect("parses");
    assert_eq!(settings, Settings::default());
}

#[test]
fn theme_serializes_lowercase() {
    for (theme, text) in [
        (ThemePreference::System, "system"),
        (ThemePreference::Light, "light"),
        (ThemePreference::Dark, "dark"),
    ] {
        assert_eq!(
            serde_json::to_value(theme).expect("serializes"),
            json!(text)
        );
    }
}

#[test]
fn default_file_is_minimal() {
    let value = serde_json::to_value(Settings::default()).expect("serializes");
    assert_eq!(
        value,
        json!({ "version": 1, "theme": "system", "registry": {} })
    );
}

/// The keys a fully populated value writes; AC 6. Later steps and specs extend the list.
#[test]
fn settings_keys_are_the_allow_list() {
    let value = serde_json::to_value(full_settings()).expect("serializes");
    let mut keys = Vec::new();
    collect_keys("", &value, &mut keys);
    keys.sort();
    keys.dedup();
    assert_eq!(
        keys,
        [
            "dock",
            "dock.height",
            "port_forward",
            "port_forward.presets",
            "port_forward.presets.cluster",
            "port_forward.presets.cluster.context",
            "port_forward.presets.cluster.kubeconfig",
            "port_forward.presets.spec",
            "port_forward.presets.spec.local_port",
            "port_forward.presets.spec.local_port.exact",
            "port_forward.presets.spec.namespace",
            "port_forward.presets.spec.remote_port",
            "port_forward.presets.spec.target",
            "port_forward.presets.spec.target.kind",
            "port_forward.presets.spec.target.name",
            "registry",
            "registry.clusters",
            "registry.clusters.allow_node_shell",
            "registry.clusters.confirm",
            "registry.clusters.context",
            "registry.clusters.debug_image",
            "registry.clusters.default_namespace",
            "registry.clusters.display_name",
            "registry.clusters.environment",
            "registry.clusters.kubeconfig",
            "registry.clusters.node_shell_namespace",
            "registry.clusters.read_only",
            "registry.kubeconfigs",
            "registry.last_used",
            "registry.last_used.context",
            "registry.last_used.kubeconfig",
            "tables",
            "tables.pods",
            "tables.pods.hidden",
            "tables.pods.sort",
            "tables.pods.sort.column",
            "tables.pods.sort.direction",
            "theme",
            "version",
        ]
    );
}

fn collect_keys(prefix: &str, value: &Value, keys: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                collect_keys(&path, child, keys);
                keys.push(path);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_keys(prefix, item, keys);
            }
        }
        _ => {}
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("k8sboard-0024-app-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn install(writes: WriteMode, cx: &mut TestAppContext) {
    cx.update(|cx| {
        AppSettings::install(
            LoadedSettings {
                settings: Settings::default(),
                writes,
                notice: None,
            },
            cx,
        );
    });
}

fn set_theme(theme: ThemePreference, cx: &mut TestAppContext) {
    cx.update(|cx| AppSettings::update(cx, |settings| settings.theme = theme));
}

fn read_theme(dir: &Path) -> Option<Value> {
    let bytes = std::fs::read(settings_path(dir)).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value.get("theme").cloned()
}

#[gpui_kit::test]
fn update_writes_the_newest_snapshot(cx: &mut TestAppContext) {
    let dir = temp_dir("newest");
    install(WriteMode::Enabled(dir.clone()), cx);
    set_theme(ThemePreference::Light, cx);
    set_theme(ThemePreference::Dark, cx);
    cx.run_until_parked();
    assert_eq!(read_theme(&dir), Some(json!("dark")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn unchanged_update_does_not_write(cx: &mut TestAppContext) {
    let dir = temp_dir("unchanged");
    install(WriteMode::Enabled(dir.clone()), cx);
    set_theme(ThemePreference::System, cx);
    cx.run_until_parked();
    assert!(!settings_path(&dir).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn disabled_writes_never_touch_disk(cx: &mut TestAppContext) {
    install(WriteMode::Disabled, cx);
    set_theme(ThemePreference::Dark, cx);
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(AppSettings::get(cx).theme, ThemePreference::Dark);
        AppSettings::flush(cx);
    });
}

#[gpui_kit::test]
fn write_failure_sets_notice(cx: &mut TestAppContext) {
    let dir = temp_dir("failure");
    let _ = std::fs::remove_file(&dir);
    // The config dir path is an existing file, so the folder cannot be created.
    std::fs::write(&dir, "not a folder").expect("file in place of the folder");
    install(WriteMode::Enabled(dir.clone()), cx);
    set_theme(ThemePreference::Dark, cx);
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(
            matches!(
                AppSettings::notice(cx),
                Some(SettingsNotice::WriteFailed { .. })
            ),
            "{:?}",
            AppSettings::notice(cx)
        );
    });
    let _ = std::fs::remove_file(&dir);
}

#[gpui_kit::test]
fn flush_writes_the_last_snapshot(cx: &mut TestAppContext) {
    let dir = temp_dir("flush");
    install(WriteMode::Enabled(dir.clone()), cx);
    set_theme(ThemePreference::Dark, cx);
    // No parking: the background write has not run, as at a quit right after a change.
    cx.update(AppSettings::flush);
    assert_eq!(read_theme(&dir), Some(json!("dark")));
    cx.run_until_parked();
    assert_eq!(read_theme(&dir), Some(json!("dark")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn dismiss_clears_the_notice(cx: &mut TestAppContext) {
    cx.update(|cx| {
        AppSettings::install(
            LoadedSettings {
                settings: Settings::default(),
                writes: WriteMode::Disabled,
                notice: Some(SettingsNotice::NoConfigDir),
            },
            cx,
        );
        assert!(AppSettings::notice(cx).is_some());
        AppSettings::dismiss_notice(cx);
        assert!(AppSettings::notice(cx).is_none());
    });
}

#[test]
fn screen_key_per_screen() {
    use crate::app_shell::Screen;
    use crate::resource_kind::ResourceKind;
    assert_eq!(screen_key(Screen::Pods), "pods");
    assert_eq!(screen_key(Screen::Nodes), "nodes");
    assert_eq!(screen_key(Screen::Issues), "issues");
    assert_eq!(
        screen_key(Screen::Kind(ResourceKind::Deployments)),
        "deployments"
    );
}

#[test]
fn table_prefs_serialize_by_name() {
    let prefs = TablePrefs {
        sort: Some(SavedSort {
            column: "Restarts".to_owned(),
            direction: SortDirection::Descending,
        }),
        hidden: vec!["Node".to_owned()],
    };
    assert_eq!(
        serde_json::to_value(prefs).expect("serializes"),
        json!({ "sort": { "column": "Restarts", "direction": "descending" }, "hidden": ["Node"] })
    );
}

#[test]
fn empty_table_prefs_serialize_to_an_empty_object() {
    let empty = TablePrefs {
        sort: None,
        hidden: Vec::new(),
    };
    assert_eq!(serde_json::to_value(empty).expect("serializes"), json!({}));
}

#[test]
fn custom_kind_key_is_the_crd_name_never_a_builtin_key() {
    use crate::custom_kind::{CustomKindCache, custom_kinds};
    let crd = cluster::CrdSummary {
        name: "nodes.longhorn.io".to_owned(),
        group: "longhorn.io".to_owned(),
        kind: "Node".to_owned(),
        plural: "nodes".to_owned(),
        singular: "node".to_owned(),
        scope: cluster::ResourceScope::Namespaced,
        versions: vec![cluster::CrdVersion {
            name: "v1".to_owned(),
            is_served: true,
            is_storage: true,
            is_deprecated: false,
            deprecation_warning: None,
            printer_columns: Vec::new(),
            schema: cluster::SchemaOutline::default(),
        }],
        state: cluster::CrdState::Established,
        created_at: None,
    };
    let kind = custom_kinds(&[crd], &mut CustomKindCache::default())
        .pop()
        .expect("an Established CRD becomes a kind");
    let key = screen_key(Screen::Kind(ResourceKind::Custom(kind)));
    assert_eq!(key, "nodes.longhorn.io");
    assert_ne!(key, screen_key(Screen::Nodes));
}

#[test]
fn theme_label_round_trip() {
    for (theme, _) in THEME_OPTIONS {
        assert_eq!(theme_from_label(theme_label(theme)), theme);
    }
    let labels: Vec<&str> = THEME_OPTIONS.iter().map(|(_, label)| *label).collect();
    assert_eq!(labels, ["Follow the system", "Light", "Dark"]);
}

#[test]
fn unknown_theme_label_is_system() {
    assert_eq!(theme_from_label("Sepia"), ThemePreference::System);
    assert_eq!(theme_from_label(""), ThemePreference::System);
}

#[test]
fn theme_choices_use_the_label_as_key() {
    for (key, label) in theme_choices() {
        assert_eq!(key, label);
    }
}

#[gpui_kit::test]
fn applying_a_theme_preference_sets_the_theme_mode(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        ThemePreference::Dark.apply(cx);
        assert!(Theme::global(cx).is_dark());
        ThemePreference::Light.apply(cx);
        assert!(!Theme::global(cx).is_dark());
    });
}
