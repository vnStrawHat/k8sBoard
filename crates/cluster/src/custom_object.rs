//! Objects of a custom resource: a list summary with one typed value per printer column, a
//! flattened view of `spec` and `status` for one selected object, and an instance count.
//!
//! Custom objects can hold inline credentials, so every value passes the custom object rules of
//! `object_yaml` (secret-like kinds and keys, URL userinfo, password-format columns) before it
//! leaves this module. The raw object is dropped inside the summarizers. Nothing here logs or
//! traces object content.

use futures::future::Either;
use futures::{Stream, future, stream};
use kube::Api;
use kube::api::DynamicObject;
use kube::runtime::watcher::{self, ListSemantic};
use serde_json::{Map, Value};

use crate::column_path::{ColumnPath, column_value, cut_text, shown_text, shown_text_within};
use crate::connection::{ClusterConnection, ClusterError};
use crate::custom_resource_definition::{
    ColumnType, CustomResourceType, PrinterColumn, ResourceScope, custom_api_resource,
};
use crate::edit_placeholders::HIDDEN;
use crate::namespace::NamespaceScope;
use crate::node::ConditionStatus;
use crate::object_yaml::{
    ObjectRef, is_secret_kind, mask_custom_object, mask_manifest_annotations,
};
use crate::resource_watch::{WatchUpdate, selected_summary_watch};
use crate::workload::label_terms;

const OBJECTS_ACTION: &str = "watching custom resources";
const FIELDS_ACTION: &str = "watching a custom resource";
const COUNT_ACTION: &str = "counting custom resources";
/// Object sizes are unknown, so a small page bounds the page body like the Secrets watch.
const WATCH_PAGE_SIZE: u32 = 50;
const MAX_CONDITIONS: usize = 10;
const MAX_CONDITION_MESSAGE_CHARS: usize = 300;
const MAX_FIELD_ENTRIES: usize = 60;
const MAX_FIELD_CHARS: usize = 200;
/// Fields deeper than this read as a field count.
const MAX_FIELD_DEPTH: usize = 3;
const MAX_JOINED_ITEMS: usize = 5;

