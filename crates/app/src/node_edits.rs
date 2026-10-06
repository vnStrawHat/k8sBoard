//! Node edits (spec 0034 step 2): the rules of the taint and label editors and of the bulk Cordon
//! and Uncordon, as pure builders of a `WriteIntent` or a `BatchIntent`. Nothing here opens a
//! dialog or sends a request; the dialogs are `node_editor`, and every change goes through the
//! guarded flow of the node's own cluster.

use std::collections::{BTreeMap, HashSet};

use cluster::{
    LabelChange, NodeEdit, NodeScheduling, NodeTaint, ObjectKind, ObjectRef, WriteOperation,
    WriteRequest,
};
use gpui_kit::SharedString;

use crate::app_shell::batch_write::{
    BatchExtras, BatchFailure, BatchIntent, BatchItem, BatchPlan, SkippedItem,
};
use crate::app_shell::write_flow::WriteIntent;
use crate::cluster_registry::ClusterRef;
use crate::resource_actions::{ResourceAction, action_risk};
use crate::write_guard::ActionRisk;

/// Taints that Kubernetes controllers own: node lifecycle, the cordon, and cloud initialization.
const SYSTEM_TAINT_PREFIXES: [&str; 2] =
    ["node.kubernetes.io/", "node.cloudprovider.kubernetes.io/"];
/// Labels the kubelet sets again after an edit. `node-role.kubernetes.io/*` is not among them.
const KUBELET_LABEL_PREFIXES: [&str; 4] = [
    "kubernetes.io/",
    "beta.kubernetes.io/",
    "topology.kubernetes.io/",
    "node.kubernetes.io/",
];
const EFFECTS: [&str; 3] = ["NoSchedule", "PreferNoSchedule", "NoExecute"];
const NO_EXECUTE: &str = "NoExecute";
pub(crate) const NO_EXECUTE_WARNING: &str = "NoExecute evicts pods that do not tolerate it";
const NO_CHANGES: &str = "No changes";
const INVALID_LABEL: &str = "A key or value is not valid for Kubernetes (letters, digits, - _ .)";
/// What a bulk label edit adds to its confirm when it removes a key: node labels drive where
/// DaemonSets place their pods.
const REMOVED_LABEL_WARNING: &str =
    "Removing a label can make DaemonSets that select nodes by it delete their pods on these nodes";

/// How many node names the bulk label editor lists before it counts the rest.
const TARGET_NAMES_SHOWN: usize = 5;

/// The nodes a bulk edit acts on, `a, b, c, d, e +3` after the first five.
pub(crate) fn node_names_text(names: &[String]) -> String {
    let mut text = names
        .iter()
        .take(TARGET_NAMES_SHOWN)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > TARGET_NAMES_SHOWN {
        text.push_str(&format!(" +{}", names.len() - TARGET_NAMES_SHOWN));
    }
    text
}

/// A taint the editor must keep and never let the user change.
pub(crate) fn is_system_taint(key: &str) -> bool {
    SYSTEM_TAINT_PREFIXES
        .iter()
        .any(|prefix| key.starts_with(prefix))
}

/// A label the kubelet re-sets, so an edit would not last.
pub(crate) fn is_kubelet_label(key: &str) -> bool {
    KUBELET_LABEL_PREFIXES
        .iter()
        .any(|prefix| key.starts_with(prefix))
}

/// The cluster of the node and the name the dialogs call it: every builder takes them from the
/// node's own cluster, never from the primary.
pub(crate) struct NodeScope<'a> {
    pub(crate) cluster: &'a ClusterRef,
    pub(crate) cluster_name: &'a str,
}

/// One line of the taint editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TaintRow {
    pub(crate) key: String,
    pub(crate) value: String,
    pub(crate) effect: String,
    /// Kept from the node and sent back unchanged: the node lifecycle controller times its
    /// `NoExecute` evictions from it.
    pub(crate) time_added: Option<jiff::Timestamp>,
}

impl TaintRow {
    pub(crate) fn is_read_only(&self) -> bool {
        is_system_taint(&self.key)
    }
}

/// One line of the label editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LabelRow {
    pub(crate) key: String,
    pub(crate) value: String,
}

impl LabelRow {
    pub(crate) fn is_read_only(&self) -> bool {
        is_kubelet_label(&self.key)
    }
}

