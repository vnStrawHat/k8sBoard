//! Batch (spec 0032): the one bulk mechanism. A batch changes several ticked objects of one
//! cluster with the same action. It always opens the confirm dialog, dry-runs every item one at a
//! time and needs all of them to pass, then commits one at a time through `checked_write`, so every
//! commit re-checks the lock and writes its own audit line.
//!
//! A child of `app_shell`, like `write_flow`: it reads the viewed slots and starts the dialog.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use cluster::{ObjectKind, WriteError, WriteOutcome, WriteRequest};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::notification::Notification;
use gpui_kit::{AnyWindowHandle, App, AppContext as _, Context, SharedString, WeakEntity, Window};

use super::object_delete::{DeleteExtras, Removal, delete_commit_progress, delete_notice};
use super::write_flow::{
    CheckedWriteError, CommitMode, Confirmed, DryRunState, WriteIntent, WriteStep, checked_write,
    notify, notify_unavailable, notify_with, write_error_text,
};
use super::{AppShell, Screen};
use crate::cluster_registry::ClusterRef;
use crate::confirm_dialog::{ConfirmDialog, DialogInputs, DialogKind};
use crate::kind_row::KindObject;
use crate::resource_actions::{
    ActionAvailability, NOT_SHIPPED_REASON, ResourceAction, action_availability, action_label,
    unavailable_text,
};
use crate::resource_edits::DefaultClassExtras;
use crate::resource_edits::{bulk_default_class_intent, bulk_expand_intent, bulk_hpa_range_intent};
use crate::row_selection::{BulkButton, BulkState, ROLL_BACK_BULK_REASON, bulk_actions};
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::table_view::{FilteredTable as _, TableView};
use crate::value_popover::ValuePopover;
use crate::workload_actions::{
    BulkInputs, WorkloadScope, all_suspended, bulk_intent, bulk_scale_intent, named_restart_batch,
};
#[cfg(feature = "screenshot")]
use crate::write_guard::WriteLock;
use crate::write_guard::{ActionRisk, confirm_step};

/// The most objects one batch changes: more would be a long run of requests behind one click.
pub(crate) const MAX_BATCH_ITEMS: usize = 50;

/// Why the bulk buttons are off while a confirmed batch still commits on their cluster.
pub(crate) const BATCH_RUNNING_REASON: &str = "A batch is running";

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
    pub(crate) extras: BatchExtras,
    pub(crate) on_failure: BatchFailure,
}

/// What a failed commit does to the rest of the batch. A `Blocked` result (lock, session switch,
/// reconnect) always stops the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BatchFailure {
    /// Independent objects: the next item still goes (0032, 0033).
    Continue,
    /// An ordered plan where a later step depends on an earlier one: nothing more is sent after a
    /// failure (0032b Set default).
    Stop,
}

/// What one action adds to the batch beyond a list of requests.
#[derive(Clone)]
pub(crate) enum BatchExtras {
    /// Restart, Scale, and the other 0032 actions.
    None,
    /// Delete (0033): the propagation choice, the targets it rebuilds the items from, and the
    /// objects that were gone before the dialog opened.
    Delete(DeleteExtras),
    /// Set default storage class (0032b): which class becomes the default, and the text of the state
    /// a partial run leaves behind.
    DefaultClass(DefaultClassExtras),
}

