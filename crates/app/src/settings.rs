//! The settings model and the `AppSettings` global that the views read and update. File I/O
//! lives in `settings_store.rs`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use cluster::ShellCommand;
use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, BorrowAppContext as _, Global, SharedString, Subscription};
use serde::{Deserialize, Serialize};

use crate::app_shell::Screen;
use crate::cluster_registry::ClusterRegistry;
use crate::color_theme::ColorTheme;
use crate::port_forwards::ForwardPreset;
use crate::resource_kind::ResourceKind;
use crate::settings_store::{
    LoadedSettings, SettingsNotice, WriteGate, WriteMode, serialize_settings, settings_path,
    write_settings,
};
use crate::table_sort::SortDirection;

pub(crate) const SETTINGS_VERSION: u32 = 1;

/// The persisted settings. Only paths, names, and enums: never kubeconfig content or secrets.
/// Unknown fields are ignored, so a later spec adds a section without a version bump.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub(crate) version: u32,
    pub(crate) theme: ThemePreference,
    pub(crate) registry: ClusterRegistry,
    /// Per screen key (`screen_key`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) tables: BTreeMap<String, TablePrefs>,
    /// Forwards kept to start again (spec 0035).
    #[serde(skip_serializing_if = "PortForwardSettings::is_empty")]
    pub(crate) port_forward: PortForwardSettings,
    /// The log dock (spec 0044).
    #[serde(skip_serializing_if = "DockSettings::is_empty")]
    pub(crate) dock: DockSettings,
    /// The General page (spec 0043).
    #[serde(skip_serializing_if = "is_default")]
    pub(crate) general: GeneralSettings,
    /// The Appearance page beyond the theme (spec 0043).
    #[serde(skip_serializing_if = "is_default")]
    pub(crate) appearance: AppearanceSettings,
    /// The Logs page (spec 0043).
    #[serde(skip_serializing_if = "is_default")]
    pub(crate) logs: LogSettings,
    /// The Terminal & Shell page (spec 0043).
    #[serde(skip_serializing_if = "is_default")]
    pub(crate) terminal: TerminalSettings,
}

/// The `general` section: a path and a switch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct GeneralSettings {
    /// Where Export dialogs start; `None` is the home folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) export_dir: Option<PathBuf>,
    /// Whether the Issues engine lists and watches TLS Secrets for expiry.
    pub(crate) watch_tls_secrets: bool,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            export_dir: None,
            watch_tls_secrets: true,
        }
    }
}

/// The `appearance` section: the colour theme and the row density; the Mode dropdown (`theme`)
/// is separate.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AppearanceSettings {
    pub(crate) density: RowDensity,
    pub(crate) color_theme: ColorTheme,
}

/// The height of a table row, header included (the wireframe Tokens page).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RowDensity {
    #[default]
    Compact,
    Comfortable,
}

impl RowDensity {
    pub(crate) fn row_height(self) -> f32 {
        match self {
            Self::Compact => 28.,
            Self::Comfortable => 36.,
        }
    }
}

/// The `logs` section: the defaults of a new log tab. A tab's own toggles never write back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct LogSettings {
    pub(crate) tail_lines: u32,
    pub(crate) show_timestamps: bool,
    pub(crate) wrap_lines: bool,
    pub(crate) show_json: bool,
}

impl Default for LogSettings {
    fn default() -> Self {
        Self {
            tail_lines: DEFAULT_TAIL_LINES,
            show_timestamps: true,
            wrap_lines: false,
            show_json: false,
        }
    }
}

const DEFAULT_TAIL_LINES: u32 = 1_000;
const TAIL_LINES_RANGE: (u32, u32) = (10, 10_000);

impl LogSettings {
    /// The lines a log tab asks for at open. A hand-edited value is clamped here, so it never
    /// reaches a request; the buffer keeps 10,000 lines.
    pub(crate) fn tail_lines(&self) -> u32 {
        self.tail_lines
            .clamp(TAIL_LINES_RANGE.0, TAIL_LINES_RANGE.1)
    }
}

pub(crate) const TAIL_OPTIONS: OptionTable<u32> = OptionTable {
    options: &[
        (100, "100 lines"),
        (500, "500 lines"),
        (DEFAULT_TAIL_LINES, "1,000 lines (default)"),
        (5_000, "5,000 lines"),
        (10_000, "10,000 lines"),
    ],
    default: DEFAULT_TAIL_LINES,
};

