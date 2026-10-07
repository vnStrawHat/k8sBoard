//! Field paths, field changes, and the preview of an edit (spec 0031).
//!
//! Every value in a path or a change comes from a masked tree, never from a raw object. Nothing
//! here logs.

use std::collections::BTreeSet;
use std::fmt;

use serde_json::Value;

use crate::edit_placeholders::{
    HIDDEN, HIDDEN_CHANGED, HIDDEN_MOVED, MARKER_PREFIX, Restored, counterpart, has_unique_names,
    name_of,
};
use crate::object_edit::{ObjectEdit, set_target_kind, strip_server_fields};
use crate::object_yaml::{ObjectKind, mask_object, to_yaml_text};
use crate::quota_demand::{DemandChange, workload_demand};

const LAST_APPLIED: &str = "kubectl.kubernetes.io/last-applied-configuration";
const MAX_CHANGES: usize = 200;
const MAX_VALUE_CHARS: usize = 80;

/// One location in an object. Display: `.key` when the key matches `[A-Za-z0-9_-]+`, otherwise
/// `["key"]`; list items are `[name]` or `[index]`, for example
/// `spec.template.spec.containers[api].env[3].value`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FieldPath(Vec<PathSegment>);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum PathSegment {
    Key(String),
    Name(String),
    Index(usize),
}

impl FieldPath {
    pub(crate) fn new(segments: Vec<PathSegment>) -> Self {
        Self(segments)
    }

    /// Whether one path is the other or lies inside it.
    pub(crate) fn overlaps(&self, other: &Self) -> bool {
        let shorter = self.0.len().min(other.0.len());
        self.0[..shorter] == other.0[..shorter]
    }

    fn starts_with_keys(&self, keys: &[&str]) -> bool {
        self.0.len() >= keys.len()
            && self
                .0
                .iter()
                .zip(keys)
                .all(|(segment, key)| matches!(segment, PathSegment::Key(own) if own == key))
    }
}

impl fmt::Display for FieldPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (position, segment) in self.0.iter().enumerate() {
            match segment {
                PathSegment::Key(key) if is_plain_key(key) => {
                    if position > 0 {
                        formatter.write_str(".")?;
                    }
                    formatter.write_str(key)?;
                }
                PathSegment::Key(key) => write!(formatter, "[{key:?}]")?,
                PathSegment::Name(name) => write!(formatter, "[{name}]")?,
                PathSegment::Index(index) => write!(formatter, "[{index}]")?,
            }
        }
        Ok(())
    }
}

fn is_plain_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// The server's effect of an edit, masked like the editor. Manual `Debug` (counts only); never
/// audited and never in a notification.
#[derive(Clone, PartialEq, Eq)]
pub struct EditPreview {
    /// The object as it is now, masked, without the edit header.
    pub before: String,
    /// The object the dry-run answered, masked, without the edit header.
    pub after: String,
    pub changes: Vec<FieldChange>,
    /// Changes beyond the cap of `changes`.
    pub more_changes: usize,
    pub checks: Vec<EditCheck>,
    /// The change in steady-state pod demand, when the workload counts pods and the demand differs.
    pub demand: Option<DemandChange>,
}

impl fmt::Debug for EditPreview {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EditPreview")
            .field("changes", &self.changes.len())
            .field("more_changes", &self.more_changes)
            .field("checks", &self.checks.len())
            .finish()
    }
}

/// One changed field. `None` means absent on that side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldChange {
    pub path: FieldPath,
    pub old: Option<String>,
    pub new: Option<String>,
}

/// A warning the preview raises next to the diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditCheck {
    /// The pod template changed, so pods are replaced (or, with `OnDelete`, not until deleted).
    Rollout { strategy: String },
    /// The pod template of a paused Deployment changed: no pod changes until Resume.
    RolloutPaused,
    /// A `kubectl apply` user: a replace leaves `last-applied-configuration` stale.
    StaleLastApplied,
    /// A hidden value was matched by position; it may belong to another item now.
    Moved { path: FieldPath },
    /// An unquoted `0755`: YAML 1.2 reads it as decimal 755, kubectl as octal.
    LeadingZero { line: usize },
}

/// The fields at which `before` and `after` differ. Objects are compared by key; lists by unique
/// `name` when both have one on every item (a reordered one is a change of the list), otherwise
/// by index. An added or removed item, or key, is one path; a list of another length is one path.
pub(crate) fn field_paths(before: &Value, after: &Value) -> Vec<FieldPath> {
    let mut paths = Vec::new();
    collect_paths(before, after, &mut Vec::new(), &mut paths);
    paths
}

fn collect_paths(
    before: &Value,
    after: &Value,
    at: &mut Vec<PathSegment>,
    paths: &mut Vec<FieldPath>,
) {
    match (before, after) {
        (Value::Object(old), Value::Object(new)) => {
            let keys: BTreeSet<&String> = old.keys().chain(new.keys()).collect();
            for key in keys {
                at.push(PathSegment::Key(key.clone()));
                match (old.get(key), new.get(key)) {
                    (Some(old), Some(new)) => collect_paths(old, new, at, paths),
                    _ => paths.push(FieldPath::new(at.clone())),
                }
                at.pop();
            }
        }
        (Value::Array(old), Value::Array(new)) => collect_list_paths(old, new, at, paths),
        _ if before != after => paths.push(FieldPath::new(at.clone())),
        _ => {}
    }
}