/// What a popover asked for before a bulk batch can be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BulkValue {
    /// The action needs no value.
    Nothing,
    /// Scale: the replicas.
    Replicas(u32),
    /// Edit min / max: the HPA range.
    Range { min: u32, max: u32 },
    /// Expand: the storage quantity.
    Storage(String),
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
        extras: BatchExtras::None,
        on_failure: BatchFailure::Continue,
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
    /// What the `TypeName` tier asks to type: the object of a batch of one, else the cluster name,
    /// since a larger batch names no single object.
    pub(crate) fn expected(&self) -> &str {
        match self.single_object() {
            Some(item) => item.request.target().name(),
            None => &self.cluster_name,
        }
    }

    /// `the cluster name`, or `the pod name` for a batch of one object.
    pub(crate) fn typed_hint(&self) -> String {
        match self.single_object() {
            Some(item) => format!(
                "the {} name",
                item.request.target().kind_name().to_ascii_lowercase()
            ),
            None => "the cluster name".to_owned(),
        }
    }

    fn single_object(&self) -> Option<&BatchItem> {
        match self.plan.items.as_slice() {
            [item] => Some(item),
            _ => None,
        }
    }

    /// The confirm button: `Restart 4`. A delete reads `Delete` for one object and `Delete 11 of
    /// 12` otherwise, where `gone` items went away since the dialog opened.
    pub(crate) fn confirm_label(&self, gone: usize) -> String {
        let total = self.plan.items.len();
        match (&self.plan.extras, total) {
            (BatchExtras::Delete(_), 1) => self.verb.to_string(),
            // Nothing went away: the title's count and noun, `Delete 12 pods`.
            (BatchExtras::Delete(_), _) if gone == 0 => self.label.to_string(),
            (BatchExtras::Delete(_), _) => {
                format!("{} {} of {total}", self.verb, total.saturating_sub(gone))
            }
            (BatchExtras::None | BatchExtras::DefaultClass(_), _) => {
                format!("{} {total}", self.verb)
            }
        }
    }

    pub(crate) fn is_delete(&self) -> bool {
        matches!(self.plan.extras, BatchExtras::Delete(_))
    }

    /// What one commit's result means for its item.
    fn commit_progress(
        &self,
        result: Result<WriteOutcome, CheckedWriteError>,
        stopped: &mut Option<SharedString>,
    ) -> ItemProgress {
        let progress = match self.is_delete() {
            true => delete_commit_progress(result, stopped),
            false => commit_progress(result, stopped),
        };
        // An ordered plan sends nothing after a step that did not go through.
        if self.plan.on_failure == BatchFailure::Stop
            && stopped.is_none()
            && matches!(progress, ItemProgress::Failed(_) | ItemProgress::Unknown)
        {
            *stopped = Some(EARLIER_STEP_FAILED.into());
        }
        progress
    }

    /// The notice after the last commit.
    fn notice(&self, results: &[ItemProgress]) -> String {
        if matches!(self.plan.extras, BatchExtras::Delete(_)) {
            return delete_notice(self, results);
        }
        match self.stop_notice(results) {
            Some(text) => text,
            None => batch_notice(&self.verb, results),
        }
    }

    /// The notice of an ordered plan that stopped: how far it got, why, and the state it left
    /// behind. `None` for a plan that continues, or one that went through.
    fn stop_notice(&self, results: &[ItemProgress]) -> Option<String> {
        if self.plan.on_failure != BatchFailure::Stop {
            return None;
        }
        let mut text = stop_notice(&self.label, results)?;
        if let BatchExtras::DefaultClass(extras) = &self.plan.extras
            && let Some(state) = extras.state_left(results)
        {
            text.push_str(". ");
            text.push_str(&state);
        }
        Some(text)
    }

    /// What the Retry of a stopped plan runs again: the object the action started from.
    fn retry_subject(&self, results: &[ItemProgress]) -> Option<ClusterObject> {
        self.stop_notice(results)?;
        match &self.plan.extras {
            BatchExtras::DefaultClass(extras) => Some(extras.subject(&self.cluster)),
            BatchExtras::Delete(_) | BatchExtras::None => None,
        }
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
    /// A delete whose object was already gone, at the dry-run or at the commit: nothing to send,
    /// and not a failure (0033 decision 20).
    Gone,
    /// A delete the server accepted that stays until finalizers or a grace period end; the names
    /// are the finalizers, empty for a grace period.
    Pending(Vec<String>),
}

impl ItemProgress {
    /// Whether the item ended as the user wanted: done, accepted by the server, or already gone.
    pub(crate) fn is_settled(&self) -> bool {
        matches!(self, Self::Done | Self::Gone | Self::Pending(_))
    }