/// One list row of a custom resource.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomObjectSummary {
    pub namespace: Option<String>,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// Exactly one per printer column passed to the watch, in that order.
    pub columns: Vec<ColumnValue>,
    /// `status.conditions`, at most 10, in API order.
    pub conditions: Vec<ObjectCondition>,
    /// `status.phase` when it is a string, else a top-level string `status.status` (ClickHouse
    /// style). Masked and capped like any text.
    pub phase: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColumnValue {
    /// No value, an unsupported path, or a value of the wrong type.
    Absent,
    /// The value is secret (password format, secret-like key or kind).
    Hidden,
    Text(String),
    Integer(i64),
    /// The JSON text of the number.
    Number(String),
    Boolean(bool),
    Date(jiff::Timestamp),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectCondition {
    pub name: String,
    pub status: ConditionStatus,
    pub reason: Option<String>,
    /// Cut at 300 characters, otherwise as the API reports it.
    pub message: Option<String>,
    pub changed_at: Option<jiff::Timestamp>,
}

/// The flattened, masked `spec` and `status` of one object.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CustomObjectFields {
    pub spec: FieldList,
    pub status: FieldList,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldList {
    pub entries: Vec<FieldEntry>,
    /// Entries beyond the cap of 60.
    pub omitted: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldEntry {
    /// Dotted, for example `issuerRef.name`.
    pub path: String,
    pub value: FieldValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldValue {
    Text(String),
    Hidden,
    /// An array holding objects: its length.
    Items(usize),
    /// An object below the depth cap: its field count.
    Fields(usize),
}

/// A printer column with its path compiled once; `path` is `None` when unsupported.
#[derive(Clone)]
struct CompiledColumn {
    column: PrinterColumn,
    path: Option<ColumnPath>,
}

impl ClusterConnection {
    /// Watches the objects of `resource` in `scope`, with one value per printer column. A
    /// cluster-scoped resource ignores the scope. Yields batched snapshots ordered by
    /// (namespace, name). Meant to run only while the kind's screen is shown.
    pub fn watch_custom_objects(
        &self,
        resource: &CustomResourceType,
        columns: &[PrinterColumn],
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<CustomObjectSummary>> + Send + 'static {
        let api_resource = custom_api_resource(resource);
        let apis = match resource.scope {
            ResourceScope::Namespaced => self.scoped_dynamic_apis(&scope, &api_resource),
            ResourceScope::Cluster => {
                vec![(None, Api::all_with(self.client().clone(), &api_resource))]
            }
        };
        let kind = resource.kind.clone();
        let columns: Vec<CompiledColumn> = columns
            .iter()
            .map(|column| CompiledColumn {
                path: ColumnPath::parse(&column.json_path).ok(),
                column: column.clone(),
            })
            .collect();
        selected_summary_watch(
            self,
            apis,
            objects_watch_config(),
            OBJECTS_ACTION,
            move |object: &DynamicObject| custom_object_summary(object, &kind, &columns),
        )
    }

    /// Watches the one custom object `object` and flattens its `spec` and `status`. Snapshots of
    /// 0 or 1 item. `ObjectRef` already guarantees the namespace fits the resource scope; a
    /// reference to a built-in kind yields one empty snapshot and ends.
    pub fn watch_custom_object_fields(
        &self,
        object: &ObjectRef,
    ) -> impl Stream<Item = WatchUpdate<CustomObjectFields>> + Send + 'static {
        let Some((resource, namespace, name)) = object.as_custom() else {
            return Either::Left(stream::once(future::ready(WatchUpdate::Snapshot(
                Vec::new(),
            ))));
        };
        let api_resource = custom_api_resource(resource);
        let api: Api<DynamicObject> = match namespace {
            Some(namespace) => {
                Api::namespaced_with(self.client().clone(), namespace, &api_resource)
            }
            None => Api::all_with(self.client().clone(), &api_resource),
        };
        let kind = resource.kind.clone();
        Either::Right(selected_summary_watch(
            self,
            vec![(namespace.map(str::to_owned), api)],
            fields_watch_config(name),
            FIELDS_ACTION,
            move |object: &DynamicObject| object_fields(object, &kind),
        ))
    }

    /// The number of objects of `resource` in the whole cluster, whatever the explorer scope:
    /// one `limit=1` list. `None` when the server gives no remaining count.
    pub async fn count_custom_objects(
        &self,
        resource: &CustomResourceType,
    ) -> Result<Option<u64>, ClusterError> {
        let api: Api<DynamicObject> =
            Api::all_with(self.client().clone(), &custom_api_resource(resource));
        self.count_one(&api, COUNT_ACTION).await
    }
}

fn objects_watch_config() -> watcher::Config {
    watcher::Config::default()
        .list_semantic(ListSemantic::MostRecent)
        .page_size(WATCH_PAGE_SIZE)
}

fn fields_watch_config(name: &str) -> watcher::Config {
    objects_watch_config().fields(&format!("metadata.name={name}"))
}

fn custom_object_summary(
    object: &DynamicObject,
    kind: &str,
    columns: &[CompiledColumn],
) -> CustomObjectSummary {
    let is_secret_kind = is_secret_kind(kind);
    // Built once per object, and only when some column reads it.
    let metadata = if columns.iter().any(|compiled| {
        compiled
            .path
            .as_ref()
            .is_some_and(ColumnPath::reads_metadata)
    }) {
        let mut metadata = serde_json::to_value(&object.metadata).unwrap_or(Value::Null);
        // Annotations can embed a whole applied manifest, which a column path could reach.
        mask_manifest_annotations(&mut metadata, true);
        metadata
    } else {
        Value::Null
    };
    let status = object.data.get("status");
    CustomObjectSummary {
        namespace: object.metadata.namespace.clone(),
        name: object.metadata.name.clone().unwrap_or_default(),
        created_at: object
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
        labels: label_terms(&object.metadata),
        columns: columns
            .iter()
            .map(|compiled| column_cell(compiled, &metadata, &object.data, is_secret_kind))
            .collect(),
        conditions: status.map(conditions).unwrap_or_default(),
        phase: status
            .and_then(|status| status.get("phase").or_else(|| status.get("status")))
            .and_then(Value::as_str)
            .map(shown_text),
    }
}

/// The cell of one column. A secret-like kind hides every value outside `status` and
/// `metadata`, except dates.
fn column_cell(
    compiled: &CompiledColumn,
    metadata: &Value,
    data: &Value,
    is_secret_kind: bool,
) -> ColumnValue {
    let Some(path) = &compiled.path else {
        return ColumnValue::Absent;
    };
    let found = path.first_match(metadata, data);
    let cell = column_value(&compiled.column, path, found);
    let is_hidden_by_kind = is_secret_kind
        && !path.is_status_or_metadata()
        && compiled.column.column_type != ColumnType::Date;
    match cell {
        ColumnValue::Absent => ColumnValue::Absent,
        _ if is_hidden_by_kind => ColumnValue::Hidden,
        cell => cell,
    }
}

fn conditions(status: &Value) -> Vec<ObjectCondition> {
    let Some(items) = status.get("conditions").and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(condition)
        .take(MAX_CONDITIONS)
        .collect()
}

fn condition(item: &Value) -> Option<ObjectCondition> {
    let text = |key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
    };
    Some(ObjectCondition {
        name: text("type")?.to_owned(),
        status: match text("status") {
            Some("True") => ConditionStatus::True,
            Some("False") => ConditionStatus::False,
            _ => ConditionStatus::Unknown,
        },
        reason: text("reason").map(str::to_owned),
        message: text("message")
            .map(|message| shown_text_within(message, MAX_CONDITION_MESSAGE_CHARS)),
        changed_at: text("lastTransitionTime").and_then(|time| time.parse().ok()),
    })
}

/// The flattened `spec` and `status` of `object`, masked by the custom object rules.
fn object_fields(object: &DynamicObject, kind: &str) -> CustomObjectFields {
    let mut sides = Map::new();
    for side in ["spec", "status"] {
        if let Some(value) = object.data.get(side) {
            sides.insert(side.to_owned(), value.clone());
        }
    }
    let mut sides = Value::Object(sides);
    mask_custom_object(&mut sides, kind);
    CustomObjectFields {
        spec: flatten_side(sides.get("spec"), FieldSide::Spec),
        status: flatten_side(sides.get("status"), FieldSide::Status),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldSide {
    Spec,
    Status,
}

fn flatten_side(value: Option<&Value>, side: FieldSide) -> FieldList {
    let mut list = FieldList::default();
    if let Some(Value::Object(map)) = value {
        flatten_object(map, "", 1, side, &mut list);
    }
    list
}

/// `depth` is the number of path segments of the members of `map`.
fn flatten_object(
    map: &Map<String, Value>,
    prefix: &str,
    depth: usize,
    side: FieldSide,
    list: &mut FieldList,
) {
    let mut members: Vec<_> = map.iter().collect();
    members.sort_by_key(|(key, _)| key.as_str());
    for (key, value) in members {
        // The conditions have their own section; only the top level of `status` holds them.
        if side == FieldSide::Status && depth == 1 && key == "conditions" {
            continue;
        }
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            Value::Object(inner) if depth < MAX_FIELD_DEPTH => {
                flatten_object(inner, &path, depth + 1, side, list);
            }
            Value::Object(inner) if !inner.is_empty() => {
                push_entry(list, path, FieldValue::Fields(inner.len()));
            }
            Value::Array(items) if !items.is_empty() => {
                push_entry(list, path, array_value(items));
            }
            Value::String(_) | Value::Number(_) | Value::Bool(_) => {
                push_entry(list, path, scalar_value(value));
            }
            // Null, empty objects, and empty arrays carry nothing to show.
            _ => {}
        }
    }
}

fn push_entry(list: &mut FieldList, path: String, value: FieldValue) {
    if list.entries.len() >= MAX_FIELD_ENTRIES {
        list.omitted += 1;
        return;
    }
    list.entries.push(FieldEntry { path, value });
}

fn scalar_value(value: &Value) -> FieldValue {
    match value {
        Value::String(text) if text == HIDDEN => FieldValue::Hidden,
        Value::String(text) => FieldValue::Text(cut_text(text, MAX_FIELD_CHARS)),
        other => FieldValue::Text(other.to_string()),
    }
}

/// Scalars join as the first five plus `+n`; anything holding a container is an item count.
fn array_value(items: &[Value]) -> FieldValue {
    if items
        .iter()
        .any(|item| matches!(item, Value::Object(_) | Value::Array(_)))
    {
        return FieldValue::Items(items.len());
    }
    let is_all_hidden = items
        .iter()
        .all(|item| matches!(item, Value::String(text) if text == HIDDEN));
    if is_all_hidden {
        return FieldValue::Hidden;
    }
    let mut joined = items
        .iter()
        .take(MAX_JOINED_ITEMS)
        .map(|item| match item {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    if items.len() > MAX_JOINED_ITEMS {
        joined.push_str(&format!(" +{}", items.len() - MAX_JOINED_ITEMS));
    }
    FieldValue::Text(cut_text(&joined, MAX_FIELD_CHARS))
}

#[cfg(test)]
#[path = "custom_object_tests.rs"]
mod custom_object_tests;
