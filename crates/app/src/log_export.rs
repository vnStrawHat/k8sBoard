//! Export of the visible log lines to a file the user picks in the native save dialog.
//!
//! This is the only code that writes a file for the log tabs, and only after the dialog returned
//! a path (C9). Nothing here traces a path, a file name, or the text.

use gpui_kit::{AppContext as _, Context, Task};

use crate::log_tab::LogTab;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExportState {
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

/// `{label}-{YYYYMMDD-HHMMSS}Z.log`; every char outside `[A-Za-z0-9._-]` becomes `_`.
pub(crate) fn export_file_name(label: &str, now: jiff::Timestamp) -> String {
    let label: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{label}-{}Z.log", now.strftime("%Y%m%d-%H%M%S"))
}

/// Opens the save dialog, then writes the visible lines to the chosen path. The tab keeps the
/// returned task; dropping it abandons the export.
pub(crate) fn start_export(name: String, cx: &mut Context<LogTab>) -> Task<()> {
    let directory = std::env::home_dir().unwrap_or_default();
    let chosen = cx.prompt_for_new_path(&directory, Some(&name));
    cx.spawn(async move |tab, cx| {
        let path = match chosen.await {
            Ok(Ok(Some(path))) => path,
            // Cancelled, or the dialog went away: nothing is written.
            Ok(Ok(None)) | Err(_) => {
                let _ = tab.update(cx, |tab, cx| tab.set_export_state(ExportState::Idle, cx));
                return;
            }
            Ok(Err(error)) => {
                let failed = ExportState::Failed {
                    message: format!("Could not open the save dialog: {error}"),
                };
                let _ = tab.update(cx, |tab, cx| tab.set_export_state(failed, cx));
                return;
            }
        };
        // The lines are read now, after the confirmation, so the file matches what is visible.
        let Ok((text, lines)) = tab.update(cx, |tab, cx| {
            let snapshot = tab.export_snapshot();
            tab.set_export_state(ExportState::Saving, cx);
            snapshot
        }) else {
            return;
        };
        let written = cx
            .background_spawn(async move { std::fs::write(&path, text).map(|()| path) })
            .await;
        let state = match written {
            Ok(path) => ExportState::Saved {
                file_name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            },
            Err(error) => ExportState::Failed {
                message: format!("Could not save the logs: {error}"),
            },
        };
        let _ = tab.update(cx, |tab, cx| {
            tab.set_exported_lines(lines);
            tab.set_export_state(state, cx);
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_file_name_sanitizes_label_and_stamps_utc() {
        let now: jiff::Timestamp = "2024-05-01T10:47:58Z".parse().expect("valid time");
        assert_eq!(
            export_file_name("deploy/api", now),
            "deploy_api-20240501-104758Z.log"
        );
        assert_eq!(
            export_file_name("pod-1/app container", now),
            "pod-1_app_container-20240501-104758Z.log"
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
}
