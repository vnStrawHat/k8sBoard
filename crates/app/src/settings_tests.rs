use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cluster::{MetricsScheme, MetricsSourceFields, ShellCommand};
use gpui_kit::TestAppContext;
use gpui_kit::component::Theme;
use serde_json::{Value, json};

use super::*;
use crate::cluster_registry::{ClusterEntry, ClusterProxy, ClusterRef, StoredMetrics};
use crate::environment::{CustomEnvironment, EnvironmentColor, EnvironmentKey, EnvironmentTier};
use crate::kubeconfig_folder::FileStamp;
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
            environments: vec![CustomEnvironment {
                name: "QA".to_owned(),
                color: EnvironmentColor::Purple,
                tier: EnvironmentTier::Staging,
            }],
            kubeconfigs: vec![PathBuf::from("extra.yaml")],
            kubeconfig_folders: vec![PathBuf::from("watched")],
            clusters: vec![ClusterEntry {
                cluster: cluster.clone(),
                display_name: Some("name".to_owned()),
                environment: Some(EnvironmentKey::BuiltIn(EnvironmentTier::Production)),
                read_only: Some(true),
                confirm: Some(ConfirmMode::TypeName),
                default_namespace: Some("ns".to_owned()),
                allow_node_shell: Some(true),
                debug_image: Some("registry.local/busybox:1".to_owned()),
                node_shell_namespace: Some("debug".to_owned()),
                proxy: Some(ClusterProxy::Url("http://proxy.example:3128".to_owned())),
                metrics: Some(StoredMetrics::Fields(MetricsSourceFields {
                    namespace: "monitoring".to_owned(),
                    service: "vmselect".to_owned(),
                    port: "8481".to_owned(),
                    scheme: MetricsScheme::Http,
                    prefix: "/select/0/prometheus".to_owned(),
                })),
            }],
            last_used: Some(cluster),
            last_used_stamp: Some(FileStamp {
                len: 120,
                modified_ms: Some(1_730_000_000_000),
            }),
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
        logs: LogSettings {
            tail_lines: 500,
            show_timestamps: false,
            wrap_lines: true,
            show_json: true,
        },
        terminal: TerminalSettings {
            default_shell: ShellCommand::Bash,
            scrollback_lines: 10_000,
            font_size: Some(14),
        },
        general: GeneralSettings {
            export_dir: Some(PathBuf::from("exports")),
            watch_tls_secrets: false,
        },
        appearance: AppearanceSettings {
            density: RowDensity::Comfortable,
            color_theme: ColorTheme::ZedOne,
        },
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
fn the_removed_topology_edges_key_is_ignored() {
    let old = r#"{ "version": 1, "topology": { "edges": "curves" } }"#;
    let settings: Settings = serde_json::from_str(old).expect("parses");
    assert_eq!(settings, Settings::default());
    let value = serde_json::to_value(&settings).expect("serializes");
    assert!(value.get("topology").is_none(), "{value}");
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
            "appearance",
            "appearance.color_theme",
            "appearance.density",
            "dock",
            "dock.height",
            "general",
            "general.export_dir",
            "general.watch_tls_secrets",
            "logs",
            "logs.show_json",
            "logs.show_timestamps",
            "logs.tail_lines",
            "logs.wrap_lines",
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
            "registry.clusters.metrics",
            "registry.clusters.metrics.namespace",
            "registry.clusters.metrics.port",
            "registry.clusters.metrics.prefix",
            "registry.clusters.metrics.scheme",
            "registry.clusters.metrics.service",
            "registry.clusters.node_shell_namespace",
            "registry.clusters.proxy",
            "registry.clusters.proxy.url",
            "registry.clusters.read_only",
            "registry.environments",
            "registry.environments.color",
            "registry.environments.name",
            "registry.environments.tier",
            "registry.kubeconfig_folders",
            "registry.kubeconfigs",
            "registry.last_used",
            "registry.last_used.context",
            "registry.last_used.kubeconfig",
            "registry.last_used_stamp",
            "registry.last_used_stamp.len",
            "registry.last_used_stamp.modified_ms",
            "tables",
            "tables.pods",
            "tables.pods.hidden",
            "tables.pods.sort",
            "tables.pods.sort.column",
            "tables.pods.sort.direction",
            "terminal",
            "terminal.default_shell",
            "terminal.font_size",
            "terminal.scrollback_lines",
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
fn only_a_reset_marks_the_run_as_reset(cx: &mut TestAppContext) {
    cx.update(|cx| {
        assert!(!AppSettings::was_reset(cx), "no global yet");
        let loaded = |notice| LoadedSettings {
            settings: Settings::default(),
            writes: WriteMode::Disabled,
            notice,
        };
        AppSettings::install(loaded(Some(SettingsNotice::NoConfigDir)), cx);
        assert!(!AppSettings::was_reset(cx));
        let backup = std::path::PathBuf::from("settings.json.bak");
        AppSettings::install(loaded(Some(SettingsNotice::Reset { backup })), cx);
        assert!(AppSettings::was_reset(cx));
        // Dismissing the notice does not unlock later sessions.
        AppSettings::dismiss_notice(cx);
        assert!(AppSettings::was_reset(cx));
    });
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
        ThemePreference::Dark.apply(ColorTheme::Default, cx);
        assert!(Theme::global(cx).is_dark());
        ThemePreference::Light.apply(ColorTheme::Default, cx);
        assert!(!Theme::global(cx).is_dark());
    });
}

