//! Keeping the promise that a node shell pod never outlives its session (spec 0037 decisions 12
//! and 13). The app holds one `NodeShellCleanup` per open node shell tab and runs it on every end:
//! the shell exits or fails, the tab is closed, a switch or a released cluster closes the tab, the
//! main window closes (the close waits), or the app quits (best effort inside GPUI's 200 ms).
//!
//! A child of `app_shell`, like `shell_open`: the registry lives on the shell because the window
//! and quit hooks do. Every delete writes its own audit line; one that has not reported when the
//! app quits leaves an `abandoned` line first.

use std::collections::HashMap;
use std::future::Future;

use futures::future::join_all;
use gpui_kit::{AppContext as _, Context, EntityId, Subscription, Task, Window};

use super::AppShell;
use super::leaving_work::LeavingWork;
use super::write_flow::{CleanupOutcome, NodeShellCleanup, notify, run_cleanup};
use crate::audit_log::{AuditEntry, AuditOutcome, append_audit};
use crate::cluster_runtime::ClusterRuntime;
use crate::settings::AppSettings;

/// The text of the `abandoned` line of a delete the app quit before it reported.
const ABANDONED_TEXT: &str = "the delete did not report before the app quit";

/// The cleanups of the open node shell tabs, and the deletes in flight.
#[derive(Default)]
pub(super) struct NodeShellRuns {
    /// One per open node shell tab: removed and run on the first end of that tab.
    cleanups: HashMap<EntityId, NodeShellCleanup>,
    /// The deletes that started and have not reported, each with the line to write if the app
    /// quits first.
    pending: HashMap<u64, AuditEntry>,
    next_serial: u64,
    /// The running deletes by serial, so a quit can wait for them. A finished one is dropped at the
    /// next start (`begin_cleanup`).
    in_flight: HashMap<u64, Task<()>>,
    /// Node shell pods whose create is on its way: the window does not close under them, because
    /// the pod would exist with nobody to delete it.
    creating: usize,
    /// The main window was asked to close while pods remained: it closes when the last delete
    /// reports.
    is_closing: bool,
    quit: Option<Subscription>,
}

impl NodeShellRuns {
    /// Whether the window may close now: no pod waits for its delete.
    fn is_idle(&self) -> bool {
        self.cleanups.is_empty() && self.pending.is_empty() && self.creating == 0
    }

    /// A node shell create was sent; `create_finished` follows when its result has been handled.
    pub(super) fn create_started(&mut self) {
        self.creating += 1;
    }

    pub(super) fn create_finished(&mut self) {
        self.creating = self.creating.saturating_sub(1);
    }

    pub(super) fn is_closing(&self) -> bool {
        self.is_closing
    }
}

impl AppShell {
    /// Remembers how to delete the pod behind `tab`. The first call also hooks the app quit.
    pub(super) fn register_cleanup(
        &mut self,
        tab: EntityId,
        cleanup: NodeShellCleanup,
        cx: &mut Context<Self>,
    ) {
        self.node_shell_runs.cleanups.insert(tab, cleanup);
        self.ensure_quit_hook(cx);
    }

    /// The first delete of the run hooks the app quit, whatever started it (a tab, the sweep).
    fn ensure_quit_hook(&mut self, cx: &mut Context<Self>) {
        if self.node_shell_runs.quit.is_none() {
            self.node_shell_runs.quit =
                Some(cx.on_app_quit(|shell, cx| shell.cleanup_for_quit(cx)));
        }
    }

    /// The tab ended or is gone: its pod is deleted now. A tab with no cleanup (an exec, a debug
    /// container, or one that already ran it) does nothing, so the cleanup runs at most once.
    pub(super) fn cleanup_tab(&mut self, tab: EntityId, cx: &mut Context<Self>) {
        if let Some(cleanup) = self.node_shell_runs.cleanups.remove(&tab) {
            self.begin_cleanup(cleanup, cx);
        }
    }

    /// Starts one delete (a tab's, a sweep row's, or one the window close or a late start owes).
    /// The line to write if the app quits first is remembered until the delete reports.
    pub(super) fn begin_cleanup(&mut self, cleanup: NodeShellCleanup, cx: &mut Context<Self>) {
        self.ensure_quit_hook(cx);
        let runs = &mut self.node_shell_runs;
        // Finished deletes are no longer pending: their tasks can go.
        runs.in_flight
            .retain(|serial, _| runs.pending.contains_key(serial));
        let serial = runs.next_serial;
        runs.next_serial += 1;
        runs.pending.insert(
            serial,
            cleanup.audit_entry(AuditOutcome::Abandoned, Some(ABANDONED_TEXT.to_owned())),
        );
        let runtime = cx.global::<ClusterRuntime>().clone();
        let config_dir = AppSettings::config_dir(cx).map(std::path::Path::to_path_buf);
        let (namespace, pod) = (cleanup.namespace().to_owned(), cleanup.pod().to_owned());
        let task = cx.spawn(async move |this, cx| {
            let outcome = run_cleanup(cleanup, &runtime, config_dir).await;
            let _ = this.update(cx, |shell, cx| {
                shell.cleanup_finished(serial, &namespace, &pod, outcome, cx);
            });
        });
        self.node_shell_runs.in_flight.insert(serial, task);
    }

