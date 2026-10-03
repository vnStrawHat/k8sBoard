# 0047 · Cluster API (step 1)

[Back to index](README.md) · New module `crates/cluster/src/config_values.rs` (+ `config_values_tests.rs`); changes in `object_write.rs`, `access_review.rs`, `lib.rs`. Decisions 1–7, 12, 13, 16. Nothing here logs.

## Types

```rust
/// What the editor opens: key metadata, flags, and ConfigMap text. Private fields; built only by
/// `values_base`. Manual Debug: kind, namespace, name, resourceVersion, key count.
pub struct ValuesBase { target: ObjectRef, resource_version: String, keys: Vec<ValueKey>, notes: BaseNotes }
/// No Debug (a ConfigMap text can be sensitive). Sorted by name.
#[derive(Clone)]
pub struct ValueKey { pub name: String, pub field: DataField, pub size_bytes: usize, pub content: KeyContent }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum DataField { Data, BinaryData }
#[derive(Clone)] pub enum KeyContent {
    Text(String),   // ConfigMap only, ≤ MAX_INLINE_VALUE (128 KiB)
    Hidden,         // Secret text: never loaded, never shown (decision 3)
    Binary,         // non-UTF-8 Secret value or ConfigMap binaryData
    TooLarge,       // ConfigMap text over the inline cap
}
#[derive(Clone, Debug, Default)] pub struct BaseNotes { pub is_helm_managed: bool, pub owner: Option<String> /* "Kind/name" */ }

/// Plaintext of one new value. Clone + PartialEq + Eq (WriteOperation derives them); no Debug,
/// Display, or Serialize.
#[derive(Clone, PartialEq, Eq)] pub struct NewValue(Zeroizing<String>);
/// Manual Debug: variant and key only.
#[derive(Clone, PartialEq, Eq)] pub enum KeyChange {
    Add { key: String, value: NewValue },
    Set { key: String, value: NewValue },
    Remove { key: String },
}
/// A checked edit. Manual Debug: kind, name, resourceVersion, change counts.
#[derive(Clone, PartialEq, Eq)]
pub struct ValuesEdit { target: ObjectRef, resource_version: String, changes: Vec<FieldChange> /* sorted by key */ }
/// Derived Debug is safe: it prints `field` and KeyChange's manual Debug (variant and key).
#[derive(Clone, Debug, PartialEq, Eq)] pub struct FieldChange { pub field: DataField, pub change: KeyChange }
```

## Functions

```rust
impl ClusterConnection {
    /// One GET. Refuses (decision 4) before anything is kept; a Secret's data is zeroized before return.
    pub async fn values_base(&self, target: &ObjectRef) -> Result<ValuesBase, ValuesBaseError>;
}
impl ValuesBase {
    pub fn target(&self) -> &ObjectRef;  pub fn resource_version(&self) -> &str;
    pub fn keys(&self) -> &[ValueKey];   pub fn notes(&self) -> &BaseNotes;  pub fn is_secret(&self) -> bool;
    /// Local checks of AC 18; the error names the first offending key.
    pub fn edit(&self, changes: Vec<KeyChange>) -> Result<ValuesEdit, ValuesEditError>;
}
impl ValuesEdit { pub fn target(&self) -> &ObjectRef; pub fn changes(&self) -> &[FieldChange]; }
pub(crate) fn values_patch(edit: &ValuesEdit) -> serde_json::Value;   // pure; the request body
```

| Error | Variants (Display is fixed text plus at most a key name) |
|---|---|
| `ValuesBaseError` | `HelmRelease` (type or `owner=helm` label), `ServiceAccountToken`, `Immutable`, `NotEditable` (another kind), `Cluster(ClusterError)` |
| `ValuesEditError` | `NoChange`, `InvalidKey(String)`, `DuplicateKey(String)`, `UnknownKey(String)`, `BinaryValue(String)` (a `Set` on a binary key), `TooLarge(String)` (a value > 1 MiB), `ObjectTooLarge` (estimated object > 1 MiB, decision 13) |

