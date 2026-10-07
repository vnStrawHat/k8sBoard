//! The guarded write flow (spec 0030): gate, confirm dialog, server-side dry-run, lock re-check,
//! commit, audit line, notice. `checked_write` is the only caller of `ClusterConnection::write`.
//!
//! A child of `app_shell`, like `workspace`, because the flow reads the viewed slots. Every step
//! names the cluster of the row or cursor (`WriteIntent::cluster`) and takes its guard and its
//! connection from that cluster's own slot session, never from the primary.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use cluster::{
    AttachPermit, ClusterConnection, ClusterError, ExecPermit, NodeScheduling, ObjectKind,
    ObjectRef, PortForwardPermit, WriteError, WriteMode, WriteOperation, WriteOutcome,
    WriteRequest,
};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::notification::Notification;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, AsyncApp, Context, Entity, SharedString, WeakEntity,
    Window,
};

use super::AppShell;
use super::certificate_renewal::renewal_notice;
use super::rollout_watch::{RolloutToast, rollout_toast_id};
use super::values_edit_flow::{
    env_consumers_after, notify_with_restart, values_success_notice, yaml_success_notice,
};
use crate::audit_log::{
    AuditEntry, AuditField, AuditObject, AuditOutcome, AuditReceipt, append_audit, audit_entry,
    created_name_field, submit_audit, timestamp_now,
};
use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::AccessState;
use crate::confirm_dialog::{ConfirmDialog, DialogInputs, DialogKind};
use crate::kind_row::KindObject;
use crate::live_sections::loaded_replica_sets;
use crate::port_forwards::LocalPortSpec;
use crate::resource_actions::{
    ActionAvailability, NOT_PERMITTED, ResourceAction, action_availability, action_label,
    action_risk, unavailable_text,
};
use crate::resource_kind::ResourceKind;
use crate::revision_diff::RollBackOffer;
use crate::settings::AppSettings;
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::value_popover::ValuePopover;
use crate::workload_actions::{
    PAUSED_REASON, RevisionTarget, ScaleTarget, WorkloadScope, named_restart_intent,
    pending_changes_note, roll_back_intent, row_block, scale_intent, state_label, workload_intent,
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
    /// Non-blocking context lines of the dialog.
    pub(crate) warnings: Vec<SharedString>,
}

impl WriteIntent {
    /// What the `TypeName` tier asks to type: the name of the object the change is on.
    pub(crate) fn expected(&self) -> &str {
        self.request.target().name()
    }