#[test]
fn section_with_one_change_round_trips() {
    let settings: Settings =
        serde_json::from_value(json!({ "general": { "watch_tls_secrets": false } }))
            .expect("parses");
    assert!(!settings.general.watch_tls_secrets);
    assert_eq!(settings.general.export_dir, None);
    let value = serde_json::to_value(&settings).expect("serializes");
    assert_eq!(value["general"], json!({ "watch_tls_secrets": false }));
    assert!(value.get("appearance").is_none());
}

#[test]
fn color_theme_round_trips() {
    for (colors, text) in [
        (ColorTheme::ZedOne, "zed-one"),
        (ColorTheme::Default, "default"),
    ] {
        let value = serde_json::to_value(colors).expect("serializes");
        assert_eq!(value, json!(text));
        assert_eq!(
            serde_json::from_value::<ColorTheme>(value).expect("parses"),
            colors
        );
    }
}

#[test]
fn a_file_without_the_color_theme_gets_zed_one() {
    let settings: Settings = serde_json::from_value(json!({ "version": 1 })).expect("parses");
    assert_eq!(settings.appearance.color_theme, ColorTheme::ZedOne);
    assert_eq!(
        Settings::default().appearance.color_theme,
        ColorTheme::ZedOne
    );
}

#[test]
fn a_saved_default_color_theme_stays_default() {
    let mut settings = Settings::default();
    settings.appearance.color_theme = ColorTheme::Default;
    let value = serde_json::to_value(&settings).expect("serializes");
    let loaded: Settings = serde_json::from_value(value).expect("parses");
    assert_eq!(loaded.appearance.color_theme, ColorTheme::Default);
}

#[test]
fn unknown_color_theme_loads_as_zed_one() {
    let value = json!({
        "version": 1,
        "theme": "dark",
        "appearance": { "color_theme": "solarized", "density": "comfortable" }
    });
    let settings: Settings = serde_json::from_value(value).expect("an unknown name is not corrupt");
    assert_eq!(settings.appearance.color_theme, ColorTheme::ZedOne);
    assert_eq!(settings.appearance.density, RowDensity::Comfortable);
    assert_eq!(settings.theme, ThemePreference::Dark);
}

#[test]
fn old_cluster_color_is_ignored() {
    let value = json!({
        "version": 1,
        "registry": { "clusters": [{
            "kubeconfig": "a.yaml",
            "context": "ctx",
            "display_name": "x",
            "color": "teal"
        }] }
    });
    let settings: Settings =
        serde_json::from_value(value).expect("an old color key is not corrupt");
    assert_eq!(settings.registry.clusters.len(), 1);
    assert_eq!(
        settings.registry.clusters[0].display_name.as_deref(),
        Some("x")
    );
    let saved =
        String::from_utf8(serialize_settings(&settings).expect("serializes")).expect("utf-8");
    assert!(!saved.contains("\"color\""), "{saved}");
}

#[test]
fn density_row_heights() {
    assert_eq!(RowDensity::default(), RowDensity::Compact);
    assert_eq!(RowDensity::Compact.row_height(), 28.);
    assert_eq!(RowDensity::Comfortable.row_height(), 36.);
    assert_eq!(
        serde_json::to_value(RowDensity::Compact).expect("serializes"),
        json!("compact")
    );
    assert_eq!(
        serde_json::to_value(RowDensity::Comfortable).expect("serializes"),
        json!("comfortable")
    );
}

/// Every option labels and parses back; the default labels itself `(default)`; an unknown label
/// is the default; a value the table lacks shows as `unlisted` says.
fn assert_table<T: Copy + PartialEq + std::fmt::Debug>(
    table: &OptionTable<T>,
    values: &[T],
    default: T,
) {
    for value in values {
        let label = table.label(*value, || "unlisted".to_owned());
        assert_ne!(label, "unlisted", "{value:?} is an option");
        assert_eq!(table.value(&label), *value);
    }
    assert_eq!(table.value("nonsense"), default);
    assert!(
        table.label(default, String::new).ends_with(" (default)"),
        "the default option names itself"
    );
    assert_eq!(table.choices().len(), values.len());
}

