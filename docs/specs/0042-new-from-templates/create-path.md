# 0042 · Cluster crate: draft and `CreateObject`

[Back to index](README.md) · Step 1 (no app caller) · Modules: `object_create.rs` (new) + `object_create_tests.rs`, `object_write.rs` + `object_write_create_tests.rs` (new), `object_yaml.rs`, `object_edit.rs`, `access_review.rs`, `lib.rs`. Decisions 1, 4–7, 9, 10, 12.

## API (`object_create.rs`)

```rust
impl ObjectKind {
    /// The kinds W7 gives a `New` button (decision 1).
    pub fn is_creatable(self) -> bool;   // Namespace | ConfigMap | ResourceQuota | PodDisruptionBudget | RoleBinding
}
/// A new object parsed from the editor text and checked. Manual Debug: kind, namespace, name.
#[derive(Clone, PartialEq, Eq)]
pub struct ObjectDraft { target: ObjectRef, body: Value, warnings: Vec<DraftWarning> }
impl ObjectDraft {
    pub fn new(kind: ObjectKind, text: &str) -> Result<Self, DraftError>;
    pub fn target(&self) -> &ObjectRef;
    pub fn warnings(&self) -> &[DraftWarning];
    pub(crate) fn body(&self) -> &Value;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DraftWarning { PowerfulRole { role: String }, BroadSubject { kind: String, name: String }, PrivilegedPodSecurity }
impl DraftWarning { pub fn needs_typed_name(&self) -> bool; }   // PowerfulRole, BroadSubject (decision 15)
/// Leaf paths of `draft` absent from `answer` (decision 14). Pure; paths only.
pub(crate) fn missing_paths(draft: &Value, answer: &Value) -> Vec<FieldPath>;
#[derive(Debug, thiserror::Error)]
pub enum DraftError {
    #[error(transparent)] Text(#[from] EditError),   // Syntax, NotAnObject, LeadingZero, TooLarge (shared parse)
    #[error("{0} cannot be created here")] NotCreatable(&'static str),
    #[error("kind must be {expected}")] WrongKind { expected: &'static str },
    #[error("apiVersion must be {expected}")] WrongApiVersion { expected: String },
    #[error("metadata.name is required")] MissingName,
    #[error("metadata.generateName is not supported; write a name")] GenerateName,
    #[error("metadata.name is not a valid {kind} name")] InvalidName { kind: &'static str },
    #[error("metadata.namespace is required")] MissingNamespace,
    #[error("a {kind} has no namespace")] UnexpectedNamespace { kind: &'static str },
    #[error("{field} is set by the server; remove it")] ServerField { field: &'static str },
    #[error("{path} holds <hidden>; write a value")] Placeholder { path: String },
}
```

## `ObjectDraft::new` (pure, in order)

1. `kind.is_creatable()` → else `NotCreatable`.
2. `parse_mapping(text)` (from `object_edit.rs`, made `pub(crate)`): size limit, leading-zero refusal, syntax, a mapping. A text with more than one YAML document (a `---` line followed by content) → `Text(NotAnObject)`; test `draft_refuses_multi_document_yaml`.
3. `kind` equals `kind.name()`; `apiVersion` equals `api_resource(kind).api_version` (`v1`, `policy/v1`, `rbac.authorization.k8s.io/v1`).
4. `status` present → `ServerField { "status" }`; any of 0031's `SERVER_METADATA` (made `pub(crate)`) under `metadata` → `ServerField`; `ownerReferences` → `ServerField` too (a new object owned by another is out of scope).
5. `metadata.generateName` → `GenerateName`; `metadata.name` a non-empty string → else `MissingName`; name rule (decision 9) → else `InvalidName`.
6. Namespace: namespaced kind → `metadata.namespace` a DNS label, else `MissingNamespace`; Namespace kind with a namespace → `UnexpectedNamespace`.
7. Any string value equal to `HIDDEN` or a diff-marker text (`edit_placeholders` constants) → `Placeholder { path }` (first in key order).
8. `target = ObjectRef::new(kind, namespace, name)`; warnings (decision 7): RoleBinding `roleRef` ClusterRole `cluster-admin` / `admin` / `edit` → `PowerfulRole`; each Group or User subject named `system:*` → `BroadSubject`; Namespace label `pod-security.kubernetes.io/enforce: privileged` → `PrivilegedPodSecurity`.

