//! The guarded write flow (spec 0030): gate, confirm dialog, server-side dry-run, lock re-check,
//! commit, audit line, notice. `checked_write` is the only caller of `ClusterConnection::write`.
//!
//! A child of `app_shell`, like `workspace`, because the flow reads the viewed slots. Every step
//! names the cluster of the row or cursor (`WriteIntent::cluster`) and takes its guard and its
//! connection from that cluster's own slot session, never from the primary.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use cluster::{
    ClusterConnection, ClusterError, NodeScheduling, ObjectKind, ObjectRef, WriteError, WriteMode,
    WriteOperation, WriteOutcome, WriteRequest,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, AsyncApp, Context, SharedString, WeakEntity, Window,
};

use super::AppShell;
use crate::audit_log::{AuditEntry, AuditOutcome, append_audit, audit_entry};
use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::ClusterRuntime;
use crate::confirm_dialog::{ConfirmDialog, DialogInputs, DialogKind};
use crate::resource_actions::{
    ActionAvailability, ResourceAction, action_availability, action_label, action_risk,
    unavailable_text,
};
use crate::settings::AppSettings;
use crate::write_guard::{ActionRisk, ClusterGuard, DialogConfirm, WriteLock, confirm_step};

/// One guarded change, as the action builds it. `cluster` is the row's or cursor's cluster: the
/// gate, the tier, the typed name, the dialog badge, the connection, and the audit line all come
/// from it.
pub(crate) struct WriteIntent {
    pub(crate) cluster: ClusterRef,
    /// The cluster display name at the time the action started, for texts that outlive the session.
    pub(crate) cluster_name: SharedString,
    pub(crate) action: ResourceAction,
    /// What the change is called in the dialog title and the notices: `Cordon node wk-04`.
    pub(crate) label: SharedString,
    /// The text of the confirm button: `Cordon`.
    pub(crate) button: SharedString,
    pub(crate) request: WriteRequest,
    pub(crate) risk: ActionRisk,
    /// The text to type in the `TypeName` tier when the action names its object; `None` types the
    /// cluster display name.
    pub(crate) expected_name: Option<String>,
    /// Non-blocking context lines of the dialog.
    pub(crate) warnings: Vec<SharedString>,
}

impl WriteIntent {
    /// What the `TypeName` tier asks to type.
    pub(crate) fn expected(&self) -> &str {
        self.expected_name.as_deref().unwrap_or(&self.cluster_name)
    }

    /// `the cluster name`, or `the node name` for an action that names its object.
    pub(crate) fn typed_hint(&self) -> String {
        match self.expected_name {
            None => "the cluster name".to_owned(),
            Some(_) => format!(
                "the {} name",
                self.request.target().kind_name().to_ascii_lowercase()
            ),
        }
    }
}

/// The server-side check of a change, as the dialog shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DryRunState {
    Running,
    Passed {
        elapsed: Duration,
    },
    Failed(SharedString),
    /// An admission webhook does not support dry-run: the commit stays blocked.
    Rejected(SharedString),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TypedMatch {
    NotNeeded,
    Matches,
    Differs,
}

/// Whether `typed` satisfies `confirm`: exact and case-sensitive after trimming surrounding spaces.
pub(crate) fn typed_match(confirm: &DialogConfirm, typed: &str) -> TypedMatch {
    match confirm {
        DialogConfirm::Click => TypedMatch::NotNeeded,
        DialogConfirm::TypeName { expected } if typed.trim() == expected => TypedMatch::Matches,
        DialogConfirm::TypeName { .. } => TypedMatch::Differs,
    }
}

/// The cluster of the action is gone, or its connection changed since the step began.
fn gone_block(
    guard: Option<&ClusterGuard<'_>>,
    cluster_name: &str,
    generation: u64,
) -> Option<SharedString> {
    match guard {
        Some(guard) if guard.generation == generation => None,
        _ => Some(format!("{cluster_name} is no longer open; nothing was changed").into()),
    }
}

/// `gone_block`, then the lock: the checks that can change while a dialog stays open.
fn live_block(
    guard: Option<&ClusterGuard<'_>>,
    cluster_name: &str,
    generation: u64,
) -> Option<SharedString> {
    if let Some(reason) = gone_block(guard, cluster_name, generation) {
        return Some(reason);
    }
    let locked = guard.is_some_and(|guard| guard.lock == WriteLock::Locked);
    locked.then(|| format!("{cluster_name} was locked; nothing was changed").into())
}

