//! Delete (spec 0033): which objects a delete covers, the identity reads that pin each of them by
//! uid, the batch the confirm dialog shows, and the texts around it (warnings, finalizer hints,
//! notices).
//!
//! A child of `app_shell`, like `batch_write`, because the start reads the viewed slots. A single
//! delete is a one-item batch, so the dry-run list, the commit loop, the audit lines, and the
//! `running_batches` guard are the 0032 ones. Every step names the cluster of the rows and takes its
//! guard, connection, lock, and tier from that cluster's own slot, never from the primary.

use cluster::{
    ClusterConnection, ClusterError, DeletePropagation, HELM_RELEASE_SECRET_TYPE, ObjectIdentity,
    ObjectKind, ObjectRef, WriteEffect, WriteError, WriteOperation, WriteOutcome, WriteRequest,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, Context, SharedString, Window};

use super::AppShell;
use super::batch_write::{
    BatchExtras, BatchFailure, BatchIntent, BatchItem, BatchPlan, ItemProgress, MAX_BATCH_ITEMS,
    SkippedItem,
};
use super::write_flow::{CheckedWriteError, write_error_text};
use crate::age::format_age;
use crate::cluster_registry::ClusterRef;
use crate::cluster_runtime::ClusterRuntime;
use crate::cluster_session::{CompanionLists, LiveCluster, error_text};
#[cfg(feature = "screenshot")]
use crate::environment::Environment;
use crate::kind_row::KindObject;
use crate::resource_actions::{
    ActionAvailability, ResourceAction, action_availability, action_label, action_risk,
    delete_kind, unavailable_text,
};
use crate::row_selection::{BulkButton, BulkState};
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::yaml_edit::EditFailure;
use crate::yaml_view::object_ref;

/// Why a delete of Helm release records is off: `Uninstall release…` is a later version (0038).
pub(crate) const HELM_RECORD_REASON: &str =
    "Helm release records are removed by Uninstall release (a later version)";
/// A uid precondition that failed: another object holds the name now.
const RECREATED_TEXT: &str = "A new object with this name exists; nothing was deleted";
/// The most finalizer names a line spells out before it counts the rest.
const FINALIZER_NAMES: usize = 3;

// ---- Scope ----

/// Which objects a delete covers (decision 21): the checked set when the cursor row is one of at
/// least two checked rows, else the cursor row alone. `Err` is the reason the delete is off.
pub(crate) fn delete_scope(
    subject: &ClusterObject,
    checked: &[ClusterObject],
) -> Result<Vec<ClusterObject>, SharedString> {
    if checked.len() < 2 || !checked.contains(subject) {
        return Ok(vec![subject.clone()]);
    }
    if checked
        .iter()
        .any(|object| object.cluster != subject.cluster)
    {
        return Err("Select rows of one cluster".into());
    }
    if checked.len() > MAX_BATCH_ITEMS {
        return Err(format!("Select at most {MAX_BATCH_ITEMS} rows").into());
    }
    Ok(checked.to_vec())
}

// ---- Targets and the batch ----

/// What the warnings of a delete read about one object, taken from the cluster's lists when the
/// delete starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TargetFacts {
    Plain,
    Pod {
        has_controller: bool,
    },
    /// A persistent volume: whether its reclaim policy deletes the storage asset.
    Volume {
        deletes_asset: bool,
    },
    Claim(ClaimVolume),
}

/// The volume a persistent volume claim is bound to, as far as the session knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClaimVolume {
    Unbound,
    /// Bound to a volume whose reclaim policy keeps the data.
    Keeps,
    /// Bound to this volume, whose reclaim policy deletes the data.
    Deletes(String),
    /// Bound, but the volume list is not loaded.
    NotLoaded,
}

/// One object of a delete with the uid its precondition pins.
#[derive(Clone)]
pub(crate) struct DeleteTarget {
    pub(crate) object: ObjectRef,
    pub(crate) identity: ObjectIdentity,
    pub(crate) facts: TargetFacts,
}

impl DeleteTarget {
    /// `namespace/name`, as the list of the dialog shows it.
    fn text(&self) -> SharedString {
        object_text(&self.object)
    }

    /// `None` when the object cannot form a request (a name that is not a path segment).
    fn request(&self, propagation: DeletePropagation) -> Option<WriteRequest> {
        WriteRequest::new(
            self.object.clone(),
            WriteOperation::DeleteObject {
                uid: self.identity.uid.clone(),
                propagation,
            },
        )
    }
}

fn object_text(object: &ObjectRef) -> SharedString {
    match object.namespace() {
        Some(namespace) => format!("{namespace}/{}", object.name()).into(),
        None => object.name().to_owned().into(),
    }
}

/// What a delete adds to the batch (decisions 19 and 20).
#[derive(Clone)]
pub(crate) struct DeleteExtras {
    pub(crate) propagation: DeletePropagation,
    pub(crate) kind: ObjectKind,
    /// The objects the items are built from, so a propagation change can rebuild them.
    pub(crate) targets: Vec<DeleteTarget>,
    /// Objects that were gone before the dialog opened, as `namespace/name`.
    pub(crate) already_gone: Vec<SharedString>,
}