/// The rows of the taint editor for a node as it is now, in the API order.
pub(crate) fn taint_rows(edit: &NodeEdit) -> Vec<TaintRow> {
    edit.taints
        .iter()
        .map(|taint| TaintRow {
            key: taint.key.clone(),
            value: taint.value.clone().unwrap_or_default(),
            effect: taint.effect.clone(),
            time_added: taint.time_added,
        })
        .collect()
}

/// The rows of the label editor for a node as it is now, by key.
pub(crate) fn label_rows(edit: &NodeEdit) -> Vec<LabelRow> {
    edit.labels
        .iter()
        .map(|(key, value)| LabelRow {
            key: key.clone(),
            value: value.clone(),
        })
        .collect()
}

/// The taints of a request that hit a conflict, as the editor rows the user had.
pub(crate) fn taint_rows_of_request(operation: &WriteOperation) -> Option<Vec<TaintRow>> {
    let WriteOperation::SetNodeTaints { taints, .. } = operation else {
        return None;
    };
    Some(
        taints
            .iter()
            .map(|taint| TaintRow {
                key: taint.key.clone(),
                value: taint.value.clone().unwrap_or_default(),
                effect: taint.effect.clone(),
                time_added: taint.time_added,
            })
            .collect(),
    )
}

/// The rows of the editor reopened after a conflict. The node's own managed taints are as it has
/// them now (locked rows cannot be edited, and an old copy would be refused). With the taints the
/// editor first read (`base`), the rows the user did not touch take the node's values, so the retry
/// never puts back what someone else changed: only an edited or added row is kept as typed, a row
/// the user removed stays removed, and a taint that is new on the node joins the rows. Without
/// `base`, every row of the user is kept.
pub(crate) fn rows_after_conflict(
    current: &NodeEdit,
    base: Option<&[NodeTaint]>,
    kept: &[TaintRow],
) -> Vec<TaintRow> {
    let now = taint_rows(current);
    let mut rows: Vec<TaintRow> = now
        .iter()
        .filter(|row| row.is_read_only())
        .cloned()
        .collect();
    let kept = kept.iter().filter(|row| !row.is_read_only());
    let Some(base) = base else {
        rows.extend(kept.cloned());
        return rows;
    };
    let is_same =
        |left: &TaintRow, right: &TaintRow| left.key == right.key && left.effect == right.effect;
    let base_rows: Vec<TaintRow> = base
        .iter()
        .map(|taint| TaintRow {
            key: taint.key.clone(),
            value: taint.value.clone().unwrap_or_default(),
            effect: taint.effect.clone(),
            time_added: taint.time_added,
        })
        .collect();
    let kept: Vec<&TaintRow> = kept.collect();
    for row in &kept {
        let original = base_rows.iter().find(|original| is_same(original, row));
        let is_untouched = original.is_some_and(|original| original.value == row.value.trim());
        if !is_untouched {
            rows.push((*row).clone());
        } else if let Some(server) = now.iter().find(|server| is_same(server, row)) {
            // Untouched rows follow the node; one the node dropped is dropped here too.
            rows.push(server.clone());
        }
    }
    let is_unseen = |server: &&TaintRow| {
        !server.is_read_only()
            && !base_rows.iter().any(|original| is_same(original, server))
            && !kept.iter().any(|row| is_same(row, server))
    };
    rows.extend(now.iter().filter(is_unseen).cloned());
    rows
}

fn taint_text(key: &str, value: &str, effect: &str) -> String {
    if value.is_empty() {
        format!("{key}:{effect}")
    } else {
        format!("{key}={value}:{effect}")
    }
}

/// The line a reopened editor starts with: the user's rows were kept, and what other hands did to
/// the node's taints since the editor read it (`base`), or all of them now when that is unknown.
pub(crate) fn conflict_notice(base: Option<&[NodeTaint]>, current: &NodeEdit) -> String {
    let text = |taint: &NodeTaint| {
        taint_text(
            &taint.key,
            taint.value.as_deref().unwrap_or_default(),
            &taint.effect,
        )
    };
    let Some(base) = base else {
        let now: Vec<String> = current.taints.iter().map(text).collect();
        let now = if now.is_empty() {
            "none".to_owned()
        } else {
            now.join(", ")
        };
        return format!("The node changed; your rows are kept. Taints on the node now: {now}.");
    };
    let find = |list: &[NodeTaint], taint: &NodeTaint| {
        list.iter()
            .find(|other| taint_identity(other) == taint_identity(taint))
            .cloned()
    };
    let mut changes = Vec::new();
    for taint in &current.taints {
        match find(base, taint) {
            None => changes.push(format!("added {}", text(taint))),
            Some(old) if old.value != taint.value => {
                changes.push(format!("{} became {}", text(&old), text(taint)));
            }
            Some(_) => {}
        }
    }
    for taint in base {
        if find(&current.taints, taint).is_none() {
            changes.push(format!("removed {}", text(taint)));
        }
    }
    if changes.is_empty() {
        return "The node changed; your rows are kept. Review before applying.".to_owned();
    }
    format!(
        "The node changed (by someone else): {}. Rows you did not touch follow the node; your edits are kept. Review before applying.",
        changes.join(", ")
    )
}