    /// `the deployment name`, `the pod name`, and so on.
    pub(crate) fn typed_hint(&self) -> String {
        format!(
            "the {} name",
            self.request.target().kind_name().to_ascii_lowercase()
        )
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
    pub(crate) open: ConnectOpen,
}

/// The call that opens the stream of a `ConnectIntent`, and so which proof it needs.
pub(crate) enum ConnectOpen {
    Exec(Rc<ExecOpen>),
    /// A forward start: the dialog also asks for the local port.
    PortForward(Rc<PortForwardOpen>, LocalPortChoice),
    /// Creates or changes an object first (a debug container, a node shell pod), then attaches to
    /// it (spec 0037).
    CreateThenAttach(CreateThenAttach),
    /// Attaches to a running container of the pod's own spec (spec 0040): no write, no dry-run.
    Attach(Rc<ContainerAttachOpen>),
}

/// The local port of a forward start, which the confirm dialog lets the user change before it
/// starts anything.
pub(crate) struct LocalPortChoice {
    /// What the field starts with: an exact port shows it, an automatic one leaves it empty.
    pub(crate) initial: LocalPortSpec,
    /// The port an automatic choice tries first; the empty field names it.
    pub(crate) automatic: u16,
    /// Where the dialog leaves the confirmed choice for the open call to read.
    pub(crate) chosen: Rc<Cell<Option<LocalPortSpec>>>,
}

/// A start that writes before it attaches. The write is a `WriteIntent` in every respect: the
/// dialog shows its dry-run and changed fields, `checked_write` sends and audits it. The attach
/// permit is taken before that commit, so a user who cannot attach creates nothing.
pub(crate) struct CreateThenAttach {
    pub(crate) create: Rc<WriteIntent>,
    /// Runs after the commit succeeded, with the permit, the cluster's connection, and what the
    /// server reported (the created pod's name and uid).
    pub(crate) open: Rc<AttachOpen>,
    /// Runs instead of `open` when the commit succeeded but the window is gone, so what was created
    /// is not forgotten: a node shell deletes its pod.
    pub(crate) discard: Rc<DiscardStart>,
}

/// What a start does with the object it created when no window is left to open its tab.
pub(crate) type DiscardStart =
    dyn Fn(&mut AppShell, ClusterConnection, WriteOutcome, &mut Context<AppShell>);

/// The two ways a start that created something goes on: `open` with a window, `discard` without.
pub(crate) struct AttachOpens {
    open: Rc<AttachOpen>,
    discard: Rc<DiscardStart>,
}

pub(crate) type AttachOpen = dyn Fn(
    &mut AppShell,
    AttachPermit,
    ClusterConnection,
    WriteOutcome,
    &mut Window,
    &mut Context<AppShell>,
);

pub(crate) type ContainerAttachOpen =
    dyn Fn(&mut AppShell, AttachPermit, ClusterConnection, &mut Window, &mut Context<AppShell>);

pub(crate) type ExecOpen =
    dyn Fn(&mut AppShell, ExecPermit, ClusterConnection, &mut Window, &mut Context<AppShell>);
pub(crate) type PortForwardOpen = dyn Fn(
    &mut AppShell,
    PortForwardPermit,
    ClusterConnection,
    &mut Window,
    &mut Context<AppShell>,
);

/// A `ConnectOpen` with its proof in hand: only `ConnectOpen::granted` builds one, so the call and
/// the permit always match.
enum GrantedOpen {
    Exec(Rc<ExecOpen>, ExecPermit),
    PortForward(Rc<PortForwardOpen>, PortForwardPermit),
    Attach(Rc<ContainerAttachOpen>, AttachPermit),
    CreateThenAttach(
        Rc<WriteIntent>,
        Rc<AttachOpen>,
        Rc<DiscardStart>,
        AttachPermit,
    ),
}

impl ConnectOpen {
    /// The open call with the proof `access` gives for it; `None` while the permissions are
    /// checking, unknown, or deny a verb the stream needs.
    fn granted(&self, access: &AccessState) -> Option<GrantedOpen> {
        match self {
            Self::Exec(open) => Some(GrantedOpen::Exec(Rc::clone(open), exec_permit_of(access)?)),
            Self::PortForward(open, _) => Some(GrantedOpen::PortForward(
                Rc::clone(open),
                port_forward_permit_of(access)?,
            )),
            Self::Attach(open) => Some(GrantedOpen::Attach(
                Rc::clone(open),
                attach_permit_of(access)?,
            )),
            Self::CreateThenAttach(start) => Some(GrantedOpen::CreateThenAttach(
                Rc::clone(&start.create),
                Rc::clone(&start.open),
                Rc::clone(&start.discard),
                attach_permit_of(access)?,
            )),
        }
    }
}

/// What the dialog hands to the commit: the generation it opened on, and for a start that writes
/// first, the proof that its dry-run passed and the audit note.
pub(crate) struct ConnectCommit {
    pub(crate) generation: u64,
    pub(crate) confirmed: Option<Confirmed>,
    pub(crate) note: Option<String>,
}

impl GrantedOpen {
    fn run(
        self,
        shell: &mut AppShell,
        connection: ClusterConnection,
        commit: ConnectCommit,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) {
        match self {
            Self::Exec(open, permit) => open(shell, permit, connection, window, cx),
            Self::PortForward(open, permit) => open(shell, permit, connection, window, cx),
            Self::Attach(open, permit) => open(shell, permit, connection, window, cx),
            Self::CreateThenAttach(create, open, discard, permit) => {
                // The dialog never confirms a write without its passed dry-run.
                let Some(confirmed) = commit.confirmed else {
                    return;
                };
                let step = WriteStep {
                    intent: create,
                    generation: commit.generation,
                    mode: CommitMode::Commit { confirmed },
                    note: commit.note,
                };
                let opens = AttachOpens { open, discard };
                shell.commit_then_attach(step, opens, permit, connection, window, cx);
            }
        }
    }
}

impl ConnectIntent {
    /// What the `TypeName` tier asks to type: the name of the object the session opens on.
    pub(crate) fn expected(&self) -> &str {
        &self.object.name
    }

    /// `the pod name`, `the node name`, and so on.
    pub(crate) fn typed_hint(&self) -> String {
        format!("the {} name", self.object.kind.to_ascii_lowercase())
    }

    /// Why the gate of the action no longer allows this start, `None` when it does. The dialog reads
    /// it on every render and the commit reads it again: the permissions, the node shell setting, or
    /// the tier may have changed while the dialog stood open.
    pub(crate) fn gate_block(&self, guard: &ClusterGuard<'_>) -> Option<SharedString> {
        match action_availability(self.action, guard) {
            ActionAvailability::Disabled { reason } => Some(reason),
            ActionAvailability::Enabled => None,
        }
    }

    /// The local port a forward start lets the dialog change.
    pub(crate) fn local_port(&self) -> Option<&LocalPortChoice> {
        match &self.open {
            ConnectOpen::PortForward(_, choice) => Some(choice),
            ConnectOpen::Exec(_) | ConnectOpen::CreateThenAttach(_) | ConnectOpen::Attach(_) => {
                None
            }
        }
    }

