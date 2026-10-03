# 0033 · Cluster crate: delete

[Back to index](README.md) · Step 1 · Modules: `object_write.rs` (+ `object_write_tests.rs`), `object_yaml.rs` (+ tests), `access_review.rs`, `lib.rs`. Decisions 1–8, 16, 25, 28. Builds on the merged 0030 `object_write.rs` ([write-path.md](../0030-guardrails-write-path/write-path.md)) and on 0031 steps 0–1 (`get_object`, `ObjectKind::{ALL, resource}`, `review_access_for`, `AccessReport::all_of` over the reviewed list).

## Verified APIs (kube 4.2)

| API | Where |
|---|---|
| `Api::delete(name, &DeleteParams) -> Result<Either<K, Status>>` ("via `Left` your delete has started") | `kube-client-4.2.0/src/api/core_methods.rs:291–313` |
| `Request::delete`: `DELETE {url_path}/{name}`, `DeleteParams` serialized as the JSON body, `Content-Type: application/json` | `kube-core-4.2.0/src/request.rs:109` |
| `DeleteParams { dry_run (→ "dryRun":["All"]), grace_period_seconds, propagation_policy, preconditions }` | `kube-core-4.2.0/src/params.rs:763–790, 861` |
| `PropagationPolicy::{Orphan, Background, Foreground}`; `Preconditions { resource_version, uid }` (camelCase) | `params.rs:948–969` |
| `Api::get_metadata(name) -> Result<PartialObjectMeta<K>>` (Accept: PartialObjectMetadata) | `core_methods.rs:56`, `request.rs:266` |

## API

```rust
pub enum WriteOperation {
    // 0030, 0031, 0032 variants …
    /// DELETE with a uid precondition and an explicit propagation policy.
    DeleteObject { uid: String, propagation: DeletePropagation },
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeletePropagation { #[default] Background, Foreground, Orphan }
impl DeletePropagation { pub fn as_str(self) -> &'static str; } // "Background" …; audit value; test: equals the body string
pub enum WriteEffect { /* 0030, 0031 … */ Deleted, DeletionPending { finalizers: Vec<String> } } // 0033 variants of the 0030 enum

/// Read when a delete starts: the uid the precondition pins, and why the delete may wait.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectIdentity { pub uid: String, pub finalizers: Vec<String>, pub deletion_started: Option<jiff::Timestamp> }
impl ClusterConnection {
    /// One metadata GET. action: "reading the object before deleting it". 404 → `ClusterError::Api { code: 404 }`.
    pub async fn object_identity(&self, object: &ObjectRef) -> Result<ObjectIdentity, ClusterError>;
}
impl ObjectKind { pub fn owns_dependents(self) -> bool; } // Deployment, StatefulSet, DaemonSet, ReplicaSet, Job, CronJob
```

- `WriteEffect` (`Patched` on main; `Created` from 0032, `Replaced` from 0031) and `WriteOutcome.effect` are merged; 0033 adds only its two variants through the per-operation effect seam (0031/0032 step 1).
- **Delete without a GET is never sent** (decision 3): the app always calls `object_identity` first, and the request needs its uid. The precondition is `uid` only; there is no `resourceVersion` precondition (0030 decision 14).
- `object_identity` lives in `object_yaml.rs` next to `get_object` (single-object reads). A missing `uid` → `UnexpectedResponse` with a fixed text. Annotations and labels of the metadata are dropped unread.

## `WriteRequest` rules

| Item | Rule |
|---|---|
| `new(target, DeleteObject { uid, .. })` | `None` when `uid` is empty, when the target is a custom resource (`builtin_kind()` is `None`), or when the name fails the merged DNS rule (0031 decision 27 for the RBAC kinds); otherwise any `ObjectKind` (decision 16) |
| `access_check()` | `AccessCheck::Delete(kind)` (`kind` = `target.builtin_kind()`, always `Some` after `new`) |
| `changed_fields()` | `[ChangedField { path: "deleteOptions.propagationPolicy", value: Some(propagation.as_str()) }]`. The value is recordable: it is not object data |
| `supports_dry_run()` | true |
| Manual `Debug` | `DeleteObject` / `WriteRequest { operation: DeleteObject, kind: Pod, namespace: Some("payments"), name: "api-x" }`; no uid |

## Sequence inside `write`

1. Policy check (0030): `Blocked` → `WritesBlocked`, zero requests.
2. `DeleteParams { dry_run: mode == DryRun, grace_period_seconds: None, propagation_policy: Some(map(propagation)), preconditions: Some(Preconditions { uid: Some(uid), resource_version: None }) }`.
3. `Api::<DynamicObject>::delete(name, &params)` as a new arm of the excepted `send` match, inside `run_raw` (no new allow).
4. `Left(object)`: `metadata.deletion_timestamp` is set → `DeletionPending { finalizers: metadata.finalizers }`, else `Deleted`. `Right(_)` → `Deleted`. The object (for a Secret, with its data) is dropped at once; only finalizer names are kept.

## Allow-list row (added to the 0030 table)

| Operation | HTTP | Path | Body | Dry-run | Spec |
|---|---|---|---|---|---|
| `DeleteObject` | DELETE `application/json` | `{api_resource path}/{name}` (no query) | `{"propagationPolicy":P,"preconditions":{"uid":U}}` (+ `"dryRun":["All"]`) | yes (body) | 0033 |

No new clippy exception (`Api::delete` is in the excepted `match`; `get_metadata` is a read).

## Errors (0030 variants; no new variant)

| Status | Variant | App text (delete-flow.md) |
|---|---|---|
| 409 (uid precondition) | `Conflict` | `A new object with this name exists; nothing was deleted` |
| 404 | `NotFound` | dry-run: `already gone` (skipped in bulk); commit: `{name} was already deleted` |
| 403 | `Denied` | `not permitted: …` |
| 422 / 400 webhook / timeout | 0030 rows | 0030 texts; Secret targets redacted |

## RBAC (`access_review.rs`)

`AccessCheck::Delete(ObjectKind)` → `("delete", resource().0, resource().1, None, kind.is_namespaced())`, shown as `delete pods`. It is **lazy** (decision 25) and not in `ALL`. 0031's session `request_kind_access(kind)` reviews `[Update(kind) if editable, Delete(kind)]` through `review_access_for` when a screen of the kind is first shown. Tests: `delete_check_text_and_group`, `kind_access_includes_delete` (app), `lazy_checks_are_distinct_permissions`.
