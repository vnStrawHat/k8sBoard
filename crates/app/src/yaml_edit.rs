//! The Edit YAML view (spec 0031, wireframe W10): the object in the kit code editor, a Diff tab
//! from the server's dry-run, a side panel of changes and checks, and the Apply flow. It replaces the
//! table and the drawer in the workspace while it is open.
//!
//! The editor holds the masked YAML of the cluster crate (`<hidden>` means "keep the server's
//! value"), so the app never holds a raw secret. Nothing here logs, writes to disk, or sends a
//! request itself: the preview and the commit go through `checked_write`, the dialog, and the audit
//! line of the write flow, always on the cluster of the edited object.

use std::rc::Rc;
use std::time::Duration;

use cluster::{
    ClusterConnection, EditBase, EditCheck, EditError, EditPreview, EnvValues, FieldChange,
    FieldPath, HELM_MANAGED_WARNING, ObjectEdit, ObjectKind, ObjectRef, WriteEffect, WriteError,
    WriteOperation, WriteOutcome, WriteRequest, format_yaml, rebase,
};
use gpui_kit::component::input::{
    EditorState, InputEvent, Position, RangeDecoration, RangeDecorationCollection,
    RangeDecorationStyle,
};
use gpui_kit::{
    AppContext as _, Context, Entity, FocusHandle, Focusable, KeyDownEvent, ListAlignment,
    ListState, SharedString, Subscription, Task, WeakEntity, Window, px,
};

use crate::app_shell::AppShell;
use crate::app_shell::write_flow::{CheckedWriteError, WriteIntent, checked_write};
use crate::audit_log::AuditField;
use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::error_text;
use crate::edit_error_line::{line_byte_range, line_of_field, local_error_line};
use crate::edit_quota::{QuotaLine, quota_line};
use crate::resource_actions::ResourceAction;
use crate::revision_diff::RollBackOffer;
use crate::revision_history::{HistoryInputs, RevisionHistory};
use crate::status_tone::{StatusTone, tone_color};
use crate::table_selection::ClusterObject;
use crate::write_guard::ActionRisk;
use crate::yaml_diff::{DiffRow, diff_rows};

/// How strongly the line of a failure is tinted by the danger token.
const ERROR_LINE_TINT: f32 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditTab {
    Editor,
    Diff,
    /// The Revision history of a Deployment (spec 0041).
    History,
}

/// Where the first read of the object stands.
enum LoadState {
    Loading { _task: Task<()> },
    Ready,
    Failed(SharedString),
}

/// What the server's dry-run says about the text the editor held when it was asked.
pub(crate) enum PreviewState {
    NotChecked,
    /// Dropping the task cancels the check; Ctrl S does nothing meanwhile (decision 15).
    Running {
        _task: Task<()>,
    },
    Passed(Box<PassedPreview>),
    Failed(PreviewFailure),
}

/// A dry-run that passed. It is the display form of the cluster crate's `EditPreview`, which never
/// leaves this module's call: it holds masked values and paths, and it reaches neither the audit
/// nor a notification (decision 9).
pub(crate) struct PassedPreview {
    /// The editor text the check was for; a different text makes the preview stale.
    pub(crate) for_text: SharedString,
    /// The request the second press of Apply hands to the confirm dialog. `None` in a fixture.
    pub(crate) request: Option<WriteRequest>,
    pub(crate) changes: Vec<ChangeLine>,
    /// Changes beyond the cap of `changes`.
    pub(crate) more_changes: usize,
    pub(crate) checks: Vec<SharedString>,
    /// What the namespace quotas say about the pods and resources the change adds (advisory).
    pub(crate) quota: QuotaLine,
    pub(crate) rows: Vec<DiffRow>,
    pub(crate) elapsed: Duration,
}

/// One changed field in the side panel. A missing side reads as an em dash.
pub(crate) struct ChangeLine {
    pub(crate) path: SharedString,
    pub(crate) old: Option<SharedString>,
    pub(crate) new: Option<SharedString>,
}

impl ChangeLine {
    fn of(change: &FieldChange) -> Self {
        Self {
            path: change.path.to_string().into(),
            old: change.old.clone().map(Into::into),
            new: change.new.clone().map(Into::into),
        }
    }
}

pub(crate) enum PreviewFailure {
    /// The text is not an applicable edit; nothing was sent.
    Local(EditError),
    /// A 422 or an admission refusal: the side panel lists the field paths exactly as the server
    /// spelled them.
    Invalid {
        message: SharedString,
        fields: Vec<SharedString>,
    },
    Server(SharedString),
}

/// A condition of the object itself, shown over the editor.
pub(crate) enum EditBanner {
    /// A 409, or a newer `resourceVersion` than the one the edit started from.
    Conflict,
    /// The edit was moved onto the newer object; these paths had no place on it.
    Rebased {
        unreachable: Vec<SharedString>,
        /// Where the user's value replaced a change made on the server.
        overwritten: Vec<SharedString>,
    },
    /// A rebase found another object under the name: the text was for the deleted one. Only
    /// Discard is offered.
    Recreated,
    /// A 404: the text is kept but cannot be applied.
    Deleted,
    /// A commit whose request may have left the client before it failed.
    OutcomeUnknown,
}

/// Why Apply is off after the check refused a Secret value: the tooltip of the button.
const SECRET_VALUES_REASON: &str = "Secret values are changed with Edit values";

/// How a failed write is told in the editor, whichever request failed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EditFailure {
    Conflict,
    Deleted,
    OutcomeUnknown,
    Invalid {
        message: SharedString,
        fields: Vec<SharedString>,
    },
    /// A 429: nothing changed, and the check can be run again.
    Refused(SharedString),
    /// The write flow stopped it (a lock, a missing permission, a reconnect) or another error.
    Other(SharedString),
}