#[test]
fn option_labels_round_trip() {
    assert_table(
        &DENSITY_OPTIONS,
        &[RowDensity::Compact, RowDensity::Comfortable],
        RowDensity::Compact,
    );
    assert_table(&TAIL_OPTIONS, &[100, 500, 1_000, 5_000, 10_000], 1_000);
    assert_table(&SCROLLBACK_OPTIONS, &[1_000, 5_000, 10_000], 5_000);
    assert_table(
        &FONT_SIZE_OPTIONS,
        &[None, Some(12), Some(13), Some(14), Some(16), Some(18)],
        None,
    );
    assert_table(
        &SHELL_OPTIONS,
        &[ShellCommand::Auto, ShellCommand::Bash, ShellCommand::Sh],
        ShellCommand::Auto,
    );
}

#[test]
fn an_unlisted_value_shows_its_own_label() {
    let label = TAIL_OPTIONS.label(50, || "50 lines".to_owned());
    assert_eq!(label, "50 lines");
}

#[test]
fn tail_lines_is_clamped() {
    let tail = |lines| LogSettings {
        tail_lines: lines,
        ..LogSettings::default()
    };
    assert_eq!(tail(1).tail_lines(), 10);
    assert_eq!(tail(500).tail_lines(), 500);
    assert_eq!(tail(1_000_000).tail_lines(), 10_000);
    // The stored value is not rewritten.
    assert_eq!(tail(1).tail_lines, 1);
}

#[test]
fn scrollback_is_clamped() {
    let scrollback = |lines| TerminalSettings {
        scrollback_lines: lines,
        ..TerminalSettings::default()
    };
    assert_eq!(scrollback(10).scrollback_lines(), 1_000);
    assert_eq!(scrollback(5_000).scrollback_lines(), 5_000);
    assert_eq!(scrollback(50_000).scrollback_lines(), 10_000);
}

#[test]
fn font_size_is_clamped() {
    let font = |size| TerminalSettings {
        font_size: size,
        ..TerminalSettings::default()
    };
    assert_eq!(font(None).font_size(), None);
    assert_eq!(font(Some(4)).font_size(), Some(10));
    assert_eq!(font(Some(14)).font_size(), Some(14));
    assert_eq!(font(Some(99)).font_size(), Some(24));
}

#[test]
fn log_and_terminal_sections_stay_out_of_a_default_file() {
    let value = serde_json::to_value(Settings::default()).expect("serializes");
    for key in ["logs", "terminal", "general", "appearance"] {
        assert!(value.get(key).is_none(), "{key}");
    }
}

/// A hand-edited metrics entry of the wrong shape must not make the whole file corrupt.
#[test]
fn a_bad_metrics_entry_does_not_fail_the_settings_file() {
    let bad_scheme =
        json!({"namespace": "monitoring", "service": "vmselect", "port": "8481", "scheme": "ftp"});
    let missing_port = json!({"namespace": "monitoring", "service": "vmselect", "scheme": "http"});
    let wrong_type = json!("not an object");
    for bad in [bad_scheme, missing_port, wrong_type] {
        let file = json!({
            "version": 1,
            "theme": "dark",
            "registry": {"clusters": [
                {"kubeconfig": "a.yaml", "context": "bad", "metrics": bad},
                {"kubeconfig": "a.yaml", "context": "good", "display_name": "Good"},
            ]},
        });
        let settings: Settings = serde_json::from_value(file).expect("the file still loads");
        assert_eq!(
            settings.theme,
            ThemePreference::Dark,
            "the rest of the file loads"
        );
        let clusters = &settings.registry.clusters;
        assert_eq!(clusters[1].display_name.as_deref(), Some("Good"));
        assert_eq!(
            clusters[0].metrics,
            Some(StoredMetrics::Unreadable(bad.clone()))
        );
        // It reads as an invalid source, so no request is made for it.
        let summary = cluster::ContextSummary {
            name: "bad".to_owned(),
            cluster: "c".to_owned(),
            user: None,
            namespace: None,
            source: PathBuf::from("a.yaml"),
        };
        let profile = settings.registry.profile(&summary);
        assert_eq!(
            profile.metrics,
            Some(Err(cluster::MetricsSourceError::Unreadable))
        );
        // And it is written back as it was, not rewritten.
        let written = serde_json::to_value(&settings).expect("serializes");
        assert_eq!(written["registry"]["clusters"][0]["metrics"], bad);
    }
}
