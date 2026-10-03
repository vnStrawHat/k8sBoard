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
    ClusterConnection, ClusterError, ExecPermit, NodeScheduling, ObjectKind, ObjectRef, WriteError,
    WriteMode, WriteOperation, WriteOutcome, WriteRequest,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, AsyncApp, Context, Entity, SharedString, WeakEntity,
    Window,
};

use super::AppShell;
use crate::audit_log::{
    AuditEntry, AuditField, AuditObject, AuditOutcome, append_audit, audit_entry,
    created_name_field,
};
use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::AccessState;
use crate::confirm_dialog::{ConfirmDialog, DialogInputs, DialogKind};
use crate::kind_row::KindObject;
use crate::resource_actions::{
    ActionAvailability, ResourceAction, action_availability, action_label, action_risk,
    unavailable_text,
};
use crate::settings::AppSettings;
use crate::table_selection::ClusterObject;
use crate::value_popover::ValuePopover;
use crate::workload_actions::{
    PAUSED_REASON, RevisionTarget, ScaleTarget, WorkloadScope, roll_back_intent, row_block,
    scale_intent, state_label, workload_intent,
};
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

/// One guarded session start (spec 0036 exec; port-forward and node shells reuse it): an action
/// that opens a stream instead of changing an object. It has no dry-run, so the dialog says so, and
/// the audit line is written when the stream reports whether it came up.
pub(crate) struct ConnectIntent {
    /// The row's or cursor's cluster: the gate, the tier, the permit, the connection, and the
    /// audit line all come from it.
    pub(crate) cluster: ClusterRef,
    /// The cluster display name at the time the action started.
    pub(crate) cluster_name: SharedString,
    pub(crate) action: ResourceAction,
    /// What the dialog title and notices call it: `Open shell`.
    pub(crate) label: SharedString,
    /// The text of the confirm button.
    pub(crate) button: SharedString,
    pub(crate) risk: ActionRisk,
    /// Non-blocking context lines of the dialog.
    pub(crate) warnings: Vec<SharedString>,
    /// What the dialog names and the audit line records: the target and its parameters. Never
    /// stream bytes.
    pub(crate) object: AuditObject,
    pub(crate) fields: Vec<AuditField>,
    /// Runs after the confirm, with the proof from the cluster's own report and its connection,
    /// both read at that moment.
    pub(crate) open: Rc<ConnectOpen>,
}

/// The call that opens the stream of a `ConnectIntent`.
pub(crate) type ConnectOpen =
    dyn Fn(&mut AppShell, ExecPermit, ClusterConnection, &mut Window, &mut Context<AppShell>);

impl ConnectIntent {
    /// What the `TypeName` tier asks to type: the cluster name.
    pub(crate) fn expected(&self) -> &str {
        &self.cluster_name
    }
}

/// The server-side check of a change, as the dialog shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DryRunState {
    /// A stream start has nothing to check on the server.
    NotSupported,
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
        DryRunState::NotSupported | DryRunState::Passed { .. } => {}
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
    let is_passed = matches!(
        dry_run,
        DryRunState::Passed { .. } | DryRunState::NotSupported
    );
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
        if let Ok(done) = &result
            && let Some(name) = &done.created_name
        {
            entry.fields.push(created_name_field(name));
        }
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