/// The `terminal` section: the defaults of a new shell tab, and the font of every terminal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct TerminalSettings {
    pub(crate) default_shell: ShellCommand,
    pub(crate) scrollback_lines: u32,
    /// `None` is the theme's monospace size.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) font_size: Option<u16>,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            default_shell: ShellCommand::Auto,
            scrollback_lines: DEFAULT_SCROLLBACK_LINES,
            font_size: None,
        }
    }
}

const DEFAULT_SCROLLBACK_LINES: u32 = 5_000;
/// 0036 sized 5,000 lines at about 8 MB per tab; 10,000 lines double that (about 128 MB for
/// the 8-tab cap), so a higher value is not offered and a hand edit is clamped.
const SCROLLBACK_RANGE: (u32, u32) = (1_000, 10_000);
const FONT_SIZE_RANGE: (u16, u16) = (10, 24);

impl TerminalSettings {
    pub(crate) fn scrollback_lines(&self) -> u32 {
        self.scrollback_lines
            .clamp(SCROLLBACK_RANGE.0, SCROLLBACK_RANGE.1)
    }

    pub(crate) fn font_size(&self) -> Option<u16> {
        self.font_size
            .map(|size| size.clamp(FONT_SIZE_RANGE.0, FONT_SIZE_RANGE.1))
    }
}

pub(crate) const SCROLLBACK_OPTIONS: OptionTable<u32> = OptionTable {
    options: &[
        (1_000, "1,000 lines"),
        (DEFAULT_SCROLLBACK_LINES, "5,000 lines (default)"),
        (10_000, "10,000 lines"),
    ],
    default: DEFAULT_SCROLLBACK_LINES,
};

pub(crate) const FONT_SIZE_OPTIONS: OptionTable<Option<u16>> = OptionTable {
    options: &[
        (None, "Theme size (default)"),
        (Some(12), "12 px"),
        (Some(13), "13 px"),
        (Some(14), "14 px"),
        (Some(16), "16 px"),
        (Some(18), "18 px"),
    ],
    default: None,
};

pub(crate) const SHELL_OPTIONS: OptionTable<ShellCommand> = OptionTable {
    options: &[
        (ShellCommand::Auto, "Auto: bash, ash, sh (default)"),
        (ShellCommand::Bash, "bash"),
        (ShellCommand::Sh, "sh"),
    ],
    default: ShellCommand::Auto,
};
/// One dropdown: the options with their labels (the label is the dropdown key, so there is no
/// second string table) and the default, which is also what an unknown label means.
pub(crate) struct OptionTable<T: 'static> {
    options: &'static [(T, &'static str)],
    default: T,
}

impl<T: Copy + PartialEq> OptionTable<T> {
    /// The label of `value`; a value the table lacks (a hand edit) shows as `unlisted` says, so
    /// the file's value stays visible and is not silently replaced.
    pub(crate) fn label(&self, value: T, unlisted: impl FnOnce() -> String) -> SharedString {
        self.options
            .iter()
            .find(|(option, _)| *option == value)
            .map_or_else(|| unlisted().into(), |(_, label)| (*label).into())
    }

    /// The value a label names; an unknown label is the default.
    pub(crate) fn value(&self, label: &str) -> T {
        self.options
            .iter()
            .find(|(_, option_label)| *option_label == label)
            .map_or(self.default, |(value, _)| *value)
    }

    /// The dropdown choices as `(key, label)` pairs.
    pub(crate) fn choices(&self) -> Vec<(SharedString, SharedString)> {
        self.options
            .iter()
            .map(|(_, label)| ((*label).into(), (*label).into()))
            .collect()
    }
}

pub(crate) const COLOR_THEME_OPTIONS: OptionTable<ColorTheme> = OptionTable {
    options: &[
        (ColorTheme::Default, "Default"),
        (ColorTheme::ZedOne, "Zed One"),
    ],
    default: ColorTheme::ZedOne,
};

pub(crate) const DENSITY_OPTIONS: OptionTable<RowDensity> = OptionTable {
    options: &[
        (RowDensity::Compact, "Compact (28 px) (default)"),
        (RowDensity::Comfortable, "Comfortable (36 px)"),
    ],
    default: RowDensity::Compact,
};

/// Serializes a section only when it differs from its default, so an untouched file stays minimal.
fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