/// One `DeleteObject` per target; a target that cannot form a request is left out (the start
/// refuses those before it reads their identity).
pub(crate) fn delete_items(
    targets: &[DeleteTarget],
    propagation: DeletePropagation,
) -> Vec<BatchItem> {
    let kind_word = |target: &DeleteTarget| target.object.kind_name().to_ascii_lowercase();
    targets
        .iter()
        .filter_map(|target| {
            Some(BatchItem {
                object: target.text(),
                label: format!("Delete {} {}", kind_word(target), target.text()).into(),
                request: target.request(propagation)?,
            })
        })
        .collect()
}

/// The words of the kind in a title: `pod`, or `12 pods` (the API's plural resource name).
fn kind_noun(kind: ObjectKind, count: usize) -> String {
    let word = if count == 1 {
        kind.name().to_ascii_lowercase()
    } else {
        kind.resource().1.to_owned()
    };
    if count == 1 {
        word
    } else {
        format!("{count} {word}")
    }
}

/// The batch of a delete with its dialog texts. `now` ages an object that is already terminating.
pub(crate) fn delete_batch(
    cluster: &ClusterRef,
    cluster_name: &str,
    extras: DeleteExtras,
    now: jiff::Timestamp,
) -> BatchIntent {
    let items = delete_items(&extras.targets, extras.propagation);
    let kind = extras.kind;
    let label = format!("Delete {}", kind_noun(kind, items.len()));
    let expected_name = match (items.as_slice(), extras.targets.as_slice()) {
        ([_], [target]) => Some(target.object.name().to_owned()),
        _ => None,
    };
    let mut warnings = kind_warnings(kind, &extras.targets);
    warnings.extend(finalizer_lines(&extras.targets, now));
    BatchIntent {
        cluster: cluster.clone(),
        cluster_name: cluster_name.to_owned().into(),
        action: ResourceAction::Delete(kind),
        label: label.into(),
        verb: "Delete".into(),
        button: action_label(ResourceAction::Delete(kind)).into(),
        risk: action_risk(ResourceAction::Delete(kind)),
        warnings,
        expected_name,
        plan: BatchPlan {
            cluster: cluster.clone(),
            items,
            skipped: Vec::new(),
            extras: BatchExtras::Delete(extras),
            on_failure: BatchFailure::Continue,
        },
    }
}

/// The same delete with another propagation policy: the items are rebuilt, and the dialog runs the
/// dry-runs again. `None` for a batch that is not a delete.
pub(crate) fn with_propagation(
    batch: &BatchIntent,
    propagation: DeletePropagation,
    now: jiff::Timestamp,
) -> Option<BatchIntent> {
    let BatchExtras::Delete(extras) = &batch.plan.extras else {
        return None;
    };
    let extras = DeleteExtras {
        propagation,
        ..extras.clone()
    };
    let mut rebuilt = delete_batch(&batch.cluster, &batch.cluster_name, extras, now);
    rebuilt.plan.skipped = batch.plan.skipped.clone();
    Some(rebuilt)
}

// ---- Warnings and hints ----

/// The warnings of the kind table (decision 13), one line per kind of surprise.
pub(crate) fn kind_warnings(kind: ObjectKind, targets: &[DeleteTarget]) -> Vec<SharedString> {
    let count = targets.len();
    let is_single = count == 1;
    let name = targets.first().map_or("", |target| target.object.name());
    let mut lines: Vec<String> = Vec::new();
    match kind {
        ObjectKind::Namespace if is_single => {
            lines.push(format!("Deletes every object in {name}"));
        }
        ObjectKind::Namespace => {
            lines.push(format!("Deletes every object in these {count} namespaces"))
        }
        ObjectKind::Node => {
            lines.push("Removes the node object; its pods are not drained first".to_owned());
        }
        ObjectKind::StatefulSet => {
            lines.push("Volume claims stay unless the retention policy deletes them".to_owned());
        }
        ObjectKind::PersistentVolume => {
            let deleting = count_of(targets, |facts| {
                matches!(
                    facts,
                    TargetFacts::Volume {
                        deletes_asset: true
                    }
                )
            });
            match (deleting, is_single) {
                (0, _) => {}
                (_, true) => {
                    lines.push("Reclaim policy Delete: the storage asset is deleted too".to_owned())
                }
                (k, false) => lines.push(format!(
                    "{k} volumes have reclaim policy Delete: their storage assets are deleted too"
                )),
            }
        }
        ObjectKind::PersistentVolumeClaim => lines.extend(claim_lines(targets)),
        ObjectKind::Pod => {
            let loose = count_of(targets, |facts| {
                matches!(
                    facts,
                    TargetFacts::Pod {
                        has_controller: false
                    }
                )
            });
            match (loose, is_single) {
                (0, _) => {}
                (_, true) => {
                    lines.push("Not managed by a controller; it will not come back".to_owned());
                }
                (k, false) => lines.push(format!("{k} pods are not managed by a controller")),
            }
        }
        _ => {}
    }
    lines.into_iter().map(Into::into).collect()
}