## `values_base` by kind

| Kind | Read | Kept | Refused |
|---|---|---|---|
| Secret | `secret_text(ns, name, ACTION)` (0016: `Zeroizing` body, decoded in-crate, kube never decodes it) → `Secret` | `secret_summary` keys (name, size, `is_binary`), `resourceVersion`, the Helm label, first owner | `type` `helm.sh/release.v1` (reuse `object_edit::HELM_RELEASE_TYPE`), label `owner=helm`, `kubernetes.io/service-account-token`, `immutable: true` |
| ConfigMap | `get_object(target, ACTION)` → typed `ConfigMap` | `data` (Text or TooLarge), `binaryData` (Binary), `resourceVersion`, Helm label, first owner | label `owner=helm` (a Helm release record of the ConfigMap storage driver), `immutable: true` |

After the Secret is summarized, every `data` value and every annotation is zeroized (the `values_of` pattern), then dropped. Decode errors are fixed text (0016 `unexpected`).

## `ValuesBase::edit` rules

- At least one change; at most one change per key. Keys follow the ConfigMap key rule: `[-._a-zA-Z0-9]+`, ≤ 253 bytes, not `.` or `..`.
- `Add`: key absent from both fields; one key per change. `Set`: key exists, is `Text`/`Hidden`, not `Binary`/`TooLarge`. `Remove`: key exists.
- A value ≤ 1 MiB (`TooLarge`); the estimated object ≤ 1 MiB (`ObjectTooLarge`, decision 13). A ConfigMap `Set` equal to the base text is dropped by the app before `edit`; a Secret `Set` is always a change (decision 16).
- Each change is paired with its field: an `Add` goes to `Data`; a `Set`/`Remove` keeps the key's field.

## Write operation (`object_write.rs`)

```rust
/// Merge patch of the changed keys of a ConfigMap or Secret, guarded by the base resourceVersion (0047).
SetDataValues(Box<ValuesEdit>),
```

| Item | Rule |
|---|---|
| `WriteRequest::new` | `None` unless `edit.target() == &target` and the kind is ConfigMap or Secret; name rule `is_dns_subdomain` |
| `fitting_access_check` | `AccessCheck::Patch(kind)` (new, lazy, not in `ALL`; verb `patch`, resource of the kind) |
| `checked_operation` | listed with the pass-through arm |
| `changed_fields()` | one per change, sorted by key: `{field}[{key}] added` / `value changed` / `removed`, `value: None` (decision 6) |
| `supports_dry_run()` | true |
| `send` arm | `api.patch(name, &patch_params(mode), &Patch::Merge(&values_patch(edit)))`, `settle`, `Answer::patched()`; the response object is dropped at once |
| `Debug` | `SetDataValues` (name only, as today) |

## Body

```json
{"metadata": {"resourceVersion": "<base>"},
 "data": {"NEW_KEY": "<value or base64>", "OLD_KEY": null},
 "binaryData": {"BLOB": null}}
```

- Secret: text is `base64::engine::general_purpose::STANDARD`; no `stringData`, no `binaryData`. ConfigMap: text as typed.
- A field with no change is absent. Nothing else is in the body.

## Allow-list row (0030 table)

| Operation | HTTP | Path and query | Body | Dry-run | Spec |
|---|---|---|---|---|---|
| `SetDataValues` | PATCH `application/merge-patch+json` | `{path}/{name}?dryRun=All&fieldManager=k8sboard` (commit: `?fieldManager=k8sboard`) | base `resourceVersion` + changed keys | yes (query) | 0047 |

No new clippy exception: the arm sits in the excepted `send` match; `secret_text` and `get_object` are reads.

## Errors (existing variants)

409 → `Conflict` (banner, decision 12); 422 → `Invalid` (paths verbatim; Secret text redacted by `redact_error`); 403 → `Denied`/`Invalid`; 404 → `NotFound`; 429 → `TooManyRequests`; webhook dry-run 400 → `DryRunRejected`; commit timeout → `OutcomeUnknown`.
