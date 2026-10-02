# 0018 · Cluster crate API

[Back to index](README.md) · Steps 1a (CRDs, `column_path.rs`, CRD access check, `ObjectKind`, `is_secret_key`, probe CRD lines) and 1b (custom objects, masking, SSAR, counts, watch helpers, probe object lines); namespace fields in step 5 · Modules: `custom_resource_definition.rs` (new) + `custom_resource_definition_tests.rs`, `custom_object.rs` (new) + `custom_object_tests.rs`, `column_path.rs` (new, [column-path.md](column-path.md)), `object_yaml.rs`, `access_review.rs`, `resource_watch.rs`, `lib.rs`, `examples/probe.rs`. No new dependency.

## CRDs (`custom_resource_definition.rs`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub enum ResourceScope { Namespaced, Cluster }
/// One served version of a custom resource. Built from a `CrdSummary`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CustomResourceType { pub group: String, pub version: String, pub kind: String, pub plural: String, pub scope: ResourceScope }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrdSummary {
    pub name: String,          // certificates.cert-manager.io
    pub group: String, pub kind: String, pub plural: String,
    pub singular: String,      // names.singular, else lowercase kind
    pub scope: ResourceScope,
    pub versions: Vec<CrdVersion>,   // spec order
    pub state: CrdState,
    pub created_at: Option<jiff::Timestamp>,
}
pub enum CrdState { Established, NamesNotAccepted { message: Option<String> }, NotEstablished { reason: Option<String> }, Terminating }
pub struct CrdVersion { pub name: String, pub is_served: bool, pub is_storage: bool, pub is_deprecated: bool,
    pub deprecation_warning: Option<String>, pub printer_columns: Vec<PrinterColumn>, pub schema: SchemaOutline }
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PrinterColumn { pub name: String, pub column_type: ColumnType, pub json_path: String,
    /// `format: password` (decision 36): every value reads `Hidden`.
    pub is_password: bool,
    /// `ColumnPath::parse` accepted the path.
    pub is_supported: bool,
    /// The path reads a condition's status, so the app tones the cell.
    pub is_condition_status: bool }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub enum ColumnType { String, Integer, Number, Boolean, Date } // other text → String
