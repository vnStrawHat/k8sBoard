//! Edit values on the shell side (spec 0047): opening the editor on the cursor ConfigMap or Secret
//! and what a commit leaves in it.
//!
//! A child of `app_shell`, like `edit_yaml_flow`, because it reads the viewed slots and owns the
//! shared edit slot (`AppShell.edit`). Every step names the cluster of the edited object and takes
//! its guard and its connection from that slot. Nothing here logs, and nothing here holds a value.

use cluster::{ObjectKind, WriteOutcome};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, AppContext as _, Context, SharedString, WeakEntity, Window};

use super::AppShell;
use super::edit_yaml_flow::OpenEdit;
use super::write_flow::{CheckedWriteError, WriteIntent, notify};
use crate::cluster_registry::ClusterRef;
use crate::kind_join::env_consumers;
use crate::kind_row::KindObject;
use crate::resource_actions::{
    ActionAvailability, ResourceAction, RowAction, action_availability, action_label,
    subject_action, unavailable_text, values_edit_block,
};
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::values_edit::{ValuesEditView, ValuesSubject};
use crate::yaml_edit::{EditSubject, edit_failure_of};
use crate::yaml_view::object_ref;

impl AppShell {
    /// Opens Edit values on `subject`, the cursor ConfigMap or Secret, in its own cluster. The key,
    /// the menu item, and the palette entry all end here, after the gate said yes; the gate and the
    /// row are read again because they may be a moment old.
    pub(crate) fn open_values_edit(
        &mut self,
        subject: ClusterObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::EditValues(cluster::ObjectKind::Secret));
        // One edit at a time, whichever editor it is.
        if self.edit.is_some() {
            return;
        }
        let (Some(ResourceAction::EditValues(kind)), Some(object)) = (
            subject_action(RowAction::EditValues, &subject.key),
            object_ref(&subject.key),
        ) else {
            notify(
                window,
                cx,
                unavailable_text(label, "these values cannot be edited here"),
            );
            return;
        };
        let (name, secret_type) = {
            let Some(guard) = self.guard_for(&subject.cluster, cx) else {
                notify(
                    window,
                    cx,
                    unavailable_text(label, "the cluster is not open"),
                );
                return;
            };
            if let ActionAvailability::Disabled { reason } =
                action_availability(ResourceAction::EditValues(kind), &guard)
            {
                notify(window, cx, unavailable_text(label, &reason));
                return;
            }
            let row = self
                .live_of(&subject.cluster, cx)
                .and_then(|live| live.row_of(&subject.key));
            if let Some(reason) = row.and_then(|row| values_edit_block(&row.object)) {
                notify(window, cx, unavailable_text(label, &reason));
                return;
            }
            let secret_type = row.and_then(|row| match &row.object {
                KindObject::Secret(secret) => Some(SharedString::from(secret.secret_type.clone())),
                _ => None,
            });
            (
                SharedString::from(guard.display_name().to_owned()),
                secret_type,
            )
        };
        let connection = self.connection_of(&subject.cluster, cx);
        let access = self.secret_value_access();
        self.close_value_popover(cx);
        let shell = cx.weak_entity();
        let edit = cx.new(|cx| {
            let subject = ValuesSubject {
                edit: EditSubject {
                    target: subject,
                    cluster_name: name,
                    object,
                    kind,
                },
                secret_type,
            };
            ValuesEditView::new(shell, subject, access, connection, window, cx)
        });
        self.edit = Some(OpenEdit::Values(edit));
        cx.notify();
    }

    /// Apply of the editor `open_id` starts a write: its result is for that editor only.
    pub(crate) fn note_values_commit(&mut self, open_id: u64) {
        self.values_commit_open = Some(open_id);
    }

    /// A commit of the values edit finished, or its dry-run hit a conflict: success closes the
    /// editor, and a failure that the editor can show (a conflict, an invalid key, a deleted object)
    /// is shown in place. The dialog is gone by now, and the write flow has already audited the commit.
    pub(crate) fn values_commit_finished(
        &mut self,
        intent: &WriteIntent,
        result: &Result<WriteOutcome, CheckedWriteError>,
        cx: &mut Context<Self>,
    ) {
        let submitted = self.values_commit_open.take();
        let Some(OpenEdit::Values(edit)) = self.edit.clone() else {
            return;
        };
        // Another editor may have been opened since, on the same object; this commit is not for it.
        let is_ours = {
            let view = edit.read(cx);
            submitted == Some(view.open_id())
                && view.cluster() == &intent.cluster
                && view.object() == intent.request.target()
        };
        if !is_ours {
            return;
        }
        match result {
            Ok(_) => self.close_edit(cx),
            Err(error) => {
                let failure = edit_failure_of(error);
                edit.update(cx, |view, cx| view.commit_failed(failure, cx));
            }
        }
    }
}