/// The notice of a commit that went through: a create names what it made.
fn success_notice(label: &str, created: Option<&str>) -> String {
    match created {
        Some(name) => format!("{label}: created {name}"),
        None => format!("{label}: done"),
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

    /// A workload action on the cursor row `subject`: Restart, Pause or Resume, Suspend or Resume,
    /// Trigger now, Re-run. The row is read from the subject's own cluster, so the intent names the
    /// object and the state as they are now; `row_block` is checked again here because a menu or a
    /// key may be a moment old.
    pub(crate) fn start_workload_action(
        &mut self,
        action: ResourceAction,
        subject: &ClusterObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(action);
        let intent = {
            let (Some(guard), Some(live)) = (
                self.guard_for(&subject.cluster, cx),
                self.slot_live(&subject.cluster, cx),
            ) else {
                notify(
                    window,
                    cx,
                    unavailable_text(label, "the cluster is not open"),
                );
                return;
            };
            let Some(row) = live.row_of(&subject.key) else {
                let text = unavailable_text(label, "the object is no longer listed");
                notify(window, cx, text);
                return;
            };
            if let Some(reason) = row_block(action, &row.object, None) {
                let label = state_label(action, label, &row.object);
                notify(window, cx, unavailable_text(label, &reason));
                return;
            }
            let scope = WorkloadScope {
                cluster: &subject.cluster,
                cluster_name: guard.display_name(),
            };
            workload_intent(action, &scope, &row.object, jiff::Timestamp::now())
        };
        match intent {
            Some(intent) => self.start_write(intent, window, cx),
            None => notify(
                window,
                cx,
                unavailable_text(label, "the object name is not valid"),
            ),
        }
    }

    /// Roll back of the Deployment `subject` to `target`: a button of its drawer's Revisions, or the
    /// palette entry. The Deployment is read again from its own cluster, so a rollout paused since
    /// the button was drawn is refused here, and the dialog names what is there now.
    pub(crate) fn start_roll_back(
        &mut self,
        subject: &ClusterObject,
        target: &RevisionTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::RollBack);
        let intent = {
            let (Some(guard), Some(live)) = (
                self.guard_for(&subject.cluster, cx),
                self.slot_live(&subject.cluster, cx),
            ) else {
                notify(
                    window,
                    cx,
                    unavailable_text(label, "the cluster is not open"),
                );
                return;
            };
            let Some(KindObject::Deployment(deployment)) =
                live.row_of(&subject.key).map(|row| &row.object)
            else {
                let text = unavailable_text(label, "the deployment is no longer listed");
                notify(window, cx, text);
                return;
            };
            if deployment.is_paused {
                notify(window, cx, unavailable_text(label, PAUSED_REASON));
                return;
            }
            let scope = WorkloadScope {
                cluster: &subject.cluster,
                cluster_name: guard.display_name(),
            };
            roll_back_intent(&scope, deployment, target)
        };
        match intent {
            Some(intent) => self.start_write(intent, window, cx),
            None => notify(
                window,
                cx,
                unavailable_text(label, "the object name is not valid"),
            ),
        }
    }

    /// The row under `subject` as a Scale target. The HPA is read from the Issues feed only when that
    /// list is already loaded: no list starts for a hint.
    pub(crate) fn scale_target_of(&self, subject: &ClusterObject, cx: &App) -> Option<ScaleTarget> {
        let live = self.slot_live(&subject.cluster, cx)?;
        let row = live.row_of(&subject.key)?;
        ScaleTarget::of(&row.object, live.loaded_hpas())
    }

    /// Scale on the cursor row: the one popover that the menu, the key, and the palette share.
    pub(crate) fn open_scale_popover(
        &mut self,
        subject: &ClusterObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.scale_target_of(subject, cx) else {
            let text = unavailable_text(
                action_label(ResourceAction::Scale(ObjectKind::Deployment)),
                "the object is no longer listed",
            );
            notify(window, cx, text);
            return;
        };
        let (shell, subject) = (cx.weak_entity(), subject.clone());
        let popover = cx.new(|cx| ValuePopover::scale_one(shell, subject, target, window, cx));
        self.set_value_popover(popover, cx);
    }

    pub(crate) fn set_value_popover(
        &mut self,
        popover: Entity<ValuePopover>,
        cx: &mut Context<Self>,
    ) {
        self.value_popover = Some(popover);
        cx.notify();
    }

    pub(crate) fn close_value_popover(&mut self, cx: &mut Context<Self>) {
        if self.value_popover.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn value_popover(&self) -> Option<&Entity<ValuePopover>> {
        self.value_popover.as_ref()
    }

    /// Scale of the popover: the popover closes, and the change goes to the confirm dialog.
    pub(crate) fn submit_scale(
        &mut self,
        subject: &ClusterObject,
        target: &ScaleTarget,
        replicas: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_value_popover(cx);
        self.start_scale(subject, target, replicas, window, cx);
    }

    /// The Scale argument of the palette: the same change for the cursor row, from its own state now.
    pub(crate) fn scale_cursor_row(
        &mut self,
        replicas: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(subject) = self.selected.clone() else {
            return;
        };
        let Some(target) = self.scale_target_of(&subject, cx) else {
            let label = action_label(ResourceAction::Scale(ObjectKind::Deployment));
            notify(
                window,
                cx,
                unavailable_text(label, "the object is no longer listed"),
            );
            return;
        };
        self.start_scale(&subject, &target, replicas, window, cx);
    }

    /// What the palette says about the cursor row while it asks for replicas: `deployment/api (now
    /// 3)`. `None` when the cursor row cannot scale.
    pub(crate) fn scale_prompt(&self, cx: &App) -> Option<String> {
        let subject = self.selected.as_ref()?;
        let target = self.scale_target_of(subject, cx)?;
        Some(format!(
            "{} (now {})",
            target.subject_text(),
            target.desired
        ))
    }

    /// The intent of scaling `subject` to `replicas`, on the row's own cluster. The row is read again
    /// so the label and the warnings name what is there now, not what the form was opened on.
    fn start_scale(
        &mut self,
        subject: &ClusterObject,
        opened_on: &ScaleTarget,
        replicas: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::Scale(opened_on.kind));
        // A row that left the list since the form opened is refused: the form's own copy of it is
        // old, and the change would name an object that may be gone.
        let Some(target) = self.scale_target_of(subject, cx) else {
            notify(
                window,
                cx,
                unavailable_text(label, "the object is no longer listed"),
            );
            return;
        };
        let intent = {
            let Some(guard) = self.guard_for(&subject.cluster, cx) else {
                notify(
                    window,
                    cx,
                    unavailable_text(label, "the cluster is not open"),
                );
                return;
            };
            let scope = WorkloadScope {
                cluster: &subject.cluster,
                cluster_name: guard.display_name(),
            };
            scale_intent(&scope, &target, replicas)
        };
        match intent {
            Some(intent) => self.start_write(intent, window, cx),
            None => notify(
                window,
                cx,
                unavailable_text(label, "the object name or the count is not valid"),
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
    let created = result
        .as_ref()
        .ok()
        .and_then(|outcome| outcome.created_name.clone());
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
        Ok(()) => success_notice(&label, created.as_deref()),
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

pub(super) fn notify(window: &mut Window, cx: &mut App, text: String) {
    notify_with(window, cx, text, false);
}

pub(super) fn notify_with(window: &mut Window, cx: &mut App, text: String, is_success: bool) {
    let notification = if is_success {
        Notification::success(text)
    } else {
        Notification::warning(text)
    };
    window.push_notification(notification, cx);
}

/// The proof that a shell may open, from the report of the cluster the action started on: both
/// exec verbs allowed. `None` while the permissions are checking, unknown, or denied.
fn exec_permit_of(access: &AccessState) -> Option<ExecPermit> {
    match access {
        AccessState::Known(report) => report.exec_permit(),
        AccessState::Checking { .. } | AccessState::Unknown => None,
    }
}

impl AppShell {
    /// The guarded start of a stream (spec 0036): the gate of the intent's own cluster, then the
    /// confirm dialog of its tier. Nothing opens without the dialog, for every tier and trigger.
    pub(crate) fn start_connect(
        &mut self,
        intent: ConnectIntent,
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
                notify(window, cx, unavailable_text(&intent.button, &reason));
                return;
            }
            (
                guard.generation,
                confirm_step(guard.profile.confirm, intent.risk, intent.expected()),
                guard.profile.environment,
            )
        };
        let inputs = DialogInputs {
            shell: cx.weak_entity(),
            kind: DialogKind::Connect(Rc::new(intent)),
            confirm,
            environment,
            generation,
        };
        let dialog = cx.new(|cx| ConfirmDialog::new(inputs, window, cx));
        #[cfg(test)]
        {
            self.last_dialog = Some(dialog.downgrade());
        }
        ConfirmDialog::open(&dialog, window, cx);
    }

    /// The confirmed start of a dialog. The guard and the connection are read again from the
    /// intent's own cluster: if it was locked, reconnected, or lost a permission since the dialog
    /// opened, nothing opens and nothing is audited.
    pub(crate) fn commit_connect(
        &mut self,
        intent: &ConnectIntent,
        generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prepared = {
            let guard = self.guard_for(&intent.cluster, cx);
            match live_block(guard.as_ref(), &intent.cluster_name, generation) {
                Some(reason) => Err(reason),
                None => {
                    let permit = guard
                        .as_ref()
                        .and_then(|guard| exec_permit_of(guard.access));
                    match (permit, self.slot_connection(&intent.cluster, cx)) {
                        (Some(permit), Some(connection)) => Ok((permit, connection)),
                        _ => Err(guard
                            .as_ref()
                            .map(|guard| match action_availability(intent.action, guard) {
                                ActionAvailability::Disabled { reason } => reason,
                                ActionAvailability::Enabled => "Not permitted".into(),
                            })
                            .unwrap_or_else(|| "The permissions could not be read".into())),
                    }
                }
            }
        };
        match prepared {
            Ok((permit, connection)) => (intent.open)(self, permit, connection, window, cx),
            Err(reason) => notify(window, cx, format!("{}: {reason}", intent.label)),
        }
    }
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
        let intent = {
            let Some(guard) = self.guard_for(cluster, cx) else {
                return false;
            };
            cordon_intent(
                cluster,
                guard.display_name(),
                &node.name,
                &node.status.scheduling,
            )
        };
        match intent {
            Some(intent) => self.show_fixture_dialog(intent, window, cx),
            None => true,
        }
    }

    /// `--screen scale-confirm`: the Scale dialog of the cursor row, to the count two above the
    /// current one, in a fixed state over an unlocked session. `false` while the row is not listed.
    pub(super) fn open_scale_fixture(
        &mut self,
        subject: &ClusterObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let (Some(session), Some(target)) = (
            self.slot_session(&subject.cluster).cloned(),
            self.scale_target_of(subject, cx),
        ) else {
            return false;
        };
        session.update(cx, |session, cx| session.set_lock(WriteLock::Unlocked, cx));
        let intent = {
            let Some(guard) = self.guard_for(&subject.cluster, cx) else {
                return false;
            };
            let scope = WorkloadScope {
                cluster: &subject.cluster,
                cluster_name: guard.display_name(),
            };
            scale_intent(&scope, &target, target.desired + 2)
        };
        match intent {
            Some(intent) => self.show_fixture_dialog(intent, window, cx),
            None => true,
        }
    }

    /// Opens the dialog of `intent` with the dry-run passed and the buttons dead, over the guard of
    /// the intent's own cluster. `false` when that cluster has no guard yet.
    fn show_fixture_dialog(
        &mut self,
        intent: WriteIntent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let inputs = {
            let Some(guard) = self.guard_for(&intent.cluster, cx) else {
                return false;
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