/// Why the commit may not go now, `None` when it may. It runs on every render of the dialog (the
/// confirm button is disabled while it holds) and `checked_write` runs its live half right before
/// every commit. The first match wins: the cluster is gone or reconnected, locked, the dry-run
/// has not passed, the typed name differs.
pub(crate) fn commit_block(
    guard: Option<&ClusterGuard<'_>>,
    cluster_name: &str,
    dry_run_generation: u64,
    dry_run: &DryRunState,
    typed: TypedMatch,
    expected: &str,
) -> Option<SharedString> {
    if let Some(reason) = live_block(guard, cluster_name, dry_run_generation) {
        return Some(reason);
    }
    match dry_run {
        DryRunState::Running => return Some("Waiting for the dry-run…".into()),
        DryRunState::Failed(text) => return Some(text.clone()),
        DryRunState::Rejected(reason) => {
            return Some(
                format!(
                    "An admission webhook does not support dry-run, so this change cannot be checked: {reason}. Nothing was changed."
                )
                .into(),
            );
        }
        DryRunState::Passed { .. } => {}
    }
    (typed == TypedMatch::Differs).then(|| format!("Type {expected} to confirm").into())
}

/// Why an unlock may not go now: the cluster is gone or reconnected, or the name differs.
pub(crate) fn unlock_block(
    guard: Option<&ClusterGuard<'_>>,
    cluster_name: &str,
    generation: u64,
    typed: TypedMatch,
    expected: &str,
) -> Option<SharedString> {
    if let Some(reason) = gone_block(guard, cluster_name, generation) {
        return Some(reason);
    }
    (typed == TypedMatch::Differs).then(|| format!("Type {expected} to confirm").into())
}

/// The proof that the confirm step was satisfied for one dry-run: built only by `confirmed`.
/// `Copy`, so a later batch can reuse it per commit.
#[derive(Clone, Copy)]
pub(crate) struct Confirmed {
    dry_run_generation: u64,
}

/// `Some` only when the dry-run passed and the typed name matches or is not needed.
pub(crate) fn confirmed(
    dry_run: &DryRunState,
    typed: TypedMatch,
    dry_run_generation: u64,
) -> Option<Confirmed> {
    let is_passed = matches!(dry_run, DryRunState::Passed { .. });
    (is_passed && typed != TypedMatch::Differs).then_some(Confirmed { dry_run_generation })
}

/// App-side mode: a commit carries the proof that the confirm step was satisfied. It maps to
/// `cluster::WriteMode`.
pub(crate) enum CommitMode {
    DryRun,
    Commit { confirmed: Confirmed },
}

pub(crate) struct WriteStep {
    pub(crate) intent: Rc<WriteIntent>,
    /// The session generation the dialog opened on.
    pub(crate) generation: u64,
    pub(crate) mode: CommitMode,
    pub(crate) note: Option<String>,
}

pub(crate) enum CheckedWriteError {
    /// `commit_block` or the gate said no; nothing was sent, and nothing is audited.
    Blocked(SharedString),
    Write(WriteError),
}

/// What `checked_write` needs from the shell, taken in one read of the intent's own slot.
struct PreparedWrite {
    connection: ClusterConnection,
    runtime: ClusterRuntime,
    /// The audit line of the commit, with the outcome still to fill in.
    entry: AuditEntry,
    config_dir: Option<PathBuf>,
}

impl AppShell {
    /// Looks up everything one write step needs from `intent.cluster`'s own slot.
    fn prepare_write(&self, step: &WriteStep, cx: &App) -> Result<PreparedWrite, SharedString> {
        let intent = &step.intent;
        let guard = self.guard_for(&intent.cluster, cx);
        let block = match step.mode {
            CommitMode::DryRun => gone_block(guard.as_ref(), &intent.cluster_name, step.generation),
            CommitMode::Commit { confirmed } => live_block(
                guard.as_ref(),
                &intent.cluster_name,
                confirmed.dry_run_generation,
            ),
        };
        if let Some(reason) = block {
            return Err(reason);
        }
        let (Some(guard), Some(connection)) = (guard, self.slot_connection(&intent.cluster, cx))
        else {
            return Err(format!(
                "{} is no longer open; nothing was changed",
                intent.cluster_name
            )
            .into());
        };
        Ok(PreparedWrite {
            connection,
            runtime: cx.global::<ClusterRuntime>().clone(),
            entry: audit_entry(
                intent,
                &guard,
                AuditOutcome::Applied,
                None,
                step.note.as_deref(),
            ),
            config_dir: AppSettings::config_dir(cx).map(std::path::Path::to_path_buf),
        })
    }

