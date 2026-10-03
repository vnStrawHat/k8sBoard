# 0031 · Write path: `ReplaceObject`

[Back to index](README.md) · Step 2 (no app caller until step 4) · Modules: `object_write.rs` (+ tests), `edit_placeholders.rs`, `edit_preview.rs`. Decisions 1–4, 6–8, 11, 27. Builds on the merged 0030 `object_write.rs` ([write-path.md](../0030-guardrails-write-path/write-path.md)): `WriteRequest`, `WriteOperation`, `WriteMode`, `WriteOutcome { mode, elapsed, effect, created_name, uid }`, `WriteEffect::Patched`, `WritePolicy`, `WriteError` (incl. `TooManyRequests`, `OutcomeUnknown`), private `send` / `run_raw` / `error_from_status` → `redact_error`, `is_dns_subdomain`, `FakeApi`.

## New operation

```rust
pub enum WriteOperation {
    // SetNodeSchedulable (0030) …
    /// PUT of an edited object with its base resourceVersion and uid; placeholders restored from a fresh GET.
    ReplaceObject(Box<ObjectEdit>),   // boxed: clippy::large_enum_variant (an edit is ~330 bytes, the other variants ~32)
}
pub enum WriteEffect { Patched /* 0030 */, Replaced(EditPreview) }   // EditPreview: Clone + PartialEq + Eq, manual Debug
```

| Item | Rule |
|---|---|
| `WriteRequest::new(target, ReplaceObject(edit))` | `None` unless `edit.target() == &target`, `target.builtin_kind()` is `Some` and `is_editable()`, and the name passes the name rule: the merged `is_dns_subdomain`, or for the four RBAC kinds the path-segment rule of decision 27 (a namespace stays DNS) |
| `access_check()` | `AccessCheck::Update(kind)` (a lazy check, decision 24) |
| `changed_fields()` | one `ChangedField { path, value: None }` per `edit.changed_paths()` (Display); never a value. `ChangedField.path` is `&'static str` on main: it becomes `Cow<'static, str>` (owned for edit paths) |
| `supports_dry_run()` | true |
| Manual `Debug` | `ReplaceObject` / `WriteRequest { operation: ReplaceObject, kind: Deployment, namespace: Some("payments"), name: "api" }` |

## Sequence inside `ClusterConnection::write` (both modes)

On main `write` builds the outcome with a fixed `effect: Patched`. This step makes the effect per operation (`send` returns what `write` needs); 0032 step 1 needs the same seam for `Created`, so whichever lands first adds it.

1. Policy check (0030 kill switch): `Blocked` → `WritesBlocked`, zero requests.
2. Fresh GET: `get_object(target, "reading the object before the change")` through `self.run` (a classified `ClusterError`, returned as `WriteError::Cluster` directly, never through `write_error`, so a failed read is never `OutcomeUnknown`). The raw value lives only in this call.
3. Fresh `metadata.resourceVersion` ≠ base → `Conflict { message: "the object changed since it was opened (resourceVersion {base} → {fresh})", managers: vec![] }`; no `PUT`.
4. `body = edit.edited.clone()`; `restored = edit_placeholders::restore(&mut body, &fresh)`. An unmatched path cannot happen after step 3 (decision 7), but if it does → `Conflict` with the same text.
5. `metadata.resourceVersion = base`, `metadata.uid = base uid`. **No** `status`, `managedFields`, `generation`, `creationTimestamp`, `deletionTimestamp`, `deletionGracePeriodSeconds`, or `selfLink`: the body keeps what step 1's strip left (decision 3).
6. `Api::<DynamicObject>::replace(name, &PostParams { dry_run: mode == DryRun, field_manager: Some(FIELD_MANAGER.into()) }, &serde_json::from_value(body)?)` in `run_raw`, as a new arm of the excepted `send` match (no new `#[allow]`).
7. Response → `serde_json::to_value`; `effect = Replaced(build_preview(edit, fresh, response, &restored))` (edit-preview.md). The raw values are dropped here.

- Conversion failures (body or response) → `Cluster(UnexpectedResponse { source: fixed text })`, never the library error.
- A commit repeats steps 2–7. The commit `PUT` carries the base `resourceVersion`, so it fails with 409 if anything changed after the dialog's dry-run.
- Server errors go through the merged `error_from_status`, which classifies on the raw status and then runs `redact_error` (Secret targets). The local `Conflict` of step 3 carries fixed text only.

## Allow-list row (added to the 0030 table by this spec's step 2)

| Operation | HTTP | Path and query | Body | Dry-run | Spec |
|---|---|---|---|---|---|
| `ReplaceObject` | GET, then PUT `application/json` | `{api_resource path}/{name}?dryRun=All&fieldManager=k8sboard` (commit: `?fieldManager=k8sboard`) | edited object, base `resourceVersion` and `uid`, no status or other server metadata | yes (query) | 0031 |

Query order follows `PostParams::populate_qp` (`kube-core-4.2.0/src/params.rs:543`: `dryRun` first, then `fieldManager`). There is no new clippy exception: `Api::replace` sits in the excepted `match`, and `Api::get` is a read.

## Errors (0030 variants; no new variant)

| Status | Variant | Editor surface |
|---|---|---|
| 409 (`Conflict`, step 3, or a `uid` mismatch) | `Conflict` | conflict banner |
| 422 | `Invalid { message, fields }` | side panel; `fields` verbatim; message redacted for Secrets |
| 403 RBAC form | `Denied` | footer and side panel |
| other 403 (admission) | `Invalid` (merged mapping) | side panel |
| 404 | `NotFound` | banner `The object was deleted. Your text is kept but cannot be applied.` |
| 429 | `TooManyRequests` | footer `The server refused for now: {message}`; the preview can be re-run |
| 400 webhook dry-run | `DryRunRejected` | footer; Apply blocked (0030 decision 26) |
| commit timeout or transport | `OutcomeUnknown` | 0030 notification; banner with `Reload` |

## Secret targets

- The fresh GET and the response hold real data. They stay in `write`, are masked before any text is built, and are never traced (the 0016 decision 30 log filter also covers kube's error-body logging).
- The `PUT` carries the restored server data, never an empty map (decision 8). Errors pass through 0030's redaction.

## Fake transport

`FakeApi::connection(policy, respond)` answers the GET with a fixture, then the `PUT` with the body echoed back and a bumped `resourceVersion`. Recorded requests pin method, path, query, content type, and body ([test-plan.md](test-plan.md)).