fn node_target(node: &str) -> Result<ObjectRef, SharedString> {
    ObjectRef::new(ObjectKind::Node, None, node.to_owned())
        .ok_or_else(|| "The node name is not valid".into())
}

fn node_taint(row: &TaintRow) -> NodeTaint {
    let value = row.value.trim();
    NodeTaint {
        key: row.key.trim().to_owned(),
        value: (!value.is_empty()).then(|| value.to_owned()),
        effect: row.effect.clone(),
        time_added: row.time_added,
    }
}

/// The identity of a taint: two taints with the same key and effect are one.
fn taint_identity(taint: &NodeTaint) -> (&str, &str) {
    (taint.key.as_str(), taint.effect.as_str())
}

/// Which input of an editor row a problem is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowField {
    Key,
    Value,
}

/// The first row Kubernetes would refuse, with the input to mark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RowProblem {
    /// Position among all rows, from 0; the text counts from 1.
    pub(crate) index: usize,
    pub(crate) field: RowField,
    pub(crate) text: SharedString,
}

/// The start of the problem line of a row with no key. The editors show it only once Review… was
/// pressed, not while the user is still typing.
const EMPTY_KEY_PROBLEM: &str = "Enter a key for every";

/// Whether `problem` is the line of a row with no key.
pub(crate) fn is_empty_key_problem(problem: &str) -> bool {
    problem.starts_with(EMPTY_KEY_PROBLEM)
}

/// What a key or value may look like, shown under the editor and in the tooltip of a bad row.
pub(crate) const KEY_HINT: &str = "optional prefix/ then name: letters, digits, - _ ., max 63";
/// Long input is cut so the line stays one line.
const QUOTED_INPUT_MAX: usize = 40;

fn quoted(text: &str) -> String {
    match text.char_indices().nth(QUOTED_INPUT_MAX) {
        Some((end, _)) => format!("'{}…'", &text[..end]),
        None => format!("'{text}'"),
    }
}

/// The write path is the one judge of a valid key and value; each row is asked on its own so the
/// message can name it. An empty key is left to the checks that name no field.
fn first_row_problem<'a>(
    rows: impl Iterator<Item = (&'a str, &'a str)>,
    is_accepted: impl Fn(&str, Option<&str>) -> bool,
) -> Option<RowProblem> {
    for (index, (key, value)) in rows.enumerate() {
        let (key, value) = (key.trim(), value.trim());
        if key.is_empty() {
            continue;
        }
        let row = index + 1;
        if !is_accepted(key, None) {
            return Some(RowProblem {
                index,
                field: RowField::Key,
                text: format!(
                    "Row {row}: key {} is not a valid Kubernetes key",
                    quoted(key)
                )
                .into(),
            });
        }
        if !value.is_empty() && !is_accepted(key, Some(value)) {
            return Some(RowProblem {
                index,
                field: RowField::Value,
                text: format!(
                    "Row {row}: value {} is not valid (letters, digits, - _ ., max 63)",
                    quoted(value)
                )
                .into(),
            });
        }
    }
    None
}

/// The first taint row with a key or value the API would refuse.
pub(crate) fn taint_row_problem(rows: &[TaintRow]) -> Option<RowProblem> {
    first_row_problem(
        rows.iter()
            .map(|row| (row.key.as_str(), row.value.as_str())),
        |key, value| {
            let taint = NodeTaint {
                key: key.to_owned(),
                value: value.map(str::to_owned),
                effect: EFFECTS[0].to_owned(),
                time_added: None,
            };
            node_target("node").ok().is_some_and(|target| {
                WriteRequest::new(
                    target,
                    WriteOperation::SetNodeTaints {
                        taints: vec![taint],
                        resource_version: "1".to_owned(),
                        previous: Vec::new(),
                    },
                )
                .is_some()
            })
        },
    )
}