    /// The warning behind the title-bar button when the audit line could not be written.
    fn audit_failed(&mut self, kind: std::io::ErrorKind, cx: &mut Context<Self>) {
        self.write_notice = Some(format!("Could not write the audit log ({kind})"));
        cx.notify();
    }
}

/// Steps 5-6 of the flow and the **only** caller of `ClusterConnection::write`. A commit
/// re-resolves the guard of the intent's cluster, runs the live checks, sends, and appends the
/// audit line. A dry-run re-resolves the guard, sends, and appends nothing.
pub(crate) async fn checked_write(
    shell: &WeakEntity<AppShell>,
    step: WriteStep,
    cx: &mut AsyncApp,
) -> Result<WriteOutcome, CheckedWriteError> {
    let prepared = shell
        .update(cx, |shell, cx| shell.prepare_write(&step, cx))
        .map_err(|_| CheckedWriteError::Blocked("k8sBoard is closing".into()))?
        .map_err(CheckedWriteError::Blocked)?;
    let PreparedWrite {
        connection,
        runtime,
        mut entry,
        config_dir,
    } = prepared;
    let (mode, is_commit) = match step.mode {
        CommitMode::DryRun => (WriteMode::DryRun, false),
        CommitMode::Commit { .. } => (WriteMode::Commit, true),
    };
    let request = step.intent.request.clone();
    let sent = runtime
        .spawn(async move { connection.write(&request, mode).await })
        .await;
    let result = sent.unwrap_or_else(|_| {
        Err(match mode {
            WriteMode::Commit => WriteError::OutcomeUnknown,
            WriteMode::DryRun => WriteError::Cluster(ClusterError::Rendered {
                message: "the request task stopped".to_owned(),
            }),
        })
    });
    if is_commit && let Some(outcome) = audit_outcome(&result) {
        entry.outcome = outcome;
        entry.error = result.as_ref().err().map(ToString::to_string);
        append_in_background(shell, config_dir, entry, cx).await;
    }
    result.map_err(CheckedWriteError::Write)
}

/// How the audit records a commit, or `None` when nothing was sent or nothing changed: a debug
/// build that blocks writes, and a 429 refusal (drain retries would flood the log).
fn audit_outcome(result: &Result<WriteOutcome, WriteError>) -> Option<AuditOutcome> {
    match result {
        Ok(_) => Some(AuditOutcome::Applied),
        Err(WriteError::WritesBlocked | WriteError::TooManyRequests { .. }) => None,
        Err(WriteError::OutcomeUnknown) => Some(AuditOutcome::Unknown),
        Err(_) => Some(AuditOutcome::Failed),
    }
}

/// Appends `entry` off the main thread; with no settings folder nothing is written. A failure
/// warns and shows a title-bar notice, but never undoes the change.
pub(super) async fn append_in_background(
    shell: &WeakEntity<AppShell>,
    config_dir: Option<PathBuf>,
    entry: AuditEntry,
    cx: &mut AsyncApp,
) {
    let Some(dir) = config_dir else {
        return;
    };
    let written = cx
        .background_executor()
        .spawn(async move { append_audit(&dir, &entry) })
        .await;
    if let Err(error) = written {
        tracing::warn!(kind = ?error.kind(), "could not append to the audit log");
        let _ = shell.update(cx, |shell, cx| shell.audit_failed(error.kind(), cx));
    }
}

/// The dry-run line a finished dry-run gives.
pub(crate) fn dry_run_state_of(result: Result<WriteOutcome, CheckedWriteError>) -> DryRunState {
    match result {
        Ok(outcome) => DryRunState::Passed {
            elapsed: outcome.elapsed,
        },
        Err(CheckedWriteError::Blocked(text)) => DryRunState::Failed(text),
        Err(CheckedWriteError::Write(WriteError::DryRunRejected { reason })) => {
            DryRunState::Rejected(reason.into())
        }
        Err(CheckedWriteError::Write(error)) => {
            DryRunState::Failed(format!("Dry-run failed: {}", write_error_text(&error)).into())
        }
    }
}

/// The text of a write failure; an invalid change lists its field paths.
pub(crate) fn write_error_text(error: &WriteError) -> String {
    match error {
        WriteError::Invalid { fields, .. } if !fields.is_empty() => {
            format!("{error} ({})", fields.join(", "))
        }
        other => other.to_string(),
    }
}

/// What the user reads after a commit that did not succeed and whose dialog is gone.
pub(crate) fn failure_notice(label: &str, error: &CheckedWriteError) -> String {
    match error {
        CheckedWriteError::Blocked(text) => format!("{label}: {text}"),
        CheckedWriteError::Write(WriteError::OutcomeUnknown) => format!(
            "{label}: the outcome is unknown; the change may have been applied. Refresh to check."
        ),
        CheckedWriteError::Write(error) => format!("{label} failed: {}", write_error_text(error)),
    }
}