/// The `dock` section: a pixel height, never a secret.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct DockSettings {
    /// The dock height in pixels; `None` is the default height.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) height: Option<f32>,
}

impl DockSettings {
    fn is_empty(&self) -> bool {
        self.height.is_none()
    }
}

/// The `port_forward` section: names and numbers only, never a secret.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PortForwardSettings {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) presets: Vec<ForwardPreset>,
}

impl PortForwardSettings {
    fn is_empty(&self) -> bool {
        self.presets.is_empty()
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            theme: ThemePreference::default(),
            registry: ClusterRegistry::default(),
            tables: BTreeMap::new(),
            port_forward: PortForwardSettings::default(),
            dock: DockSettings::default(),
            general: GeneralSettings::default(),
            appearance: AppearanceSettings::default(),
            logs: LogSettings::default(),
            terminal: TerminalSettings::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

/// The dropdown options: the key is the label, so there is no second string table.
const THEME_OPTIONS: [(ThemePreference, &str); 3] = [
    (ThemePreference::System, "Follow the system"),
    (ThemePreference::Light, "Light"),
    (ThemePreference::Dark, "Dark"),
];

impl ThemePreference {
    /// Re-themes every window now: the colour family first, then the mode picks its light or dark
    /// config.
    pub(crate) fn apply(self, colors: ColorTheme, cx: &mut App) {
        colors.install(cx);
        match self {
            Self::Light => Theme::change(ThemeMode::Light, None, cx),
            Self::Dark => Theme::change(ThemeMode::Dark, None, cx),
            Self::System => Theme::sync_system_appearance(None, cx),
        }
        // The kit highlights the selected row with a faint tint; the theme's selection colour
        // makes the open drawer's row easy to find in both modes.
        Theme::update(cx, |theme| theme.table_active = theme.selection);
    }
}

/// The dropdown label of `theme`.
pub(crate) fn theme_label(theme: ThemePreference) -> &'static str {
    THEME_OPTIONS
        .iter()
        .find(|(option, _)| *option == theme)
        .map_or("Follow the system", |(_, label)| label)
}

/// The theme a dropdown label names; an unknown label is `System`.
pub(crate) fn theme_from_label(label: &str) -> ThemePreference {
    THEME_OPTIONS
        .iter()
        .find(|(_, option_label)| *option_label == label)
        .map_or(ThemePreference::System, |(theme, _)| *theme)
}

/// The dropdown choices as `(key, label)` pairs.
pub(crate) fn theme_choices() -> Vec<(SharedString, SharedString)> {
    THEME_OPTIONS
        .iter()
        .map(|(_, label)| ((*label).into(), (*label).into()))
        .collect()
}

/// What one table remembers: the sort and the hidden columns, by column name so they survive a
/// reordering of columns. Filters are not saved.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct TablePrefs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sort: Option<SavedSort>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) hidden: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedSort {
    pub(crate) column: String,
    pub(crate) direction: SortDirection,
}

/// The key of a screen in `Settings::tables`.
pub(crate) fn screen_key(screen: Screen) -> &'static str {
    match screen {
        Screen::Pods => "pods",
        Screen::Nodes => "nodes",
        Screen::Overview => "overview",
        Screen::Issues => "issues",
        Screen::Topology => "topology",
        Screen::PortForwarding => "port-forwarding",
        // A custom plural has no group, so it could equal a built-in key; the CRD name never does.
        Screen::Kind(ResourceKind::Custom(kind)) => kind.crd_name(),
        Screen::Kind(kind) => kind.plural(),
    }
}

/// The settings in memory, plus the writer that saves every change.
pub(crate) struct AppSettings {
    settings: Settings,
    /// `None` while writes are off; the settings still change in memory.
    writer: Option<SettingsWriter>,
    last_sent: Vec<u8>,
    generation: u64,
    notice: Option<SettingsNotice>,
    /// Set when the file was unparsable and replaced by defaults: every session opens Locked for
    /// the rest of the run, so a reset never downgrades a Production cluster.
    was_reset: bool,
    _quit_flush: Option<Subscription>,
}

struct SettingsWriter {
    dir: std::path::PathBuf,
    sender: UnboundedSender<(u64, Vec<u8>)>,
    gate: Arc<WriteGate>,
}

impl Global for AppSettings {}

