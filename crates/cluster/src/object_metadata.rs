//! The labels and annotations of one object, and the per-key patch that changes them (spec 0032b).
//!
//! Annotation values can hold anything a tool wrote, so nothing here prints one: `Debug` counts,
//! and the annotations that embed a whole applied manifest are never returned.

use std::collections::{BTreeMap, HashSet};
use std::fmt;

use serde_json::{Map, Value, json};

use crate::connection::{ClusterConnection, ClusterError};
use crate::debug_pod_bodies::is_label_value;
use crate::node_maintenance_bodies::{LabelChange, is_qualified_name};
use crate::object_yaml::{MASKED_ANNOTATIONS, ObjectRef};

const READ_ACTION: &str = "reading an object for its labels and annotations";
/// The API server's limit on the size of all annotations of an object together.
const MAX_ANNOTATION_BYTES: usize = 256 * 1024;

/// What the labels and annotations editor needs, fresh from the server: the summaries do not
/// carry annotations.
#[derive(Clone, PartialEq, Eq)]
pub struct ObjectMetadata {
    pub labels: BTreeMap<String, String>,
    /// Without the annotations in `hidden_annotations`.
    pub annotations: BTreeMap<String, String>,
    /// The keys of the annotations that embed an applied manifest (`kubectl apply`): the editor
    /// lists them without a value and never sends them.
    pub hidden_annotations: Vec<String>,
}

// Manual: counts only, never an annotation value.
impl fmt::Debug for ObjectMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObjectMetadata")
            .field("labels", &self.labels.len())
            .field("annotations", &self.annotations.len())
            .finish_non_exhaustive()
    }
}

impl ClusterConnection {
    /// Reads the labels and annotations of `object` (a single GET).
    pub async fn object_metadata(
        &self,
        object: &ObjectRef,
    ) -> Result<ObjectMetadata, ClusterError> {
        let value = self.get_object(object, READ_ACTION).await?;
        Ok(metadata_of(&value))
    }
}

fn string_map(value: Option<&Value>) -> BTreeMap<String, String> {
    value
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
        .collect()
}

fn metadata_of(object: &Value) -> ObjectMetadata {
    let metadata = object.get("metadata");
    let mut annotations = string_map(metadata.and_then(|metadata| metadata.get("annotations")));
    let hidden_annotations = MASKED_ANNOTATIONS
        .iter()
        .filter(|key| annotations.remove(**key).is_some())
        .map(|key| (*key).to_owned())
        .collect();
    ObjectMetadata {
        labels: string_map(metadata.and_then(|metadata| metadata.get("labels"))),
        annotations,
        hidden_annotations,
    }
}

/// A per-key merge patch: keys are independent, so no `resourceVersion`; `null` removes a key.
pub(crate) fn metadata_patch(labels: &[LabelChange], annotations: &[LabelChange]) -> Value {
    let map = |changes: &[LabelChange]| -> Map<String, Value> {
        changes
            .iter()
            .map(|change| (change.key.clone(), json!(change.value)))
            .collect()
    };
    let mut metadata = Map::new();
    if !labels.is_empty() {
        metadata.insert("labels".to_owned(), Value::Object(map(labels)));
    }
    if !annotations.is_empty() {
        metadata.insert("annotations".to_owned(), Value::Object(map(annotations)));
    }
    json!({ "metadata": metadata })
}

/// Whether the edit changes something, every key is a qualified name that appears once in its
/// list, every label value is a label value, and the annotations stay under the server's size
/// limit and leave the applied-manifest annotations alone.
pub(crate) fn are_valid_metadata_changes(
    labels: &[LabelChange],
    annotations: &[LabelChange],
) -> bool {
    let has_unique_keys = |changes: &[LabelChange]| {
        let mut seen = HashSet::new();
        changes
            .iter()
            .all(|change| is_qualified_name(&change.key) && seen.insert(change.key.as_str()))
    };
    let annotation_bytes: usize = annotations
        .iter()
        .map(|change| change.key.len() + change.value.as_deref().map_or(0, str::len))
        .sum();
    (!labels.is_empty() || !annotations.is_empty())
        && has_unique_keys(labels)
        && has_unique_keys(annotations)
        && labels
            .iter()
            .all(|change| change.value.as_deref().is_none_or(is_label_value))
        && annotations
            .iter()
            .all(|change| !MASKED_ANNOTATIONS.contains(&change.key.as_str()))
        && annotation_bytes <= MAX_ANNOTATION_BYTES
}

#[cfg(test)]
#[path = "object_metadata_tests.rs"]
mod object_metadata_tests;