/// A commit failure the dialog shows in place, with a Retry that runs the dry-run again.
pub(crate) fn retryable_text(error: &CheckedWriteError) -> Option<String> {
    match error {
        CheckedWriteError::Write(WriteError::Conflict { message, .. }) => {
            Some(format!("The object changed since the check: {message}"))
        }
        CheckedWriteError::Write(WriteError::TooManyRequests { message, .. }) => {
            Some(format!("The server refused for now: {message}"))
        }
        _ => None,
    }
}

/// The intent of Cordon, or Uncordon when the node is already cordoned: the label follows the
/// node's scheduling. Cordon is a `Change` and types the cluster name.
fn cordon_intent(
    cluster: &ClusterRef,
    cluster_name: &str,
    node: &str,
    scheduling: &NodeScheduling,
) -> Option<WriteIntent> {
    let schedulable = matches!(scheduling, NodeScheduling::Disabled);
    let target = ObjectRef::new(ObjectKind::Node, None, node.to_owned())?;
    let request = WriteRequest::new(target, WriteOperation::SetNodeSchedulable { schedulable })?;
    let verb = if schedulable { "Uncordon" } else { "Cordon" };
    Some(WriteIntent {
        cluster: cluster.clone(),
        cluster_name: cluster_name.to_owned().into(),
        action: ResourceAction::Cordon,
        label: format!("{verb} node {node}").into(),
        button: verb.into(),
        request,
        risk: action_risk(ResourceAction::Cordon),
        expected_name: None,
        warnings: Vec::new(),
    })
}

/// The line of the dialog when the node changed between the menu and the click.
fn changed_since_menu_warning(shown: &NodeScheduling, now: &str) -> SharedString {
    let offered = cordon_label(shown);
    format!("The menu offered {offered}, but the node has changed since, so this is {now}.").into()
}

/// The menu and notice label of Cordon for a node in `scheduling`.
pub(crate) fn cordon_label(scheduling: &NodeScheduling) -> &'static str {
    match scheduling {
        NodeScheduling::Disabled => "Uncordon",
        NodeScheduling::Enabled => action_label(ResourceAction::Cordon),
    }
}

impl AppShell {
    /// Cordon or uncordon `node` of `cluster`, the row's or cursor's own cluster. The label follows
    /// the node's scheduling as the cluster's own session reports it now. `shown` is the scheduling
    /// the menu item was built for, when a menu started this: if the node changed since, the dialog
    /// says so, because it then does the opposite of what the menu offered.
    pub(crate) fn start_cordon(
        &mut self,
        cluster: &ClusterRef,
        node: &str,
        shown: Option<NodeScheduling>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let intent = {
            let (Some(guard), Some(live)) =
                (self.guard_for(cluster, cx), self.slot_live(cluster, cx))
            else {
                return;
            };
            let Some(summary) = live
                .nodes
                .items()
                .iter()
                .find(|summary| summary.name == node)
            else {
                return;
            };
            let intent = cordon_intent(
                cluster,
                guard.display_name(),
                node,
                &summary.status.scheduling,
            );
            intent.map(|mut intent| {
                if let Some(shown) = shown.filter(|shown| *shown != summary.status.scheduling) {
                    intent
                        .warnings
                        .push(changed_since_menu_warning(&shown, &intent.button));
                }
                intent
            })
        };
        match intent {
            Some(intent) => self.start_write(intent, window, cx),
            None => notify(
                window,
                cx,
                "Cordon is unavailable: the node name is not valid".to_owned(),
            ),
        }
    }

    /// The one entry of a guarded change: the gate, then the confirm dialog with the dry-run
    /// already running. Nothing is sent without the dialog, for every tier, risk, and trigger.
    pub(crate) fn start_write(
        &mut self,
        intent: WriteIntent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (generation, confirm, environment) = {
            let Some(guard) = self.guard_for(&intent.cluster, cx) else {
                notify(window, cx, format!("{} is not open", intent.cluster_name));
                return;
            };
            // A stale menu or a key pressed in a gap cannot bypass the gate.
            if let ActionAvailability::Disabled { reason } =
                action_availability(intent.action, &guard)
            {
                let text = unavailable_text(&intent.button, &reason);
                notify(window, cx, text);
                return;
            }
            (
                guard.generation,
                confirm_step(guard.profile.confirm, intent.risk, intent.expected()),
                guard.profile.environment,
            )
        };
        let intent = Rc::new(intent);
        let shell = cx.weak_entity();
        let inputs = DialogInputs {
            shell,
            kind: DialogKind::Write(Rc::clone(&intent)),
            confirm,
            environment,
            generation,
        };
        let dialog = cx.new(|cx| ConfirmDialog::new(inputs, window, cx));
        dialog.update(cx, |dialog, cx| dialog.start_dry_run(cx));
        #[cfg(test)]
        {
            self.last_dialog = Some(dialog.downgrade());
        }
        ConfirmDialog::open(&dialog, window, cx);
    }