pub struct SchemaOutline { pub spec: Vec<SchemaField>, pub status: Vec<SchemaField>, pub omitted: usize } // empty without a schema
pub struct SchemaField { pub name: String, pub field_type: Option<String> }             // name order
impl PrinterColumn {
    /// Parses `json_path` once to set `is_supported` and `is_condition_status`; the app builds
    /// `BUILT_IN_COLUMNS` with it.
    pub fn new(name: &str, column_type: ColumnType, json_path: &str, is_password: bool) -> Self;
}
impl CrdSummary {
    pub fn preferred_version(&self) -> Option<&CrdVersion>;   // served, highest Version::priority
    pub fn resource(&self, version: &CrdVersion) -> CustomResourceType;
}
impl ClusterConnection { pub fn watch_crds(&self) -> impl Stream<Item = WatchUpdate<CrdSummary>> + Send + 'static; }
```

- `CrdState` (first match): `metadata.deletionTimestamp` set → `Terminating`; `NamesAccepted=False` → `NamesNotAccepted`; `Established=True` → `Established`; else `NotEstablished` with the Established reason.
- Lean decoding (decision 2): private `type CrdObject = kube::core::Object<CrdSpecHead, CrdStatusHead>` over `ApiResource { group: "apiextensions.k8s.io", version: "v1", kind: "CustomResourceDefinition", plural: "customresourcedefinitions" }`. Serde structs, camelCase, unknown fields ignored: `names { kind, plural, singular }`, `scope`, `versions[] { name, served, storage, deprecated, deprecationWarning, additionalPrinterColumns[] { name, type, format, jsonPath }, schema.openAPIV3Schema.properties.{spec,status} { type, properties: map name → { type } } }`, status `conditions[] { type, status, reason, message }`. Deeper schema levels are never declared, so serde skips them. `priority` is not read (decision 14).
- Schema outline: at most 40 fields per side; the rest of both sides is counted in `omitted`.
- Watch: `selected_summary_watch` (0012), `Api::all_with`, config `MostRecent` + `page_size(20)`, action `watching custom resource definitions`, ordered by name.

## Custom objects (`custom_object.rs`)

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomObjectSummary { pub namespace: Option<String>, pub name: String, pub created_at: Option<jiff::Timestamp>,
    pub labels: Vec<String>,             // key=value, key order
    pub columns: Vec<ColumnValue>,       // exactly one per printer column passed to the watch
    pub conditions: Vec<ObjectCondition>, // status.conditions, ≤ 10, API order
    pub phase: Option<String> }          // status.phase when a string
pub enum ColumnValue { Absent, Hidden, Text(String), Integer(i64), Number(String), Boolean(bool), Date(jiff::Timestamp) }
pub struct ObjectCondition { pub name: String, pub status: ConditionStatus, pub reason: Option<String>,
    pub message: Option<String> /* ≤ 300 chars */, pub changed_at: Option<jiff::Timestamp> }
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CustomObjectFields { pub spec: FieldList, pub status: FieldList }
pub struct FieldList { pub entries: Vec<FieldEntry>, pub omitted: usize }
pub struct FieldEntry { pub path: String /* issuerRef.name */, pub value: FieldValue }
pub enum FieldValue { Text(String), Hidden, Items(usize), Fields(usize) }
impl ClusterConnection {
    pub fn watch_custom_objects(&self, resource: &CustomResourceType, columns: &[PrinterColumn], scope: NamespaceScope)
        -> impl Stream<Item = WatchUpdate<CustomObjectSummary>> + Send + 'static;
    /// Snapshots of 0 or 1 item.
    pub fn watch_custom_object_fields(&self, object: &ObjectRef)
        -> impl Stream<Item = WatchUpdate<CustomObjectFields>> + Send + 'static;
    /// Cluster-wide, whatever the explorer scope (decision 31).
    pub async fn count_custom_objects(&self, resource: &CustomResourceType) -> Result<Option<u64>, ClusterError>;
}
```

- APIs: `scoped_dynamic_apis(&scope, &api_resource)` for namespaced resources; one `Api::all_with` for cluster-scoped ones (scope ignored). `custom_api_resource(&CustomResourceType) -> ApiResource` is one `pub(crate)` helper shared with `object_yaml.rs`.
- Objects watch: config `MostRecent` + `page_size(50)`; columns compiled once (`ColumnPath::parse`), moved into the summarizer closure (decision 10). Fixed action `watching custom resources` (`ClusterError.action` is `&'static str`, so the text cannot name the resource; the app adds that context).
- Fields watch: `watch_custom_object_fields(&self, object: &ObjectRef)` reads the resource, namespace, and name from a custom `ObjectRef` (which already enforces the scope; a built-in ref yields one empty snapshot), fixed action `watching a custom resource`; `Api` namespaced or all, config `.fields("metadata.name={name}")`, same page size; summarizer `object_fields` ([custom-object-safety.md](custom-object-safety.md) rules apply to every value).
- `object_fields` flattening: walk `spec` and `status` (skip `status.conditions`); key order; path depth ≤ 3; scalar → `Text` (strings as is, numbers and booleans as JSON text, `null` skipped); array of scalars → `Text` of the first 5 joined `, ` plus ` +{n}`; array with an object → `Items(len)`; object at depth 3 → `Fields(len)`; empty object or array skipped; text cut at 200 chars with `…`; at most 60 entries per side, the rest counted in `omitted`.
- Counts: 0012's private count params (`limit=1`, no resource version), `list_metadata` on one `Api::all_with` (one request per CRD); fixed action `counting custom resources` (same reason).

## Access (`access_review.rs`)

- `AccessCheck::ListCustomResourceDefinitions`: `("list", "apiextensions.k8s.io", "customresourcedefinitions", None, false)`; appended to `ALL`.
- `pub async fn review_custom_access(&self, resource: &CustomResourceType, scope: &NamespaceScope) -> Result<AccessDecision, ClusterError>`: verb `list` only (decision 21, like the built-in kinds); one namespace-less review for `All` or a cluster-scoped resource, else one per picked namespace; concurrent; the first denial wins. The private review builder takes `&str` fields so both paths share it.

## YAML (`object_yaml.rs`)

- `ObjectKind::CustomResourceDefinition` (cluster-scoped, `api_resource` = `ApiResource::erase::<CustomResourceDefinition>(&())`).
- `ObjectRef` holds a private `enum ObjectTarget { Builtin(ObjectKind), Custom(CustomResourceType) }`; `pub fn custom(resource: CustomResourceType, namespace: Option<String>, name: String) -> Option<ObjectRef>` (`None` when the namespace does not fit the scope). `object_yaml` uses the shared `api_resource` for custom targets and adds the custom masking pass.
- 0014's `is_secret_parameter` moves here (step 1a) as `pub(crate) fn is_secret_key(key: &str) -> bool`, **widened** ([custom-object-safety.md](custom-object-safety.md) "Secret-key matcher"); `storage_class.rs` calls it and keeps its results.

## Watch helpers (`resource_watch.rs`)

`summary_watch`, `limited_summary_watch`, `selected_summary_watch`, `watch_apis`, `batch_updates`, `Batcher`: `summarize: F` with `F: Fn(&K) -> T + Clone + Send + 'static` (one clone per scoped API). No behavior change.

## `lib.rs`

`pub use custom_resource_definition::{ColumnType, CrdState, CrdSummary, CrdVersion, CustomResourceType, PrinterColumn, ResourceScope, SchemaField, SchemaOutline};` and `pub use custom_object::{ColumnValue, CustomObjectFields, CustomObjectSummary, FieldEntry, FieldList, FieldValue, ObjectCondition};`. `ColumnPath` stays private.

## Probe (`examples/probe.rs`, `--crds`)

Step 1a, after the access section: `crds: {n} ({e} established)` or the error; per established CRD (first 10, name order): `crd {name} {version} {scope} columns {k} unsupported {u}` plus `unsupported {crd}: {column name}` lines. Step 1b, for the first established CRD: `access list {plural}.{group}: allowed|denied`, `count {plural}.{group} {n|unknown}`, then with `--watch-seconds` `custom {plural}.{group}: {n} objects, {f} of {c} column values filled`, and with `--yaml` `yaml custom {name}: {lines} lines, masked {yes|no}` (header present). Never prints values. Update `USAGE`.
