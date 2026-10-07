//! The rules of the labels and annotations editor of a pod or a workload (spec 0032b): a pure
//! builder of a `WriteIntent`. Nothing here opens a dialog or sends a request; the dialog is
//! `metadata_editor`, and the change goes through the guarded flow of the object's own cluster.

use std::collections::{BTreeMap, HashSet};

use cluster::{LabelChange, ObjectKind, ObjectMetadata, ObjectRef, WriteOperation, WriteRequest};
use gpui_kit::SharedString;

use crate::app_shell::write_flow::WriteIntent;
use crate::node_edits::{NodeScope, RowProblem, first_row_problem};
use crate::resource_actions::{ResourceAction, action_risk};

const NO_CHANGES: &str = "No changes";
const EMPTY_KEY_PROBLEM: &str = "Enter a key for every";
const INVALID_ANNOTATIONS: &str = "The annotations are not valid (all of them together may not pass 256 KiB, and a key is an optional prefix/ then a name)";
/// What a value shows in a change line before it is cut, so a long annotation stays one line.
const VALUE_SHOWN_MAX: usize = 60;
const HIDDEN_VALUE: &str = "(hidden)";
const PODS_MATCH_BY_LABEL: &str =
    "Services and controllers match pods by label: changing one can detach this pod from them";

/// Which list a row belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MetadataList {
    Labels,
    Annotations,
}

impl MetadataList {
    fn noun(self) -> &'static str {
        match self {
            Self::Labels => "label",
            Self::Annotations => "annotation",
        }
    }

    fn path(self) -> &'static str {
        match self {
            Self::Labels => "metadata.labels",
            Self::Annotations => "metadata.annotations",
        }
    }
}

/// One line of an editor list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MetadataRow {
    pub(crate) key: String,
    pub(crate) value: String,
}

/// The rows of a list as the object has it now, by key.
pub(crate) fn metadata_rows(map: &BTreeMap<String, String>) -> Vec<MetadataRow> {
    map.iter()
        .map(|(key, value)| MetadataRow {
            key: key.clone(),
            value: value.clone(),
        })
        .collect()
}

/// The first row of `list` Kubernetes would refuse, with the input to mark. The write path is the
/// judge of a key (and of a label value); an annotation value is any text.
pub(crate) fn metadata_row_problem(list: MetadataList, rows: &[MetadataRow]) -> Option<RowProblem> {
    first_row_problem(
        rows.iter()
            .map(|row| (row.key.as_str(), row.value.as_str())),
        |key, value| {
            // Any text is an annotation value; the size limit is checked on the whole edit.
            let value = match list {
                MetadataList::Labels => value.unwrap_or_default(),
                MetadataList::Annotations => "",
            };
            let change = LabelChange {
                key: key.to_owned(),
                value: Some(value.to_owned()),
            };
            let (labels, annotations) = match list {
                MetadataList::Labels => (vec![change], Vec::new()),
                MetadataList::Annotations => (Vec::new(), vec![change]),
            };
            ObjectRef::new(ObjectKind::Pod, Some("ns".to_owned()), "pod".to_owned())
                .and_then(|target| {
                    WriteRequest::new(
                        target,
                        WriteOperation::SetObjectMetadata {
                            labels,
                            annotations,
                        },
                    )
                })
                .is_some()
        },
    )
}