fn count_of(targets: &[DeleteTarget], wanted: impl Fn(&TargetFacts) -> bool) -> usize {
    targets
        .iter()
        .filter(|target| wanted(&target.facts))
        .count()
}

/// What deleting claims does to their bound volumes' data.
fn claim_lines(targets: &[DeleteTarget]) -> Vec<String> {
    let volumes: Vec<&str> = targets
        .iter()
        .filter_map(|target| match &target.facts {
            TargetFacts::Claim(ClaimVolume::Deletes(volume)) => Some(volume.as_str()),
            _ => None,
        })
        .collect();
    let unknown = count_of(targets, |facts| {
        matches!(facts, TargetFacts::Claim(ClaimVolume::NotLoaded))
    });
    let mut lines = Vec::new();
    match volumes.as_slice() {
        [] => {}
        [volume] => lines.push(format!(
            "The bound volume {volume} has reclaim policy Delete: its data is deleted too"
        )),
        many => lines.push(format!(
            "{} bound volumes have reclaim policy Delete: their data is deleted too",
            many.len()
        )),
    }
    match unknown {
        0 => {}
        1 => lines.push(
            "If the bound volume's reclaim policy is Delete, its data is deleted too".to_owned(),
        ),
        k => lines.push(format!(
            "{k} claims are bound to volumes that may have reclaim policy Delete: their data would be deleted too"
        )),
    }
    lines
}

/// Up to three names, then `+N`.
fn finalizer_names(finalizers: &[String]) -> String {
    let shown: Vec<&str> = finalizers
        .iter()
        .take(FINALIZER_NAMES)
        .map(String::as_str)
        .collect();
    let rest = finalizers.len().saturating_sub(FINALIZER_NAMES);
    match rest {
        0 => shown.join(", "),
        rest => format!("{} +{rest}", shown.join(", ")),
    }
}

/// The finalizer lines before the delete (decisions 14 and 15): finalizers that make the object
/// wait, and an object that is already terminating. No polling: the identity read is the source.
pub(crate) fn finalizer_lines(targets: &[DeleteTarget], now: jiff::Timestamp) -> Vec<SharedString> {
    if let [target] = targets {
        return single_finalizer_line(&target.identity, now)
            .map(SharedString::from)
            .into_iter()
            .collect();
    }
    let terminating = targets
        .iter()
        .filter(|target| target.identity.deletion_started.is_some())
        .count();
    let waiting = targets
        .iter()
        .filter(|target| {
            target.identity.deletion_started.is_none() && !target.identity.finalizers.is_empty()
        })
        .count();
    let mut lines = Vec::new();
    if waiting > 0 {
        lines.push(format!("{waiting} objects have finalizers"));
    }
    if terminating > 0 {
        lines.push(format!("{terminating} are already being deleted"));
    }
    lines.into_iter().map(Into::into).collect()
}

fn single_finalizer_line(identity: &ObjectIdentity, now: jiff::Timestamp) -> Option<String> {
    let names = finalizer_names(&identity.finalizers);
    match (identity.deletion_started, identity.finalizers.is_empty()) {
        (Some(started), true) => Some(format!(
            "Already terminating for {}",
            format_age(Some(started), now)
        )),
        (Some(started), false) => Some(format!(
            "Already being deleted for {}; waiting for finalizers: {names}. Deleting again does not remove them",
            format_age(Some(started), now)
        )),
        (None, true) => None,
        (None, false) => Some(format!(
            "Has finalizers: {names}. Deletion waits until their controllers remove them"
        )),
    }
}

// ---- Propagation texts ----

/// What an owner kind's dependents are called, for the propagation choice.
fn dependents_of(kind: ObjectKind, is_single: bool) -> &'static str {
    let (single, many) = match kind {
        ObjectKind::Deployment => ("its ReplicaSets and pods", "their ReplicaSets and pods"),
        ObjectKind::CronJob => ("its Jobs and their pods", "their Jobs and their pods"),
        _ => ("its pods", "their pods"),
    };
    if is_single { single } else { many }
}