/// The first label row with a key or value the API would refuse.
pub(crate) fn label_row_problem(rows: &[LabelRow]) -> Option<RowProblem> {
    first_row_problem(
        rows.iter()
            .map(|row| (row.key.as_str(), row.value.as_str())),
        |key, value| {
            let change = LabelChange {
                key: key.to_owned(),
                value: Some(value.unwrap_or_default().to_owned()),
            };
            node_target("node").ok().is_some_and(|target| {
                WriteRequest::new(
                    target,
                    WriteOperation::SetNodeLabels {
                        changes: vec![change],
                    },
                )
                .is_some()
            })
        },
    )
}

/// Why `rows` cannot be sent, `None` when they can. The managed rows must come back as the node
/// has them, so a change to one (or a new one) is refused here and not only in the editor.
fn taint_problem(edit: &NodeEdit, taints: &[NodeTaint]) -> Option<SharedString> {
    let mut seen = HashSet::new();
    for taint in taints {
        if taint.key.is_empty() {
            return Some(format!("{EMPTY_KEY_PROBLEM} taint").into());
        }
        if !EFFECTS.contains(&taint.effect.as_str()) {
            return Some(format!("{} is not a taint effect", taint.effect).into());
        }
        if !seen.insert(taint_identity(taint)) {
            return Some(format!("{}:{} is listed twice", taint.key, taint.effect).into());
        }
    }
    let managed_gone = edit
        .taints
        .iter()
        .filter(|taint| is_system_taint(&taint.key))
        .find(|taint| !taints.contains(taint));
    if let Some(taint) = managed_gone {
        return Some(format!("{} is managed by Kubernetes", taint.key).into());
    }
    let managed_added = taints
        .iter()
        .filter(|taint| is_system_taint(&taint.key))
        .find(|taint| !edit.taints.contains(taint));
    managed_added.map(|taint| format!("{} is managed by Kubernetes", taint.key).into())
}

/// Edit taints of `node`: the full list of `rows` with the `resourceVersion` the node was read at,
/// so a change made meanwhile is a conflict and never a lost update. `Err` is the line the editor
/// shows under the rows; `No changes` keeps its Review button off.
pub(crate) fn taint_intent(
    scope: &NodeScope<'_>,
    node: &str,
    edit: &NodeEdit,
    rows: &[TaintRow],
) -> Result<WriteIntent, SharedString> {
    let taints: Vec<NodeTaint> = rows.iter().map(node_taint).collect();
    if let Some(problem) = taint_problem(edit, &taints) {
        return Err(problem);
    }
    if let Some(problem) = taint_row_problem(rows) {
        return Err(problem.text);
    }
    if taints == edit.taints {
        return Err(NO_CHANGES.into());
    }
    // A `NoExecute` taint that is new (by key and effect), or whose value changed, evicts the pods
    // that tolerated the old one at once.
    let adds_no_execute = taints.iter().any(|taint| {
        taint.effect == NO_EXECUTE
            && edit
                .taints
                .iter()
                .find(|original| taint_identity(original) == taint_identity(taint))
                .is_none_or(|original| original.value != taint.value)
    });
    let request = WriteRequest::new(
        node_target(node)?,
        WriteOperation::SetNodeTaints {
            taints,
            resource_version: edit.resource_version.clone(),
            previous: edit.taints.clone(),
        },
    )
    .ok_or_else(|| SharedString::from(INVALID_LABEL))?;
    let (risk, warnings) = if adds_no_execute {
        (ActionRisk::Destructive, vec![NO_EXECUTE_WARNING.into()])
    } else {
        (action_risk(ResourceAction::EditTaints), Vec::new())
    };
    Ok(WriteIntent {
        cluster: scope.cluster.clone(),
        cluster_name: scope.cluster_name.to_owned().into(),
        action: ResourceAction::EditTaints,
        label: format!("Edit taints of node {node}").into(),
        button: "Edit taints".into(),
        request,
        risk,
        warnings,
    })
}

/// The per-key difference between the node's labels and the rows: a value to set for an added or
/// changed key, `None` to remove a key. Unchanged keys are not in it.
fn label_changes(edit: &NodeEdit, rows: &[LabelRow]) -> Vec<LabelChange> {
    let wanted: BTreeMap<&str, &str> = rows
        .iter()
        .map(|row| (row.key.trim(), row.value.trim()))
        .collect();
    let mut changes = Vec::new();
    for (key, value) in &wanted {
        if edit.labels.get(*key).map(String::as_str) != Some(*value) {
            changes.push(LabelChange {
                key: (*key).to_owned(),
                value: Some((*value).to_owned()),
            });
        }
    }
    for key in edit.labels.keys() {
        if !wanted.contains_key(key.as_str()) {
            changes.push(LabelChange {
                key: key.clone(),
                value: None,
            });
        }
    }
    changes.sort_by(|left, right| left.key.cmp(&right.key));
    changes
}

