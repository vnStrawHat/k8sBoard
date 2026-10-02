//! CustomResourceDefinitions: one lean summary per CRD (names, scope, versions, printer
//! columns, a two-level schema outline, state), watched for the whole session.
//!
//! The typed k8s-openapi CRD builds the whole JSON schema tree, which can be megabytes per
//! CRD, so the watch decodes through serde heads that name only what is read. Deeper schema
//! levels are never declared, so serde skips them. Nothing here logs or keeps instance data.

use std::collections::BTreeMap;

use futures::Stream;
use kube::Api;
use kube::core::{ApiResource, GroupVersionKind, Object, Version};
use kube::runtime::watcher::{self, ListSemantic};
use serde::Deserialize;

use crate::column_path::ColumnPath;
use crate::connection::ClusterConnection;
use crate::resource_watch::{WatchUpdate, selected_summary_watch};

const WATCH_ACTION: &str = "watching custom resource definitions";
/// A CRD can be several hundred KiB with its schema, so 20 per page bounds the page body.
const WATCH_PAGE_SIZE: u32 = 20;
const CRD_GROUP: &str = "apiextensions.k8s.io";
/// Most schema fields kept per side (`spec`, `status`) of one version.
const MAX_OUTLINE_FIELDS: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceScope {
    Namespaced,
    Cluster,
}

