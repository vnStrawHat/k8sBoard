//! Edit YAML on the shell side (spec 0031): opening the editor on the cursor row, the prompt before
//! unsaved text is thrown away, the gate of the dry-run, and what a commit leaves in the editor.
//!
//! A child of `app_shell`, like `write_flow`, because it reads the viewed slots and owns
//! `AppShell.edit`. Every step names the cluster of the edited object and takes its guard and its
//! connection from that slot, never from the primary.
//!
//! The editor view is being updated while it calls the shell (a button, Ctrl S), so nothing here
//! reads it from a method the view calls; the discard prompt reads it after the update ended.

use std::cell::RefCell;
use std::rc::Rc;

use cluster::{AccessCheck, ClusterConnection, ObjectKind, ObjectRef, WriteOutcome};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, IntoElement as _, ParentElement as _,
    SharedString, Window,
};

use super::write_flow::{CheckedWriteError, CommitMode, WriteIntent, WriteStep, notify};
use super::{AppShell, Screen};
use crate::cluster_registry::ClusterRef;
use crate::cluster_session::AccessState;
use crate::edit_quota::QuotaInput;
use crate::fresh_enter::FreshEnter;
use crate::issue_feeds::FeedState;
use crate::kind_row::KindObject;
use crate::object_create_view::ObjectCreateView;
use crate::object_templates::{template_namespace, template_text};
use crate::resource_actions::{
    ActionAvailability, CHECKING_PERMISSIONS, ResourceAction, RowAction, action_availability,
    action_label, subject_action, unavailable_text,
};
use crate::resource_kind::ResourceKind;
use crate::revision_history::HistoryInputs;
use crate::table_selection::ClusterObject;
use crate::values_edit::ValuesEditView;
use crate::yaml_edit::{EditFailure, EditSubject, YamlEditView, edit_failure_of};
use crate::yaml_view::object_ref;
/// How long `reveal_and_edit` waits for the permission review of a kind, in polls of this length.
const EDIT_GATE_POLL: std::time::Duration = std::time::Duration::from_millis(200);
const EDIT_GATE_POLLS: u32 = 25;

/// What runs once the user agreed to throw the unsaved text away.
type AfterDiscard = Box<dyn FnOnce(&mut AppShell, &mut Context<AppShell>)>;

/// The one open editor of the shell (`AppShell.edit`): Edit YAML, Edit values (spec 0047 decision 8),
/// or a New object (spec 0042). They share the slot, so one edit is open at a time and the discard
/// prompt, the leaving dialog, and the inert table keys work for all of them.
#[derive(Clone)]
pub(crate) enum OpenEdit {
    Yaml(Entity<YamlEditView>),
    Values(Entity<ValuesEditView>),
    Create(Entity<ObjectCreateView>),
}

impl OpenEdit {
    pub(crate) fn cluster<'a>(&'a self, cx: &'a App) -> &'a ClusterRef {
        match self {
            Self::Yaml(edit) => edit.read(cx).cluster(),
            Self::Values(edit) => edit.read(cx).cluster(),
            Self::Create(edit) => edit.read(cx).cluster(),
        }
    }

    /// The object being edited; a New object has none yet.
    pub(crate) fn object<'a>(&'a self, cx: &'a App) -> Option<&'a ObjectRef> {
        match self {
            Self::Yaml(edit) => Some(edit.read(cx).object()),
            Self::Values(edit) => Some(edit.read(cx).object()),
            Self::Create(_) => None,
        }
    }

    pub(crate) fn is_dirty(&self, cx: &App) -> bool {
        match self {
            Self::Yaml(edit) => edit.read(cx).is_dirty(),
            Self::Values(edit) => edit.read(cx).is_dirty(),
            Self::Create(edit) => edit.read(cx).is_dirty(),
        }
    }

    /// `Secret/payments/api-db`, or `new ConfigMap`: what the edit is called in the prompts.
    pub(crate) fn subject_text(&self, cx: &App) -> String {
        match self {
            Self::Yaml(edit) => edit.read(cx).subject_text(),
            Self::Values(edit) => edit.read(cx).subject_text(),
            Self::Create(edit) => edit.read(cx).subject_text(),
        }
    }

