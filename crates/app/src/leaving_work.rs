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
use gpui_kit::{App, AppContext as _, Context, IntoElement as _, ParentElement as _, SharedString};

use super::AppShell;
use crate::cluster_registry::ClusterRef;
use crate::fresh_enter::FreshEnter;

/// What releasing some clusters would end.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LeavingWork {
    /// Open shell tabs: each session ends and a new shell starts empty.
    pub(crate) shells: usize,
    /// Clusters with a batch still committing: it stops at the next item and the rest read `Not sent`.
    pub(crate) batches: usize,
    /// The open Edit YAML text that was not applied, as `Deployment/payments/api`: it is thrown away.
    pub(crate) unsaved_edit: Option<String>,
    /// Open node shells: each pod is deleted with its tab.
    pub(crate) node_shells: usize,
    /// The clusters (by display name) with a drain running: it stops, and its nodes stay cordoned.
    pub(crate) drains: Vec<SharedString>,
}

impl LeavingWork {
    pub(crate) fn is_empty(&self) -> bool {
        self.shells == 0
            && self.batches == 0
            && self.node_shells == 0
            && self.drains.is_empty()
            && self.unsaved_edit.is_none()
    }

    /// One line per kind of work: `2 shells will close`.
    pub(crate) fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        match self.shells {
            0 => {}
            1 => lines.push("1 shell will close".to_owned()),
            count => lines.push(format!("{count} shells will close")),
        }
        match self.batches {
            0 => {}
            1 => {
                lines.push("1 running batch will stop; its remaining items are not sent".to_owned())
            }
            count => lines.push(format!(
                "{count} running batches will stop; their remaining items are not sent"
            )),
        }
        if let Some(subject) = &self.unsaved_edit {
            lines.push(format!("Unsaved changes to {subject}"));
        }
        match self.node_shells {
            0 => {}
            1 => lines.push("1 node shell will close; its pod is deleted".to_owned()),
            count => lines.push(format!(
                "{count} node shells will close; their pods are deleted"
            )),
        }
        for cluster in &self.drains {
            lines.push(format!(
                "A drain on {cluster} will stop; its nodes stay cordoned"
            ));
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
            node_shells: self.dock.read(cx).node_shell_count_of(leaving, cx),
            drains: self.running_drain_names_of(leaving, cx),
            batches: leaving
                .iter()
                .filter(|cluster| self.running_batches.contains(cluster))
                .count(),
            unsaved_edit: self.unsaved_edit_of(leaving, cx),
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
                // Runs the release once, whichever of the button and a fresh Enter confirms.
                let confirm: Rc<dyn Fn(&mut App)> = Rc::new(move |cx| {
                    let release = release.borrow_mut().take();
                    if let Some(release) = release {
                        let _ = shell.update(cx, |shell, cx| release(shell, cx));
                    }
                });
                let on_enter = Rc::clone(&confirm);
                let text = SharedString::from(lines.join("\n"));
                // Held Enter must not close live shells under a dialog nobody read.
                let body = cx.new(|cx| {
                    FreshEnter::new(
                        move |_| text.clone().into_any_element(),
                        move |window, cx| {
                            on_enter(cx);
                            window.close_dialog(cx);
                        },
                        cx,
                    )
                });
                window.open_alert_dialog(cx, move |alert, _, _| {
                    let confirm = Rc::clone(&confirm);
                    alert
                        .title("Close open work?")
                        .child(body.clone())
                        .confirm()
                        .button_props(
                            DialogButtonProps::default()
                                .ok_text("Continue")
                                .ok_variant(ButtonVariant::Primary)
                                .cancel_text("Keep everything")
                                .show_cancel(true),
                        )
                        .on_ok(move |_, _, cx| {
                            confirm(cx);
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
            LeavingWork {
                shells: 1,
                ..LeavingWork::default()
            }
            .lines(),
            ["1 shell will close".to_owned()]
        );
        assert_eq!(
            LeavingWork {
                shells: 2,
                ..LeavingWork::default()
            }
            .lines(),
            ["2 shells will close".to_owned()]
        );
    }

    #[test]
    fn running_batches_are_counted_in_singular_and_plural() {
        let batches = |batches| LeavingWork {
            batches,
            ..LeavingWork::default()
        };
        assert!(!batches(1).is_empty());
        assert_eq!(
            batches(1).lines(),
            ["1 running batch will stop; its remaining items are not sent".to_owned()]
        );
        assert_eq!(
            batches(2).lines(),
            ["2 running batches will stop; their remaining items are not sent".to_owned()]
        );
    }

    #[test]
    fn an_unsaved_edit_is_a_line_of_its_own() {
        let work = LeavingWork {
            unsaved_edit: Some("Deployment/team-a/api".to_owned()),
            ..LeavingWork::default()
        };
        assert!(!work.is_empty());
        assert_eq!(
            work.lines(),
            ["Unsaved changes to Deployment/team-a/api".to_owned()]
        );
    }

    #[test]
    fn node_shells_say_their_pods_are_deleted() {
        let node_shells = |node_shells| LeavingWork {
            node_shells,
            ..LeavingWork::default()
        };
        assert!(!node_shells(1).is_empty());
        assert_eq!(
            node_shells(1).lines(),
            ["1 node shell will close; its pod is deleted".to_owned()]
        );
        assert_eq!(
            node_shells(3).lines(),
            ["3 node shells will close; their pods are deleted".to_owned()]
        );
    }
}