    /// The write a start makes before it attaches, if it makes one.
    pub(crate) fn create(&self) -> Option<&Rc<WriteIntent>> {
        match &self.open {
            ConnectOpen::CreateThenAttach(start) => Some(&start.create),
            ConnectOpen::Exec(_) | ConnectOpen::PortForward(..) | ConnectOpen::Attach(_) => None,
        }
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
        let (Some(guard), Some(connection)) = (guard, self.connection_of(&intent.cluster, cx))
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
    // The first of the two senders of a write (the other is `run_cleanup`; spec 0030 AC 9).
    #[expect(clippy::disallowed_methods)]
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

/// Queues `entry` on the audit writer at once; with no settings folder nothing is written.
pub(super) fn queue_audit(config_dir: Option<&Path>, entry: &AuditEntry) -> Option<AuditReceipt> {
    config_dir.map(|dir| submit_audit(dir, entry))
}

/// Waits for a queued line. A failure warns and shows a title-bar notice, but never undoes the
/// change.
pub(super) async fn report_audit(
    queued: Option<AuditReceipt>,
    shell: &WeakEntity<AppShell>,
    cx: &mut AsyncApp,
) {
    let Some(queued) = queued else {
        return;
    };
    if let Err(error) = queued.await {
        tracing::warn!(kind = ?error.kind(), "could not append to the audit log");
        let _ = shell.update(cx, |shell, cx| shell.audit_failed(error.kind(), cx));
    }
}

/// Queues `entry` and waits for it, for a flow that goes on after its line is written.
pub(super) async fn append_in_background(
    shell: &WeakEntity<AppShell>,
    config_dir: Option<PathBuf>,
    entry: AuditEntry,
    cx: &mut AsyncApp,
) {
    report_audit(queue_audit(config_dir.as_deref(), &entry), shell, cx).await;
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

/// The leading verb of an intent label and its past tense: `Cordon node wk-04` reads
/// `Cordoned node wk-04` once it went through.
const PAST_TENSE: [(&str, &str); 14] = [
    ("Restart", "Restarted"),
    ("Pause", "Paused"),
    ("Resume", "Resumed"),
    ("Suspend", "Suspended"),
    ("Scale", "Scaled"),
    ("Roll back", "Rolled back"),
    ("Re-run", "Re-ran"),
    ("Run", "Ran"),
    ("Uncordon", "Uncordoned"),
    ("Cordon", "Cordoned"),
    ("Expand", "Expanded"),
    ("Edit", "Edited"),
    ("Set", "Set"),
    ("Make", "Made"),
];

/// The notice of a commit that went through: a create names what it made, a rollout action says
/// the rollout is under way (`is_watched`), and a label of an unknown verb keeps the plain `done`.
fn success_notice(label: &str, created: Option<&str>, is_watched: bool) -> String {
    if let Some(name) = created {
        return format!("{label}: created {name}");
    }
    let past = PAST_TENSE.iter().find_map(|(verb, past)| {
        let rest = label.strip_prefix(verb)?.strip_prefix(' ')?;
        Some(format!("{past} {rest}"))
    });
    match (past, is_watched) {
        (Some(past), true) => format!("{past}. {WATCHING_ROLLOUT}"),
        (Some(past), false) => format!("{past}."),
        (None, _) => format!("{label}: done"),
    }
}

const WATCHING_ROLLOUT: &str = "Watching rollout…";

/// Whether the commit starts a Deployment rollout the app follows to its end (`rollout_watch`):
/// only a Deployment reports the status it needs. A Restart, a Roll back, a Resume, a Scale, and
/// an Edit YAML that changes the pod template or the replicas qualify.
pub(crate) fn watches_rollout(intent: &WriteIntent) -> bool {
    match (intent.action, intent.request.operation()) {
        (ResourceAction::RestartRollout(ObjectKind::Deployment), _)
        | (ResourceAction::RollBack, _)
        | (ResourceAction::Scale(ObjectKind::Deployment), _) => true,
        (ResourceAction::PauseRollout, WriteOperation::SetRolloutPaused { paused }) => !paused,
        (ResourceAction::EditYaml(ObjectKind::Deployment), _) => {
            intent.request.changed_fields().iter().any(|field| {
                field.path.starts_with("spec.template") || field.path == "spec.replicas"
            })
        }
        _ => false,
    }
}

/// Whether the commit edited a Deployment that is paused: its pods change only after Resume, so
/// there is no rollout to follow. Read from the lists the session holds.
fn is_paused_edit(shell: &WeakEntity<AppShell>, intent: &WriteIntent, cx: &AsyncApp) -> bool {
    if !matches!(intent.action, ResourceAction::EditYaml(_)) {
        return false;
    }
    let target = intent.request.target();
    let Some(namespace) = target.namespace() else {
        return false;
    };
    shell
        .read_with(cx, |shell, cx| {
            let live = shell.live_of(&intent.cluster, cx)?;
            Some(live.deployment_of(namespace, target.name())?.is_paused)
        })
        .ok()
        .flatten()
        .unwrap_or(false)
}

/// The Deployment (`namespace`, `name`) whose rollout the app follows after this commit.
fn watched_workload(intent: &WriteIntent, is_watched: bool) -> Option<(String, String)> {
    if !is_watched {
        return None;
    }
    let target = intent.request.target();
    Some((target.namespace()?.to_owned(), target.name().to_owned()))
}

/// Whether the action starts a rollout the drawer can show progress for.
fn starts_rollout(action: ResourceAction) -> bool {
    matches!(
        action,
        ResourceAction::RestartRollout(_) | ResourceAction::RollBack
    )
}

/// The workload a rollout started, for the notice's View button.
fn rollout_subject(intent: &WriteIntent, is_watched: bool) -> Option<ClusterObject> {
    if !is_watched && !starts_rollout(intent.action) {
        return None;
    }
    let target = intent.request.target();
    let key = ResourceKey::of_object(target.kind_name(), target.namespace(), target.name())?;
    Some(ClusterObject::new(intent.cluster.clone(), key))
}

/// The Job a Trigger now or Re-run created, for the notice's View button. The row may not have
/// reached the watch yet; the reveal waits for it.
fn created_job_subject(intent: &WriteIntent, created: Option<&str>) -> Option<ClusterObject> {
    if !matches!(
        intent.action,
        ResourceAction::TriggerCronJob | ResourceAction::RerunJob
    ) {
        return None;
    }
    let namespace = intent.request.target().namespace();
    let key = ResourceKey::of_object("Job", namespace, created?)?;
    Some(ClusterObject::new(intent.cluster.clone(), key))
}

/// `Created ConfigMap payments/new-config` (a Namespace has no `payments/`).
fn create_success_notice(request: &WriteRequest) -> String {
    let target = request.target();
    match target.namespace() {
        Some(namespace) => format!(
            "Created {} {namespace}/{}",
            target.kind_name(),
            target.name()
        ),
        None => format!("Created {} {}", target.kind_name(), target.name()),
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
pub(crate) fn cordon_intent(
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
                (self.guard_for(cluster, cx), self.live_of(cluster, cx))
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
                self.live_of(&subject.cluster, cx),
            ) else {
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
            let now = jiff::Timestamp::now();
            match (live.row_of(&subject.key), action, &subject.key) {
                (Some(row), _, _) => {
                    if let Some(reason) = row_block(action, &row.object, None) {
                        let label = state_label(action, label, &row.object);
                        notify(window, cx, unavailable_text(label, &reason));
                        return;
                    }
                    let mut intent = workload_intent(action, &scope, &row.object, now);
                    // A Resume rolls out what was changed while paused: the confirm lists it.
                    if let (
                        Some(intent),
                        ResourceAction::PauseRollout,
                        KindObject::Deployment(deployment),
                    ) = (intent.as_mut(), action, &row.object)
                        && deployment.is_paused
                    {
                        let sets = loaded_replica_sets(ResourceKind::Deployments, row, live);
                        intent.warnings.push(pending_changes_note(deployment, sets));
                    }
                    intent
                }
                // A Restart of a Used by consumer: the drawer of a ConfigMap or Secret is open, so
                // no list holds the workload, and it is restarted by the name its pods gave.
                (
                    None,
                    ResourceAction::RestartRollout(kind),
                    ResourceKey::Kind {
                        namespace: Some(namespace),
                        name,
                        ..
                    },
                ) => named_restart_intent(&scope, kind, namespace, name, now),
                (None, _, _) => {
                    let text = unavailable_text(label, "the object is no longer listed");
                    notify(window, cx, text);
                    return;
                }
            }
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
    /// palette entry. Refused while an edit is open; the Revision history tab of that edit and the
    /// revision diff dialog come to `begin_roll_back` through their `RollBackOffer`.
    pub(crate) fn start_roll_back(
        &mut self,
        subject: &ClusterObject,
        target: &RevisionTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The palette hides these entries while an edit is open; a direct call is refused too.
        if self.is_editing() {
            return;
        }
        self.begin_roll_back(subject, target, window, cx);
    }

    /// What the Roll back buttons of the revision diff dialog and of the Revision history tab do,
    /// from the gate of the cluster of the Deployment `subject`: the same reasons as the drawer's
    /// buttons. The Deployment's own state is read again when the button is pressed.
    pub(crate) fn roll_back_offer(
        &self,
        shell: WeakEntity<Self>,
        subject: ClusterObject,
        cx: &App,
    ) -> RollBackOffer {
        let Some(guard) = self.guard_for(&subject.cluster, cx) else {
            return RollBackOffer::Disabled("Not connected".into());
        };
        if let ActionAvailability::Disabled { reason } =
            action_availability(ResourceAction::RollBack, &guard)
        {
            return RollBackOffer::Disabled(reason);
        }
        let is_paused = self.live_of(&subject.cluster, cx).is_some_and(|live| {
            matches!(
                live.row_of(&subject.key).map(|row| &row.object),
                Some(KindObject::Deployment(deployment)) if deployment.is_paused
            )
        });
        if is_paused {
            return RollBackOffer::Disabled(PAUSED_REASON.into());
        }
        RollBackOffer::Enabled { shell, subject }
    }

    /// The roll back itself. The Deployment is read again from its own cluster, so a rollout paused
    /// since the button was drawn is refused here, and the dialog names what is there now.
    pub(crate) fn begin_roll_back(
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
                self.live_of(&subject.cluster, cx),
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
        let live = self.live_of(&subject.cluster, cx)?;
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
        // The palette hides these entries while an edit is open; a direct call is refused too.
        if self.is_editing() {
            return;
        }
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
                let (shell, cluster) = (cx.weak_entity(), intent.cluster.clone());
                notify_unavailable(window, cx, &intent.button, &reason, shell, cluster);
                return;
            }
            if let Some(reason) = self.drain_conflict(&intent.cluster, intent.action, cx) {
                notify(window, cx, unavailable_text(&intent.button, &reason));
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
            // An edit shows its own outcome: the editor closes, or tells what went wrong in place.
            if matches!(intent.action, ResourceAction::EditYaml(_)) {
                let _ = shell.update(cx, |shell, cx| {
                    shell.edit_commit_finished(&intent, &result, cx)
                });
            }
            if intent.action == ResourceAction::RollBack && result.is_ok() {
                let _ = shell.update(cx, |shell, cx| shell.roll_back_finished(&intent, cx));
            }
            if matches!(intent.action, ResourceAction::EditValues(_)) {
                let _ = shell.update(cx, |shell, cx| {
                    shell.values_commit_finished(&intent, &result, cx)
                });
            }
            // A New object closes its view on success and shows a failure in place; the notice
            // below is the write flow's own.
            if matches!(intent.action, ResourceAction::CreateObject(_)) {
                let _ = shell.update(cx, |shell, cx| {
                    shell.create_commit_finished(&intent, &result, cx)
                });
            }
            finish_commit(&shell, &dialog, &intent, handle, result, cx);
        })
        .detach();
    }
}

/// Shows the result of a commit: the dialog closes with a notice, or stays with a Retry when the
/// failure is one the user can retry and the dialog is still open.
fn finish_commit(
    shell: &WeakEntity<AppShell>,
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
    // A taint edit that conflicts goes straight back to its editor, with the node reloaded.
    let is_taint_conflict = intent.action == ResourceAction::EditTaints
        && matches!(
            &result,
            Err(CheckedWriteError::Write(WriteError::Conflict { .. }))
        );
    if is_taint_conflict {
        let reopened = cx.update_window(window, |_, window, cx| {
            dialog
                .update(cx, |dialog, cx| dialog.reload_after_conflict(window, cx))
                .unwrap_or(false)
        });
        if reopened.is_ok_and(|reopened| reopened) {
            return;
        }
    }
    if let Err(error) = &result
        // A stale `resourceVersion` cannot pass a second time, so an edit never offers Retry: its
        // editor rebases instead. A create never does either: after an unknown outcome a repeat
        // reads as a 409, and after a refusal the text must change first (spec 0042 decision 12).
        && !matches!(
            intent.action,
            ResourceAction::EditYaml(_)
                | ResourceAction::EditValues(_)
                | ResourceAction::CreateObject(_)
        )
        && let Some(text) = retryable_text(error)
        && dialog
            .update(cx, |dialog, cx| {
                let is_conflict = matches!(
                    error,
                    CheckedWriteError::Write(WriteError::Conflict { .. })
                );
                dialog.commit_failed(text.clone(), is_conflict, cx)
            })
            .unwrap_or(false)
    {
        return;
    }
    let is_watched =
        result.is_ok() && watches_rollout(intent) && !is_paused_edit(shell, intent, cx);
    let notice = match &result {
        Ok(()) if matches!(intent.action, ResourceAction::EditValues(_)) => {
            let target = intent.request.target();
            let count = intent.request.changed_fields().len();
            values_success_notice(target.kind_name(), target.name(), count)
        }
        Ok(()) if matches!(intent.action, ResourceAction::EditYaml(_)) => {
            let target = intent.request.target();
            let notice = yaml_success_notice(target.kind_name(), target.name());
            match is_watched {
                true => format!("{notice}. {WATCHING_ROLLOUT}"),
                false => notice,
            }
        }
        Ok(()) if matches!(intent.action, ResourceAction::CreateObject(_)) => {
            create_success_notice(&intent.request)
        }
        Ok(()) if intent.action == ResourceAction::RenewCertificate => {
            renewal_notice(intent.request.target())
        }
        Ok(()) => success_notice(&label, created.as_deref(), is_watched),
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
        let created_view =
            created_job_subject(intent, created.as_deref()).filter(|_| result.is_ok());
        match rollout_subject(intent, is_watched).filter(|_| result.is_ok()) {
            Some(subject) => {
                let workload = watched_workload(intent, is_watched);
                let toast = workload
                    .as_ref()
                    .map(|workload| rollout_toast_id(std::slice::from_ref(workload)));
                notify_with_view(window, cx, notice, shell, subject, toast);
                if let Some(workload) = workload {
                    let (cluster, handle) = (intent.cluster.clone(), window.window_handle());
                    let _ = shell.update(cx, |shell, cx| {
                        shell.watch_rollouts(cluster, vec![workload], handle, cx);
                    });
                }
                watch_hpa_after_scale(shell, intent, window.window_handle(), cx);
            }
            None => match created_view {
                Some(subject) => notify_with_view(window, cx, notice, shell, subject, None),
                None => {
                    let consumers = env_consumers_after(shell, intent, result.is_ok(), cx);
                    if consumers.is_empty() {
                        notify_with(window, cx, notice, result.is_ok());
                    } else {
                        let source = intent.request.target().name();
                        notify_with_restart(
                            window,
                            cx,
                            notice,
                            shell,
                            &intent.cluster,
                            source,
                            consumers,
                        );
                    }
                    if result.is_ok() {
                        watch_hpa_after_scale(shell, intent, window.window_handle(), cx);
                    }
                }
            },
        }
    });
}

/// After a Scale of a workload that an HPA targets: follows its replicas for a while, because the
/// HPA may set them back. The HPA is read from the Issues feed as the Scale warning reads it.
fn watch_hpa_after_scale(
    shell: &WeakEntity<AppShell>,
    intent: &WriteIntent,
    window: AnyWindowHandle,
    cx: &mut App,
) {
    let (ResourceAction::Scale(_), WriteOperation::ScaleWorkload { replicas }) =
        (intent.action, intent.request.operation())
    else {
        return;
    };
    let target = intent.request.target();
    let Some(key) = ResourceKey::of_object(target.kind_name(), target.namespace(), target.name())
    else {
        return;
    };
    let subject = ClusterObject::new(intent.cluster.clone(), key);
    let hpa = shell
        .read_with(cx, |shell, cx| shell.scale_target_of(&subject, cx))
        .ok()
        .flatten()
        .and_then(|target| target.hpa);
    let Some(hpa) = hpa else {
        return;
    };
    let replicas = *replicas;
    let _ = shell.update(cx, |shell, cx| {
        shell.watch_hpa_scale(subject, replicas, hpa, window, cx);
    });
}

pub(super) fn notify(window: &mut Window, cx: &mut App, text: String) {
    notify_with(window, cx, text, false);
}

/// A success notice with a View button that reveals `subject` (recorded for Back). `toast` is the
/// id the end of the rollout watch replaces the notice under.
fn notify_with_view(
    window: &mut Window,
    cx: &mut App,
    text: String,
    shell: &WeakEntity<AppShell>,
    subject: ClusterObject,
    toast: Option<SharedString>,
) {
    let shell = shell.clone();
    let notification = Notification::success(text).action(move |_, _, cx| {
        let (shell, subject) = (shell.clone(), subject.clone());
        Button::new("rollout-view")
            .label("View")
            .small()
            .outline()
            .on_click(cx.listener(move |notification, _, window, cx| {
                notification.dismiss(window, cx);
                let subject = subject.clone();
                let _ = shell.update(cx, |shell, cx| shell.reveal_object(subject, cx));
            }))
    });
    let notification = match toast {
        Some(id) => notification.id1::<RolloutToast>(id),
        None => notification,
    };
    window.push_notification(notification, cx);
}

/// The notice of an action the gate refused. A permission denial offers Check permissions, which
/// opens the dialog for the cluster the action was on.
pub(super) fn notify_unavailable(
    window: &mut Window,
    cx: &mut App,
    label: &str,
    reason: &str,
    shell: WeakEntity<AppShell>,
    cluster: ClusterRef,
) {
    let text = unavailable_text(label, reason);
    if !reason.starts_with(NOT_PERMITTED) {
        notify(window, cx, text);
        return;
    }
    let notification = Notification::warning(text).action(move |_, _, cx| {
        let (shell, cluster) = (shell.clone(), cluster.clone());
        Button::new("check-permissions")
            .label("Check permissions")
            .small()
            .outline()
            .on_click(cx.listener(move |notification, _, window, cx| {
                notification.dismiss(window, cx);
                let cluster = cluster.clone();
                let _ = shell.update(cx, |shell, cx| {
                    let namespace = shell.tool_namespace(cx);
                    shell.open_permissions(&cluster, None, namespace, true, window, cx);
                });
            }))
    });
    window.push_notification(notification, cx);
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

/// The proof that a forward may start: both port-forward verbs allowed.
fn port_forward_permit_of(access: &AccessState) -> Option<PortForwardPermit> {
    match access {
        AccessState::Known(report) => report.port_forward_permit(),
        AccessState::Checking { .. } | AccessState::Unknown => None,
    }
}

/// The proof that a debug shell may attach: both attach verbs allowed.
fn attach_permit_of(access: &AccessState) -> Option<AttachPermit> {
    match access {
        AccessState::Known(report) => report.attach_permit(),
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
                let (shell, cluster) = (cx.weak_entity(), intent.cluster.clone());
                notify_unavailable(window, cx, &intent.button, &reason, shell, cluster);
                return;
            }
            (
                guard.generation,
                confirm_step(guard.profile.confirm, intent.risk, intent.expected()),
                guard.profile.environment,
            )
        };
        let has_dry_run = intent.create().is_some();
        let inputs = DialogInputs {
            shell: cx.weak_entity(),
            kind: DialogKind::Connect(Rc::new(intent)),
            confirm,
            environment,
            generation,
        };
        let dialog = cx.new(|cx| ConfirmDialog::new(inputs, window, cx));
        // A start that writes first checks the write on the server before the user can confirm.
        if has_dry_run {
            dialog.update(cx, |dialog, cx| dialog.start_dry_run(cx));
        }
        #[cfg(test)]
        {
            self.last_dialog = Some(dialog.downgrade());
        }
        ConfirmDialog::open(&dialog, window, cx);
    }

    /// The confirmed start of a dialog. The guard, the permit, and the connection are read again
    /// from the intent's own cluster: if it was locked, reconnected, or lost a permission since the
    /// dialog opened, nothing opens, nothing is created, and nothing is audited. A start that
    /// writes first has its permit in hand before the commit goes out.
    pub(crate) fn commit_connect(
        &mut self,
        intent: &ConnectIntent,
        commit: ConnectCommit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let generation = commit.generation;
        let prepared = {
            let guard = self.guard_for(&intent.cluster, cx);
            // The gate is read again, not only the lock: the setting, the tier, or a re-review may
            // have changed since the dialog opened, and a privileged pod must not follow a stale yes.
            let gate_block = guard.as_ref().and_then(|guard| intent.gate_block(guard));
            match live_block(guard.as_ref(), &intent.cluster_name, generation).or(gate_block) {
                Some(reason) => Err(reason),
                None => {
                    let granted = guard
                        .as_ref()
                        .and_then(|guard| intent.open.granted(guard.access));
                    match (granted, self.connection_of(&intent.cluster, cx)) {
                        (Some(granted), Some(connection)) => Ok((granted, connection)),
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
            Ok((granted, connection)) => granted.run(self, connection, commit, window, cx),
            Err(reason) => notify(window, cx, format!("{}: {reason}", intent.label)),
        }
    }

    /// The commit of a start that writes first, then the attach: the write is `checked_write`'s
    /// (lock and connection re-checked, audit line), and `open` runs only if it succeeded. A failed
    /// write opens nothing and says why.
    ///
    /// A node shell create counts as in flight until its result is handled, so the window does not
    /// close under it. If the commit succeeded but the window is gone, `discard` runs instead of
    /// `open`: the pod exists and must not be forgotten.
    fn commit_then_attach(
        &mut self,
        step: WriteStep,
        opens: AttachOpens,
        permit: AttachPermit,
        connection: ClusterConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let handle: AnyWindowHandle = window.window_handle();
        let shell = cx.weak_entity();
        let create = Rc::clone(&step.intent);
        let is_node_shell = matches!(
            create.request.operation(),
            WriteOperation::CreateNodeShellPod { .. }
        );
        if is_node_shell {
            self.node_shell_runs.create_started();
        }
        // Whatever the answer, the create stops counting only after its pod has an owner.
        let finish = move |shell: &mut AppShell, cx: &mut Context<AppShell>| {
            if is_node_shell {
                shell.node_shell_runs.create_finished();
                shell.close_window_when_idle(cx);
            }
        };
        cx.spawn(async move |_, cx| {
            match checked_write(&shell, step, cx).await {
                Ok(outcome) => {
                    let AttachOpens { open, discard } = opens;
                    let spare = (connection.clone(), outcome.clone());
                    let opened = cx.update_window(handle, |_, window, cx| {
                        shell.update(cx, |shell, cx| {
                            open(shell, permit, connection, outcome, window, cx);
                            finish(shell, cx);
                        })
                    });
                    if !matches!(opened, Ok(Ok(()))) {
                        // The window is gone (or the shell is): nothing opened the tab.
                        let (connection, outcome) = spare;
                        let _ = shell.update(cx, |shell, cx| {
                            discard(shell, connection, outcome, cx);
                            finish(shell, cx);
                        });
                    }
                }
                Err(error) => {
                    let notice = failure_notice(&create.label, &error);
                    let _ = cx.update_window(handle, |_, window, cx| notify(window, cx, notice));
                    let _ = shell.update(cx, |shell, cx| finish(shell, cx));
                }
            }
        })
        .detach();
    }
}

/// Deletes one node shell pod of this run (or one a sweep found): the cleanup of spec 0037. It has
/// no gate, lock, tier, or dialog, because removing a k8sBoard privileged pod must work after a
/// lock, a cluster switch, and inside the shutdown window. What keeps that safe is the type: it is
/// built only from a `DeleteNodeShellPod` request, which the write path accepts only for a pod
/// named `k8sboard-node-shell-…` and sends with the pod's uid as a precondition.
pub(crate) struct NodeShellCleanup {
    /// The connection of the pod's own cluster, held since the start: the delete outlives the slot.
    connection: ClusterConnection,
    request: WriteRequest,
    /// The cluster, context, and user, copied at the start for the audit line.
    audit: CleanupAudit,
}

/// The cluster, context, and user of a cleanup, copied when the pod is made.
#[derive(Clone)]
pub(crate) struct CleanupAudit {
    cluster: String,
    context: String,
    user: Option<String>,
}

/// What a cleanup delete came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CleanupOutcome {
    /// Deleted, or already gone (a 404 counts as done).
    Done,
    /// Debug builds block writes: nothing was sent, so nothing is audited.
    Blocked,
    /// Not deleted; the text names why.
    Failed(String),
}

impl CleanupAudit {
    pub(crate) fn of(guard: &ClusterGuard<'_>) -> Self {
        Self {
            cluster: guard.display_name().to_owned(),
            context: guard.summary.name.clone(),
            user: guard.summary.user.clone(),
        }
    }
}

impl NodeShellCleanup {
    /// `None` unless `request` is a `DeleteNodeShellPod`: no other write can take this path.
    pub(crate) fn new(
        connection: ClusterConnection,
        request: WriteRequest,
        audit: CleanupAudit,
    ) -> Option<Self> {
        if !matches!(
            request.operation(),
            WriteOperation::DeleteNodeShellPod { .. }
        ) {
            return None;
        }
        Some(Self {
            connection,
            request,
            audit,
        })
    }

    pub(crate) fn namespace(&self) -> &str {
        self.request.target().namespace().unwrap_or_default()
    }

    pub(crate) fn pod(&self) -> &str {
        self.request.target().name()
    }

    /// The audit line of the delete. `Abandoned` is the one written for a delete that never
    /// reported (the app quit first).
    pub(crate) fn audit_entry(&self, outcome: AuditOutcome, error: Option<String>) -> AuditEntry {
        let target = self.request.target();
        AuditEntry {
            at: timestamp_now(),
            cluster: self.audit.cluster.clone(),
            context: self.audit.context.clone(),
            user: self.audit.user.clone(),
            action: CLEANUP_ACTION.to_owned(),
            object: Some(AuditObject {
                kind: target.kind_name().to_owned(),
                namespace: target.namespace().map(str::to_owned),
                name: target.name().to_owned(),
            }),
            fields: self
                .request
                .changed_fields()
                .into_iter()
                .map(|field| AuditField {
                    path: field.path.into_owned(),
                    value: field.value,
                })
                .collect(),
            outcome,
            error,
            note: None,
        }
    }
}

const CLEANUP_ACTION: &str = "Delete node shell pod";

/// The second sender of a write (the first is `checked_write`): a commit-only delete of a node
/// shell pod, then its audit line. It runs entirely on the tokio runtime, so it also works from the
/// quit hook, where the main thread only waits. The audit line is appended here, not by the caller,
/// so no end of a session can skip it.
pub(crate) async fn run_cleanup(
    cleanup: NodeShellCleanup,
    runtime: &ClusterRuntime,
    config_dir: Option<PathBuf>,
) -> CleanupOutcome {
    let sent = runtime
        .spawn(async move { delete_and_audit(&cleanup, config_dir.as_deref()).await })
        .await;
    sent.unwrap_or_else(|_| CleanupOutcome::Failed("the delete task stopped".to_owned()))
}

async fn delete_and_audit(
    cleanup: &NodeShellCleanup,
    config_dir: Option<&std::path::Path>,
) -> CleanupOutcome {
    // The one write outside `checked_write` (spec 0030 AC 9, spec 0037): a delete of this
    // application's own node shell pod under a uid precondition. See `NodeShellCleanup`.
    #[expect(clippy::disallowed_methods)]
    let result = cleanup
        .connection
        .write(&cleanup.request, WriteMode::Commit)
        .await;
    let (outcome, audit) = match result {
        Ok(_) | Err(WriteError::NotFound) => (CleanupOutcome::Done, Some(AuditOutcome::Applied)),
        Err(WriteError::WritesBlocked) => (CleanupOutcome::Blocked, None),
        Err(WriteError::Conflict { .. }) => (
            // Another pod took the name since: it is not ours to delete.
            CleanupOutcome::Failed("another pod now has that name".to_owned()),
            Some(AuditOutcome::Failed),
        ),
        Err(WriteError::OutcomeUnknown) => (
            CleanupOutcome::Failed(WriteError::OutcomeUnknown.to_string()),
            Some(AuditOutcome::Unknown),
        ),
        Err(error) => (
            CleanupOutcome::Failed(write_error_text(&error)),
            Some(AuditOutcome::Failed),
        ),
    };
    if let (Some(audit), Some(dir)) = (audit, config_dir) {
        let error = match &outcome {
            CleanupOutcome::Failed(text) => Some(text.clone()),
            CleanupOutcome::Done | CleanupOutcome::Blocked => None,
        };
        // One short append; the runtime thread may wait for it.
        if let Err(error) = append_audit(dir, &cleanup.audit_entry(audit, error)) {
            tracing::warn!(kind = ?error.kind(), "could not append to the audit log");
        }
    }
    outcome
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
        let Some(session) = self.session_of(cluster).cloned() else {
            return false;
        };
        let node = match self.live_of(cluster, cx) {
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
            self.session_of(&subject.cluster).cloned(),
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
                environment: guard.profile.environment.clone(),
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