    /// The title of the discard prompt.
    pub(crate) fn discard_title(&self, cx: &App) -> String {
        match self {
            Self::Yaml(_) | Self::Values(_) => {
                format!("Discard changes to {}?", self.subject_text(cx))
            }
            Self::Create(edit) => format!("Discard the new {}?", edit.read(cx).kind().name()),
        }
    }

    /// The line of the leaving dialog about this edit.
    pub(crate) fn leaving_line(&self, cx: &App) -> String {
        match self {
            Self::Yaml(_) | Self::Values(_) => {
                format!("Unsaved changes to {}", self.subject_text(cx))
            }
            Self::Create(edit) => format!("Unsaved new {}", edit.read(cx).kind().name()),
        }
    }

    /// Shows a failed write in the editor, whichever one it is.
    pub(crate) fn commit_failed(&self, failure: EditFailure, cx: &mut App) {
        match self {
            Self::Yaml(edit) => edit.update(cx, |view, cx| view.commit_failed(failure, cx)),
            Self::Values(edit) => edit.update(cx, |view, cx| view.commit_failed(failure, cx)),
            Self::Create(edit) => edit.update(cx, |view, cx| view.commit_failed(failure, cx)),
        }
    }

    pub(crate) fn element(&self) -> AnyElement {
        match self {
            Self::Yaml(edit) => edit.clone().into_any_element(),
            Self::Values(edit) => edit.clone().into_any_element(),
            Self::Create(edit) => edit.clone().into_any_element(),
        }
    }