/// The notice of a commit: `Updated 3 keys of Secret api-db`. The count and the object name, never a
/// key or a value.
pub(crate) fn values_success_notice(kind_name: &str, name: &str, count: usize) -> String {
    let unit = if count == 1 { "key" } else { "keys" };
    format!("Updated {count} {unit} of {kind_name} {name}")
}

/// The workloads that read the edited ConfigMap or Secret through env, once an Edit values commit
/// went through: the pods list the drawer's Used by reads decides. Empty for any other write, a
/// failed commit, or while that list is not loaded.
pub(super) fn env_consumers_after(
    shell: &WeakEntity<AppShell>,
    intent: &WriteIntent,
    is_success: bool,
    cx: &mut App,
) -> Vec<(ObjectKind, ResourceKey)> {
    let ResourceAction::EditValues(kind) = intent.action else {
        return Vec::new();
    };
    if !is_success {
        return Vec::new();
    }
    let target = intent.request.target();
    let Some(namespace) = target.namespace() else {
        return Vec::new();
    };
    shell
        .read_with(cx, |shell, cx| {
            let pods = shell.live_of(&intent.cluster, cx)?.pods.ready_items()?;
            Some(env_consumers(pods, kind, namespace, target.name()))
        })
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// `Restart 3 consumers` (a count of one reads `Restart 1 consumer`).
fn restart_consumers_label(count: usize) -> String {
    let unit = if count == 1 { "consumer" } else { "consumers" };
    format!("Restart {count} {unit}")
}

/// The success notice of a commit with a Restart button: the workloads that read the object
/// through env keep the old value until restarted. The button opens the Restart rollout batch of
/// each workload kind, which asks for its own confirmation.
pub(super) fn notify_with_restart(
    window: &mut Window,
    cx: &mut App,
    text: String,
    shell: &WeakEntity<AppShell>,
    cluster: &ClusterRef,
    consumers: Vec<(ObjectKind, ResourceKey)>,
) {
    let (shell, cluster) = (shell.clone(), cluster.clone());
    let notification = Notification::success(text).action(move |_, _, cx| {
        let (shell, cluster, consumers) = (shell.clone(), cluster.clone(), consumers.clone());
        Button::new("restart-consumers")
            .label(restart_consumers_label(consumers.len()))
            .small()
            .outline()
            .on_click(cx.listener(move |notification, _, window, cx| {
                notification.dismiss(window, cx);
                let (cluster, consumers) = (cluster.clone(), consumers.clone());
                let _ = shell.update(cx, |shell, cx| {
                    shell.restart_consumers(&cluster, &consumers, window, cx);
                });
            }))
    });
    window.push_notification(notification, cx);
}

#[cfg(feature = "screenshot")]
impl AppShell {
    /// `--screen values-edit`: the editor from fixed data over a fixed cluster. It waits for no
    /// cluster and sends nothing; every value is fixture text and every Secret field is masked.
    pub(super) fn open_values_fixture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::screenshot::{SHELL_FIXTURE_CLUSTER, shell_fixture_target};
        use crate::table_selection::ResourceKey;
        let target = ClusterObject::new(
            shell_fixture_target().cluster,
            ResourceKey::Kind {
                kind: crate::resource_kind::ResourceKind::Secrets,
                namespace: Some("payments".to_owned()),
                name: "api-db-credentials".to_owned(),
            },
        );
        let Some(object) = cluster::ObjectRef::new(
            cluster::ObjectKind::Secret,
            Some("payments".to_owned()),
            "api-db-credentials".to_owned(),
        ) else {
            return;
        };
        let shell = cx.weak_entity();
        let subject = ValuesSubject {
            edit: EditSubject {
                target,
                cluster_name: SHELL_FIXTURE_CLUSTER.into(),
                object,
                kind: cluster::ObjectKind::Secret,
            },
            secret_type: Some("Opaque".into()),
        };
        let access = self.secret_value_access();
        let edit = cx.new(|cx| ValuesEditView::fixture(shell, subject, access, window, cx));
        self.edit = Some(OpenEdit::Values(edit));
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::restart_consumers_label;

    #[test]
    fn the_restart_button_counts_consumers() {
        assert_eq!(restart_consumers_label(1), "Restart 1 consumer");
        assert_eq!(restart_consumers_label(3), "Restart 3 consumers");
    }
}
