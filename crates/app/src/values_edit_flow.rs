//! Edit values on the shell side (spec 0047): opening the editor on the cursor ConfigMap or Secret
//! and what a commit leaves in it.
//!
//! A child of `app_shell`, like `edit_yaml_flow`, because it reads the viewed slots and owns the
//! shared edit slot (`AppShell.edit`). Every step names the cluster of the edited object and takes
//! its guard and its connection from that slot. Nothing here logs, and nothing here holds a value.

use cluster::WriteOutcome;
use gpui_kit::{AppContext as _, Context, SharedString, Window};

use super::AppShell;
use super::edit_yaml_flow::OpenEdit;
use super::write_flow::{CheckedWriteError, WriteIntent, notify};
use crate::kind_row::KindObject;
use crate::resource_actions::{
    ActionAvailability, ResourceAction, RowAction, action_availability, action_label,
    subject_action, unavailable_text, values_edit_block,
};
use crate::table_selection::ClusterObject;
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
                .slot_live(&subject.cluster, cx)
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
        let connection = self.slot_connection(&subject.cluster, cx);
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

    /// A commit of the values edit finished: success closes the editor, and a failure that the
    /// editor can show (a conflict, an invalid key, a deleted object) is shown in place. The dialog
    /// is gone by now, and the write flow has already audited the commit.
    pub(super) fn values_commit_finished(
        &mut self,
        intent: &WriteIntent,
        result: &Result<WriteOutcome, CheckedWriteError>,
        cx: &mut Context<Self>,
    ) {
        let Some(OpenEdit::Values(edit)) = self.edit.clone() else {
            return;
        };
        // Another edit may have been opened since; this commit's result is not for it.
        let is_ours = {
            let view = edit.read(cx);
            view.cluster() == &intent.cluster && view.object() == intent.request.target()
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
