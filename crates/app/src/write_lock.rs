//! The read-only lock of each viewed cluster (spec 0030): the toggle behind the title-bar badge
//! and Ctrl Shift R, the unlock dialog, and the audit lines of both.
//!
//! A child of `app_shell`, like `write_flow`. The lock lives on each cluster's own session, so
//! every function here names the cluster it acts on.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{AppContext as _, Context, Window};

use super::AppShell;
use crate::audit_log::lock_entry;
use crate::cluster_registry::ClusterRef;
use crate::confirm_dialog::{ConfirmDialog, DialogInputs, DialogKind};
use crate::write_guard::{ActionRisk, WriteLock, confirm_step};

impl AppShell {
    /// Ctrl Shift R: the lock of the open cluster, which holds the cursor row and the drawer too.
    pub(crate) fn toggle_open_cluster_lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(cluster) = self.active_cluster() {
            self.toggle_write_lock(&cluster, window, cx);
        }
    }

    /// Toggles the lock of `cluster` for this session only: locking is immediate, and unlocking
    /// asks the cluster's own confirm tier. Nothing is written to the settings.
    pub(crate) fn toggle_write_lock(
        &mut self,
        cluster: &ClusterRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The lock is offered only on a live session: the audit line names its guard, and a
        // session that has not connected has nothing to lock yet.
        let Some((lock, name)) = self
            .guard_for(cluster, cx)
            .map(|guard| (guard.lock, guard.display_name().to_owned()))
        else {
            let name = self
                .label_of(cluster)
                .unwrap_or_else(|| cluster.context.clone());
            let text = format!("{name} is not connected yet");
            window.push_notification(Notification::warning(text), cx);
            return;
        };
        match lock {
            WriteLock::Unlocked => {
                self.set_write_lock(cluster, WriteLock::Locked, cx);
                // The badge may show another cluster's state, so the notice names this one.
                let notice = Notification::info(read_only_notice(&name));
                window.push_notification(notice, cx);
            }
            WriteLock::Locked => self.begin_unlock(cluster, window, cx),
        }
    }

    /// Asks to unlock `cluster`: the same dialog as a change, titled `Unlock {cluster} for changes?`,
    /// with no object and no dry-run.
    pub(super) fn begin_unlock(
        &mut self,
        cluster: &ClusterRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let inputs = {
            let Some(guard) = self.guard_for(cluster, cx) else {
                let name = self
                    .label_of(cluster)
                    .unwrap_or_else(|| cluster.context.clone());
                let text = format!("{name} is not connected yet");
                window.push_notification(Notification::warning(text), cx);
                return;
            };
            DialogInputs {
                shell: cx.weak_entity(),
                kind: DialogKind::Unlock {
                    cluster: cluster.clone(),
                    cluster_name: guard.display_name().to_owned().into(),
                },
                confirm: confirm_step(
                    guard.profile.confirm,
                    ActionRisk::Change,
                    guard.display_name(),
                ),
                environment: guard.profile.environment,
                generation: guard.generation,
            }
        };
        let dialog = cx.new(|cx| ConfirmDialog::new(inputs, window, cx));
        #[cfg(test)]
        {
            self.last_dialog = Some(dialog.downgrade());
        }
        ConfirmDialog::open(&dialog, window, cx);
    }

    /// The confirmed unlock of the dialog. The cluster must still be open on the connection the
    /// dialog was opened on.
    pub(crate) fn finish_unlock(
        &mut self,
        cluster: &ClusterRef,
        generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_same_connection = self
            .guard_for(cluster, cx)
            .is_some_and(|guard| guard.generation == generation);
        if !is_same_connection {
            let name = self
                .label_of(cluster)
                .unwrap_or_else(|| cluster.context.clone());
            let text = format!("{name} is no longer open; nothing was changed");
            window.push_notification(Notification::warning(text), cx);
            return;
        }
        self.set_write_lock(cluster, WriteLock::Unlocked, cx);
    }

    /// Sets the lock of `cluster`'s own session and appends the audit line of the change. The lock
    /// of a session that is not live yet is not offered, so there is always a guard to name.
    fn set_write_lock(&mut self, cluster: &ClusterRef, lock: WriteLock, cx: &mut Context<Self>) {
        let Some(session) = self.session_of(cluster).cloned() else {
            return;
        };
        session.update(cx, |session, cx| session.set_lock(lock, cx));
        let entry = self
            .guard_for(cluster, cx)
            .map(|guard| lock_entry(&guard, lock));
        cx.notify();
        if let Some(entry) = entry {
            self.write_audit_line(entry, cx);
        }
    }
}

/// What a lock says in the notice: it names the cluster, because the badge may be showing another.
fn read_only_notice(name: &str) -> String {
    format!("{name} is read-only")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_notice_names_the_cluster() {
        assert_eq!(read_only_notice("stg-b"), "stg-b is read-only");
    }
}