## Operation (`object_write.rs`)

```rust
pub enum WriteOperation { /* … */
    /// `POST` of a new object of a creatable kind (0042). The draft holds user text: nothing prints it.
    CreateObject(Box<ObjectDraft>),
}
```

| Item | Rule |
|---|---|
| `name()` | `"CreateObject"` |
| `checked_operation` | an explicit arm: `CreateObject(draft)` kept only when `draft.is_consistent()`: kind creatable, body `kind` / `apiVersion` / `metadata.name` / `metadata.namespace` equal the target, no `status`, no server metadata, no `generateName`. Else `None` |
| `fitting_access_check` | `(CreateObject(draft), kind) if draft.target() == target && kind.is_creatable()` → `AccessCheck::Create(kind)` |
| `is_safe_path` | `CreateObject`: the namespace (it is in the path) a DNS label; the name by decision 9 (Namespace label, RoleBinding path segment, else subdomain) |
| `changed_fields` | `metadata.name` = name; `metadata.namespace` = namespace (namespaced kinds); RoleBinding `roleRef` = `{kind}/{name}` and one `subjects[{i}]` field per subject = `{Kind} {ns/}{name}`; ConfigMap one `data[KEY]` and one `binaryData[KEY]` path per key, value `None`; ResourceQuota one `spec.hard[RESOURCE]` per item with its quantity; PDB `spec.minAvailable` / `spec.maxUnavailable` with the value. Per-item lists capped at 10, then one `… and {n} more` field (decision 16) |
| `supports_dry_run` | true |
| Manual `Debug` | `CreateObject` / `WriteRequest { operation: CreateObject, kind: ConfigMap, namespace: Some("payments"), name: "new-config" }` |

## `send` arm

```rust
WriteOperation::CreateObject(draft) => {
    let body: DynamicObject = serde_json::from_value(draft.body().clone()).map_err(|_| self.unusable_object(mode))?;
    let sent = run_raw(api.create(&post_params(mode), &body)).await;   // api = object_api(target): the collection
    let created = self.settle(request, mode, sent)?;
    let answer = serde_json::to_value(&created).map_err(|_| self.unusable_object(mode))?;
    let dropped = missing_paths(draft.body(), &answer);   // decision 14; server-added fields do not matter
    Ok(Answer::created(&created, mode).with_dropped(dropped))
}
```

The response object is dropped after `created_name`, `uid`, and the missing paths are read (a ConfigMap answer holds its data). `WriteOutcome` gains `pub dropped_fields: Vec<String>` (path texts; empty for every other operation): `Answer` carries it, `write` copies it. Existing `WriteOutcome` literals in tests add `dropped_fields: Vec::new()`.

## Errors

| Status | Variant | Note |
|---|---|---|
| 409 (`reason: AlreadyExists`) | `Invalid { message: "{Kind} {name} already exists", fields: ["metadata.name"] }` | new arm in `write_error` for `CreateObject` before `error_from_status` (decision 6) |
| 422, other 403 | `Invalid` (merged mapping; `fields` verbatim) | |
| 403 RBAC form | `Denied` | |
| 404 (namespace missing), dry-run **or commit** | `Invalid { message: "The namespace {ns} does not exist", fields: ["metadata.namespace"] }` | the same new `write_error` arm (decision 6); never `NotFound` |
| 400 webhook dry-run | `DryRunRejected` (blocks) | |
| commit timeout / transport | `OutcomeUnknown` | |

## Access check

`AccessCheck::Create(ObjectKind)` → `("create", group, plural, None, kind.is_namespaced())`, shown `create configmaps`; lazy, **not** in `ALL` (like `Update`, `Delete`, `Patch`).

## Allow-list row (added to the 0030 table by step 1)

| Operation | HTTP | Path | Body | Dry-run | Spec |
|---|---|---|---|---|---|
| `CreateObject` | POST `application/json` | `{collection path}?dryRun=All&fieldManager=k8sboard` (commit: `?fieldManager=k8sboard`) | the draft: the five kinds only, no `status` or server metadata | yes (query) | 0042 |

No new clippy exception: `Api::create` sits in the excepted `send` match.
