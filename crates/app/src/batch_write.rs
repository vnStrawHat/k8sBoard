//! Batch (spec 0032): the one bulk mechanism. A batch changes several ticked objects of one
//! cluster with the same action. It always opens the confirm dialog, dry-runs every item one at a
//! time and needs all of them to pass, then commits one at a time through `checked_write`, so every
//! commit re-checks the lock and writes its own audit line.
//!
//! A child of `app_shell`, like `write_flow`: it reads the viewed slots and starts the dialog.

use std::rc::Rc;
use std::time::Duration;

use cluster::{ObjectKind, WriteError, WriteOutcome, WriteRequest};
use gpui_kit::{AnyWindowHandle, App, AppContext as _, Context, SharedString, WeakEntity, Window};

use super::write_flow::{
    CheckedWriteError, CommitMode, Confirmed, DryRunState, WriteIntent, WriteStep, checked_write,
    notify, notify_with, write_error_text,
};
use super::{AppShell, Screen};
use crate::cluster_registry::ClusterRef;
use crate::confirm_dialog::{ConfirmDialog, DialogInputs, DialogKind};
use crate::kind_row::KindObject;
use crate::resource_actions::{
    ActionAvailability, NOT_SHIPPED_REASON, ResourceAction, action_availability, action_label,
    unavailable_text,
};
use crate::row_selection::{BulkButton, BulkState, ROLL_BACK_BULK_REASON, bulk_actions};
use crate::table_view::{FilteredTable as _, TableView};
use crate::value_popover::ValuePopover;
use crate::workload_actions::{BulkInputs, all_suspended, bulk_intent, bulk_scale_intent};
#[cfg(feature = "screenshot")]
use crate::write_guard::WriteLock;
use crate::write_guard::{ActionRisk, confirm_step};

/// The most objects one batch changes: more would be a long run of requests behind one click.
pub(crate) const MAX_BATCH_ITEMS: usize = 50;

/// Why the bulk buttons are off while a confirmed batch still commits on their cluster.
const BATCH_RUNNING_REASON: &str = "A batch is running";

/// One object of a batch, with the request that changes it.
#[derive(Clone)]
pub(crate) struct BatchItem {
    /// `namespace/name`, as the list of the dialog shows it.
    pub(crate) object: SharedString,
    /// What the audit line and the notices of this item call the change.
    pub(crate) label: SharedString,
    pub(crate) request: WriteRequest,
}

/// A ticked object the batch leaves alone, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SkippedItem {
    pub(crate) object: SharedString,
    pub(crate) reason: SharedString,
}

#[derive(Clone)]
pub(crate) struct BatchPlan {
    /// The cluster of every item: the batch is refused when the ticked rows span several.
    pub(crate) cluster: ClusterRef,
    pub(crate) items: Vec<BatchItem>,
    pub(crate) skipped: Vec<SkippedItem>,
}

/// A ticked row with the cluster it came from (0027).
#[derive(Clone, Copy)]
pub(crate) struct CheckedRow<'a> {
    pub(crate) cluster: &'a ClusterRef,
    pub(crate) object: &'a KindObject,
}

