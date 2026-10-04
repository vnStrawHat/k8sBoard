//! What every exporter shares: the state of one export, from button click to result, and the file
//! name offered in the save dialog. Each export writes only after the dialog returned a path (C9),
//! and nothing here traces a path or a file name.

use std::path::{Path, PathBuf};

use gpui_kit::{AppContext as _, Context, Task};

use crate::settings::AppSettings;

/// One export, from button click to result. The log dock and Overview both use it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum ExportState {
    #[default]
    Idle,
    /// The save dialog is open.
    Choosing,
    Saving,
    Saved {
        file_name: String,
    },
    Failed {
        message: String,
    },
}

impl ExportState {
    /// The dialog or the write is still running.
    pub(crate) fn is_busy(&self) -> bool {
        matches!(self, Self::Choosing | Self::Saving)
    }
}

/// The most label characters a file name keeps, so a long object name cannot exceed a file system's
/// name limit once the time stamp and extension follow.
const LABEL_LIMIT: usize = 150;

/// `{label}-{YYYYMMDD-HHMMSS}Z.{extension}`. The one sanitizer for every exporter: each char
/// outside `[A-Za-z0-9._-]` (path-unsafe ones such as `/`, `\`, `:`, `@`, spaces, control and
/// non-ASCII chars) becomes `-`, and the label is cut at `LABEL_LIMIT` chars.
pub(crate) fn export_file_name(label: &str, extension: &str, now: jiff::Timestamp) -> String {
    let label: String = label
        .chars()
        .take(LABEL_LIMIT)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("{label}-{}Z.{extension}", now.strftime("%Y%m%d-%H%M%S"))
}

/// The one save flow of every exporter: opens the save dialog on `name`, then writes the text that
/// `snapshot` returns to the chosen path. Nothing is written unless the user confirms a path (C9),
/// and nothing is traced.
///
/// `snapshot` runs after the confirmation, so the file matches what is shown then; it returns the
/// text and what the owner needs after the write (handed to `finish`), or the message of a failure.
/// `noun` names what is saved in the error text (`Could not save the {noun}: …`). The owner keeps
/// the returned task; dropping it abandons the export.
pub(crate) fn start_export<V: 'static, A: 'static>(
    name: String,
    noun: &'static str,
    snapshot: impl FnOnce(&mut V, &mut Context<V>) -> Result<(String, A), String> + 'static,
    set_state: fn(&mut V, ExportState, &mut Context<V>),
    finish: impl FnOnce(&mut V, A) + 'static,
    cx: &mut Context<V>,
) -> Task<()> {
    start_export_with(name, noun, snapshot, write_text, set_state, finish, cx)
}

/// Where the save dialog starts: the stored folder when it is a directory (`is_dir` is the answer
/// of the background check), else the home folder.
fn export_start_folder(stored: Option<PathBuf>, is_dir: bool, home: Option<PathBuf>) -> PathBuf {
    match stored {
        Some(dir) if is_dir => dir,
        _ => home.unwrap_or_default(),
    }
}

fn write_text(path: &Path, text: String) -> std::io::Result<()> {
    std::fs::write(path, text)
}