impl AppSettings {
    /// Sets the global. `WriteMode::Enabled` starts the writer task and the quit flush.
    pub(crate) fn install(loaded: LoadedSettings, cx: &mut App) {
        let last_sent = serialize_settings(&loaded.settings).unwrap_or_default();
        let mut app = Self {
            settings: loaded.settings,
            writer: None,
            last_sent,
            generation: 0,
            was_reset: matches!(loaded.notice, Some(SettingsNotice::Reset { .. })),
            notice: loaded.notice,
            _quit_flush: None,
        };
        if let WriteMode::Enabled(dir) = loaded.writes {
            let (sender, receiver) = unbounded();
            let gate = Arc::new(WriteGate::default());
            spawn_writer(dir.clone(), Arc::clone(&gate), receiver, cx);
            app.writer = Some(SettingsWriter { dir, sender, gate });
            app._quit_flush = Some(cx.on_app_quit(|cx| {
                Self::flush(cx);
                async {}
            }));
        }
        cx.set_global(app);
    }

    /// The settings, or `None` when no global is installed (an export started from a test view).
    pub(crate) fn try_get(cx: &App) -> Option<&Settings> {
        cx.try_global::<Self>().map(|app| &app.settings)
    }

    pub(crate) fn get(cx: &App) -> &Settings {
        &cx.global::<Self>().settings
    }

    /// Applies `change`, then sends the new snapshot to the writer unless the saved bytes
    /// would be the same. Observers of the global re-render.
    pub(crate) fn update(cx: &mut App, change: impl FnOnce(&mut Settings)) {
        cx.update_global::<Self, _>(|app, _| {
            change(&mut app.settings);
            let Some(bytes) = serialize_settings(&app.settings) else {
                return;
            };
            if bytes == app.last_sent {
                return;
            }
            // Updated before the write is attempted: a failed write raises a notice, and the next
            // change differs anyway, so a retry needs no extra bookkeeping.
            app.last_sent.clone_from(&bytes);
            let Some(writer) = &app.writer else {
                return;
            };
            app.generation += 1;
            // The receiver only closes when the app is shutting down.
            let _ = writer.sender.unbounded_send((app.generation, bytes));
        });
    }

    /// The folder the settings are saved to; `None` while writes are off.
    pub(crate) fn config_dir(cx: &App) -> Option<&std::path::Path> {
        let writer = cx.global::<Self>().writer.as_ref()?;
        Some(writer.dir.as_path())
    }

    pub(crate) fn notice(cx: &App) -> Option<&SettingsNotice> {
        cx.global::<Self>().notice.as_ref()
    }

    /// Whether this run started from a reset settings file; false without a global.
    pub(crate) fn was_reset(cx: &App) -> bool {
        cx.try_global::<Self>().is_some_and(|app| app.was_reset)
    }

    pub(crate) fn dismiss_notice(cx: &mut App) {
        cx.update_global::<Self, _>(|app, _| app.notice = None);
    }

    /// Writes the last sent snapshot now, on the calling thread, when something was changed.
    /// The gate drops it when a background write of the same or a newer one already finished.
    pub(crate) fn flush(cx: &mut App) {
        let Some(app) = cx.try_global::<Self>() else {
            return;
        };
        let Some(writer) = &app.writer else {
            return;
        };
        if app.generation == 0 {
            return;
        }
        let result = write_settings(&writer.dir, &app.last_sent, app.generation, &writer.gate);
        if let Err(error) = result {
            tracing::warn!(kind = ?error.kind(), "cannot flush settings on quit");
        }
    }
}

/// One task drains the channel to the newest snapshot, then writes it on the background executor.
fn spawn_writer(
    dir: std::path::PathBuf,
    gate: Arc<WriteGate>,
    mut receiver: UnboundedReceiver<(u64, Vec<u8>)>,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        while let Some(mut next) = receiver.next().await {
            while let Ok(newer) = receiver.try_recv() {
                next = newer;
            }
            let (write_dir, write_gate) = (dir.clone(), Arc::clone(&gate));
            let result = cx
                .background_executor()
                .spawn(async move { write_settings(&write_dir, &next.1, next.0, &write_gate) })
                .await;
            let Err(error) = result else {
                continue;
            };
            let path = settings_path(&dir);
            tracing::warn!(path = %path.display(), kind = ?error.kind(), "cannot save settings");
            cx.update_global::<AppSettings, _>(|app, _| {
                app.notice = Some(SettingsNotice::WriteFailed {
                    path,
                    kind: error.kind(),
                });
            });
        }
    })
    .detach();
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod settings_tests;