    #[cfg(test)]
    pub(crate) fn yaml(&self) -> Option<Entity<YamlEditView>> {
        match self {
            Self::Yaml(edit) => Some(edit.clone()),
            Self::Values(_) | Self::Create(_) => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn values(&self) -> Option<Entity<ValuesEditView>> {
        match self {
            Self::Values(edit) => Some(edit.clone()),
            Self::Yaml(_) | Self::Create(_) => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn create(&self) -> Option<Entity<ObjectCreateView>> {
        match self {
            Self::Create(edit) => Some(edit.clone()),
            Self::Yaml(_) | Self::Values(_) => None,
        }
    }
}

impl AppShell {
    /// Shows `object` on its screen with its drawer, then opens Edit YAML on it: the `Edit role/x…`
    /// link of Check permissions. The gate is read by `open_edit`, so a denied edit says why.
    pub(crate) fn reveal_and_edit(&mut self, object: ClusterObject, cx: &mut Context<Self>) {
        let subject = object.clone();
        self.reveal_then(object, cx, move |shell, cx| {
            let handle = shell.window;
            cx.spawn(async move |weak, cx| {
                // The screen the reveal showed has just asked for the write permissions of its
                // kind: edit once the answer is in, not on the `Checking permissions…` gate.
                for _ in 0..EDIT_GATE_POLLS {
                    let is_checking = weak
                        .read_with(cx, |shell, cx| shell.is_edit_gate_checking(&subject, cx))
                        .unwrap_or(false);
                    if !is_checking {
                        break;
                    }
                    cx.background_executor().timer(EDIT_GATE_POLL).await;
                }
                let _ = cx.update_window(handle, |_, window, cx| {
                    let _ = weak.update(cx, |shell, cx| shell.open_edit(subject, window, cx));
                });
            })
            .detach();
        });
    }

    /// Whether the permissions of Edit YAML on `subject` are still being asked.
    fn is_edit_gate_checking(&self, subject: &ClusterObject, cx: &App) -> bool {
        let Some(ResourceAction::EditYaml(kind)) =
            subject_action(RowAction::EditYaml, &subject.key)
        else {
            return false;
        };
        let Some(guard) = self.guard_for(&subject.cluster, cx) else {
            return false;
        };
        matches!(
            action_availability(ResourceAction::EditYaml(kind), &guard),
            ActionAvailability::Disabled { reason } if reason == CHECKING_PERMISSIONS
        )
    }

    /// Opens Edit YAML on `subject`, the cursor row, in its own cluster. The key, the menu item, and
    /// the palette entry all end here, after the gate said yes; the gate is read again because they
    /// may be a moment old.
    pub(crate) fn open_edit(
        &mut self,
        subject: ClusterObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::EditYaml(cluster::ObjectKind::Pod));
        // One edit at a time. The keys cannot reach here while one is open, and a click on a row is
        // not possible behind the editor.
        if self.edit.is_some() {
            return;
        }
        let (Some(ResourceAction::EditYaml(kind)), Some(object)) = (
            subject_action(RowAction::EditYaml, &subject.key),
            object_ref(&subject.key),
        ) else {
            notify(
                window,
                cx,
                unavailable_text(label, "this object cannot be edited here"),
            );
            return;
        };
        let name = {
            let Some(guard) = self.guard_for(&subject.cluster, cx) else {
                notify(
                    window,
                    cx,
                    unavailable_text(label, "the cluster is not open"),
                );
                return;
            };
            if let ActionAvailability::Disabled { reason } =
                action_availability(ResourceAction::EditYaml(kind), &guard)
            {
                notify(window, cx, unavailable_text(label, &reason));
                return;
            }
            SharedString::from(guard.display_name().to_owned())
        };
        let connection = self.connection_of(&subject.cluster, cx);
        self.close_value_popover(cx);
        let shell = cx.weak_entity();
        let edit = cx.new(|cx| {
            let subject = EditSubject {
                target: subject,
                cluster_name: name,
                object,
                kind,
            };
            YamlEditView::new(shell, subject, connection, window, cx)
        });
        self.edit = Some(OpenEdit::Yaml(edit));
        cx.notify();
    }

    /// Opens the New view for `kind` on the active cluster, with the kind's template for the first
    /// namespace of the scope. The header button ends here, after the gate said yes; the gate is read
    /// again because the button may be a moment old.
    pub(crate) fn open_create(
        &mut self,
        kind: ObjectKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::CreateObject(kind));
        // One edit at a time: the header buttons are behind the editor while one is open.
        if self.edit.is_some() {
            return;
        }
        let Some(cluster) = self.active_cluster() else {
            return;
        };
        let (name, template) = {
            let (Some(guard), Some(live)) =
                (self.guard_for(&cluster, cx), self.live_of(&cluster, cx))
            else {
                notify(
                    window,
                    cx,
                    unavailable_text(label, "the cluster is not open"),
                );
                return;
            };
            if let ActionAvailability::Disabled { reason } =
                action_availability(ResourceAction::CreateObject(kind), &guard)
            {
                notify(window, cx, unavailable_text(label, &reason));
                return;
            }
            let namespace = template_namespace(&live.scope);
            (
                SharedString::from(guard.display_name().to_owned()),
                template_text(kind, namespace),
            )
        };
        let Some(template) = template else {
            notify(
                window,
                cx,
                unavailable_text(label, "this object cannot be created here"),
            );
            return;
        };
        self.close_value_popover(cx);
        let shell = cx.weak_entity();
        let view =
            cx.new(|cx| ObjectCreateView::new(shell, cluster, name, kind, template, window, cx));
        view.update(cx, |view, cx| view.focus_editor(window, cx));
        self.edit = Some(OpenEdit::Create(view));
        cx.notify();
    }

    /// The connection a command of the open edit reads or writes over.
    pub(crate) fn edit_connection(
        &self,
        cluster: &ClusterRef,
        cx: &App,
    ) -> Option<ClusterConnection> {
        self.connection_of(cluster, cx)
    }

    /// What the Revision history tab of an edit of `object` needs: the connection, the permission
    /// to list ReplicaSets, and the Deployment's selector, all from the session of its cluster.
    /// A review that is still running or failed does not block the list: it shows its own error.
    pub(crate) fn history_inputs(
        &self,
        cluster: &ClusterRef,
        object: &ObjectRef,
        cx: &App,
    ) -> HistoryInputs {
        let Some(live) = self.live_of(cluster, cx) else {
            return HistoryInputs::Unavailable("the cluster is not open".into());
        };
        if let AccessState::Known(report) = &live.access
            && !report.is_allowed(AccessCheck::ListReplicaSets)
        {
            return HistoryInputs::Denied;
        }
        let selector = object
            .namespace()
            .and_then(|namespace| live.deployment_selector(namespace, object.name()));
        match selector {
            Some(selector) => HistoryInputs::Ready {
                connection: live.connection().clone(),
                selector,
            },
            None => HistoryInputs::Unavailable("the deployment is not loaded yet".into()),
        }
    }

    /// What the session's ResourceQuotas feed says about the quotas of `namespace`, for the quota
    /// line of a passed preview. The feed already runs for the session scope, so this sends nothing.
    pub(crate) fn quota_input(
        &self,
        cluster: &ClusterRef,
        namespace: &str,
        cx: &App,
    ) -> QuotaInput {
        let Some(live) = self.live_of(cluster, cx) else {
            return QuotaInput::Off("the cluster is not open".to_owned());
        };
        let Some(feed) = live.issue_feeds.condition(ResourceKind::ResourceQuotas) else {
            // A session without the ResourceQuotas feed does not watch quotas.
            return QuotaInput::Off("quotas are not watched".to_owned());
        };
        match feed.state() {
            FeedState::Off(reason) => QuotaInput::Off(reason),
            FeedState::Loading => QuotaInput::Loading,
            FeedState::Live | FeedState::Limited(_) => QuotaInput::Quotas(
                feed.list
                    .items()
                    .iter()
                    .filter_map(|object| match object {
                        KindObject::ResourceQuota(quota) if quota.namespace == namespace => {
                            Some(quota.clone())
                        }
                        _ => None,
                    })
                    .collect(),
            ),
        }
    }

    /// The gate and the session of a dry-run of `intent`'s change: nothing is sent when the action
    /// is off (a lock, a missing permission) or the cluster is gone. The dry-run itself is
    /// `checked_write` with this step, which audits nothing.
    pub(crate) fn begin_preview(
        &self,
        intent: Rc<WriteIntent>,
        cx: &App,
    ) -> Result<WriteStep, SharedString> {
        let Some(guard) = self.guard_for(&intent.cluster, cx) else {
            return Err(format!("{} is not open", intent.cluster_name).into());
        };
        if let ActionAvailability::Disabled { reason } = action_availability(intent.action, &guard)
        {
            return Err(unavailable_text(&intent.button, &reason).into());
        }
        Ok(WriteStep {
            generation: guard.generation,
            intent,
            mode: CommitMode::DryRun,
            note: None,
        })
    }

    /// Cancel: leaves the editor, after asking when the text has unsaved changes.
    pub(crate) fn cancel_edit(&mut self, is_dirty: bool, cx: &mut Context<Self>) {
        if is_dirty {
            self.ask_discard(|shell, cx| shell.close_edit(cx), cx);
        } else {
            self.close_edit(cx);
        }
    }

    /// Closes the editor and gives the table the keyboard back. The cursor and the drawer were kept
    /// under it, so the workspace is as it was.
    pub(crate) fn close_edit(&mut self, cx: &mut Context<Self>) {
        if self.edit.take().is_none() {
            return;
        }
        let (handle, shell) = (self.window, cx.weak_entity());
        cx.defer(move |cx| {
            let _ = cx.update_window(handle, |_, window, cx| {
                let _ = shell.update(cx, |shell, cx| shell.focus_visible_table(window, cx));
            });
        });
        cx.notify();
    }

    /// `Unsaved changes to Deployment/payments/api` (or `Unsaved new ConfigMap`) when the open edit
    /// belongs to one of `leaving` and holds changes; a clean edit just closes with its cluster.
    pub(super) fn unsaved_edit_of(&self, leaving: &[ClusterRef], cx: &App) -> Option<String> {
        let edit = self.edit.as_ref()?;
        (edit.is_dirty(cx) && leaving.contains(edit.cluster(cx))).then(|| edit.leaving_line(cx))
    }

    /// Whether the open edit holds text that was not applied.
    pub(super) fn has_unsaved_edit(&self, cx: &App) -> bool {
        self.edit.as_ref().is_some_and(|edit| edit.is_dirty(cx))
    }

    /// A call that would leave the editor (another screen, a reveal, a namespace change). With
    /// unsaved text it asks first and runs `again` once the user discards; with a clean editor it
    /// closes it and lets the call go on. `true` means the call was parked behind the prompt.
    pub(super) fn parks_for_discard(
        &mut self,
        again: impl FnOnce(&mut AppShell, &mut Context<AppShell>) + 'static,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.has_unsaved_edit(cx) {
            self.ask_discard(again, cx);
            return true;
        }
        self.close_edit(cx);
        false
    }

    /// `Discard changes to {name}?` with `Discard` and `Keep editing`; Esc, `Keep editing`, or a
    /// click outside keeps the text. The dialog opens after the current update, which owns the
    /// window; a fresh Enter confirms, a held one never does.
    pub(super) fn ask_discard(
        &mut self,
        then: impl FnOnce(&mut AppShell, &mut Context<AppShell>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let then: Rc<RefCell<Option<AfterDiscard>>> = Rc::new(RefCell::new(Some(Box::new(then))));
        let (handle, shell) = (self.window, cx.weak_entity());
        cx.defer(move |cx| {
            let asked = shell
                .read_with(cx, |shell, cx| {
                    shell
                        .edit
                        .as_ref()
                        .map(|edit| (edit.subject_text(cx), edit.discard_title(cx)))
                })
                .ok()
                .flatten();
            #[cfg(test)]
            if let Some((subject, _)) = &asked {
                let _ = shell.update(cx, |shell, _| shell.last_discard = Some(subject.clone()));
            }
            let Some((_, title)) = asked else {
                return;
            };
            let _ = cx.update_window(handle, |_, window, cx| {
                let discard: Rc<dyn Fn(&mut App)> = Rc::new(move |cx| {
                    let then = then.borrow_mut().take();
                    if let Some(then) = then {
                        // The text is thrown away first, or the call would ask again.
                        let _ = shell.update(cx, |shell, cx| {
                            shell.close_edit(cx);
                            then(shell, cx);
                        });
                    }
                });
                let on_enter = Rc::clone(&discard);
                let text = SharedString::from("Your changes to this object are lost.");
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
                let title = SharedString::from(title);
                window.open_alert_dialog(cx, move |alert, _, _| {
                    let discard = Rc::clone(&discard);
                    alert
                        .title(title.clone())
                        .child(body.clone())
                        .confirm()
                        .button_props(
                            DialogButtonProps::default()
                                .ok_text("Discard")
                                .ok_variant(ButtonVariant::Danger)
                                .cancel_text("Keep editing")
                                .show_cancel(true),
                        )
                        .on_ok(move |_, _, cx| {
                            discard(cx);
                            true
                        })
                });
            });
        });
    }

    /// A commit of the edit finished: success closes the editor, and a failure that the editor can
    /// show (a conflict, an invalid field, a deleted object) is shown in place. The dialog is gone
    /// by now, and the write flow has already audited the commit.
    pub(super) fn edit_commit_finished(
        &mut self,
        intent: &WriteIntent,
        result: &Result<WriteOutcome, CheckedWriteError>,
        cx: &mut Context<Self>,
    ) {
        let Some(OpenEdit::Yaml(edit)) = self.edit.clone() else {
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
                let failure: EditFailure = edit_failure_of(error);
                edit.update(cx, |view, cx| view.commit_failed(failure, cx));
            }
        }
    }

    /// A Roll back went through. The Edit YAML view of that Deployment (its Revision history tab
    /// offers the button) holds the old text, resourceVersion and current revision, so it closes.
    /// An editor with unsaved text stays: closing it would drop what the user typed.
    pub(super) fn roll_back_finished(&mut self, intent: &WriteIntent, cx: &mut Context<Self>) {
        let Some(OpenEdit::Yaml(edit)) = &self.edit else {
            return;
        };
        let view = edit.read(cx);
        let is_stale_copy = view.cluster() == &intent.cluster
            && view.object() == intent.request.target()
            && !view.is_dirty();
        if is_stale_copy {
            self.close_edit(cx);
        }
    }

    /// A commit of the New view finished (spec 0042): success closes the view and shows the kind's
    /// screen, where the watch adds the new row (no cursor move, decision 11); a failure is shown
    /// in the view. The success notice comes from the write flow's own arm, so nothing is pushed
    /// here.
    pub(super) fn create_commit_finished(
        &mut self,
        intent: &WriteIntent,
        result: &Result<WriteOutcome, CheckedWriteError>,
        cx: &mut Context<Self>,
    ) {
        let Some(OpenEdit::Create(view)) = self.edit.clone() else {
            return;
        };
        // Another view may have been opened since; this commit's result is not for it.
        let is_ours = {
            let view = view.read(cx);
            view.cluster() == &intent.cluster
                && intent.action == ResourceAction::CreateObject(view.kind())
        };
        if !is_ours {
            return;
        }
        match result {
            Ok(_) => {
                let kind = view.read(cx).kind();
                self.close_edit(cx);
                let screen = ResourceKind::ALL
                    .into_iter()
                    .find(|resource| resource.builtin_object() == Some(kind))
                    .map(Screen::Kind);
                // The `New` button is on the kind's own screen, so this is normally the screen
                // already; showing it again would clear the cursor.
                if let Some(screen) = screen
                    && self.screen != screen
                {
                    self.show_screen(screen, cx);
                }
            }
            Err(error) => {
                let failure: EditFailure = edit_failure_of(error);
                view.update(cx, |view, cx| view.commit_failed(failure, cx));
            }
        }
    }

    /// Whether an edit is open: the keys that move the hidden cursor do nothing then.
    pub(crate) fn is_editing(&self) -> bool {
        self.edit.is_some()
    }
}

#[cfg(feature = "screenshot")]
impl AppShell {
    /// `--screen edit-yaml-diff` and `edit-yaml-history`: W10 from fixed data, over a fixed cluster.
    /// It waits for no cluster and sends nothing.
    pub(super) fn open_edit_fixture(
        &mut self,
        tab: crate::yaml_edit::EditTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::screenshot::shell_fixture_target;
        use crate::table_selection::ResourceKey;
        let target = ClusterObject::new(
            shell_fixture_target().cluster,
            ResourceKey::Kind {
                kind: crate::resource_kind::ResourceKind::Deployments,
                namespace: Some("payments".to_owned()),
                name: "api".to_owned(),
            },
        );
        let Some(object) = cluster::ObjectRef::new(
            cluster::ObjectKind::Deployment,
            Some("payments".to_owned()),
            "api".to_owned(),
        ) else {
            return;
        };
        let shell = cx.weak_entity();
        let subject = EditSubject {
            target,
            cluster_name: crate::screenshot::SHELL_FIXTURE_CLUSTER.into(),
            object,
            kind: cluster::ObjectKind::Deployment,
        };
        let edit = cx.new(|cx| YamlEditView::fixture(shell, subject, tab, window, cx));
        self.edit = Some(OpenEdit::Yaml(edit));
        cx.notify();
    }
}

#[cfg(feature = "screenshot")]
impl AppShell {
    /// `--screen new-config-map`: the New view of a ConfigMap from fixed data, over a fixed
    /// cluster. It waits for no cluster and sends nothing.
    pub(super) fn open_create_fixture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::screenshot::{SHELL_FIXTURE_CLUSTER, shell_fixture_target};
        let shell = cx.weak_entity();
        let cluster = shell_fixture_target().cluster;
        let view = cx.new(|cx| {
            ObjectCreateView::fixture(shell, cluster, SHELL_FIXTURE_CLUSTER.into(), window, cx)
        });
        self.edit = Some(OpenEdit::Create(view));
        cx.notify();
    }
}
