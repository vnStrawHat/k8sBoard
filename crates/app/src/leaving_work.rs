//! The one dialog that asks before a release ends work the user may lose (spec 0036 decision 37).
//! A switch, a view change, and "Remove from view" ask it before the release starts, because the
//! release of a session cannot wait for an answer once it began. It lists the leaving clusters'
//! work only; other features add their lines (unsaved edits, a running drain, node shells).
//!
//! A child of `app_shell`, like `write_flow`: it reads the dock and runs the release afterwards.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::{AppContext as _, Context, SharedString};

use super::AppShell;
use crate::cluster_registry::ClusterRef;

/// What releasing some clusters would end.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LeavingWork {
    /// Open shell tabs: each session ends and a new shell starts empty.
    pub(crate) shells: usize,
}

impl LeavingWork {
    pub(crate) fn is_empty(&self) -> bool {
        self.shells == 0
    }

    /// One line per kind of work: `2 shells will close`.
    pub(crate) fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        match self.shells {
            0 => {}
            1 => lines.push("1 shell will close".to_owned()),
            count => lines.push(format!("{count} shells will close")),
        }
        lines
    }
}

/// What runs once the user confirmed: the release the check stood in front of.
type Release = Box<dyn FnOnce(&mut AppShell, &mut Context<AppShell>)>;

impl AppShell {
    /// What releasing `leaving` would end, counted from the dock before anything is released.
    pub(super) fn leaving_work(&self, leaving: &[ClusterRef], cx: &gpui_kit::App) -> LeavingWork {
        LeavingWork {
            shells: self.dock.read(cx).shell_count_of(leaving, cx),
        }
    }

    /// Asks "{N} shells will close" and runs `release` on Continue; Esc, Cancel, or a click outside
    /// keeps everything as it was. The dialog opens after the current update, which owns the window.
    pub(super) fn confirm_leaving(
        &mut self,
        work: LeavingWork,
        release: impl FnOnce(&mut AppShell, &mut Context<AppShell>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let lines = work.lines();
        #[cfg(test)]
        {
            self.last_leaving = Some(lines.clone());
        }
        let release: Rc<RefCell<Option<Release>>> = Rc::new(RefCell::new(Some(Box::new(release))));
        let (handle, shell) = (self.window, cx.weak_entity());
        cx.defer(move |cx| {
            let _ = cx.update_window(handle, |_, window, cx| {
                window.open_alert_dialog(cx, move |alert, _, _| {
                    let (shell, release) = (shell.clone(), Rc::clone(&release));
                    alert
                        .title("Close open work?")
                        .description(SharedString::from(lines.join("\n")))
                        .confirm()
                        .button_props(
                            DialogButtonProps::default()
                                .ok_text("Continue")
                                .ok_variant(ButtonVariant::Primary)
                                .cancel_text("Keep everything")
                                .show_cancel(true),
                        )
                        .on_ok(move |_, _, cx| {
                            let release = release.borrow_mut().take();
                            if let Some(release) = release {
                                let _ = shell.update(cx, |shell, cx| release(shell, cx));
                            }
                            true
                        })
                });
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_work_has_no_lines() {
        let work = LeavingWork::default();
        assert!(work.is_empty());
        assert!(work.lines().is_empty());
    }

    #[test]
    fn shells_are_counted_in_singular_and_plural() {
        assert_eq!(
            LeavingWork { shells: 1 }.lines(),
            ["1 shell will close".to_owned()]
        );
        assert_eq!(
            LeavingWork { shells: 2 }.lines(),
            ["2 shells will close".to_owned()]
        );
    }
}
