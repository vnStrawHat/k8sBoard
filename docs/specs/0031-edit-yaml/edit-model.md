# 0031 · Cluster crate: the edit model

[Back to index](README.md) · Steps 0–1 (rebase: step 4; `format_yaml`: step 3) · Modules: `object_yaml.rs`, `object_edit.rs` (new) + `object_edit_tests.rs`, `edit_placeholders.rs` (new) + `edit_placeholders_tests.rs`, `access_review.rs`, `lib.rs`; app `cluster_session.rs`. Decisions 5–8, 12–13, 17, 21, 24–25.

## Step 0: `object_yaml.rs` refactor (no behavior change)

```rust
pub(crate) fn mask_object(object: &mut Value, env: EnvValues) -> MaskCount;       // today's rules
pub(crate) fn to_yaml_text(object: &Value, header: Option<&str>) -> Result<String, &'static str>; // sort + serialize
impl ClusterConnection {
    pub(crate) async fn get_object(&self, object: &ObjectRef, action: &'static str) -> Result<Value, ClusterError>;
}
```

`to_masked_yaml` = `mask_object` + `to_yaml_text` with the 0007 hid-count header. Golden test `masked_yaml_output_is_unchanged`: every 0007 fixture's text is byte-equal to a copy captured before the split (committed with step 0). `ObjectKind::{ALL, resource}` come from 0030 (0032 architect's amendment).

## Step 1 API (`object_edit.rs`)

```rust
/// The object as the editor opened it, stripped and masked. Manual Debug: kind, name, rv.
pub struct EditBase { target: ObjectRef, resource_version: String, uid: String, env: EnvValues, masked: Value, text: String }
impl EditBase { pub fn target(&self) -> &ObjectRef; pub fn resource_version(&self) -> &str; pub fn env(&self) -> EnvValues;
    pub fn text(&self) -> &str; pub fn is_secret(&self) -> bool; pub fn has_last_applied(&self) -> bool; }
impl ClusterConnection {
    /// One GET (`get_object`), strip (decision 12), mask, `to_yaml_text` with the edit header. action: "reading the object to edit".
    pub async fn edit_base(&self, object: &ObjectRef, env: EnvValues) -> Result<EditBase, ClusterError>;
}
/// A parsed edit checked against its base. Manual Debug: kind, name, change count.
#[derive(Clone, PartialEq, Eq)]
pub struct ObjectEdit { target: ObjectRef, base_resource_version: String, base_uid: String, env: EnvValues,
    edited: Value, changed: Vec<FieldPath>, leading_zero_lines: Vec<usize> }
impl ObjectEdit {
    pub fn new(base: &EditBase, text: &str) -> Result<Self, EditError>;
    pub fn target(&self) -> &ObjectRef;
    pub fn changed_paths(&self) -> &[FieldPath];        // local intent: dialog and audit (FieldPath: edit-preview.md)
    pub fn leading_zero_lines(&self) -> &[usize];       // decision 18 warning, 1-based
}
#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("YAML error at line {line}, column {column}: {message}")] Syntax { line: u64, column: u64, message: String },
    #[error("the document must be a single YAML mapping")] NotAnObject,
    #[error("{field} cannot change in an edit")] IdentityChanged { field: &'static str },
    #[error("Secret values cannot be edited here")] SecretValuesChanged,
    #[error("{path} is <hidden> but has no value on the server; replace it or remove it")] UnmatchedPlaceholder { path: String },
    #[error("nothing changed")] NoChanges,
}
pub fn format_yaml(text: &str) -> Result<String, EditError>;   // step 3: parse, to_yaml_text, keep the header
```

- Header: `# Values shown as <hidden> keep their value on the server. Replace one to set a new value.`; Secrets add `# Secret data and stringData cannot be edited here.` A YAML comment, so the parser drops it.
- `Syntax.message` comes from `serde_saphyr::Error::location()` (verified: `de/error.rs:1536`). It can only quote the user's text. Parsing needs the `deserialize` feature (files-to-touch).