    /// The confirmed commit of a dialog. Its task is detached: closing the dialog does not cancel
    /// a started commit, and it ends with the audit line and the notice.
    pub(crate) fn commit_write(
        &mut self,
        dialog: WeakEntity<ConfirmDialog>,
        step: WriteStep,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let handle: AnyWindowHandle = window.window_handle();
        let shell = cx.weak_entity();
        let intent = Rc::clone(&step.intent);
        cx.spawn(async move |_, cx| {
            let result = checked_write(&shell, step, cx).await;
            finish_commit(&dialog, &intent, handle, result, cx);
        })
        .detach();
    }
}

/// Shows the result of a commit: the dialog closes with a notice, or stays with a Retry when the
/// failure is one the user can retry and the dialog is still open.
fn finish_commit(
    dialog: &WeakEntity<ConfirmDialog>,
    intent: &WriteIntent,
    window: AnyWindowHandle,
    result: Result<WriteOutcome, CheckedWriteError>,
    cx: &mut AsyncApp,
) {
    let label = intent.label.to_string();
    let result = result.map(|_| ());
    if let Err(error) = &result
        && let Some(text) = retryable_text(error)
        && dialog
            .update(cx, |dialog, cx| dialog.commit_failed(text.clone(), cx))
            .unwrap_or(false)
    {
        return;
    }
    let notice = match &result {
        Ok(()) => format!("{label}: done"),
        Err(error) => failure_notice(&label, error),
    };
    let _ = cx.update_window(window, |_, window, cx| {
        // Only our own dialog closes, and only while it is open: after Escape or Back the notice
        // is all there is, and closing the top dialog then would take another one.
        let _ = dialog.update(cx, |dialog, cx| {
            if dialog.is_open() {
                dialog.close(window, cx);
            }
        });
        notify_with(window, cx, notice, result.is_ok());
    });
}

fn notify(window: &mut Window, cx: &mut App, text: String) {
    notify_with(window, cx, text, false);
}

fn notify_with(window: &mut Window, cx: &mut App, text: String, is_success: bool) {
    let notification = if is_success {
        Notification::success(text)
    } else {
        Notification::warning(text)
    };
    window.push_notification(notification, cx);
}

#[cfg(feature = "screenshot")]
impl AppShell {
    /// `--screen cordon-confirm`: the Cordon dialog of the first node in a fixed state, over an
    /// unlocked session so the button shows as enabled. It skips the gate and the dry-run and can
    /// never commit (`ConfirmDialog::show_fixture`). `false` while the node list has not loaded.
    pub(super) fn open_cordon_fixture(
        &mut self,
        cluster: &ClusterRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(session) = self.slot_session(cluster).cloned() else {
            return false;
        };
        let node = match self.slot_live(cluster, cx) {
            Some(live) if !live.nodes.is_loading() => live.nodes.items().first().cloned(),
            _ => return false,
        };
        let Some(node) = node else {
            return true;
        };
        session.update(cx, |session, cx| session.set_lock(WriteLock::Unlocked, cx));
        let inputs = {
            let Some(guard) = self.guard_for(cluster, cx) else {
                return false;
            };
            let Some(intent) = cordon_intent(
                cluster,
                guard.display_name(),
                &node.name,
                &node.status.scheduling,
            ) else {
                return true;
            };
            let confirm = confirm_step(guard.profile.confirm, intent.risk, intent.expected());
            DialogInputs {
                shell: cx.weak_entity(),
                kind: DialogKind::Write(Rc::new(intent)),
                confirm,
                environment: guard.profile.environment,
                generation: guard.generation,
            }
        };
        let dialog = cx.new(|cx| ConfirmDialog::new(inputs, window, cx));
        dialog.update(cx, |dialog, _| dialog.show_fixture());
        ConfirmDialog::open(&dialog, window, cx);
        true
    }
}

#[cfg(test)]
#[path = "write_flow_tests.rs"]
mod write_flow_tests;