    /// The state as the list of the dialog shows it.
    pub(crate) fn text(&self) -> String {
        match self {
            Self::Gone => "already gone".to_owned(),
            Self::Pending(finalizers) if finalizers.is_empty() => "terminating".to_owned(),
            Self::Pending(finalizers) => format!("waiting for {}", finalizers.join(", ")),
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
        // One object: its row only says `failed`, so the cause is read once, here.
        Some(first) if states.len() == 1 => {
            DryRunState::Failed(format!("Dry-run failed: {first}").into())
        }
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

/// What the items after a Stop of the user read.
const STOPPED_BY_USER: &str = "stopped by user";

/// What the items after a failed step of an ordered plan read.
const EARLIER_STEP_FAILED: &str = "an earlier step failed";

/// `Make gp3 the default storage class: stopped after 0 of 2: {error}`, or `None` when every item
/// went through. The error is the first one that did not.
pub(crate) fn stop_notice(label: &str, results: &[ItemProgress]) -> Option<String> {
    let error = results.iter().find_map(|state| match state {
        ItemProgress::Failed(reason) | ItemProgress::NotSent(reason) => Some(reason.to_string()),
        ItemProgress::Unknown => {
            Some("the outcome is unknown; the change may have been applied".to_owned())
        }
        _ => None,
    })?;
    let settled = results.iter().filter(|state| state.is_settled()).count();
    Some(format!(
        "{label}: stopped after {settled} of {}: {error}",
        results.len()
    ))
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
                let (shell, cluster) = (cx.weak_entity(), intent.cluster.clone());
                notify_unavailable(window, cx, &intent.verb, &reason, shell, cluster);
                return;
            }
            if let Some(reason) = self.drain_conflict(&intent.cluster, intent.action, cx) {
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
        // Two commit loops on one cluster would race over the same objects and interleave their
        // audit lines. The dialog is being updated by the caller, so it hears of the refusal later.
        if self.running_batches.contains(&cluster) {
            let reason = format!("{BATCH_RUNNING_REASON} on {}", batch.cluster_name);
            cx.defer(move |cx| {
                let _ = dialog.update(cx, |dialog, cx| dialog.commit_failed(reason, false, cx));
            });
            return;
        }
        self.running_batches.insert(cluster.clone());
        cx.notify();
        cx.spawn(async move |_, cx| {
            let mut results = vec![ItemProgress::Waiting; batch.plan.items.len()];
            let mut stopped: Option<SharedString> = None;
            for (index, item) in batch.plan.items.iter().enumerate() {
                // The Stop button takes effect between two items: the one in flight finishes.
                if stopped.is_none() && commit.stop.get() {
                    stopped = Some(STOPPED_BY_USER.into());
                }
                let progress = match &stopped {
                    // Gone at the dry-run: nothing to send, whatever happened to the rest.
                    _ if commit.gone.get(index).copied().unwrap_or(false) => ItemProgress::Gone,
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
                        let progress = batch.commit_progress(result, &mut stopped);
                        if progress.is_settled() && batch.is_delete() {
                            let _ = shell.update(cx, |shell, cx| {
                                shell.object_deleted(&cluster, item.request.target(), cx);
                            });
                        }
                        progress
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
            let notice = batch.notice(&results);
            let is_success = results.iter().all(ItemProgress::is_settled);
            // The plan was frozen when the dialog opened: a class made the default meanwhile is
            // still one, so a clean run is not a plain success then.
            let many_defaults = match is_success {
                true => shell
                    .update(cx, |shell, cx| {
                        shell.many_defaults_note(&batch, &results, cx)
                    })
                    .ok()
                    .flatten(),
                false => None,
            };
            let (notice, is_success) = match many_defaults {
                Some(warning) => (format!("{notice}. {warning}"), false),
                None => (notice, is_success),
            };
            let retry = batch
                .retry_subject(&results)
                .map(|subject| (subject, batch.action));
            let _ = cx.update_window(handle, |_, window, cx| {
                // Only our own dialog closes, and only while it is open: after Escape or Back the
                // notice is all there is.
                let _ = dialog.update(cx, |dialog, cx| {
                    if dialog.is_open() {
                        dialog.close(window, cx);
                    }
                });
                match retry {
                    Some((subject, action)) => {
                        notify_with_retry(window, cx, notice, shell, subject, action);
                    }
                    None => notify_with(window, cx, notice, is_success),
                }
            });
        })
        .detach();
    }

    /// The Restart consumers button of an Edit values notice: one Restart rollout batch per
    /// workload kind that reads the object through env, since a batch carries one action and the
    /// gate of each kind is its own permission. Each opens its own confirm dialog, the first kind
    /// on top. The workloads are named from the pods of the Used by section, so no list needs to
    /// hold them.
    pub(crate) fn restart_consumers(
        &mut self,
        cluster: &ClusterRef,
        consumers: &[(ObjectKind, ResourceKey)],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = action_label(ResourceAction::RestartRollout(ObjectKind::Deployment));
        let Some(name) = self
            .guard_for(cluster, cx)
            .map(|guard| guard.display_name().to_owned())
        else {
            notify(
                window,
                cx,
                unavailable_text(label, "the cluster is not open"),
            );
            return;
        };
        let scope = WorkloadScope {
            cluster,
            cluster_name: &name,
        };
        let now = jiff::Timestamp::now();
        let mut intents = Vec::new();
        for kind in [
            ObjectKind::Deployment,
            ObjectKind::StatefulSet,
            ObjectKind::DaemonSet,
        ] {
            let named: Vec<(&str, &str)> = consumers
                .iter()
                .filter(|(consumer, _)| *consumer == kind)
                .filter_map(|(_, key)| match key {
                    ResourceKey::Kind {
                        namespace: Some(namespace),
                        name,
                        ..
                    } => Some((namespace.as_str(), name.as_str())),
                    _ => None,
                })
                .collect();
            if named.is_empty() {
                continue;
            }
            match named_restart_batch(&scope, kind, &named, now) {
                Ok(intent) => intents.push(intent),
                Err(reason) => notify(window, cx, unavailable_text(label, &reason)),
            }
        }
        // The last dialog opened is the one on top, so the first kind opens last.
        for intent in intents.into_iter().rev() {
            self.start_batch(intent, window, cx);
        }
    }
}

// ---- The selection bar: which bulk buttons work, and what they start ----

impl AppShell {
    /// The buttons of the selection bar for the shown screen, each decided for the ticked rows now:
    /// the gate of the rows' cluster first, then what the batch would do with them. A screen whose
    /// bulk actions are not shipped (Nodes) shows its labels off.
    pub(crate) fn bulk_buttons(&self, cx: &App) -> Vec<BulkButton> {
        let mut buttons = match self.screen {
            Screen::Nodes => self.node_bulk_buttons(cx),
            _ => self.workload_bulk_buttons(cx),
        };
        buttons.extend(self.delete_bulk_button(cx));
        buttons
    }

    /// The workload and node buttons of the screen (the Delete button comes after them).
    fn workload_bulk_buttons(&self, cx: &App) -> Vec<BulkButton> {
        let actions = bulk_actions(self.screen);
        let Screen::Kind(_) = self.screen else {
            return actions
                .iter()
                .map(|item| BulkButton {
                    label: item.label.into(),
                    state: BulkState::Off(NOT_SHIPPED_REASON.into()),
                    is_danger: false,
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
                    is_danger: false,
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
        // These ask for their value in a popover, so the batch exists only once it is typed.
        if matches!(
            action,
            ResourceAction::Scale(_) | ResourceAction::EditHpaRange | ResourceAction::ExpandClaim
        ) {
            return BulkState::Ready(action);
        }
        match self.bulk_batch_of(checked, action, BulkValue::Nothing, cx) {
            Ok(_) => BulkState::Ready(action),
            Err(reason) => BulkState::Off(reason),
        }
    }

    /// The batch of `action` over the ticked rows as they are now. `value` is what a popover asked for.
    pub(super) fn bulk_batch(
        &self,
        action: ResourceAction,
        value: BulkValue,
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
        self.bulk_batch_of(&checked, action, value, cx)
    }

    /// The batch over `checked`, which the caller read once: the selection bar builds it for every
    /// button of a frame.
    fn bulk_batch_of(
        &self,
        checked: &[CheckedRow<'_>],
        action: ResourceAction,
        value: BulkValue,
        cx: &App,
    ) -> Result<BatchIntent, SharedString> {
        let first = checked
            .first()
            .ok_or_else(|| SharedString::from("Select rows first"))?;
        let guard = self
            .guard_for(first.cluster, cx)
            .ok_or_else(|| SharedString::from("Not connected"))?;
        let live = self
            .live_of(first.cluster, cx)
            .ok_or_else(|| SharedString::from("Not connected"))?;
        let inputs = BulkInputs {
            cluster_name: guard.display_name(),
            rows: checked,
            now: jiff::Timestamp::now(),
            hpas: live.loaded_hpas(),
        };
        match (action, value) {
            (ResourceAction::Scale(kind), BulkValue::Replicas(replicas)) => {
                bulk_scale_intent(&inputs, replicas, kind)
            }
            (ResourceAction::Scale(_), BulkValue::Nothing) => {
                Err("Enter the replicas first".into())
            }
            (ResourceAction::EditHpaRange, BulkValue::Range { min, max }) => {
                bulk_hpa_range_intent(&inputs, min, max)
            }
            (ResourceAction::EditHpaRange, _) => Err("Enter the min and max first".into()),
            (ResourceAction::ExpandClaim, BulkValue::Storage(storage)) => {
                bulk_expand_intent(&inputs, &storage, &live.loaded_storage_classes())
            }
            (ResourceAction::ExpandClaim, _) => Err("Enter the size first".into()),
            (ResourceAction::SetDefaultStorageClass, _) => {
                bulk_default_class_intent(&inputs, &live.loaded_storage_classes())
            }
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
        if action == ResourceAction::EditHpaRange {
            self.open_bulk_hpa_range_popover(window, cx);
            return;
        }
        if action == ResourceAction::ExpandClaim {
            self.open_bulk_expand_popover(window, cx);
            return;
        }
        if let ResourceAction::Delete(_) = action {
            let scope = self.checked_objects(cx);
            self.start_removal(Removal::Delete, scope, window, cx);
            return;
        }
        if self.screen == Screen::Nodes {
            self.run_node_bulk(action, window, cx);
            return;
        }
        match self.bulk_batch(action, BulkValue::Nothing, cx) {
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
        match self.bulk_batch(
            ResourceAction::Scale(kind),
            BulkValue::Replicas(replicas),
            cx,
        ) {
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
        let Ok(intent) = self.bulk_batch(action, BulkValue::Nothing, cx) else {
            return;
        };
        if let Some(session) = self.session_of(&intent.cluster).cloned() {
            session.update(cx, |session, cx| session.set_lock(WriteLock::Unlocked, cx));
        }
        let inputs = {
            let Some(guard) = self.guard_for(&intent.cluster, cx) else {
                return;
            };
            DialogInputs {
                shell: cx.weak_entity(),
                confirm: confirm_step(guard.profile.confirm, intent.risk, intent.expected()),
                environment: guard.profile.environment.clone(),
                generation: guard.generation,
                kind: DialogKind::Batch(Rc::new(intent)),
            }
        };
        let dialog = cx.new(|cx| ConfirmDialog::new(inputs, window, cx));
        dialog.update(cx, |dialog, _| dialog.show_fixture());
        ConfirmDialog::open(&dialog, window, cx);
    }
}

/// A warning notice with a Retry button: it dismisses the notice and starts `action` again on
/// `subject`, which re-plans from the data of that moment.
fn notify_with_retry(
    window: &mut Window,
    cx: &mut App,
    text: String,
    shell: WeakEntity<AppShell>,
    subject: ClusterObject,
    action: ResourceAction,
) {
    let notification = Notification::warning(text).action(move |_, _, cx| {
        let (shell, subject) = (shell.clone(), subject.clone());
        Button::new("batch-retry")
            .label("Retry")
            .small()
            .outline()
            .on_click(cx.listener(move |notification, _, window, cx| {
                notification.dismiss(window, cx);
                let _ = shell.update(cx, |shell, cx| {
                    shell.retry_batch(action, &subject, window, cx);
                });
            }))
    });
    window.push_notification(notification, cx);
}

/// What a batch commit carries from the dialog: the proof that the confirm step was satisfied for
/// this dry-run, the session generation it opened on, and the note of the audit lines.
pub(crate) struct BatchCommit {
    pub(crate) proof: Confirmed,
    pub(crate) generation: u64,
    pub(crate) note: Option<String>,
    /// Per item of the plan: gone at the dry-run, so the commit sends nothing for it.
    pub(crate) gone: Vec<bool>,
    /// Set by the Stop button of the dialog: the items not yet sent read `Not sent`.
    pub(crate) stop: Rc<Cell<bool>>,
}

#[cfg(test)]
#[path = "batch_write_tests.rs"]
mod batch_write_tests;