/// Edit labels of `node`: only the changed keys, as a per-key merge patch (keys are independent,
/// so there is no `resourceVersion`). The kubelet's labels cannot change.
pub(crate) fn label_intent(
    scope: &NodeScope<'_>,
    node: &str,
    edit: &NodeEdit,
    rows: &[LabelRow],
) -> Result<WriteIntent, SharedString> {
    let mut seen = HashSet::new();
    for row in rows {
        let key = row.key.trim();
        if key.is_empty() {
            return Err(format!("{EMPTY_KEY_PROBLEM} label").into());
        }
        if !seen.insert(key) {
            return Err(format!("{key} is listed twice").into());
        }
    }
    if let Some(problem) = label_row_problem(rows) {
        return Err(problem.text);
    }
    let changes = label_changes(edit, rows);
    if let Some(change) = changes.iter().find(|change| is_kubelet_label(&change.key)) {
        return Err(format!("{} is set by the kubelet", change.key).into());
    }
    if changes.is_empty() {
        return Err(NO_CHANGES.into());
    }
    let request = WriteRequest::new(
        node_target(node)?,
        WriteOperation::SetNodeLabels { changes },
    )
    .ok_or_else(|| SharedString::from(INVALID_LABEL))?;
    Ok(WriteIntent {
        cluster: scope.cluster.clone(),
        cluster_name: scope.cluster_name.to_owned().into(),
        action: ResourceAction::EditLabels,
        label: format!("Edit labels of node {node}").into(),
        button: "Edit labels".into(),
        request,
        risk: action_risk(ResourceAction::EditLabels),
        warnings: Vec::new(),
    })
}

/// Which way a bulk cordon goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CordonMode {
    Cordon,
    Uncordon,
}

impl CordonMode {
    fn action(self) -> ResourceAction {
        match self {
            Self::Cordon => ResourceAction::Cordon,
            Self::Uncordon => ResourceAction::Uncordon,
        }
    }

    fn verb(self) -> &'static str {
        match self {
            Self::Cordon => "Cordon",
            Self::Uncordon => "Uncordon",
        }
    }

    /// The scheduling a node already has when there is nothing to do for it.
    fn target_scheduling(self) -> NodeScheduling {
        match self {
            Self::Cordon => NodeScheduling::Disabled,
            Self::Uncordon => NodeScheduling::Enabled,
        }
    }

    fn already(self) -> &'static str {
        match self {
            Self::Cordon => "already cordoned",
            Self::Uncordon => "already schedulable",
        }
    }
}

/// A ticked node as its own cluster reports it now.
pub(crate) struct TickedNode {
    pub(crate) name: String,
    pub(crate) scheduling: NodeScheduling,
    /// The `key=value` terms of the node's labels, as `NodeSummary.labels` has them.
    pub(crate) labels: Vec<String>,
}

impl TickedNode {
    fn label(&self, key: &str) -> Option<&str> {
        self.labels.iter().find_map(|term| {
            let (name, value) = term.split_once('=').unwrap_or((term.as_str(), ""));
            (name == key).then_some(value)
        })
    }
}