/// `start_export` for a payload that is not text: `write` turns what `snapshot` returned into the
/// file, on the background executor, once the path is known (an image export picks its format from
/// the extension and renders there). A failure of `write` reads like an I/O error.
pub(crate) fn start_export_with<V: 'static, A: 'static, T: Send + 'static>(
    name: String,
    noun: &'static str,
    snapshot: impl FnOnce(&mut V, &mut Context<V>) -> Result<(T, A), String> + 'static,
    write: fn(&Path, T) -> std::io::Result<()>,
    set_state: fn(&mut V, ExportState, &mut Context<V>),
    finish: impl FnOnce(&mut V, A) + 'static,
    cx: &mut Context<V>,
) -> Task<()> {
    let stored = AppSettings::try_get(cx).and_then(|settings| settings.general.export_dir.clone());
    cx.spawn(async move |view, cx| {
        // The stored folder may be gone (a removed drive), so the check runs off the UI thread.
        let is_dir = match stored.clone() {
            Some(dir) => cx.background_spawn(async move { dir.is_dir() }).await,
            None => false,
        };
        let directory = export_start_folder(stored, is_dir, std::env::home_dir());
        let Ok(chosen) = view.update(cx, |_, cx| cx.prompt_for_new_path(&directory, Some(&name)))
        else {
            return;
        };
        let path = match chosen.await {
            Ok(Ok(Some(path))) => path,
            // Cancelled, or the dialog went away: nothing is written.
            Ok(Ok(None)) | Err(_) => {
                let _ = view.update(cx, |view, cx| set_state(view, ExportState::Idle, cx));
                return;
            }
            Ok(Err(error)) => {
                let failed = ExportState::Failed {
                    message: format!("Could not open the save dialog: {error}"),
                };
                let _ = view.update(cx, |view, cx| set_state(view, failed, cx));
                return;
            }
        };
        let Ok(taken) = view.update(cx, |view, cx| {
            let taken = snapshot(view, cx);
            let state = match &taken {
                Ok(_) => ExportState::Saving,
                Err(message) => ExportState::Failed {
                    message: message.clone(),
                },
            };
            set_state(view, state, cx);
            taken
        }) else {
            return;
        };
        let Ok((payload, extra)) = taken else {
            return;
        };
        let written = cx
            .background_spawn(async move { write(&path, payload).map(|()| path) })
            .await;
        let mut saved_in = None;
        let state = match written {
            Ok(path) => {
                saved_in = path.parent().map(Path::to_path_buf);
                ExportState::Saved {
                    file_name: path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                }
            }
            Err(error) => ExportState::Failed {
                message: format!("Could not save the {noun}: {error}"),
            },
        };
        let _ = view.update(cx, |view, cx| {
            // Only a saved file moves the folder: a cancel or a failure leaves it.
            if let Some(folder) = saved_in
                && cx.has_global::<AppSettings>()
            {
                AppSettings::update(cx, |settings| settings.general.export_dir = Some(folder));
            }
            finish(view, extra);
            set_state(view, state, cx);
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> jiff::Timestamp {
        "2024-05-01T10:47:58Z".parse().expect("valid time")
    }

    #[test]
    fn export_file_name_sanitizes_label_and_stamps_utc() {
        assert_eq!(
            export_file_name("deploy/api", "log", now()),
            "deploy-api-20240501-104758Z.log"
        );
        assert_eq!(
            export_file_name("pod-1/app container", "log", now()),
            "pod-1-app-container-20240501-104758Z.log"
        );
    }

    #[test]
    fn export_file_name_uses_extension() {
        assert_eq!(
            export_file_name("overview", "md", now()),
            "overview-20240501-104758Z.md"
        );
        assert_eq!(
            export_file_name("topology", "png", now()),
            "topology-20240501-104758Z.png"
        );
    }

    #[test]
    fn export_file_name_replaces_path_unsafe_chars() {
        assert_eq!(
            export_file_name("readonly@Monitor:a/b", "md", now()),
            "readonly-Monitor-a-b-20240501-104758Z.md"
        );
        assert_eq!(
            export_file_name("a\\b c", "md", now()),
            "a-b-c-20240501-104758Z.md"
        );
    }

    #[test]
    fn export_file_name_cuts_a_long_label_and_replaces_other_chars() {
        let long = "a".repeat(400);
        let name = export_file_name(&long, "md", now());
        assert_eq!(name, format!("{}-20240501-104758Z.md", "a".repeat(150)));
        assert_eq!(
            export_file_name("caf\u{e9}\u{4e2d}\t\n\u{0}x", "md", now()),
            "caf-----x-20240501-104758Z.md"
        );
    }

    #[test]
    fn export_state_is_busy_while_choosing_or_saving() {
        assert!(ExportState::Choosing.is_busy());
        assert!(ExportState::Saving.is_busy());
        assert!(!ExportState::Idle.is_busy());
        assert!(
            !ExportState::Saved {
                file_name: "a".into()
            }
            .is_busy()
        );
        assert!(
            !ExportState::Failed {
                message: "x".into()
            }
            .is_busy()
        );
    }

    #[test]
    fn export_start_folder_falls_back_to_home() {
        let home = Some(PathBuf::from("home"));
        let stored = Some(PathBuf::from("exports"));
        assert_eq!(
            export_start_folder(stored.clone(), true, home.clone()),
            PathBuf::from("exports")
        );
        assert_eq!(
            export_start_folder(stored, false, home.clone()),
            PathBuf::from("home")
        );
        assert_eq!(
            export_start_folder(None, false, home),
            PathBuf::from("home")
        );
        assert_eq!(export_start_folder(None, false, None), PathBuf::new());
    }

    /// A view that exports a note, to drive the save flow headlessly.
    struct Exporter {
        state: ExportState,
        _task: Option<Task<()>>,
    }

    fn set_state(exporter: &mut Exporter, state: ExportState, _: &mut Context<Exporter>) {
        exporter.state = state;
    }

    /// Installs settings (writes off) and starts the export of a note; the save dialog is next.
    fn start_note_export(cx: &mut gpui_kit::TestAppContext) -> gpui_kit::Entity<Exporter> {
        use crate::settings::Settings;
        use crate::settings_store::{LoadedSettings, WriteMode};
        cx.update(|cx| {
            AppSettings::install(
                LoadedSettings {
                    settings: Settings::default(),
                    writes: WriteMode::Disabled,
                    notice: None,
                },
                cx,
            );
        });
        let exporter = cx.new(|_| Exporter {
            state: ExportState::Idle,
            _task: None,
        });
        let task = exporter.update(cx, |_, cx| {
            start_export(
                "note.txt".to_owned(),
                "note",
                |_, _| Ok(("text".to_owned(), ())),
                set_state,
                |_, ()| {},
                cx,
            )
        });
        exporter.update(cx, |exporter, _| exporter._task = Some(task));
        cx.run_until_parked();
        exporter
    }

    #[gpui_kit::test]
    fn saved_export_remembers_its_folder(cx: &mut gpui_kit::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("k8sboard-0043-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp folder");
        let exporter = start_note_export(cx);
        let target = dir.join("note.txt");
        cx.simulate_new_path_selection(move |_| Some(target.clone()));
        cx.run_until_parked();
        let stored = cx.read(|cx| AppSettings::get(cx).general.export_dir.clone());
        assert_eq!(stored, Some(dir.clone()));
        assert!(matches!(
            exporter.read_with(cx, |exporter, _| exporter.state.clone()),
            ExportState::Saved { .. }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[gpui_kit::test]
    fn cancelled_export_keeps_the_folder(cx: &mut gpui_kit::TestAppContext) {
        let exporter = start_note_export(cx);
        cx.simulate_new_path_selection(|_| None);
        cx.run_until_parked();
        assert_eq!(
            cx.read(|cx| AppSettings::get(cx).general.export_dir.clone()),
            None
        );
        assert_eq!(
            exporter.read_with(cx, |exporter, _| exporter.state.clone()),
            ExportState::Idle
        );
    }
}