/// The three labels of the propagation choice with the consequence of each, in radio order
/// (`Background`, `Foreground`, `Orphan`).
pub(crate) fn propagation_choices(
    kind: ObjectKind,
    is_single: bool,
) -> [(DeletePropagation, &'static str, String); 3] {
    let dependents = dependents_of(kind, is_single);
    let owner = kind.name().to_ascii_lowercase();
    let owner = if is_single {
        format!("the {owner}")
    } else {
        format!("the {owner}s")
    };
    let capital = capitalized(dependents);
    [
        (
            DeletePropagation::Background,
            "Delete in the background (default)",
            format!("{capital} are deleted after {owner}"),
        ),
        (
            DeletePropagation::Foreground,
            "Delete them first (foreground)",
            format!("{} stays until {dependents} are gone", capitalized(&owner)),
        ),
        (
            DeletePropagation::Orphan,
            "Keep them (orphan)",
            format!("{capital} keep running without an owner"),
        ),
    ]
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

// ---- Progress and notices ----

/// What a dry-run means for a delete item: an object that is gone is not a failure, and a uid
/// mismatch says so.
pub(crate) fn delete_dry_run_progress(
    result: &Result<WriteOutcome, CheckedWriteError>,
) -> ItemProgress {
    match result {
        Err(CheckedWriteError::Write(WriteError::NotFound)) => ItemProgress::Gone,
        Err(CheckedWriteError::Write(WriteError::Conflict { .. })) => {
            ItemProgress::Rejected(RECREATED_TEXT.into())
        }
        other => super::batch_write::dry_run_progress(other),
    }
}

/// What a commit's result means for a delete item. A blocked commit stops the rest; an object that
/// went away meanwhile counts as gone.
pub(crate) fn delete_commit_progress(
    result: Result<WriteOutcome, CheckedWriteError>,
    stopped: &mut Option<SharedString>,
) -> ItemProgress {
    match result {
        Ok(outcome) => match outcome.effect {
            WriteEffect::DeletionPending { finalizers } => ItemProgress::Pending(finalizers),
            _ => ItemProgress::Done,
        },
        Err(CheckedWriteError::Blocked(reason)) => {
            *stopped = Some(reason.clone());
            ItemProgress::NotSent(reason)
        }
        Err(CheckedWriteError::Write(WriteError::NotFound)) => ItemProgress::Gone,
        Err(CheckedWriteError::Write(WriteError::Conflict { .. })) => {
            ItemProgress::Failed(RECREATED_TEXT.into())
        }
        Err(CheckedWriteError::Write(WriteError::OutcomeUnknown)) => ItemProgress::Unknown,
        Err(CheckedWriteError::Write(error)) => {
            ItemProgress::Failed(write_error_text(&error).into())
        }
    }
}

/// The notice after the last delete commit: the per-object line for one object (decision 14), the
/// counts for several.
pub(crate) fn delete_notice(batch: &BatchIntent, results: &[ItemProgress]) -> String {
    match (batch.plan.items.as_slice(), results) {
        ([item], [progress]) => {
            single_notice(&item.label, progress, item.request.target().kind_name())
        }
        _ => bulk_notice(results),
    }
}

fn single_notice(label: &str, progress: &ItemProgress, kind_name: &str) -> String {
    match progress {
        ItemProgress::Done => format!("{label}: done"),
        ItemProgress::Pending(finalizers) if !finalizers.is_empty() => format!(
            "{label}: marked for deletion; waiting for finalizers: {}",
            finalizer_names(finalizers)
        ),
        ItemProgress::Pending(_) if kind_name == ObjectKind::Pod.name() => {
            format!("{label}: terminating (grace period)")
        }
        ItemProgress::Pending(_) => format!("{label}: terminating"),
        ItemProgress::Gone => format!(
            "{} was already deleted",
            label.trim_start_matches("Delete ")
        ),
        ItemProgress::Failed(reason) | ItemProgress::NotSent(reason) => {
            format!("{label} failed: {reason}")
        }
        ItemProgress::Unknown => format!(
            "{label}: the outcome is unknown; the change may have been applied. Refresh to check."
        ),
        ItemProgress::Waiting
        | ItemProgress::Checking
        | ItemProgress::Passed
        | ItemProgress::Rejected(_)
        | ItemProgress::Applying => format!("{label}: not finished"),
    }
}

/// `Deleted 9 of 12, 2 waiting for finalizers, 1 failed (reason), stopped: reason`.
fn bulk_notice(results: &[ItemProgress]) -> String {
    let count = |wanted: fn(&ItemProgress) -> bool| results.iter().filter(|r| wanted(r)).count();
    let accepted = count(|state| matches!(state, ItemProgress::Done | ItemProgress::Pending(_)));
    let waiting =
        count(|state| matches!(state, ItemProgress::Pending(finalizers) if !finalizers.is_empty()));
    let gone = count(|state| matches!(state, ItemProgress::Gone));
    let failed = count(|state| matches!(state, ItemProgress::Failed(_)));
    let unknown = count(|state| matches!(state, ItemProgress::Unknown));
    let first_failure = results.iter().find_map(|state| match state {
        ItemProgress::Failed(reason) => Some(reason.clone()),
        _ => None,
    });
    let stopped = results.iter().find_map(|state| match state {
        ItemProgress::NotSent(reason) => Some(reason.clone()),
        _ => None,
    });
    let mut text = format!("Deleted {accepted} of {}", results.len());
    if waiting > 0 {
        text.push_str(&format!(", {waiting} waiting for finalizers"));
    }
    if gone > 0 {
        text.push_str(&format!(", {gone} already gone"));
    }
    if failed > 0 {
        text.push_str(&format!(", {failed} failed"));
        if let Some(reason) = first_failure {
            text.push_str(&format!(" ({reason})"));
        }
    }
    if unknown > 0 {
        text.push_str(&format!(
            ", {unknown} unknown (the change may have been applied)"
        ));
    }
    if let Some(reason) = stopped {
        text.push_str(&format!(", stopped: {reason}"));
    }
    text
}

// ---- The start ----

/// What a delete needs from the shell before it reads anything: the objects to read, on which
/// cluster and connection, and the guard generation they were gated on.
struct DeletePlan {
    cluster: ClusterRef,
    cluster_name: SharedString,
    kind: ObjectKind,
    generation: u64,
    connection: ClusterConnection,
    objects: Vec<(ObjectRef, TargetFacts)>,
    /// The rows left out (a Helm release record, an invalid name), listed in the dialog.
    skipped: Vec<SkippedItem>,
}

/// Marks the notice of a refused or failed delete. One id, so a held Del replaces it instead of
/// stacking one notice per key repeat.
struct DeleteNotice;

/// Marks the notice that the objects are being read.
struct DeleteReadNotice;

fn notify_delete(window: &mut Window, cx: &mut App, text: String) {
    window.push_notification(Notification::warning(text).id::<DeleteNotice>(), cx);
}

fn notify_reading(window: &mut Window, cx: &mut App, text: String) {
    window.push_notification(Notification::info(text).id::<DeleteReadNotice>(), cx);
}

/// `Reading api-x…`-style text for the notice: `Reading 1 object…`, `Reading 12 objects…`.
fn reading_text(count: usize) -> String {
    match count {
        1 => "Reading 1 object…".to_owned(),
        count => format!("Reading {count} objects…"),
    }
}

/// An identity read: the uid, or why there is none.
type IdentityRead = Result<ObjectIdentity, ClusterError>;

impl AppShell {
    /// The objects of the shown screen that are ticked, in display order.
    pub(crate) fn checked_objects(&self, cx: &App) -> Vec<ClusterObject> {
        use crate::app_shell::Screen;
        match self.screen {
            Screen::Pods => self.pod_table.read(cx).delegate().checked_objects(cx),
            Screen::Nodes => self.node_table.read(cx).delegate().checked_objects(cx),
            Screen::Kind(_) => self.kind_table.read(cx).delegate().checked_objects(cx),
            Screen::Overview | Screen::Issues | Screen::Topology | Screen::PortForwarding => {
                Vec::new()
            }
        }
    }

    /// The `Delete…` button of the selection bar, last before the clear button: the same gate as the
    /// menu and Del, read for the ticked rows. Screens whose rows are not objects (Issues) have none.
    // ponytail: `checked_objects` merges every slot's rows on each frame while rows are ticked, so
    // the cost is linear in the rows shown; cache the ticked objects per table version if a
    // screen with tens of thousands of rows makes the selection bar slow.
    pub(crate) fn delete_bulk_button(&self, cx: &App) -> Option<BulkButton> {
        self.screen.access_kind()?;
        let state = match self.delete_gate(&self.checked_objects(cx), cx) {
            Ok(kind) => BulkState::Ready(ResourceAction::Delete(kind)),
            Err(reason) => BulkState::Off(reason),
        };
        Some(BulkButton {
            label: "Delete…".into(),
            state,
            is_danger: true,
        })
    }

    /// How many objects Del would delete now: the ticked set when the cursor row is among two or more
    /// ticked rows, else one. The menu item reads it for its label.
    // ponytail: same per-draw merge as `delete_bulk_button`, paid only while a menu is open.
    pub(crate) fn delete_scope_size(&self, cx: &App) -> usize {
        let Some(subject) = &self.selected else {
            return 1;
        };
        delete_scope(subject, &self.checked_objects(cx)).map_or(1, |scope| scope.len())
    }

    /// Del, the menu item, and the palette entry end here: the cursor row, or the ticked set.
    pub(crate) fn delete_at_cursor(
        &mut self,
        subject: &ClusterObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match delete_scope(subject, &self.checked_objects(cx)) {
            Ok(scope) => self.start_delete(scope, window, cx),
            Err(reason) => {
                let label = action_label(ResourceAction::Delete(ObjectKind::Pod));
                notify_delete(window, cx, unavailable_text(label, &reason));
            }
        }
    }

    /// The first step of a delete: gate, then read the uid of every object (decision 2), then the
    /// confirm dialog. Nothing is sent without the dialog, and nothing is deleted from here.
    ///
    /// A second call while a read is running, or while a dialog is open, does nothing: a held Del
    /// repeats, and must not open the dialog twice.
    pub(crate) fn start_delete(
        &mut self,
        scope: Vec<ClusterObject>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_editing() || self.delete_start.is_some() || window.has_active_dialog(cx) {
            return;
        }
        let plan = match self.delete_plan(&scope, cx) {
            Ok(plan) => plan,
            Err(reason) => {
                let label = action_label(ResourceAction::Delete(ObjectKind::Pod));
                notify_delete(window, cx, unavailable_text(label, &reason));
                return;
            }
        };
        // The reads take a moment on a busy cluster; say so, so the key does not look dead.
        notify_reading(window, cx, reading_text(plan.objects.len()));
        let runtime = cx.global::<ClusterRuntime>().clone();
        let objects: Vec<ObjectRef> = plan
            .objects
            .iter()
            .map(|(object, _)| object.clone())
            .collect();
        let connection = plan.connection.clone();
        let reading = runtime.spawn(async move { read_identities(&connection, &objects).await });
        self.delete_start = Some(cx.spawn_in(window, async move |shell, cx| {
            let reads = reading.await.unwrap_or_default();
            let _ = shell.update_in(cx, |shell, window, cx| {
                shell.finish_delete_start(plan, reads, window, cx);
            });
        }));
    }

    /// The gate of a delete of `scope`: one cluster, at most 50 rows of one deletable kind, the
    /// permission, the lock, and no batch running on the cluster. The selection bar reads it for
    /// its button, and the start reads it again because a menu or a key may be a moment old.
    pub(crate) fn delete_gate(
        &self,
        scope: &[ClusterObject],
        cx: &App,
    ) -> Result<ObjectKind, SharedString> {
        let first = scope
            .first()
            .ok_or_else(|| SharedString::from("Select rows first"))?;
        if scope.iter().any(|object| object.cluster != first.cluster) {
            return Err("Select rows of one cluster".into());
        }
        if scope.len() > MAX_BATCH_ITEMS {
            return Err(format!("Select at most {MAX_BATCH_ITEMS} rows").into());
        }
        let kind = delete_kind(&first.key)
            .ok_or_else(|| SharedString::from("this object cannot be deleted here"))?;
        let guard = self
            .guard_for(&first.cluster, cx)
            .ok_or_else(|| SharedString::from("the cluster is not open"))?;
        if let ActionAvailability::Disabled { reason } =
            action_availability(ResourceAction::Delete(kind), &guard)
        {
            return Err(reason);
        }
        if self.running_batches.contains(&first.cluster) {
            return Err(super::batch_write::BATCH_RUNNING_REASON.into());
        }
        let live = self
            .slot_live(&first.cluster, cx)
            .ok_or_else(|| SharedString::from("the cluster is not open"))?;
        if scope.iter().all(|object| is_helm_record(live, &object.key)) {
            return Err(HELM_RECORD_REASON.into());
        }
        Ok(kind)
    }

    /// The gate, then the facts the warnings use and the objects to read.
    fn delete_plan(&self, scope: &[ClusterObject], cx: &App) -> Result<DeletePlan, SharedString> {
        let kind = self.delete_gate(scope, cx)?;
        let cluster = scope[0].cluster.clone();
        let (Some(guard), Some(live)) =
            (self.guard_for(&cluster, cx), self.slot_live(&cluster, cx))
        else {
            return Err("the cluster is not open".into());
        };
        let (mut objects, mut skipped) = (Vec::new(), Vec::new());
        for subject in scope {
            if delete_kind(&subject.key) != Some(kind) {
                return Err("Select rows of one kind".into());
            }
            match delete_target_of(live, &subject.key) {
                Ok(target) => objects.push(target),
                Err(skip) => skipped.push(skip),
            }
        }
        if objects.is_empty() {
            let reason = skipped
                .first()
                .map_or_else(|| "Nothing to delete".into(), |skip| skip.reason.clone());
            return Err(reason);
        }
        Ok(DeletePlan {
            cluster_name: guard.display_name().to_owned().into(),
            generation: guard.generation,
            connection: live.connection().clone(),
            cluster,
            kind,
            objects,
            skipped,
        })
    }

    /// The identities are in: stop on a read that failed, drop the objects that are gone, and open
    /// the batch dialog for the rest.
    fn finish_delete_start(
        &mut self,
        plan: DeletePlan,
        reads: Vec<IdentityRead>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Detached, not dropped: this runs inside that task.
        if let Some(task) = self.delete_start.take() {
            task.detach();
        }
        let DeletePlan {
            cluster,
            cluster_name,
            kind,
            generation,
            objects,
            skipped,
            ..
        } = plan;
        let label = action_label(ResourceAction::Delete(kind));
        // Another dialog or an editor opened while the objects were read (the reads take time):
        // a second dialog on top of it could start a second batch on the cluster.
        if self.is_editing() || window.has_active_dialog(cx) {
            let text = unavailable_text(
                label,
                "another dialog or editor opened; nothing was deleted",
            );
            notify_delete(window, cx, text);
            return;
        }
        // The cluster may have reconnected or locked while the objects were read.
        let still_ready = self.guard_for(&cluster, cx).is_some_and(|guard| {
            guard.generation == generation
                && matches!(
                    action_availability(ResourceAction::Delete(kind), &guard),
                    ActionAvailability::Enabled
                )
        });
        if !still_ready {
            let text = unavailable_text(
                label,
                &format!("{cluster_name} changed; nothing was deleted"),
            );
            notify_delete(window, cx, text);
            return;
        }
        let total = objects.len();
        let (mut targets, mut already_gone) = (Vec::new(), Vec::new());
        for ((object, facts), read) in objects.into_iter().zip(reads) {
            match read {
                Ok(identity) => targets.push(DeleteTarget {
                    object,
                    identity,
                    facts,
                }),
                Err(ClusterError::Api { code: 404, .. }) => already_gone.push(object_text(&object)),
                Err(error) => {
                    notify_delete(window, cx, identity_failure(&object, &error));
                    return;
                }
            }
        }
        // Fewer answers than objects means the read task was lost: nothing is pinned, so nothing
        // is deleted.
        if targets.len() + already_gone.len() < total {
            let text =
                unavailable_text(label, "the objects could not be read; nothing was deleted");
            notify_delete(window, cx, text);
            return;
        }
        if targets.is_empty() {
            notify_delete(window, cx, gone_notice(&already_gone));
            return;
        }
        let extras = DeleteExtras {
            propagation: DeletePropagation::Background,
            kind,
            targets,
            already_gone,
        };
        let mut intent = delete_batch(&cluster, &cluster_name, extras, jiff::Timestamp::now());
        intent.plan.skipped = skipped;
        self.start_batch(intent, window, cx);
    }

    /// A delete of `object` was accepted: an Edit YAML view open on it learns it is gone, and its
    /// text stays for the user to copy. The drawer and the cursor follow the watch, which drops the
    /// row once the object is gone.
    pub(crate) fn object_deleted(
        &mut self,
        cluster: &ClusterRef,
        object: &ObjectRef,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self.edit.clone() else {
            return;
        };
        let is_open_here = {
            let view = edit.read(cx);
            view.cluster() == cluster && view.object() == object
        };
        if is_open_here {
            edit.update(cx, |view, cx| view.commit_failed(EditFailure::Deleted, cx));
        }
    }
}

/// Reads the identities one after the other (decision 22) and stops at the first failure that is
/// not a missing object: the rest would not be deleted anyway.
async fn read_identities(
    connection: &ClusterConnection,
    objects: &[ObjectRef],
) -> Vec<IdentityRead> {
    let mut reads = Vec::with_capacity(objects.len());
    for object in objects {
        let read = connection.object_identity(object).await;
        let is_fatal =
            matches!(&read, Err(error) if !matches!(error, ClusterError::Api { code: 404, .. }));
        reads.push(read);
        if is_fatal {
            break;
        }
    }
    reads
}

/// `Could not read api-x to pin its uid (…); nothing was deleted` (decision 3).
fn identity_failure(object: &ObjectRef, error: &ClusterError) -> String {
    format!(
        "Could not read {} to pin its uid ({}); nothing was deleted",
        object.name(),
        error_text(error)
    )
}

/// `api-x not found (already deleted or not served)`: a 404 cannot tell a deleted object from a
/// kind the server does not serve.
fn gone_notice(names: &[SharedString]) -> String {
    match names {
        [name] => format!("{name} not found (already deleted or not served)"),
        names => format!(
            "None of the {} objects was found (already deleted or not served)",
            names.len()
        ),
    }
}

/// The object and facts of a row, or why the row is left out.
fn delete_target_of(
    live: &LiveCluster,
    key: &ResourceKey,
) -> Result<(ObjectRef, TargetFacts), SkippedItem> {
    let skip = |object: Option<&ObjectRef>, reason: &str| SkippedItem {
        object: object.map_or_else(|| "object".into(), object_text),
        reason: reason.to_owned().into(),
    };
    let Some(object) = object_ref(key) else {
        return Err(skip(None, "this object cannot be deleted here"));
    };
    if WriteRequest::new(
        object.clone(),
        WriteOperation::DeleteObject {
            uid: "-".to_owned(),
            propagation: DeletePropagation::Background,
        },
    )
    .is_none()
    {
        return Err(skip(Some(&object), "the object name is not valid"));
    }
    if is_helm_record(live, key) {
        return Err(skip(Some(&object), HELM_RECORD_REASON));
    }
    let facts = target_facts(live, key);
    Ok((object, facts))
}

/// Whether the row is the Secret that stores a Helm release. A Secret row that cannot be found
/// counts too: its type is unknown, so it is not deleted (fail closed).
fn is_helm_record(live: &LiveCluster, key: &ResourceKey) -> bool {
    let ResourceKey::Kind {
        kind: crate::resource_kind::ResourceKind::Secrets,
        ..
    } = key
    else {
        return false;
    };
    match live.row_of(key).map(|row| &row.object) {
        Some(KindObject::Secret(secret)) => secret.secret_type == HELM_RELEASE_SECRET_TYPE,
        _ => true,
    }
}

fn target_facts(live: &LiveCluster, key: &ResourceKey) -> TargetFacts {
    match key {
        ResourceKey::Pod { .. } => {
            live.pods
                .items()
                .iter()
                .find(|pod| key.is_pod(pod))
                .map_or(TargetFacts::Plain, |pod| TargetFacts::Pod {
                    has_controller: pod.controller.is_some(),
                })
        }
        ResourceKey::Node { .. } => TargetFacts::Plain,
        ResourceKey::Kind { .. } => match live.row_of(key).map(|row| &row.object) {
            Some(KindObject::PersistentVolume(volume)) => TargetFacts::Volume {
                deletes_asset: volume.reclaim_policy == "Delete",
            },
            Some(KindObject::PersistentVolumeClaim(claim)) => {
                TargetFacts::Claim(claim_volume(live, claim.volume.as_deref()))
            }
            _ => TargetFacts::Plain,
        },
    }
}

/// The reclaim policy of the volume a claim is bound to, read from the volume list the claims
/// screen holds (0014).
fn claim_volume(live: &LiveCluster, volume: Option<&str>) -> ClaimVolume {
    let Some(volume) = volume else {
        return ClaimVolume::Unbound;
    };
    let Some(list) = live
        .companion()
        .and_then(CompanionLists::persistent_volumes)
    else {
        return ClaimVolume::NotLoaded;
    };
    if list.is_loading() {
        return ClaimVolume::NotLoaded;
    }
    match list.items().iter().find(|known| known.name == volume) {
        Some(known) if known.reclaim_policy == "Delete" => ClaimVolume::Deletes(volume.to_owned()),
        Some(_) => ClaimVolume::Keeps,
        None => ClaimVolume::NotLoaded,
    }
}

#[cfg(feature = "screenshot")]
impl AppShell {
    /// `--screen delete-confirm` and `--screen delete-bulk-confirm`: the Delete dialog of fixed
    /// objects of a fixed cluster (Production with one Deployment, Staging with twelve pods). It needs
    /// no cluster at all, skips the gate and the identity read, and its confirm button and Enter do
    /// nothing (`ConfirmDialog::show_fixture`), so it can never delete anything.
    pub(super) fn open_delete_fixture(
        &mut self,
        is_bulk: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use gpui_kit::AppContext as _;

        use crate::confirm_dialog::{ConfirmDialog, DialogInputs, DialogKind};
        use crate::write_guard::{ConfirmMode, confirm_step};
        let (cluster, cluster_name, environment) = if is_bulk {
            (
                fixture_cluster("stg-eu-1"),
                "stg-eu-1",
                Environment::Staging,
            )
        } else {
            (
                fixture_cluster("prod-eu-1"),
                "prod-eu-1",
                Environment::Production,
            )
        };
        let batch = if is_bulk {
            fixture_bulk_batch(&cluster, cluster_name)
        } else {
            fixture_single_batch(&cluster, cluster_name)
        };
        let confirm = confirm_step(
            ConfirmMode::for_environment(environment),
            batch.risk,
            batch.expected(),
        );
        let inputs = DialogInputs {
            shell: cx.weak_entity(),
            kind: DialogKind::Batch(std::rc::Rc::new(batch)),
            confirm,
            environment,
            generation: 0,
        };
        let dialog = cx.new(|cx| ConfirmDialog::new(inputs, window, cx));
        dialog.update(cx, |dialog, _| dialog.show_fixture());
        ConfirmDialog::open(&dialog, window, cx);
    }
}

#[cfg(feature = "screenshot")]
fn fixture_cluster(context: &str) -> ClusterRef {
    ClusterRef {
        kubeconfig: std::path::PathBuf::from("fixture.yaml"),
        context: context.to_owned(),
    }
}

#[cfg(feature = "screenshot")]
fn fixture_target(
    kind: ObjectKind,
    namespace: &str,
    name: &str,
    finalizers: &[&str],
    facts: TargetFacts,
) -> Option<DeleteTarget> {
    Some(DeleteTarget {
        object: ObjectRef::new(kind, Some(namespace.to_owned()), name.to_owned())?,
        identity: ObjectIdentity {
            uid: format!("fixture-{name}"),
            finalizers: finalizers.iter().map(ToString::to_string).collect(),
            deletion_started: None,
        },
        facts,
    })
}

/// One Deployment with a finalizer, so the dialog shows the Dependents choice and a finalizer line.
#[cfg(feature = "screenshot")]
fn fixture_single_batch(cluster: &ClusterRef, cluster_name: &str) -> BatchIntent {
    let targets = fixture_target(
        ObjectKind::Deployment,
        "payments",
        "api",
        &["foregroundDeletion"],
        TargetFacts::Plain,
    )
    .into_iter()
    .collect();
    let extras = DeleteExtras {
        propagation: DeletePropagation::Background,
        kind: ObjectKind::Deployment,
        targets,
        already_gone: Vec::new(),
    };
    delete_batch(cluster, cluster_name, extras, jiff::Timestamp::now())
}

/// Twelve pods, two of them without a controller.
#[cfg(feature = "screenshot")]
fn fixture_bulk_batch(cluster: &ClusterRef, cluster_name: &str) -> BatchIntent {
    let targets = (0..12)
        .filter_map(|index| {
            fixture_target(
                ObjectKind::Pod,
                "payments",
                &format!("worker-7d9f8c-{index:05}"),
                &[],
                TargetFacts::Pod {
                    has_controller: index >= 2,
                },
            )
        })
        .collect();
    let extras = DeleteExtras {
        propagation: DeletePropagation::Background,
        kind: ObjectKind::Pod,
        targets,
        already_gone: Vec::new(),
    };
    delete_batch(cluster, cluster_name, extras, jiff::Timestamp::now())
}

#[cfg(test)]
#[path = "object_delete_tests.rs"]
mod object_delete_tests;