pub(crate) fn edit_failure_of(error: &CheckedWriteError) -> EditFailure {
    match error {
        CheckedWriteError::Blocked(text) => EditFailure::Other(text.clone()),
        CheckedWriteError::Write(error) => match error {
            WriteError::Conflict { .. } => EditFailure::Conflict,
            WriteError::NotFound => EditFailure::Deleted,
            WriteError::OutcomeUnknown => EditFailure::OutcomeUnknown,
            WriteError::Invalid { message, fields } => EditFailure::Invalid {
                message: message.clone().into(),
                fields: fields.iter().cloned().map(Into::into).collect(),
            },
            WriteError::TooManyRequests { message, .. } => {
                EditFailure::Refused(format!("The server refused for now: {message}").into())
            }
            other => EditFailure::Other(other.to_string().into()),
        },
    }
}

/// How a press of Apply arrived: a repeat of a held key never confirms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyPress {
    Fresh,
    Held,
}

/// What a load of the object is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoadPurpose {
    /// The first read, Discard, and the Env values toggle: the editor shows the new text.
    Replace,
    /// Reload and keep my changes: the user's changed paths move onto the new object.
    Rebase,
}

/// What an edit is on: the cursor's cluster and key, the object it names, and the kind the gate
/// checked.
pub(crate) struct EditSubject {
    pub(crate) target: ClusterObject,
    /// The cluster display name when the edit opened.
    pub(crate) cluster_name: SharedString,
    pub(crate) object: ObjectRef,
    pub(crate) kind: ObjectKind,
}

/// The open edit, in `AppShell.edit`.
pub(crate) struct YamlEditView {
    shell: WeakEntity<AppShell>,
    /// The cursor's cluster and key; the guard, the connection, the tier, and the audit line are
    /// resolved from this slot for every request, never from the primary.
    target: ClusterObject,
    cluster_name: SharedString,
    object: ObjectRef,
    kind: ObjectKind,
    /// `None` while loading, and in a fixture.
    base: Option<EditBase>,
    /// What the header shows after `resourceVersion`; the fixture has no base to read it from.
    resource_version: Option<SharedString>,
    env: EnvValues,
    editor: Entity<EditorState>,
    tab: EditTab,
    is_dirty: bool,
    load: LoadState,
    preview: PreviewState,
    banner: Option<EditBanner>,
    /// Paths that changed on the server between the two bases of the last rebase.
    server_changed: Vec<SharedString>,
    /// The user's paths that replace a server change after the last rebase, as warning lines: the
    /// side panel and the confirm dialog show them until the text is replaced.
    overwritten: Vec<SharedString>,
    /// A held Ctrl S was seen in this key event (see `apply_from_key`).
    is_apply_key_held: bool,
    /// The tint over the line the shown failure points at; it follows edits, and any edit clears it.
    error_mark: RangeDecorationCollection,
    /// The Diff tab rows; its count follows `PassedPreview::rows`, and its rows wrap.
    diff_list: ListState,
    /// The Revision history tab, created on its first show and dropped with the view.
    history: Option<Entity<RevisionHistory>>,
    focus_handle: FocusHandle,
    /// `--screen edit-yaml-diff`: a picture drawn from fixed data that sends nothing.
    #[cfg(feature = "screenshot")]
    is_fixture: bool,
    _subscription: Subscription,
}

impl YamlEditView {
    /// Opens the editor on `object` and reads it over `connection`, the one of the object's own
    /// cluster, which the caller resolved: the shell is being updated while this runs, so the view
    /// cannot ask it. The caller has checked the gate; `kind` is the kind the gate checked.
    pub(crate) fn new(
        shell: WeakEntity<AppShell>,
        subject: EditSubject,
        connection: Option<ClusterConnection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self::empty(shell, subject, window, cx);
        view.begin_load(
            connection,
            EnvValues::Hidden,
            LoadPurpose::Replace,
            window,
            cx,
        );
        view
    }