    fn cleanup_finished(
        &mut self,
        serial: u64,
        namespace: &str,
        pod: &str,
        outcome: CleanupOutcome,
        cx: &mut Context<Self>,
    ) {
        self.node_shell_runs.pending.remove(&serial);
        if let CleanupOutcome::Failed(error) = outcome {
            // The sweep at the next session start offers the pod again.
            self.notify_later(
                format!("Could not delete node shell pod {namespace}/{pod}: {error}"),
                cx,
            );
        }
        self.close_window_when_idle(cx);
    }

    /// A close waits for the deletes. Once the last one reported, the window goes.
    pub(super) fn close_window_when_idle(&mut self, cx: &mut Context<Self>) {
        if !self.node_shell_runs.is_closing || !self.node_shell_runs.is_idle() {
            return;
        }
        let handle = self.window;
        cx.defer(move |cx| {
            let _ = cx.update_window(handle, |_, window, _| window.remove_window());
        });
    }

    /// `Window::on_window_should_close` of the main window: `true` when no node shell pod waits
    /// for its delete. Otherwise it starts every delete, says so, and returns `false`; the window
    /// closes itself when the last one reports (each bounded by the request timeout).
    pub(crate) fn main_window_may_close(&mut self, cx: &mut Context<Self>) -> bool {
        // A running drain is asked about first (spec 0034): leaving stops it, and its nodes stay
        // cordoned. The answer starts the close again, which then does not ask a second time.
        if !self.is_quit_confirmed {
            let drains = self.running_drain_names_of(&self.view.clusters(), cx);
            if !drains.is_empty() {
                let work = LeavingWork {
                    drains,
                    ..LeavingWork::default()
                };
                self.confirm_leaving(
                    work,
                    |shell, cx| {
                        shell.is_quit_confirmed = true;
                        shell.stop_all_drains_now(cx);
                        if shell.main_window_may_close(cx) {
                            let handle = shell.window;
                            cx.defer(move |cx| {
                                let _ = cx.update_window(handle, |_, window, _| {
                                    window.remove_window();
                                });
                            });
                        }
                    },
                    cx,
                );
                return false;
            }
        }
        if self.node_shell_runs.is_idle() {
            return true;
        }
        if !self.node_shell_runs.is_closing {
            self.node_shell_runs.is_closing = true;
            let waiting: Vec<_> = self.node_shell_runs.cleanups.drain().collect();
            for (_, cleanup) in waiting {
                self.begin_cleanup(cleanup, cx);
            }
            self.notify_later("Removing node shell pods…".to_owned(), cx);
        }
        false
    }

    /// The close of the main window by a path that does not ask the platform window (the Linux
    /// title-bar X): the window goes at once when no pod waits, else when the last delete reports.
    pub(crate) fn close_main_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.main_window_may_close(cx) {
            window.remove_window();
        }
    }

    /// The app is quitting by another path: GPUI waits at most 200 ms for what this returns. The
    /// waiting deletes start first, so every delete of the quit has its `abandoned` line; those lines
    /// are written synchronously, because the process may end before a delete reports, and a delete
    /// that does report appends its own line after.
    fn cleanup_for_quit(&mut self, cx: &mut Context<Self>) -> impl Future<Output = ()> + use<> {
        self.stop_all_drains_now(cx);
        let waiting: Vec<_> = self.node_shell_runs.cleanups.drain().collect();
        for (_, cleanup) in waiting {
            self.begin_cleanup(cleanup, cx);
        }
        if let Some(dir) = AppSettings::config_dir(cx).map(std::path::Path::to_path_buf) {
            for entry in self.node_shell_runs.pending.values() {
                if let Err(error) = append_audit(&dir, entry) {
                    tracing::warn!(kind = ?error.kind(), "could not append to the audit log");
                }
            }
        }
        let running = std::mem::take(&mut self.node_shell_runs.in_flight);
        async move {
            join_all(running.into_values()).await;
        }
    }

    /// A notice from a place without a window in hand (a finished delete, a close).
    pub(super) fn notify_later(&self, text: String, cx: &mut Context<Self>) {
        let handle = self.window;
        cx.defer(move |cx| {
            let _ = cx.update_window(handle, |_, window, cx| notify(window, cx, text));
        });
    }
}

#[cfg(test)]
#[path = "node_shell_cleanup_tests.rs"]
mod node_shell_cleanup_tests;