fn collect_list_paths(
    old: &[Value],
    new: &[Value],
    at: &mut Vec<PathSegment>,
    paths: &mut Vec<FieldPath>,
) {
    if has_unique_names(old) && has_unique_names(new) {
        for item in old {
            let Some(name) = name_of(item) else { continue };
            at.push(PathSegment::Name(name.to_owned()));
            match new
                .iter()
                .find(|candidate| name_of(candidate) == Some(name))
            {
                Some(counterpart) => collect_paths(item, counterpart, at, paths),
                None => paths.push(FieldPath::new(at.clone())),
            }
            at.pop();
        }
        for item in new {
            let Some(name) = name_of(item) else { continue };
            if old.iter().all(|candidate| name_of(candidate) != Some(name)) {
                at.push(PathSegment::Name(name.to_owned()));
                paths.push(FieldPath::new(at.clone()));
                at.pop();
            }
        }
        // Item order is part of the object (container order), so a reorder is a change.
        if common_names(old, new) != common_names(new, old) {
            paths.push(FieldPath::new(at.clone()));
        }
        return;
    }
    if old.len() != new.len() {
        paths.push(FieldPath::new(at.clone()));
        return;
    }
    for (index, (old, new)) in old.iter().zip(new).enumerate() {
        at.push(PathSegment::Index(index));
        collect_paths(old, new, at, paths);
        at.pop();
    }
}

/// The names of `list` that `other` also has, in the order of `list`.
fn common_names<'a>(list: &'a [Value], other: &[Value]) -> Vec<&'a str> {
    list.iter()
        .filter_map(name_of)
        .filter(|name| other.iter().any(|item| name_of(item) == Some(name)))
        .collect()
}

fn value_at<'a>(value: &'a Value, path: &FieldPath) -> Option<&'a Value> {
    path.0
        .iter()
        .try_fold(value, |value, segment| match segment {
            PathSegment::Key(key) => value.get(key),
            PathSegment::Name(name) => value
                .as_array()?
                .iter()
                .find(|item| name_of(item) == Some(name)),
            PathSegment::Index(index) => value.get(index),
        })
}

/// Sets the value `edited` holds at `path` in `target`, and removes the field when `edited` has
/// none there. A named item that `target` lacks is added. `false` when the parent of the path does
/// not exist in `target`, so the change cannot be placed.
pub(crate) fn copy_path(target: &mut Value, edited: &Value, path: &FieldPath) -> bool {
    let Some((last, parents)) = path.0.split_last() else {
        return false;
    };
    let Some(parent) = value_at_mut(target, &FieldPath(parents.to_vec())) else {
        return false;
    };
    let value = value_at(edited, path).cloned();
    match (last, parent, value) {
        (PathSegment::Key(key), Value::Object(map), Some(value)) => {
            map.insert(key.clone(), value);
            true
        }
        (PathSegment::Key(key), Value::Object(map), None) => {
            map.remove(key);
            true
        }
        (PathSegment::Name(name), Value::Array(items), value) => {
            let position = items.iter().position(|item| name_of(item) == Some(name));
            match (position, value) {
                (Some(index), Some(value)) => items[index] = value,
                (None, Some(value)) => items.push(value),
                (Some(index), None) => {
                    items.remove(index);
                }
                (None, None) => {}
            }
            true
        }
        // A positional item is only ever changed, never added or removed: a list of another
        // length is one path at the list itself.
        (PathSegment::Index(index), Value::Array(items), Some(value)) if *index < items.len() => {
            items[*index] = value;
            true
        }
        _ => false,
    }
}

fn value_at_mut<'a>(value: &'a mut Value, path: &FieldPath) -> Option<&'a mut Value> {
    path.0
        .iter()
        .try_fold(value, |value, segment| match segment {
            PathSegment::Key(key) => value.get_mut(key),
            PathSegment::Name(name) => value
                .as_array_mut()?
                .iter_mut()
                .find(|item| name_of(item) == Some(name)),
            PathSegment::Index(index) => value.get_mut(index),
        })
}

/// The changes at `paths`, at most `MAX_CHANGES`, plus the count of the rest.
pub(crate) fn field_changes(
    before: &Value,
    after: &Value,
    paths: &[FieldPath],
) -> (Vec<FieldChange>, usize) {
    let changes = paths
        .iter()
        .take(MAX_CHANGES)
        .map(|path| FieldChange {
            path: path.clone(),
            old: value_at(before, path).map(describe),
            new: value_at(after, path).map(describe),
        })
        .collect();
    (changes, paths.len().saturating_sub(MAX_CHANGES))
}