/// The per-key difference between the object's map and the rows: a value to set for an added or
/// changed key, `None` to remove a key. Unchanged keys are not in it.
fn changes_of(
    list: MetadataList,
    current: &BTreeMap<String, String>,
    rows: &[MetadataRow],
) -> Vec<LabelChange> {
    let wanted: BTreeMap<&str, &str> = rows
        .iter()
        .map(|row| {
            let value = row.value.as_str();
            // A label value is trimmed; an annotation is kept as typed.
            let value = if list == MetadataList::Labels {
                value.trim()
            } else {
                value
            };
            (row.key.trim(), value)
        })
        .collect();
    let mut changes = Vec::new();
    for (key, value) in &wanted {
        if current.get(*key).map(String::as_str) != Some(*value) {
            changes.push(LabelChange {
                key: (*key).to_owned(),
                value: Some((*value).to_owned()),
            });
        }
    }
    for key in current.keys() {
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

/// Why the rows of `list` cannot be sent, `None` when they can.
fn list_problem(list: MetadataList, rows: &[MetadataRow]) -> Option<SharedString> {
    let mut seen = HashSet::new();
    for row in rows {
        let key = row.key.trim();
        if key.is_empty() {
            return Some(format!("{EMPTY_KEY_PROBLEM} {}", list.noun()).into());
        }
        if !seen.insert(key) {
            return Some(format!("{key} is listed twice").into());
        }
    }
    metadata_row_problem(list, rows).map(|problem| problem.text)
}

/// `metadata.labels.app` for a plain key, `metadata.annotations[example.com/owner]` for a key
/// with a dot or a slash in it.
fn field_path(list: MetadataList, key: &str) -> String {
    if key.contains(['.', '/']) {
        format!("{}[{key}]", list.path())
    } else {
        format!("{}.{key}", list.path())
    }
}

fn shown(value: &str) -> String {
    match value.char_indices().nth(VALUE_SHOWN_MAX) {
        Some((end, _)) => format!("{}…", &value[..end]),
        None => value.to_owned(),
    }
}

/// `metadata.annotations.x: (none) → y`, `metadata.labels.old: v → (removed)`. A value under a key
/// that looks like a credential is not shown.
fn change_line(
    list: MetadataList,
    change: &LabelChange,
    current: &BTreeMap<String, String>,
) -> SharedString {
    let is_secret = list == MetadataList::Annotations && cluster::is_secret_key(&change.key);
    let text = |value: Option<&str>, absent: &str| match value {
        Some(_) if is_secret => HIDDEN_VALUE.to_owned(),
        Some(value) => shown(value),
        None => absent.to_owned(),
    };
    let old = text(current.get(&change.key).map(String::as_str), "(none)");
    let new = text(change.value.as_deref(), "(removed)");
    format!("{}: {old} → {new}", field_path(list, &change.key)).into()
}

/// Edit labels / annotations of `target`: only the changed keys, as a per-key merge patch (keys are
/// independent, so there is no `resourceVersion`). `Err` is the line the editor shows under the
/// rows; `No changes` keeps its Review button off.
pub(crate) fn metadata_intent(
    scope: &NodeScope<'_>,
    kind: ObjectKind,
    target: &ObjectRef,
    edit: &ObjectMetadata,
    labels: &[MetadataRow],
    annotations: &[MetadataRow],
) -> Result<WriteIntent, SharedString> {
    for (list, rows) in [
        (MetadataList::Labels, labels),
        (MetadataList::Annotations, annotations),
    ] {
        if let Some(problem) = list_problem(list, rows) {
            return Err(problem);
        }
    }
    let label_changes = changes_of(MetadataList::Labels, &edit.labels, labels);
    let annotation_changes = changes_of(MetadataList::Annotations, &edit.annotations, annotations);
    if label_changes.is_empty() && annotation_changes.is_empty() {
        return Err(NO_CHANGES.into());
    }
    let change_lines = label_changes
        .iter()
        .map(|change| change_line(MetadataList::Labels, change, &edit.labels))
        .chain(
            annotation_changes
                .iter()
                .map(|change| change_line(MetadataList::Annotations, change, &edit.annotations)),
        )
        .collect();
    let warnings = if kind == ObjectKind::Pod && !label_changes.is_empty() {
        vec![PODS_MATCH_BY_LABEL.into()]
    } else {
        Vec::new()
    };
    let request = WriteRequest::new(
        target.clone(),
        WriteOperation::SetObjectMetadata {
            labels: label_changes,
            annotations: annotation_changes,
        },
    )
    .ok_or_else(|| SharedString::from(INVALID_ANNOTATIONS))?;
    let action = ResourceAction::EditMetadata(kind);
    Ok(WriteIntent {
        cluster: scope.cluster.clone(),
        cluster_name: scope.cluster_name.to_owned().into(),
        action,
        label: format!(
            "Edit labels / annotations of {} {}",
            target.kind_name().to_ascii_lowercase(),
            target.name()
        )
        .into(),
        button: "Apply changes".into(),
        request,
        risk: action_risk(action),
        warnings,
        change_lines,
        audit_fields: Vec::new(),
    })
}

#[cfg(test)]
#[path = "metadata_edits_tests.rs"]
mod metadata_edits_tests;