/// One served version of a custom resource. Built from a `CrdSummary`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CustomResourceType {
    pub group: String,
    pub version: String,
    pub kind: String,
    pub plural: String,
    pub scope: ResourceScope,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrdSummary {
    /// For example `certificates.cert-manager.io`.
    pub name: String,
    pub group: String,
    pub kind: String,
    pub plural: String,
    /// `names.singular`, else the lowercase kind.
    pub singular: String,
    pub scope: ResourceScope,
    /// In spec order.
    pub versions: Vec<CrdVersion>,
    pub state: CrdState,
    pub created_at: Option<jiff::Timestamp>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CrdState {
    Established,
    NamesNotAccepted { message: Option<String> },
    NotEstablished { reason: Option<String> },
    Terminating,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrdVersion {
    pub name: String,
    pub is_served: bool,
    pub is_storage: bool,
    pub is_deprecated: bool,
    pub deprecation_warning: Option<String>,
    pub printer_columns: Vec<PrinterColumn>,
    pub schema: SchemaOutline,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PrinterColumn {
    pub name: String,
    pub column_type: ColumnType,
    pub json_path: String,
    /// `format: password`: every value reads hidden.
    pub is_password: bool,
    /// `ColumnPath::parse` accepted the path.
    pub is_supported: bool,
    /// The path reads a condition's status, so the app tones the cell.
    pub is_condition_status: bool,
}

/// Any type text this build does not know reads as `String`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColumnType {
    String,
    Integer,
    Number,
    Boolean,
    Date,
}

/// The top-level `spec` and `status` fields of one version's schema, in name order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SchemaOutline {
    pub spec: Vec<SchemaField>,
    pub status: Vec<SchemaField>,
    /// Fields beyond the cap of both sides together.
    pub omitted: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaField {
    pub name: String,
    pub field_type: Option<String>,
}

impl PrinterColumn {
    /// Parses `json_path` once to set `is_supported` and `is_condition_status`.
    pub fn new(name: &str, column_type: ColumnType, json_path: &str, is_password: bool) -> Self {
        let path = ColumnPath::parse(json_path).ok();
        Self {
            name: name.to_owned(),
            column_type,
            json_path: json_path.to_owned(),
            is_password,
            is_supported: path.is_some(),
            is_condition_status: path.is_some_and(|path| path.is_condition_status()),
        }
    }
}

impl CrdSummary {
    /// The served version with the highest Kubernetes version priority, like kubectl.
    pub fn preferred_version(&self) -> Option<&CrdVersion> {
        self.versions
            .iter()
            .filter(|version| version.is_served)
            .max_by_key(|version| Version::parse(&version.name).priority())
    }

    pub fn resource(&self, version: &CrdVersion) -> CustomResourceType {
        CustomResourceType {
            group: self.group.clone(),
            version: version.name.clone(),
            kind: self.kind.clone(),
            plural: self.plural.clone(),
            scope: self.scope,
        }
    }
}

impl ClusterConnection {
    /// Watches every CRD. Yields batched snapshots ordered by name.
    pub fn watch_crds(&self) -> impl Stream<Item = WatchUpdate<CrdSummary>> + Send + 'static {
        let api: Api<CrdObject> = Api::all_with(self.client().clone(), &crd_resource());
        selected_summary_watch(
            self,
            vec![(None, api)],
            crd_watch_config(),
            WATCH_ACTION,
            crd_summary,
        )
    }
}

fn crd_watch_config() -> watcher::Config {
    watcher::Config::default()
        .list_semantic(ListSemantic::MostRecent)
        .page_size(WATCH_PAGE_SIZE)
}

fn crd_resource() -> ApiResource {
    ApiResource::from_gvk_with_plural(
        &GroupVersionKind::gvk(CRD_GROUP, "v1", "CustomResourceDefinition"),
        "customresourcedefinitions",
    )
}

/// The API resource of one served version of a custom resource. Shared by the object watches
/// and the YAML read.
pub(crate) fn custom_api_resource(resource: &CustomResourceType) -> ApiResource {
    ApiResource::from_gvk_with_plural(
        &GroupVersionKind::gvk(&resource.group, &resource.version, &resource.kind),
        &resource.plural,
    )
}

type CrdObject = Object<CrdSpecHead, CrdStatusHead>;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CrdSpecHead {
    #[serde(default)]
    group: String,
    #[serde(default)]
    names: CrdNames,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    versions: Vec<VersionHead>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct CrdNames {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    plural: String,
    #[serde(default)]
    singular: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionHead {
    #[serde(default)]
    name: String,
    #[serde(default)]
    served: bool,
    #[serde(default)]
    storage: bool,
    #[serde(default)]
    deprecated: bool,
    deprecation_warning: Option<String>,
    #[serde(default)]
    additional_printer_columns: Vec<PrinterColumnHead>,
    schema: Option<SchemaHead>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrinterColumnHead {
    #[serde(default)]
    name: String,
    #[serde(default, rename = "type")]
    column_type: String,
    format: Option<String>,
    #[serde(default)]
    json_path: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct SchemaHead {
    #[serde(rename = "openAPIV3Schema")]
    root: Option<SchemaRoot>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct SchemaRoot {
    properties: Option<RootProperties>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct RootProperties {
    spec: Option<SchemaLevel>,
    status: Option<SchemaLevel>,
}

/// One level of properties; the schemas under each property are never declared.
#[derive(Clone, Debug, Default, Deserialize)]
struct SchemaLevel {
    #[serde(default)]
    properties: BTreeMap<String, FieldHead>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct FieldHead {
    #[serde(rename = "type")]
    field_type: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct CrdStatusHead {
    #[serde(default)]
    conditions: Vec<ConditionHead>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ConditionHead {
    #[serde(default, rename = "type")]
    condition_type: String,
    #[serde(default)]
    status: String,
    reason: Option<String>,
    message: Option<String>,
}

fn crd_summary(object: &CrdObject) -> CrdSummary {
    let spec = &object.spec;
    let kind = spec.names.kind.clone();
    let singular = match spec.names.singular.as_str() {
        "" => kind.to_lowercase(),
        singular => singular.to_owned(),
    };
    CrdSummary {
        name: object.metadata.name.clone().unwrap_or_default(),
        group: spec.group.clone(),
        kind,
        plural: spec.names.plural.clone(),
        singular,
        scope: match spec.scope.as_str() {
            "Cluster" => ResourceScope::Cluster,
            _ => ResourceScope::Namespaced,
        },
        versions: spec.versions.iter().map(crd_version).collect(),
        state: crd_state(object),
        created_at: object
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
    }
}

fn crd_version(head: &VersionHead) -> CrdVersion {
    CrdVersion {
        name: head.name.clone(),
        is_served: head.served,
        is_storage: head.storage,
        is_deprecated: head.deprecated,
        deprecation_warning: head.deprecation_warning.clone(),
        printer_columns: head.additional_printer_columns.iter().map(column).collect(),
        schema: schema_outline(head.schema.as_ref()),
    }
}

fn column(head: &PrinterColumnHead) -> PrinterColumn {
    let column_type = match head.column_type.as_str() {
        "integer" => ColumnType::Integer,
        "number" => ColumnType::Number,
        "boolean" => ColumnType::Boolean,
        "date" => ColumnType::Date,
        _ => ColumnType::String,
    };
    let is_password = head.format.as_deref() == Some("password");
    PrinterColumn::new(&head.name, column_type, &head.json_path, is_password)
}

fn schema_outline(schema: Option<&SchemaHead>) -> SchemaOutline {
    let properties = schema
        .and_then(|schema| schema.root.as_ref())
        .and_then(|root| root.properties.as_ref());
    let Some(properties) = properties else {
        return SchemaOutline::default();
    };
    let (spec, omitted_spec) = outline_side(properties.spec.as_ref());
    let (status, omitted_status) = outline_side(properties.status.as_ref());
    SchemaOutline {
        spec,
        status,
        omitted: omitted_spec + omitted_status,
    }
}

/// The first `MAX_OUTLINE_FIELDS` fields in name order, and how many were left out.
fn outline_side(level: Option<&SchemaLevel>) -> (Vec<SchemaField>, usize) {
    let Some(level) = level else {
        return (Vec::new(), 0);
    };
    let fields = level
        .properties
        .iter()
        .take(MAX_OUTLINE_FIELDS)
        .map(|(name, field)| SchemaField {
            name: name.clone(),
            field_type: field.field_type.clone(),
        })
        .collect();
    (
        fields,
        level.properties.len().saturating_sub(MAX_OUTLINE_FIELDS),
    )
}

/// First match: terminating, names rejected, established, else not established yet.
fn crd_state(object: &CrdObject) -> CrdState {
    if object.metadata.deletion_timestamp.is_some() {
        return CrdState::Terminating;
    }
    let conditions = object
        .status
        .as_ref()
        .map_or(&[][..], |status| status.conditions.as_slice());
    let find = |name: &str| {
        conditions
            .iter()
            .find(|condition| condition.condition_type == name)
    };
    if let Some(names) = find("NamesAccepted")
        && names.status == "False"
    {
        return CrdState::NamesNotAccepted {
            message: names.message.clone().filter(|text| !text.is_empty()),
        };
    }
    let established = find("Established");
    if established.is_some_and(|condition| condition.status == "True") {
        return CrdState::Established;
    }
    CrdState::NotEstablished {
        reason: established
            .and_then(|condition| condition.reason.clone())
            .filter(|text| !text.is_empty()),
    }
}

#[cfg(test)]
#[path = "custom_resource_definition_tests.rs"]
mod custom_resource_definition_tests;