## `ObjectEdit::new` (pure, in order)

1. `serde_saphyr::from_str::<Value>(text)` → `Syntax`; the value must be an object → else `NotAnObject`.
2. Strip the decision-12 fields.
3. Identity (`apiVersion`, `kind`, `metadata.name`, `metadata.namespace`) equals the base → else `IdentityChanged`.
4. Secret lock: `data` and `stringData` equal the base's (both absent counts as equal) → else `SecretValuesChanged`.
5. `edit_placeholders::check(&edited, &base.masked)` → the first unmatched `<hidden>` in key order → `UnmatchedPlaceholder`.
6. `changed = field_paths(&base.masked, &edited)`; empty → `NoChanges`. `leading_zero_lines` = text lines whose value after `: ` or `- ` is an unquoted `0[0-9]+`.

## `edit_placeholders.rs` (the location rule; decisions 6–7)

```rust
pub(crate) const HIDDEN: &str = "<hidden>";   // moved from object_yaml.rs
/// Counterpart of `item` in `list`: by `name` when every item of BOTH lists is an object with a
/// unique string `name`; otherwise by index.
fn counterpart<'a>(list: &'a [Value], item: &Value, index: usize, other: &[Value]) -> Option<&'a Value>;
pub(crate) fn check(edited: &Value, base: &Value) -> Result<(), FieldPath>;               // step 1
pub(crate) struct Restored { pub(crate) moved: Vec<FieldPath> }
pub(crate) fn restore(edited: &mut Value, fresh: &Value) -> Result<Restored, FieldPath>;  // step 2
```

- `restore` replaces each `<hidden>` with the fresh counterpart (raw), so the server's real value comes back.
- **`moved`**: a placeholder reached through an index-matched list item whose non-placeholder fields differ from the counterpart item. The preview renders it `<hidden, moved>` (edit-preview.md).
- Invariant (decision 7): after `check` passes, `restore` against a fresh object with the base `resourceVersion` cannot fail. Every 0007 mask rule is covered by tests, including duplicate env names.

## Rebase (step 4; decision 21)

```rust
pub struct Rebased { pub text: String, pub unreachable: Vec<FieldPath>, pub server_changed: Vec<FieldPath> }
pub fn rebase(old: &EditBase, text: &str, new: &EditBase) -> Result<Rebased, EditError>; // Syntax / NotAnObject only
```

1. `edited = parse(text)` (stripped). `paths = field_paths(old.masked, edited)`; `server_changed = field_paths(old.masked, new.masked)`.
2. `result = new.masked.clone()`. For each path: present in `edited` → set it in `result` (named items are resolved by name in `result`); absent → remove it from `result`. A path whose parent does not resolve in `result` → `unreachable`.
3. `text = to_yaml_text(result, edit header)`. A `<hidden>` copied from `edited` keeps meaning "the new server value".

## Lazy write checks (decision 24)

- `AccessCheck::Update(ObjectKind)` → `("update", resource().group, resource().plural, None, kind.is_namespaced())`, shown `update deployments`. **Not** in `ALL`.
- Crate: `ClusterConnection::review_checks(&self, checks: &[AccessCheck], scope: NamespaceScope) -> Result<AccessReport, ClusterError>`. This is today's `review_access` body over a given list; `review_access` calls it with `ALL`.
- Session (app, `cluster_session.rs`): `kind_access: HashMap<ObjectKind, KindAccess>` with `KindAccess = Checking | Known(AccessReport) | Unknown`. `request_kind_access(kind)` runs once per kind and scope when a screen of that kind is first shown (or when a menu, key, or palette entry needs it). It reviews `Update(kind)` for editable kinds; 0033 appends `Delete(kind)` to the same request. A scope change clears the map. `AccessState` lookup for a lazy check reads this map; `Checking` → `Checking permissions…`.