    /// An editor with nothing loaded: no base, no request.
    fn empty(
        shell: WeakEntity<AppShell>,
        subject: EditSubject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let EditSubject {
            target,
            cluster_name,
            object,
            kind,
        } = subject;
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("yaml")
                // Wrapping measures a whole line to break it; a single line of a few MiB (a paste) hangs
                // the window under load. The 2 MiB cap is checked on Apply, so the editor scrolls
                // sideways instead.
                .soft_wrap(false)
                .line_number(true)
        });
        let error_mark = editor.update(cx, |state, cx| {
            state.create_range_decorations_collection(Vec::new(), cx)
        });
        let subscription =
            cx.subscribe_in(&editor, window, |view, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    view.refresh_dirty(cx);
                    // The text moved under the failure: its line is no longer the one it named.
                    view.error_mark.clear(cx);
                    cx.notify();
                }
            });
        Self {
            shell,
            target,
            cluster_name,
            object,
            kind,
            base: None,
            resource_version: None,
            env: EnvValues::Hidden,
            editor,
            error_mark,
            tab: EditTab::Editor,
            is_dirty: false,
            load: LoadState::Ready,
            preview: PreviewState::NotChecked,
            banner: None,
            server_changed: Vec::new(),
            overwritten: Vec::new(),
            is_apply_key_held: false,
            diff_list: ListState::new(0, ListAlignment::Top, px(200.)),
            history: None,
            focus_handle: cx.focus_handle(),
            #[cfg(feature = "screenshot")]
            is_fixture: false,
            _subscription: subscription,
        }
    }

    pub(crate) fn cluster(&self) -> &ClusterRef {
        &self.target.cluster
    }

    pub(crate) fn object(&self) -> &ObjectRef {
        &self.object
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.is_dirty
    }

    /// `Deployment payments/api`: what the discard prompt and the leaving dialog call the edit.
    pub(crate) fn subject_text(&self) -> String {
        match self.object.namespace() {
            Some(namespace) => format!(
                "{}/{namespace}/{}",
                self.object.kind_name(),
                self.object.name()
            ),
            None => format!("{}/{}", self.object.kind_name(), self.object.name()),
        }
    }

    pub(crate) fn text(&self, cx: &gpui_kit::App) -> SharedString {
        self.editor.read(cx).value()
    }

    fn refresh_dirty(&mut self, cx: &gpui_kit::App) {
        let text = self.text(cx);
        // A Secret value refused as typed is a verdict on that text: Apply is on again for the next.
        if self.is_secret_values_refused() {
            self.preview = PreviewState::NotChecked;
        }
        self.is_dirty = self
            .base
            .as_ref()
            .is_some_and(|base| base.text() != text.as_ref());
    }

    /// The 1-based line the shown failure points at in `text`: a syntax error's own line, or the
    /// first server field path that resolves.
    pub(crate) fn error_line(&self, text: &str) -> Option<usize> {
        match &self.preview {
            PreviewState::Failed(PreviewFailure::Local(error)) => local_error_line(error, text),
            PreviewState::Failed(PreviewFailure::Invalid { fields, .. }) => {
                fields.iter().find_map(|field| line_of_field(text, field))
            }
            _ => None,
        }
    }

    /// Tints the line of the shown failure in the editor, or clears the tint when there is none.
    /// The editor has no gutter hook, so the mark is a danger-token fill over the line's text.
    fn sync_error_mark(&self, cx: &mut Context<Self>) {
        let text = self.text(cx);
        let range = self
            .error_line(&text)
            .and_then(|line| line_byte_range(&text, line))
            .filter(|range| !range.is_empty());
        let tint = tone_color(StatusTone::Bad, cx).opacity(ERROR_LINE_TINT);
        let marks = range
            .map(|range| {
                RangeDecoration::new(range)
                    .with_style(RangeDecorationStyle::Fill)
                    .with_color(tint)
            })
            .into_iter()
            .collect();
        self.error_mark.set(marks, cx);
    }

    /// Puts the cursor at the start of the 1-based `line` in the editor, which scrolls to it.
    pub(crate) fn go_to_line(&mut self, line: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.show_tab(EditTab::Editor, cx);
        let row = u32::try_from(line.saturating_sub(1)).unwrap_or(u32::MAX);
        self.editor.update(cx, |editor, cx| {
            editor.set_cursor_position(Position::new(row, 0), window, cx);
        });
    }

    // ---- loading ----

    /// Reads the object again over the connection of its own cluster, which the shell knows now.
    /// For the commands of an open edit: they run from the view's own events, so the shell can be
    /// asked.
    fn reload(
        &mut self,
        env: EnvValues,
        purpose: LoadPurpose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection = self
            .shell
            .upgrade()
            .and_then(|shell| shell.read(cx).edit_connection(&self.target.cluster, cx));
        self.begin_load(connection, env, purpose, window, cx);
    }

    /// Reads the object (a fresh GET, masked) for `purpose`.
    fn begin_load(
        &mut self,
        connection: Option<ClusterConnection>,
        env: EnvValues,
        purpose: LoadPurpose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = connection else {
            self.fail_load(format!("{} is not open", self.cluster_name).into(), cx);
            return;
        };
        let object = self.object.clone();
        let runtime = cx.global::<ClusterRuntime>().clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let read = runtime
                .spawn(async move { connection.edit_base(&object, env).await })
                .await;
            let _ = this.update_in(cx, |view, window, cx| match read {
                Ok(Ok(base)) => view.finish_load(base, purpose, window, cx),
                Ok(Err(error)) => view.fail_load(error_text(&error).into(), cx),
                Err(_) => view.fail_load("The request stopped before it finished".into(), cx),
            });
        });
        if self.base.is_none() {
            self.load = LoadState::Loading { _task: task };
        } else {
            // A reload of an open edit shows in the footer, like a check.
            self.preview = PreviewState::Running { _task: task };
        }
        cx.notify();
    }

    fn fail_load(&mut self, message: SharedString, cx: &mut Context<Self>) {
        if self.base.is_none() {
            self.load = LoadState::Failed(message);
        } else {
            self.preview = PreviewState::Failed(PreviewFailure::Server(message));
        }
        cx.notify();
    }

    fn finish_load(
        &mut self,
        base: EditBase,
        purpose: LoadPurpose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.load = LoadState::Ready;
        self.env = base.env();
        let current = self.text(cx);
        let rebased = match (&self.base, purpose) {
            (Some(old), LoadPurpose::Rebase) => Some(rebase(old, &current, &base)),
            _ => None,
        };
        match rebased {
            Some(Ok(rebased)) => {
                self.resource_version = Some(base.resource_version().to_owned().into());
                self.set_text(&rebased.text, window, cx);
                self.server_changed = paths_text(&rebased.server_changed);
                self.overwritten = rebased.overwritten.iter().map(overwritten_text).collect();
                self.banner = Some(EditBanner::Rebased {
                    unreachable: rebased
                        .unreachable
                        .iter()
                        .map(|path| format!("{path}: no longer exists on the server").into())
                        .collect(),
                    overwritten: self.overwritten.clone(),
                });
                self.base = Some(base);
                self.refresh_dirty(cx);
                // The old check was for the old text: ask again for the moved one.
                self.preview = PreviewState::NotChecked;
                self.apply(window, cx);
            }
            Some(Err(EditError::Recreated)) => {
                // Another object has the old one's name. The text stays for the user to copy; the
                // old base is kept, so nothing can be applied to the new object by accident.
                self.banner = Some(EditBanner::Recreated);
                self.preview = PreviewState::NotChecked;
            }
            Some(Err(error)) => {
                // The text cannot be moved (it no longer parses, or holds a leading zero). The old
                // base is kept: applying it again reports the conflict, so the server's changes
                // are never replaced without a rebase.
                self.preview = PreviewState::Failed(PreviewFailure::Local(error));
            }
            None => {
                self.resource_version = Some(base.resource_version().to_owned().into());
                self.set_text(base.text(), window, cx);
                self.banner = None;
                self.server_changed.clear();
                self.overwritten.clear();
                self.base = Some(base);
                self.refresh_dirty(cx);
                self.preview = PreviewState::NotChecked;
                self.tab = EditTab::Editor;
                self.editor
                    .update(cx, |editor, cx| editor.focus(window, cx));
            }
        }
        self.sync_error_mark(cx);
        cx.notify();
    }

    /// Puts `text` in the editor. `set_value` emits no change event, so the dirty flag is read
    /// again by the caller once the base is in place.
    fn set_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            editor.set_value(text.to_owned(), window, cx)
        });
    }

    // ---- commands ----

    pub(crate) fn show_tab(&mut self, tab: EditTab, cx: &mut Context<Self>) {
        // The Diff shows the server's answer for the text in the editor, so opening it asks, unless a
        // passed check already covers this text.
        if tab == EditTab::Diff && self.can_check() && !self.is_checked(&self.text(cx)) {
            let text = self.text(cx);
            self.run_preview(text, cx);
        }
        self.tab = tab;
        // A failed list (the Deployment was not loaded yet, a network error) is asked again.
        let needs_history = self
            .history
            .as_ref()
            .is_none_or(|history| history.read(cx).has_failed());
        if tab == EditTab::History && needs_history {
            self.history = Some(self.new_history(cx));
        }
        cx.notify();
    }

    /// The history of this Deployment, asked of the shell for the connection, the permission, and the
    /// selector. It runs from the view's own click, so the shell can be read (see `reload`).
    fn new_history(&self, cx: &mut Context<Self>) -> Entity<RevisionHistory> {
        let inputs = match self.shell.upgrade() {
            Some(shell) => shell
                .read(cx)
                .history_inputs(&self.target.cluster, &self.object, cx),
            None => HistoryInputs::Unavailable("the window is closing".into()),
        };
        let (deployment, object) = (self.target.key.clone(), self.object.clone());
        let subject = ClusterObject::new(self.target.cluster.clone(), deployment.clone());
        let offer = match self.shell.upgrade() {
            Some(shell) => shell
                .read(cx)
                .roll_back_offer(self.shell.clone(), subject, cx),
            None => RollBackOffer::Disabled("the window is closing".into()),
        };
        cx.new(|cx| RevisionHistory::new(deployment, object, inputs, cx).with_roll_back(offer))
    }

    /// Env values: reads the object again with the other setting. Only while the text is
    /// unchanged, because the new text would replace the user's edits.
    pub(crate) fn toggle_env_values(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_dirty || self.base.is_none() || self.is_running() {
            return;
        }
        let flipped = match self.env {
            EnvValues::Hidden => EnvValues::Shown,
            EnvValues::Shown => EnvValues::Hidden,
        };
        self.reload(flipped, LoadPurpose::Replace, window, cx);
    }

    /// Format: the 0007 serializer on the text, as one undoable replacement.
    pub(crate) fn format(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.base.is_none() {
            return;
        }
        let text = self.text(cx);
        match format_yaml(&text) {
            Ok(formatted) => {
                self.editor
                    .update(cx, |editor, cx| editor.replace_all(formatted, window, cx));
                self.refresh_dirty(cx);
            }
            Err(error) => self.preview = PreviewState::Failed(PreviewFailure::Local(error)),
        }
        self.sync_error_mark(cx);
        cx.notify();
    }

    /// Reload and keep my changes: moves the user's changed paths onto the newest object.
    pub(crate) fn keep_my_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.base.is_none() || self.is_running() {
            return;
        }
        self.reload(self.env, LoadPurpose::Rebase, window, cx);
    }

    /// Discard my changes: the newest object replaces the text.
    pub(crate) fn discard_my_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_running() {
            return;
        }
        self.reload(self.env, LoadPurpose::Replace, window, cx);
    }

    pub(crate) fn dismiss_banner(&mut self, cx: &mut Context<Self>) {
        self.banner = None;
        cx.notify();
    }

    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        // The shell is told whether the text is dirty because it cannot read this view while the
        // view is being updated.
        let is_dirty = self.is_dirty;
        let _ = self
            .shell
            .update(cx, |shell, cx| shell.cancel_edit(is_dirty, cx));
    }

    fn is_running(&self) -> bool {
        matches!(self.preview, PreviewState::Running { .. })
    }

    /// Whether the check refused the text because a Secret value changed: Edit YAML cannot send it.
    fn is_secret_values_refused(&self) -> bool {
        matches!(
            &self.preview,
            PreviewState::Failed(PreviewFailure::Local(EditError::SecretValuesChanged))
        )
    }

    /// Why Apply is off, or `None`. Only a running check or an unchanged text switch it off: a quota
    /// warning never does (spec 0041, decision 11).
    pub(super) fn apply_block_reason(&self) -> Option<&'static str> {
        if self.is_running() {
            Some("Waiting for the dry-run…")
        } else if !self.is_dirty {
            Some("No changes")
        } else if self.is_secret_values_refused() {
            Some(SECRET_VALUES_REASON)
        } else {
            None
        }
    }

    /// Ctrl S and Apply…: the first press checks the edit with the server and shows the Diff, the
    /// next one for the same text opens the confirm dialog. Nothing while a check runs or while the
    /// text is unchanged (decision 15).
    pub(crate) fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_press(KeyPress::Fresh, window, cx);
    }

    /// Ctrl S as the key layer delivers it. The action runs before the key-down listeners, so the
    /// decision waits until the end of the event: a repeat of a held key (`is_held`) is seen by then,
    /// and must never open the confirm dialog.
    fn apply_from_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.defer_in(window, |view, window, cx| {
            let press = if std::mem::take(&mut view.is_apply_key_held) {
                KeyPress::Held
            } else {
                KeyPress::Fresh
            };
            view.apply_press(press, window, cx);
        });
    }

    /// Notes that the Ctrl S being delivered is a repeat of a held key.
    fn note_key_down(&mut self, event: &KeyDownEvent) {
        if event.is_held && event.keystroke.key == "s" && event.keystroke.modifiers.modified() {
            self.is_apply_key_held = true;
        }
    }

    /// Whether the text can be checked with the server now: it holds changes, the object is read,
    /// and no check runs. A recreated object cannot take the text; only Discard leaves the banner.
    fn can_check(&self) -> bool {
        #[cfg(feature = "screenshot")]
        if self.is_fixture {
            return false;
        }
        let is_recreated = matches!(self.banner, Some(EditBanner::Recreated));
        !(self.is_running()
            || self.base.is_none()
            || !self.is_dirty
            || is_recreated
            || self.is_secret_values_refused())
    }

    /// Whether a passed dry-run is for exactly `text`.
    fn is_checked(&self, text: &str) -> bool {
        matches!(&self.preview, PreviewState::Passed(passed) if passed.for_text == text)
    }

    fn apply_press(&mut self, press: KeyPress, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_check() {
            return;
        }
        let text = self.text(cx);
        // A dry-run that passed for this text, from this key or from opening the Diff, goes straight
        // to the confirm dialog.
        if self.is_checked(&text) {
            if press == KeyPress::Fresh {
                self.confirm(window, cx);
            }
            return;
        }
        self.run_preview(text, cx);
    }

    /// The second press: the confirm dialog of the write flow, with its own dry-run.
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let PreviewState::Passed(passed) = &self.preview else {
            return;
        };
        let Some(request) = passed.request.clone() else {
            return;
        };
        let mut warnings: Vec<SharedString> = self
            .base
            .as_ref()
            .is_some_and(EditBase::is_helm_managed)
            .then(|| HELM_MANAGED_WARNING.into())
            .into_iter()
            .collect();
        warnings.extend(passed.checks.iter().cloned());
        warnings.extend(passed.quota.warnings().iter().cloned());
        warnings.extend(self.overwritten.iter().cloned());
        let change_lines = edit_change_lines(self.kind, &passed.changes, passed.more_changes);
        let mut intent = self.intent(request, warnings);
        intent.change_lines = change_lines;
        intent.audit_fields = edit_audit_fields(&passed.changes);
        let _ = self
            .shell
            .update(cx, |shell, cx| shell.start_write(intent, window, cx));
    }

    fn intent(&self, request: WriteRequest, warnings: Vec<SharedString>) -> WriteIntent {
        edit_intent(
            &self.target.cluster,
            &self.cluster_name,
            self.kind,
            request,
            warnings,
        )
    }

    /// Checks `text` locally, then asks the server to dry-run the replace.
    fn run_preview(&mut self, text: SharedString, cx: &mut Context<Self>) {
        let Some(base) = &self.base else {
            return;
        };
        let edit = match ObjectEdit::new(base, &text) {
            Ok(edit) => edit,
            Err(error) => {
                self.preview = PreviewState::Failed(PreviewFailure::Local(error));
                self.sync_error_mark(cx);
                cx.notify();
                return;
            }
        };
        let Some(request) = WriteRequest::new(
            self.object.clone(),
            WriteOperation::ReplaceObject(Box::new(edit)),
        ) else {
            self.preview = PreviewState::Failed(PreviewFailure::Server(
                "This object cannot be edited here".into(),
            ));
            cx.notify();
            return;
        };
        let intent = Rc::new(self.intent(request.clone(), Vec::new()));
        let step = self
            .shell
            .update(cx, |shell, cx| shell.begin_preview(Rc::clone(&intent), cx));
        let step = match step {
            Ok(Ok(step)) => step,
            Ok(Err(reason)) => {
                self.preview = PreviewState::Failed(PreviewFailure::Server(reason));
                cx.notify();
                return;
            }
            Err(_) => return,
        };
        let shell = self.shell.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = checked_write(&shell, step, cx).await;
            // The two sides are masked and without a header; the diff runs off the main thread.
            let rows = match &result {
                Ok(WriteOutcome {
                    effect: WriteEffect::Replaced(preview),
                    ..
                }) => {
                    let (before, after) = (preview.before.clone(), preview.after.clone());
                    Some(
                        cx.background_executor()
                            .spawn(async move { diff_rows(&before, &after) })
                            .await,
                    )
                }
                _ => None,
            };
            let _ = this.update(cx, |view, cx| {
                view.finish_preview(text, request, result, rows, cx);
            });
        });
        self.preview = PreviewState::Running { _task: task };
        self.tab = EditTab::Diff;
        self.sync_error_mark(cx);
        cx.notify();
    }

    fn finish_preview(
        &mut self,
        text: SharedString,
        request: WriteRequest,
        result: Result<WriteOutcome, CheckedWriteError>,
        rows: Option<Vec<DiffRow>>,
        cx: &mut Context<Self>,
    ) {
        match (result, rows) {
            (
                Ok(WriteOutcome {
                    effect: WriteEffect::Replaced(preview),
                    elapsed,
                    ..
                }),
                Some(rows),
            ) => {
                let checks = preview
                    .checks
                    .iter()
                    .map(|check| check_text(check, &text).into())
                    .collect();
                let quota = self.quota_line_of(&preview, cx);
                self.diff_list.reset(rows.len());
                self.preview = PreviewState::Passed(Box::new(PassedPreview {
                    for_text: text,
                    request: Some(request),
                    changes: preview.changes.iter().map(ChangeLine::of).collect(),
                    more_changes: preview.more_changes,
                    checks,
                    quota,
                    rows,
                    elapsed,
                }));
            }
            (Ok(_), _) => {
                self.preview = PreviewState::Failed(PreviewFailure::Server(
                    "The server answered with something other than the edited object".into(),
                ));
            }
            (Err(error), _) => self.apply_failure(edit_failure_of(&error), true),
        }
        self.sync_error_mark(cx);
        cx.notify();
    }

    /// The quota line of a passed dry-run, from what the session's quota feed holds now. It is
    /// computed once per preview: quotas change slowly, and a later check computes it again.
    fn quota_line_of(&self, preview: &EditPreview, cx: &mut Context<Self>) -> QuotaLine {
        let Some(namespace) = self.object.namespace() else {
            return QuotaLine::None;
        };
        let input = match self.shell.upgrade() {
            Some(shell) => shell
                .read(cx)
                .quota_input(&self.target.cluster, namespace, cx),
            None => return QuotaLine::None,
        };
        quota_line(preview.demand.as_ref(), &input)
    }

    /// Shows a failed write. A check also reports the failures that are not the user's to fix
    /// (`is_preview`); a commit leaves those to the notification of the write flow.
    pub(crate) fn apply_failure(&mut self, failure: EditFailure, is_preview: bool) {
        match failure {
            EditFailure::Conflict => {
                self.banner = Some(EditBanner::Conflict);
                self.preview = PreviewState::NotChecked;
            }
            EditFailure::Deleted => {
                self.banner = Some(EditBanner::Deleted);
                self.preview = PreviewState::NotChecked;
            }
            EditFailure::OutcomeUnknown => {
                self.banner = Some(EditBanner::OutcomeUnknown);
                self.preview = PreviewState::NotChecked;
            }
            EditFailure::Invalid { message, fields } => {
                self.preview = PreviewState::Failed(PreviewFailure::Invalid { message, fields });
            }
            EditFailure::Refused(text) => {
                self.preview = PreviewState::Failed(PreviewFailure::Server(text));
            }
            EditFailure::Other(text) if is_preview => {
                self.preview = PreviewState::Failed(PreviewFailure::Server(text));
            }
            EditFailure::Other(_) => {}
        }
    }

    /// A commit of this edit failed; the dialog is gone.
    pub(crate) fn commit_failed(&mut self, failure: EditFailure, cx: &mut Context<Self>) {
        self.apply_failure(failure, false);
        self.sync_error_mark(cx);
        cx.notify();
    }

    /// The text of the checks, for the tests that read what the dialog is given.
    #[cfg(test)]
    pub(crate) fn preview_state(&self) -> &PreviewState {
        &self.preview
    }

    /// The state the check leaves when the text changed a Secret value.
    #[cfg(test)]
    pub(crate) fn refuse_secret_values_for_test(&mut self) {
        self.preview = PreviewState::Failed(PreviewFailure::Local(EditError::SecretValuesChanged));
    }

    #[cfg(test)]
    pub(crate) fn banner(&self) -> Option<&EditBanner> {
        self.banner.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn tab(&self) -> EditTab {
        self.tab
    }

    #[cfg(test)]
    pub(crate) fn base_text(&self) -> Option<String> {
        self.base.as_ref().map(|base| base.text().to_owned())
    }

    #[cfg(test)]
    pub(crate) fn server_changed(&self) -> &[SharedString] {
        &self.server_changed
    }

    /// The byte ranges the editor tints as the line of the shown failure.
    #[cfg(test)]
    pub(crate) fn error_mark_ranges(&self, cx: &gpui_kit::App) -> Vec<std::ops::Range<usize>> {
        self.error_mark.get_ranges(cx)
    }

    /// The 1-based line the editor's cursor is on.
    #[cfg(test)]
    pub(crate) fn cursor_line_for_test(&self, cx: &gpui_kit::App) -> usize {
        self.editor.read(cx).cursor_position().line as usize + 1
    }

    /// Types `text` at the cursor, which emits the change event a real edit does.
    #[cfg(test)]
    pub(crate) fn insert_for_test(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor
            .update(cx, |editor, cx| editor.insert(text.to_owned(), window, cx));
    }
    #[cfg(test)]
    pub(crate) fn set_text_for_test(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_text(text, window, cx);
        self.refresh_dirty(cx);
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn load_error(&self) -> Option<&SharedString> {
        match &self.load {
            LoadState::Failed(message) => Some(message),
            LoadState::Loading { .. } | LoadState::Ready => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn editor(&self) -> &Entity<EditorState> {
        &self.editor
    }

    #[cfg(test)]
    pub(crate) fn history(&self) -> Option<&Entity<RevisionHistory>> {
        self.history.as_ref()
    }
}

impl Focusable for YamlEditView {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// The intent of applying an edit, for the preview and for the confirm dialog: the label of the
/// action is the one the audit line records (`Edit YAML`), the confirm button says what happens.
pub(crate) fn edit_intent(
    cluster: &ClusterRef,
    cluster_name: &SharedString,
    kind: ObjectKind,
    request: WriteRequest,
    warnings: Vec<SharedString>,
) -> WriteIntent {
    let action = ResourceAction::EditYaml(kind);
    WriteIntent {
        cluster: cluster.clone(),
        cluster_name: cluster_name.clone(),
        action,
        label: "Apply changes".into(),
        button: "Apply changes".into(),
        request,
        risk: ActionRisk::Change,
        warnings,
        change_lines: Vec::new(),
        audit_fields: Vec::new(),
    }
}

/// The most changes the confirm dialog lists; the rest are counted.
const CONFIRM_CHANGE_LINES: usize = 12;

/// The lines of the confirm dialog for the changes the dry-run found: `path: old → new` for a scalar,
/// the path alone for a map or a list, and for the data of a Secret, whose values are never shown.
/// The values are the masked ones of the preview, so a hidden env value reads `<hidden>`.
pub(crate) fn edit_change_lines(
    kind: ObjectKind,
    changes: &[ChangeLine],
    more_changes: usize,
) -> Vec<SharedString> {
    let mut lines: Vec<SharedString> = changes
        .iter()
        .take(CONFIRM_CHANGE_LINES)
        .map(|change| change_line(kind, change))
        .collect();
    let rest = changes.len().saturating_sub(CONFIRM_CHANGE_LINES) + more_changes;
    if rest > 0 {
        lines.push(format!("and {rest} more").into());
    }
    lines
}

/// The old and new value the audit line keeps of the labels an edit changed, as `path`, `from`,
/// and `value`. Only labels: an edit's other scalars can hold a credential (a command-line
/// argument, a URL), so those stay paths. `ChangeLine` values are the masked ones of the preview.
pub(crate) fn edit_audit_fields(changes: &[ChangeLine]) -> Vec<AuditField> {
    changes
        .iter()
        .filter(|change| is_label_path(&change.path))
        .filter(|change| !is_container(&change.old) && !is_container(&change.new))
        .map(|change| AuditField {
            path: change.path.to_string(),
            value: change.new.as_ref().map(ToString::to_string),
            from: change.old.as_ref().map(ToString::to_string),
        })
        .collect()
}

/// `metadata.labels.team` or `metadata.labels["app.kubernetes.io/name"]`, not the map itself.
fn is_label_path(path: &str) -> bool {
    path.strip_prefix("metadata.labels")
        .is_some_and(|rest| rest.starts_with('.') || rest.starts_with('['))
}

/// Whether a side of a change is a whole map or list, which the preview writes as `{…}` or `[…]`.
fn is_container(value: &Option<SharedString>) -> bool {
    value
        .as_ref()
        .is_some_and(|text| text == "{…}" || text == "[…]")
}

fn change_line(kind: ObjectKind, change: &ChangeLine) -> SharedString {
    let is_secret_data = kind == ObjectKind::Secret
        && ["data", "stringData"].iter().any(|root| {
            change.path.strip_prefix(root).is_some_and(|rest| {
                rest.is_empty() || rest.starts_with('.') || rest.starts_with('[')
            })
        });
    if is_secret_data || is_container(&change.old) || is_container(&change.new) {
        return change.path.clone();
    }
    let shown = |value: &Option<SharedString>| value.clone().unwrap_or_else(|| "—".into());
    format!(
        "{}: {} → {}",
        change.path,
        shown(&change.old),
        shown(&change.new)
    )
    .into()
}

/// `path` cut in the middle to at most `max` characters: the end names the field that changed, and
/// the start names the object, so both stay. The tooltip carries the whole path.
pub(crate) fn elide_middle(path: &str, max: usize) -> String {
    let chars: Vec<char> = path.chars().collect();
    if chars.len() <= max || max < 3 {
        return path.to_owned();
    }
    let tail = (max - 1) * 2 / 3;
    let head = max - 1 - tail;
    let start: String = chars[..head].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{start}…{end}")
}

/// `{path}: your value replaces a change made on the server`.
fn overwritten_text(path: &FieldPath) -> SharedString {
    format!("{path}: your value replaces a change made on the server").into()
}

fn paths_text(paths: &[FieldPath]) -> Vec<SharedString> {
    paths.iter().map(|path| path.to_string().into()).collect()
}

/// The text of one check in the side panel and the confirm dialog (edit-preview.md). `text` is the
/// editor text, which tells a leading-zero check which number it means.
pub(crate) fn check_text(check: &EditCheck, text: &str) -> String {
    match check {
        EditCheck::Rollout { strategy } if strategy == "OnDelete" => {
            "Pods change only when they are deleted (OnDelete)".to_owned()
        }
        EditCheck::Rollout { strategy } => format!("Pods will be replaced ({strategy})"),
        EditCheck::RolloutPaused => "Rollout is paused: pods change after Resume".to_owned(),
        EditCheck::StaleLastApplied => "kubectl apply users: the last-applied annotation is not \
            updated, so a later kubectl apply can revert this change"
            .to_owned(),
        EditCheck::Moved { path } => {
            format!("{path}: a hidden value was matched by position; check it belongs to this item")
        }
        EditCheck::LeadingZero { line } => leading_zero_text(*line, text),
    }
}

/// `Line 12: 0644 is read as 644 (YAML 1.2); write 420 or 0o644`. Falls back to the general
/// wording when the number cannot be found on the line.
fn leading_zero_text(line: usize, text: &str) -> String {
    let number = text
        .lines()
        .nth(line.saturating_sub(1))
        .and_then(|line| line.split(" #").next())
        .and_then(|line| line.split_whitespace().last())
        .filter(|token| token.len() > 1 && token.bytes().all(|byte| byte.is_ascii_digit()));
    let Some(number) = number else {
        return format!("Line {line}: a number with a leading zero is read as decimal (YAML 1.2)");
    };
    let decimal = number.trim_start_matches('0');
    let decimal = if decimal.is_empty() { "0" } else { decimal };
    match u64::from_str_radix(number, 8) {
        Ok(octal) => {
            format!(
                "Line {line}: {number} is read as {decimal} (YAML 1.2); write {octal} or 0o{decimal}"
            )
        }
        Err(_) => format!("Line {line}: {number} is read as {decimal} (YAML 1.2)"),
    }
}

/// The footer line under the editor, from what the preview and the text say now.
pub(crate) fn footer_text(preview: &PreviewState, current: &str) -> String {
    match preview {
        PreviewState::NotChecked => "Not checked yet".to_owned(),
        PreviewState::Running { .. } => "Server dry-run…".to_owned(),
        PreviewState::Passed(passed) if passed.for_text != current => {
            "Changed since the last check".to_owned()
        }
        PreviewState::Passed(passed) => format!(
            "Dry-run OK · {} ms · unchanged since you opened it",
            passed.elapsed.as_millis()
        ),
        PreviewState::Failed(PreviewFailure::Local(error)) => error.to_string(),
        PreviewState::Failed(PreviewFailure::Invalid { message, .. }) => message.to_string(),
        PreviewState::Failed(PreviewFailure::Server(text)) => text.to_string(),
    }
}

#[cfg(feature = "screenshot")]
impl YamlEditView {
    /// `--screen edit-yaml-diff` and `edit-yaml-history`: W10 drawn from fixed data, on `tab` (the Diff
    /// or the Revision history). It has no base, so Apply is off, and it never touches a connection.
    pub(crate) fn fixture(
        shell: WeakEntity<AppShell>,
        subject: EditSubject,
        tab: EditTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self::empty(shell, subject, window, cx);
        let before = crate::screenshot::EDIT_FIXTURE_BEFORE;
        let after = before
            .replace("replicas: 3", "replicas: 5")
            .replace("memory: 512Mi", "memory: 1Gi");
        view.set_text(&after, window, cx);
        view.is_dirty = true;
        view.is_fixture = true;
        view.resource_version = Some("88412093".into());
        view.tab = tab;
        if tab == EditTab::History {
            let subject = ClusterObject::new(view.target.cluster.clone(), view.target.key.clone());
            let offer = RollBackOffer::Enabled {
                shell: view.shell.clone(),
                subject,
            };
            view.history = Some(history_fixture(&view.target.key, offer, cx));
        }
        let rows = diff_rows(before, &after);
        view.diff_list.reset(rows.len());
        view.preview = PreviewState::Passed(Box::new(PassedPreview {
            for_text: after.clone().into(),
            request: None,
            changes: vec![
                ChangeLine {
                    path: "spec.replicas".into(),
                    old: Some("3".into()),
                    new: Some("5".into()),
                },
                ChangeLine {
                    path: "spec.template.spec.containers[api].resources.limits.memory".into(),
                    old: Some("512Mi".into()),
                    new: Some("1Gi".into()),
                },
            ],
            more_changes: 0,
            checks: vec![
                check_text(
                    &EditCheck::Rollout {
                        strategy: "RollingUpdate".to_owned(),
                    },
                    &after,
                )
                .into(),
            ],
            quota: crate::edit_quota::fixture_line(),
            rows,
            elapsed: Duration::from_millis(412),
        }));
        view
    }
}

/// The Revision history of `--screen edit-yaml-history`: three fixed revisions, the previous one
/// selected, with the `--screen revision-diff` fixture diff.
#[cfg(feature = "screenshot")]
fn history_fixture(
    deployment: &crate::table_selection::ResourceKey,
    offer: RollBackOffer,
    cx: &mut Context<YamlEditView>,
) -> Entity<RevisionHistory> {
    use crate::revision_diff::{RevisionDiffView, RevisionSide, diff_request};
    use crate::screenshot::{REVISION_FIXTURE_NEWER, REVISION_FIXTURE_OLDER};
    let side =
        |replica_set: &str, revision: u64, tag: &str, hours: i64, is_current: bool| RevisionSide {
            replica_set: replica_set.to_owned(),
            revision: Some(revision),
            tag: Some(tag.to_owned()),
            is_current,
            change_cause: (revision != 36).then(|| format!("release {tag}")),
            created_at: jiff::Timestamp::now()
                .checked_sub(jiff::SignedDuration::from_hours(hours))
                .ok(),
        };
    let sides = vec![
        side("api-7d9f8c", 38, "2.14.0", 3, true),
        side("api-6c8d9f", 37, "2.13.0", 52, false),
        side("api-5b7c8e", 36, "2.12.1", 170, false),
    ];
    let request = diff_request(deployment.clone(), sides[1].clone(), sides[0].clone());
    let diff = cx.new(|_| {
        RevisionDiffView::fixture(request, REVISION_FIXTURE_OLDER, REVISION_FIXTURE_NEWER, 2)
    });
    let key = deployment.clone();
    cx.new(|_| RevisionHistory::fixture(key, sides, 1, diff).with_roll_back(offer))
}

#[path = "yaml_edit_panels.rs"]
pub(crate) mod yaml_edit_panels;

#[cfg(test)]
#[path = "yaml_edit_tests.rs"]
pub(crate) mod yaml_edit_tests;