/// Scalars as text cut to `MAX_VALUE_CHARS`, containers as `{…}` and `[…]`.
fn describe(value: &Value) -> String {
    match value {
        Value::String(text) => {
            let mut shown: String = text.chars().take(MAX_VALUE_CHARS).collect();
            if shown.len() < text.len() {
                shown.push('…');
            }
            shown
        }
        Value::Object(_) => "{…}".to_owned(),
        Value::Array(_) => "[…]".to_owned(),
        scalar => scalar.to_string(),
    }
}

/// Reads a masked placeholder in `after` as `<hidden, changed>` where its raw sides differ.
/// `raw_after` has the shape of `after`; `raw_before` is the raw object it replaces.
fn mark_changed(after: &mut Value, raw_after: &Value, raw_before: Option<&Value>) {
    match after {
        Value::String(text) if text == HIDDEN => {
            if raw_before != Some(raw_after) {
                *after = Value::from(HIDDEN_CHANGED);
            }
        }
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                let Some(raw_child) = raw_after.get(key) else {
                    continue;
                };
                mark_changed(
                    child,
                    raw_child,
                    raw_before.and_then(|before| before.get(key)),
                );
            }
        }
        Value::Array(items) => {
            let (Some(raw_items), before_list) =
                (raw_after.as_array(), raw_before.and_then(Value::as_array))
            else {
                return;
            };
            for (index, item) in items.iter_mut().enumerate() {
                let Some(raw_item) = raw_items.get(index) else {
                    continue;
                };
                let counter =
                    before_list.and_then(|list| counterpart(list, raw_item, index, raw_items));
                mark_changed(item, raw_item, counter);
            }
        }
        _ => {}
    }
}

/// The preview of `edit`: the object before (`fresh`) and after (`response`), masked, with the
/// changes and checks. The raw objects are dropped here. The error is a fixed message.
pub(crate) fn build_preview(
    edit: &ObjectEdit,
    mut fresh: Value,
    mut response: Value,
    restored: &Restored,
) -> Result<EditPreview, &'static str> {
    // Computed before `strip_server_fields` drops `status`: a DaemonSet's pod count is
    // `status.desiredNumberScheduled`.
    let demand = demand_change(edit, &fresh, &response);
    strip_server_fields(&mut fresh);
    strip_server_fields(&mut response);
    let mut before = fresh.clone();
    let mut after = response.clone();
    set_target_kind(&mut before, edit.target());
    set_target_kind(&mut after, edit.target());
    mask_object(&mut before, edit.env(), |_| 0);
    mask_object(&mut after, edit.env(), |_| 0);
    mark_changed(&mut after, &response, Some(&fresh));
    for path in &restored.moved {
        if let Some(Value::String(text)) = value_at_mut(&mut after, path)
            && text.starts_with(MARKER_PREFIX)
        {
            *text = HIDDEN_MOVED.to_owned();
        }
    }
    let paths = field_paths(&before, &after);
    let (changes, more_changes) = field_changes(&before, &after, &paths);
    let mut checks = Vec::new();
    if let Some(strategy) = rollout_strategy(edit, &paths, &after) {
        let is_paused = after.pointer("/spec/paused").and_then(Value::as_bool) == Some(true);
        checks.push(match is_paused {
            true => EditCheck::RolloutPaused,
            false => EditCheck::Rollout { strategy },
        });
    }
    if has_last_applied(&fresh) {
        checks.push(EditCheck::StaleLastApplied);
    }
    checks.extend(
        restored
            .moved
            .iter()
            .map(|path| EditCheck::Moved { path: path.clone() }),
    );
    checks.extend(
        edit.leading_zero_lines()
            .iter()
            .map(|line| EditCheck::LeadingZero { line: *line }),
    );
    Ok(EditPreview {
        before: to_yaml_text(&before, None)?,
        after: to_yaml_text(&after, None)?,
        changes,
        more_changes,
        checks,
        demand,
    })
}

/// The demand before and after, when both are known and differ.
fn demand_change(edit: &ObjectEdit, fresh: &Value, response: &Value) -> Option<DemandChange> {
    let kind = edit.target().builtin_kind()?;
    let before = workload_demand(kind, fresh)?;
    let after = workload_demand(kind, response)?;
    (before != after).then_some(DemandChange { before, after })
}

/// The update strategy when the pod template of a workload changed.
fn rollout_strategy(edit: &ObjectEdit, paths: &[FieldPath], after: &Value) -> Option<String> {
    let strategy_pointer = match edit.target().builtin_kind()? {
        ObjectKind::Deployment => "/spec/strategy/type",
        ObjectKind::StatefulSet | ObjectKind::DaemonSet => "/spec/updateStrategy/type",
        _ => return None,
    };
    paths
        .iter()
        .any(|path| path.starts_with_keys(&["spec", "template"]))
        .then(|| {
            after
                .pointer(strategy_pointer)
                .and_then(Value::as_str)
                .unwrap_or("RollingUpdate")
                .to_owned()
        })
}

pub(crate) fn has_last_applied(object: &Value) -> bool {
    object
        .pointer("/metadata/annotations")
        .and_then(Value::as_object)
        .is_some_and(|annotations| annotations.contains_key(LAST_APPLIED))
}

#[cfg(test)]
#[path = "edit_preview_tests.rs"]
mod edit_preview_tests;