/// Turns the ticked rows into a plan: each row becomes an item or a skip. `Err` is the reason the
/// button is off.
pub(crate) fn batch_plan(
    rows: &[CheckedRow<'_>],
    build: impl Fn(&CheckedRow<'_>) -> Result<BatchItem, SkippedItem>,
) -> Result<BatchPlan, SharedString> {
    let Some(first) = rows.first() else {
        return Err("Select rows first".into());
    };
    if rows.iter().any(|row| row.cluster != first.cluster) {
        return Err("Select rows of one cluster".into());
    }
    if rows.len() > MAX_BATCH_ITEMS {
        return Err(format!("Select at most {MAX_BATCH_ITEMS} rows").into());
    }
    let (mut items, mut skipped) = (Vec::new(), Vec::new());
    for row in rows {
        match build(row) {
            Ok(item) => items.push(item),
            Err(skip) => skipped.push(skip),
        }
    }
    if items.is_empty() {
        let reason = skipped
            .first()
            .map_or_else(|| "Nothing to change".into(), |skip| skip.reason.clone());
        return Err(reason);
    }
    Ok(BatchPlan {
        cluster: first.cluster.clone(),
        items,
        skipped,
    })
}

/// One guarded batch, as the action builds it. `cluster` is the cluster of the ticked rows: the
/// gate, the tier, the typed name, the dialog badge, the connection, and every audit line come
/// from it.
pub(crate) struct BatchIntent {
    pub(crate) cluster: ClusterRef,
    pub(crate) cluster_name: SharedString,
    pub(crate) action: ResourceAction,
    /// The dialog title: `Restart 4 deployments`.
    pub(crate) label: SharedString,
    /// The verb of the confirm button and the notice (`Restart`; the button adds the count).
    pub(crate) verb: SharedString,
    /// The action of each audit line (`Trigger now`).
    pub(crate) button: SharedString,
    pub(crate) risk: ActionRisk,
    /// Non-blocking context lines of the dialog.
    pub(crate) warnings: Vec<SharedString>,
    pub(crate) plan: BatchPlan,
}

impl BatchIntent {
    /// What the `TypeName` tier asks to type: the cluster, since a batch names no single object.
    pub(crate) fn expected(&self) -> &str {
        &self.cluster_name
    }

    /// The confirm button: `Restart 4`.
    pub(crate) fn confirm_label(&self) -> String {
        format!("{} {}", self.verb, self.plan.items.len())
    }

    /// The write of one item, as `checked_write` takes it: the batch's cluster, action, risk, and
    /// warnings with the item's label and request.
    pub(crate) fn item_intent(&self, item: &BatchItem) -> WriteIntent {
        WriteIntent {
            cluster: self.cluster.clone(),
            cluster_name: self.cluster_name.clone(),
            action: self.action,
            label: item.label.clone(),
            button: self.button.clone(),
            request: item.request.clone(),
            risk: self.risk,
            expected_name: None,
            warnings: self.warnings.clone(),
        }
    }
}

/// Where one item of a batch stands, from its dry-run to its commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ItemProgress {
    Waiting,
    Checking,
    Passed,
    /// A dry-run that did not pass; the text names why.
    Rejected(SharedString),
    Applying,
    Done,
    /// A commit the server refused or that failed before it left.
    Failed(SharedString),
    /// A commit whose request may have left before it failed.
    Unknown,
    NotSent(SharedString),
}

impl ItemProgress {
    /// The state as the list of the dialog shows it.
    pub(crate) fn text(&self) -> String {
        match self {
            Self::Waiting => "waiting".to_owned(),
            Self::Checking => "…".to_owned(),
            Self::Passed => "passed".to_owned(),
            Self::Rejected(reason) | Self::Failed(reason) => reason.to_string(),
            Self::Applying => "applying…".to_owned(),
            Self::Done => "done".to_owned(),
            Self::Unknown => "outcome unknown".to_owned(),
            Self::NotSent(reason) => format!("Not sent: {reason}"),
        }
    }
}

/// The dry-run line of the dialog: every item must pass. `elapsed` is the total of the checks.
pub(crate) fn summarize_dry_runs(states: &[ItemProgress], elapsed: Duration) -> DryRunState {
    if states
        .iter()
        .any(|state| matches!(state, ItemProgress::Waiting | ItemProgress::Checking))
    {
        return DryRunState::Running;
    }
    let failed: Vec<&SharedString> = states
        .iter()
        .filter_map(|state| match state {
            ItemProgress::Rejected(reason) => Some(reason),
            _ => None,
        })
        .collect();
    match failed.first() {
        None => DryRunState::Passed { elapsed },
        Some(first) => DryRunState::Failed(
            format!(
                "Dry-run failed for {} of {}: {first}",
                failed.len(),
                states.len()
            )
            .into(),
        ),
    }
}

/// What a dry-run that stopped means for the item: the text names the cause.
pub(crate) fn dry_run_progress(result: &Result<WriteOutcome, CheckedWriteError>) -> ItemProgress {
    match result {
        Ok(_) => ItemProgress::Passed,
        Err(CheckedWriteError::Blocked(text)) => ItemProgress::Rejected(text.clone()),
        Err(CheckedWriteError::Write(error)) => {
            ItemProgress::Rejected(write_error_text(error).into())
        }
    }
}

/// What one commit's result means for its item. A blocked commit (lock, session switch, reconnect)
/// is not a failure of the object: nothing was sent, and every later item stops with the same
/// reason in `stopped`. Any other failure leaves the next item free to go.
pub(crate) fn commit_progress(
    result: Result<WriteOutcome, CheckedWriteError>,
    stopped: &mut Option<SharedString>,
) -> ItemProgress {
    match result {
        Ok(_) => ItemProgress::Done,
        Err(CheckedWriteError::Blocked(reason)) => {
            *stopped = Some(reason.clone());
            ItemProgress::NotSent(reason)
        }
        Err(CheckedWriteError::Write(WriteError::OutcomeUnknown)) => ItemProgress::Unknown,
        Err(CheckedWriteError::Write(error)) => {
            ItemProgress::Failed(write_error_text(&error).into())
        }
    }
}

/// The notice after the last commit: `Restart: 4 done`, or the counts that did not go through with
/// the first reason.
pub(crate) fn batch_notice(verb: &str, results: &[ItemProgress]) -> String {
    let count = |wanted: fn(&ItemProgress) -> bool| results.iter().filter(|r| wanted(r)).count();
    let done = count(|state| matches!(state, ItemProgress::Done));
    let failed = count(|state| matches!(state, ItemProgress::Failed(_)));
    let unknown = count(|state| matches!(state, ItemProgress::Unknown));
    let not_sent = count(|state| matches!(state, ItemProgress::NotSent(_)));
    let first_reason = results.iter().find_map(|state| match state {
        ItemProgress::Failed(reason) | ItemProgress::NotSent(reason) => Some(reason.clone()),
        _ => None,
    });
    let mut parts = vec![format!("{done} done")];
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    if unknown > 0 {
        parts.push(format!("{unknown} unknown"));
    }
    if not_sent > 0 {
        parts.push(format!("{not_sent} not sent"));
    }
    let mut text = format!("{verb}: {}", parts.join(", "));
    if let Some(reason) = first_reason {
        text.push_str(&format!(" ({reason})"));
    }
    text
}

impl AppShell {
    /// The one entry of a guarded batch: the gate, then the list dialog with the dry-runs already
    /// running one after another. Nothing is sent without the dialog, for every tier and risk.
    pub(crate) fn start_batch(
        &mut self,
        intent: BatchIntent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (generation, confirm, environment) = {
            let Some(guard) = self.guard_for(&intent.cluster, cx) else {
                notify(window, cx, format!("{} is not open", intent.cluster_name));
                return;
            };
            // A stale button or a click in a gap cannot bypass the gate.
            if let ActionAvailability::Disabled { reason } =
                action_availability(intent.action, &guard)
            {
                notify(window, cx, unavailable_text(&intent.verb, &reason));
                return;
            }
            (
                guard.generation,
                confirm_step(guard.profile.confirm, intent.risk, intent.expected()),
                guard.profile.environment,
            )
        };
        if self.running_batches.contains(&intent.cluster) {
            notify(
                window,
                cx,
                format!("{BATCH_RUNNING_REASON} on {}", intent.cluster_name),
            );
            return;
        }
        let inputs = DialogInputs {
            shell: cx.weak_entity(),
            kind: DialogKind::Batch(Rc::new(intent)),
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

    /// The confirmed commit of a batch dialog, one item at a time. Its task is detached: closing the
    /// dialog does not cancel a started commit. A failed item does not stop the next one; a blocked
    /// one (lock, session switch, reconnect) stops the rest, which read `Not sent`.
    pub(crate) fn commit_batch(
        &mut self,
        dialog: WeakEntity<ConfirmDialog>,
        batch: Rc<BatchIntent>,
        commit: BatchCommit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let handle: AnyWindowHandle = window.window_handle();
        let shell = cx.weak_entity();
        let cluster = batch.cluster.clone();
        self.running_batches.insert(cluster.clone());
        cx.notify();
        cx.spawn(async move |_, cx| {
            let mut results = vec![ItemProgress::Waiting; batch.plan.items.len()];
            let mut stopped: Option<SharedString> = None;
            for (index, item) in batch.plan.items.iter().enumerate() {
                let progress = match &stopped {
                    Some(reason) => ItemProgress::NotSent(reason.clone()),
                    None => {
                        let _ = dialog.update(cx, |dialog, cx| {
                            dialog.set_item(index, ItemProgress::Applying, cx);
                        });
                        let step = WriteStep {
                            intent: Rc::new(batch.item_intent(item)),
                            generation: commit.generation,
                            mode: CommitMode::Commit {
                                confirmed: commit.proof,
                            },
                            note: commit.note.clone(),
                        };
                        let result = checked_write(&shell, step, cx).await;
                        commit_progress(result, &mut stopped)
                    }
                };
                let _ = dialog.update(cx, |dialog, cx| {
                    dialog.set_item(index, progress.clone(), cx);
                });
                results[index] = progress;
            }
            // Cleared on every way out: success, a failed item, and a blocked rest.
            let _ = shell.update(cx, |shell, cx| {
                shell.running_batches.remove(&cluster);
                cx.notify();
            });
            let notice = batch_notice(&batch.verb, &results);
            let is_success = results.iter().all(|state| *state == ItemProgress::Done);
            let _ = cx.update_window(handle, |_, window, cx| {
                // Only our own dialog closes, and only while it is open: after Escape or Back the
                // notice is all there is.
                let _ = dialog.update(cx, |dialog, cx| {
                    if dialog.is_open() {
                        dialog.close(window, cx);
                    }
                });
                notify_with(window, cx, notice, is_success);
            });
        })
        .detach();
    }
}

// ---- The selection bar: which bulk buttons work, and what they start ----

impl AppShell {
    /// The buttons of the selection bar for the shown screen, each decided for the ticked rows now:
    /// the gate of the rows' cluster first, then what the batch would do with them. A screen whose
    /// bulk actions are not shipped (Nodes) shows its labels off.
    pub(crate) fn bulk_buttons(&self, cx: &App) -> Vec<BulkButton> {
        let actions = bulk_actions(self.screen);
        let Screen::Kind(_) = self.screen else {
            return actions
                .iter()
                .map(|item| BulkButton {
                    label: item.label.into(),
                    state: BulkState::Off(NOT_SHIPPED_REASON.into()),
                })
                .collect();
        };
        let delegate = self.kind_table.read(cx).delegate();
        let ticked = delegate.view().map_or(0, TableView::checked_count);
        let rows = if ticked > MAX_BATCH_ITEMS {
            Vec::new()
        } else {
            delegate.checked_rows(cx)
        };
        let checked: Vec<CheckedRow<'_>> = rows
            .iter()
            .map(|(cluster, row)| CheckedRow {
                cluster,
                object: &row.object,
            })
            .collect();
        let resume = all_suspended(&checked);
        actions
            .iter()
            .map(|item| {
                let label = match item.action {
                    Some(ResourceAction::SuspendCronJob) if resume => "Resume",
                    _ => item.label,
                };
                let state = if ticked > MAX_BATCH_ITEMS {
                    BulkState::Off(format!("Select at most {MAX_BATCH_ITEMS} rows").into())
                } else {
                    self.bulk_state(item.action, &checked, cx)
                };
                BulkButton {
                    label: label.into(),
                    state,
                }
            })
            .collect()
    }

    fn bulk_state(
        &self,
        action: Option<ResourceAction>,
        checked: &[CheckedRow<'_>],
        cx: &App,
    ) -> BulkState {
        let off = |reason: &str| BulkState::Off(reason.to_owned().into());
        let Some(action) = action else {
            return off(NOT_SHIPPED_REASON);
        };
        if action == ResourceAction::RollBack {
            return off(ROLL_BACK_BULK_REASON);
        }
        let Some(first) = checked.first() else {
            return off("Select rows first");
        };
        // The cluster of the first row decides the gate, so rows of two clusters have no answer.
        if checked.iter().any(|row| row.cluster != first.cluster) {
            return off("Select rows of one cluster");
        }
        let Some(guard) = self.guard_for(first.cluster, cx) else {
            return off("Not connected");
        };
        if let ActionAvailability::Disabled { reason } = action_availability(action, &guard) {
            return BulkState::Off(reason);
        }
        if self.running_batches.contains(first.cluster) {
            return off(BATCH_RUNNING_REASON);
        }
        // Scale asks for its count in a popover, so the batch exists only once it is typed.
        if matches!(action, ResourceAction::Scale(_)) {
            return BulkState::Ready(action);
        }
        match self.bulk_batch_of(checked, action, None, cx) {
            Ok(_) => BulkState::Ready(action),
            Err(reason) => BulkState::Off(reason),
        }
    }

    /// The batch of `action` over the ticked rows as they are now. `replicas` is the Scale count.
    fn bulk_batch(
        &self,
        action: ResourceAction,
        replicas: Option<u32>,
        cx: &App,
    ) -> Result<BatchIntent, SharedString> {
        let ticked = self.kind_table.read(cx).delegate().checked_rows(cx);
        let checked: Vec<CheckedRow<'_>> = ticked
            .iter()
            .map(|(cluster, row)| CheckedRow {
                cluster,
                object: &row.object,
            })
            .collect();
        self.bulk_batch_of(&checked, action, replicas, cx)
    }

    /// The batch over `checked`, which the caller read once: the selection bar builds it for every
    /// button of a frame.
    fn bulk_batch_of(
        &self,
        checked: &[CheckedRow<'_>],
        action: ResourceAction,
        replicas: Option<u32>,
        cx: &App,
    ) -> Result<BatchIntent, SharedString> {
        let first = checked
            .first()
            .ok_or_else(|| SharedString::from("Select rows first"))?;
        let guard = self
            .guard_for(first.cluster, cx)
            .ok_or_else(|| SharedString::from("Not connected"))?;
        let live = self
            .slot_live(first.cluster, cx)
            .ok_or_else(|| SharedString::from("Not connected"))?;
        let inputs = BulkInputs {
            cluster_name: guard.display_name(),
            rows: checked,
            now: jiff::Timestamp::now(),
            hpas: live.loaded_hpas(),
        };
        match (action, replicas) {
            (ResourceAction::Scale(kind), Some(replicas)) => {
                bulk_scale_intent(&inputs, replicas, kind)
            }
            (ResourceAction::Scale(_), None) => Err("Enter the replicas first".into()),
            _ => bulk_intent(action, &inputs),
        }
    }

    /// A bulk button: Scale asks for its count first, the others build their batch from the rows
    /// ticked now and open the list dialog.
    pub(crate) fn run_bulk(
        &mut self,
        action: ResourceAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let ResourceAction::Scale(kind) = action {
            self.open_bulk_scale_popover(kind, window, cx);
            return;
        }
        match self.bulk_batch(action, None, cx) {
            Ok(intent) => self.start_batch(intent, window, cx),
            Err(reason) => notify(window, cx, unavailable_text(action_label(action), &reason)),
        }
    }

    /// The Scale popover for the ticked rows, empty: one count for all of them.
    fn open_bulk_scale_popover(
        &mut self,
        kind: ObjectKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = self.kind_table.read(cx).delegate().checked_rows(cx).len();
        let shell = cx.weak_entity();
        let popover = cx.new(|cx| ValuePopover::scale_ticked(shell, kind, count, window, cx));
        self.set_value_popover(popover, cx);
    }

    /// Scale of the bulk popover: the popover closes, and the batch goes to the list dialog.
    pub(crate) fn submit_bulk_scale(
        &mut self,
        kind: ObjectKind,
        replicas: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_value_popover(cx);
        match self.bulk_batch(ResourceAction::Scale(kind), Some(replicas), cx) {
            Ok(intent) => self.start_batch(intent, window, cx),
            Err(reason) => notify(
                window,
                cx,
                unavailable_text(action_label(ResourceAction::Scale(kind)), &reason),
            ),
        }
    }
}

#[cfg(feature = "screenshot")]
impl AppShell {
    /// `--screen restart-bulk-confirm`: the batch dialog of Restart over the ticked Deployments, in
    /// a fixed state (every dry-run passed, the session unlocked so the button shows as enabled).
    /// It skips the gate and the dry-runs and can never commit (`ConfirmDialog::show_fixture`). With
    /// no row ticked there is nothing to show, and the screen settles on the list.
    pub(super) fn open_restart_bulk_fixture(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let action = ResourceAction::RestartRollout(ObjectKind::Deployment);
        let Ok(intent) = self.bulk_batch(action, None, cx) else {
            return;
        };
        if let Some(session) = self.slot_session(&intent.cluster).cloned() {
            session.update(cx, |session, cx| session.set_lock(WriteLock::Unlocked, cx));
        }
        let inputs = {
            let Some(guard) = self.guard_for(&intent.cluster, cx) else {
                return;
            };
            DialogInputs {
                shell: cx.weak_entity(),
                confirm: confirm_step(guard.profile.confirm, intent.risk, intent.expected()),
                environment: guard.profile.environment,
                generation: guard.generation,
                kind: DialogKind::Batch(Rc::new(intent)),
            }
        };
        let dialog = cx.new(|cx| ConfirmDialog::new(inputs, window, cx));
        dialog.update(cx, |dialog, _| dialog.show_fixture());
        ConfirmDialog::open(&dialog, window, cx);
    }
}

/// What a batch commit carries from the dialog: the proof that the confirm step was satisfied for
/// this dry-run, the session generation it opened on, and the note of the audit lines.
pub(crate) struct BatchCommit {
    pub(crate) proof: Confirmed,
    pub(crate) generation: u64,
    pub(crate) note: Option<String>,
}

#[cfg(test)]
#[path = "batch_write_tests.rs"]
mod batch_write_tests;
