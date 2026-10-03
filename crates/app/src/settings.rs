//! The settings model and the `AppSettings` global that the views read and update. File I/O
//! lives in `settings_store.rs`.

use std::collections::BTreeMap;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, BorrowAppContext as _, Global, SharedString, Subscription};
use serde::{Deserialize, Serialize};

use crate::app_shell::Screen;
use crate::cluster_registry::ClusterRegistry;
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            theme: ThemePreference::default(),
            registry: ClusterRegistry::default(),
            tables: BTreeMap::new(),
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
    /// Re-themes every window now.
    pub(crate) fn apply(self, cx: &mut App) {
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