/// Bulk Edit labels of the ticked nodes of one cluster: one 0032 batch with one item per node,
/// each carrying only the changes that are not already true there. A node with nothing left is
/// skipped. `Err` is why the batch cannot go (the first failing check wins).
pub(crate) fn label_batch(
    scope: &NodeScope<'_>,
    nodes: &[TickedNode],
    changes: &[LabelChange],
) -> Result<BatchIntent, SharedString> {
    if changes.is_empty() {
        return Err(NO_CHANGES.into());
    }
    let changes: Vec<LabelChange> = changes
        .iter()
        .map(|change| LabelChange {
            key: change.key.trim().to_owned(),
            value: change.value.as_deref().map(|value| value.trim().to_owned()),
        })
        .collect();
    let mut seen = HashSet::new();
    for change in &changes {
        if change.key.is_empty() {
            return Err("Enter a key for every label".into());
        }
        if !seen.insert(change.key.as_str()) {
            return Err(format!("{} is listed twice", change.key).into());
        }
    }
    if let Some(change) = changes.iter().find(|change| is_kubelet_label(&change.key)) {
        return Err(format!("{} is set by the kubelet", change.key).into());
    }
    // The write path decides what a valid key and value are; any node name fits for the check.
    let is_valid = WriteRequest::new(
        node_target("node")?,
        WriteOperation::SetNodeLabels {
            changes: changes.clone(),
        },
    )
    .is_some();
    if !is_valid {
        return Err(INVALID_LABEL.into());
    }
    let (mut items, mut skipped) = (Vec::new(), Vec::new());
    for node in nodes {
        let mut own: Vec<LabelChange> = changes
            .iter()
            .filter(|change| match &change.value {
                Some(value) => node.label(&change.key) != Some(value.as_str()),
                None => node.label(&change.key).is_some(),
            })
            .cloned()
            .collect();
        if own.is_empty() {
            skipped.push(SkippedItem {
                object: node.name.clone().into(),
                reason: "already labelled".into(),
            });
            continue;
        }
        own.sort_by(|left, right| left.key.cmp(&right.key));
        let request = WriteRequest::new(
            node_target(&node.name)?,
            WriteOperation::SetNodeLabels { changes: own },
        )
        .ok_or_else(|| SharedString::from(INVALID_LABEL))?;
        items.push(BatchItem {
            object: node.name.clone().into(),
            label: format!("Edit labels of node {}", node.name).into(),
            request,
        });
    }
    if items.is_empty() {
        return Err("All selected nodes already have these labels".into());
    }
    let label = match items.len() {
        1 => "Edit labels of 1 node".to_owned(),
        count => format!("Edit labels of {count} nodes"),
    };
    let warnings = changes
        .iter()
        .any(|change| change.value.is_none())
        .then(|| SharedString::from(REMOVED_LABEL_WARNING))
        .into_iter()
        .collect();
    Ok(BatchIntent {
        cluster: scope.cluster.clone(),
        cluster_name: scope.cluster_name.to_owned().into(),
        action: ResourceAction::EditLabels,
        label: label.into(),
        verb: "Edit labels".into(),
        button: "Edit labels".into(),
        risk: action_risk(ResourceAction::EditLabels),
        warnings,
        // A bulk names no single object: the TypeName tier types the cluster name.
        plan: BatchPlan {
            cluster: scope.cluster.clone(),
            items,
            skipped,
            extras: BatchExtras::None,
            on_failure: BatchFailure::Continue,
        },
    })
}

/// Bulk Cordon or Uncordon of the ticked nodes of one cluster, as a 0032 batch of the 0030
/// operation: a node already in the target state is skipped, and every other one is dry-run before
/// anything commits. `Err` is the reason the button is off.
pub(crate) fn cordon_batch(
    scope: &NodeScope<'_>,
    mode: CordonMode,
    nodes: &[TickedNode],
) -> Result<BatchIntent, SharedString> {
    let (mut items, mut skipped) = (Vec::new(), Vec::new());
    for node in nodes {
        if node.scheduling == mode.target_scheduling() {
            skipped.push(SkippedItem {
                object: node.name.clone().into(),
                reason: mode.already().into(),
            });
            continue;
        }
        let request = WriteRequest::new(
            node_target(&node.name)?,
            WriteOperation::SetNodeSchedulable {
                schedulable: mode == CordonMode::Uncordon,
            },
        )
        .ok_or_else(|| SharedString::from("The node name is not valid"))?;
        items.push(BatchItem {
            object: node.name.clone().into(),
            label: format!("{} node {}", mode.verb(), node.name).into(),
            request,
        });
    }
    if items.is_empty() {
        let state = mode.already().trim_start_matches("already ");
        return Err(format!("All selected nodes are already {state}").into());
    }
    let label = match items.len() {
        1 => format!("{} 1 node", mode.verb()),
        count => format!("{} {count} nodes", mode.verb()),
    };
    Ok(BatchIntent {
        cluster: scope.cluster.clone(),
        cluster_name: scope.cluster_name.to_owned().into(),
        action: mode.action(),
        label: label.into(),
        verb: mode.verb().into(),
        button: mode.verb().into(),
        risk: action_risk(mode.action()),
        warnings: Vec::new(),
        plan: BatchPlan {
            cluster: scope.cluster.clone(),
            items,
            skipped,
            extras: BatchExtras::None,
            on_failure: BatchFailure::Continue,
        },
    })
}

#[cfg(test)]
#[path = "node_edits_tests.rs"]
mod node_edits_tests;
