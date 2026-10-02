# 0030 · Write path (cluster crate)

[Back to index](README.md) · Step 1 · Modules: `object_write.rs` (new) + `object_write_tests.rs`, `fake_api.rs` (new, `#[cfg(test)]`), `access_review.rs`, `connection.rs`, `lib.rs`, root `clippy.toml`. Decisions 1–5, 13–15, 24–27. C3, C8.

## API (`object_write.rs`)

```rust
/// One allow-listed mutation. Adding a variant is the only way to add a write (C3).
#[derive(Clone, PartialEq, Eq)]   // Debug is manual: the variant name only
pub enum WriteOperation {
    /// JSON merge patch `{"spec":{"unschedulable": !schedulable}}` on a Node (cordon / uncordon).
    SetNodeSchedulable { schedulable: bool },
}
/// Always valid: the target kind fits the operation. Debug is manual: operation, kind, namespace, name.
#[derive(Clone, PartialEq, Eq)]
pub struct WriteRequest { target: ObjectRef, operation: WriteOperation }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum WriteMode { DryRun, Commit }
#[derive(Clone, Debug, PartialEq, Eq)] pub struct WriteOutcome { pub mode: WriteMode, pub elapsed: Duration }
#[derive(Clone, Debug)] pub struct ChangedField { pub path: &'static str, pub value: Option<String> } // None = not recorded
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum WritePolicy { Allowed, Blocked }

impl WriteRequest {
    pub fn new(target: ObjectRef, operation: WriteOperation) -> Option<Self>; // None: kind does not fit
    pub fn target(&self) -> &ObjectRef;
    pub fn operation(&self) -> &WriteOperation;
    pub fn access_check(&self) -> AccessCheck;          // the app gate reads the same value
    pub fn changed_fields(&self) -> Vec<ChangedField>;  // confirm summary and audit (audit-log.md)
    pub fn supports_dry_run(&self) -> bool;             // true for every 0030 operation
}
impl WritePolicy {
    /// Release: Allowed. Debug: Blocked unless `opt_in == Some("1")` (`K8SBOARD_ALLOW_WRITES`).
    pub fn resolve(is_debug_build: bool, opt_in: Option<&str>) -> Self;
}
impl ClusterConnection {
    /// The only function that sends a mutating request. Returns `WritesBlocked` before building
    /// any request when the connection's policy is `Blocked`. Both modes set `fieldManager=k8sboard`;
    /// `DryRun` also sets `dryRun=All`.
    pub async fn write(&self, request: &WriteRequest, mode: WriteMode) -> Result<WriteOutcome, WriteError>;
}
```

- **Kill switch**: `ClusterConnection` gains `write_policy: WritePolicy`, set in `open` from `WritePolicy::resolve(cfg!(debug_assertions), std::env::var("K8SBOARD_ALLOW_WRITES").ok().as_deref())`. Debug builds (every agent, coder, ui-verifier run) cannot write unless a human sets the variable; agent runs never set it. Tests inject the policy instead of touching the environment.
- Transport: `Api::<DynamicObject>` from `object_yaml::api_resource(kind)`, `PatchParams { dry_run, field_manager: Some(FIELD_MANAGER.into()), ..Default::default() }` (it also has `field_validation`), `Patch::Merge(json)`.
- **Uncordon sends `false`, not `null`**: the same body shape as cordon, so the dry-run, the confirm summary, and the audit show one explicit value. `unschedulable` is `omitempty`, so the stored Node is the same as after a `null` delete.
- Timeout: a private `run_raw(action, future) -> Result<Result<T, kube::Error>, Elapsed>` around `tokio::time::timeout(REQUEST_TIMEOUT, ..)`, because `run` classifies errors before the `Status` can be read.
- `AccessCheck::PatchNodes` = `("patch", "", "nodes", None, false)`; `ALL` grows to 27.
- Manual `Debug`: `WriteOperation` → `SetNodeSchedulable`; `WriteRequest` → `WriteRequest { operation: SetNodeSchedulable, kind: Node, namespace: None, name: "wk-04" }`. Never a body or field value.

## Allow-list (the controlled 0001 grep)

| Operation | HTTP | Path | Body | Dry-run | Spec |
|---|---|---|---|---|---|
| SSAR (exists) | POST | `/apis/authorization.k8s.io/v1/selfsubjectaccessreviews` | review | n/a (non-mutating) | 0001 |
| `SetNodeSchedulable` | PATCH (merge) | `/api/v1/nodes/{name}` | `{"spec":{"unschedulable":b}}` | yes | 0030 |

Enforcement:

1. **clippy `disallowed-methods`** in the root `clippy.toml`: `kube::Client::{send, request, request_text, request_status, request_stream, connect}` and the mutating `kube::Api` methods (`create`, `patch`, `replace`, `delete`, `delete_collection`, `create_subresource`, `patch_subresource`, `replace_subresource`, `patch_status`, `replace_status`, `patch_scale`, `replace_scale`, `replace_ephemeral_containers`, `evict`, `exec`, `attach`, `portforward`). Each entry has a `reason`. The coder confirms every path resolves (a scratch call per entry fires the lint; not committed).
2. **Named exceptions**, each one `#[allow(clippy::disallowed_methods)]` on the smallest item with a comment naming its row: `access_review.rs` `review_one` (SSAR row), `object_write.rs` (the `match` that sends each allow-listed operation), and `kubelet_stats.rs` `kubelet_text` / `kubelet_lines` (`request_text` / `request_stream` GETs of the 0011 kubelet path allow-list; read-only). No other allow.
3. The grep stays as documentation: `grep -rnE "\.(create|patch|replace|delete|delete_collection|exec|attach|portforward|evict|create_subresource|patch_subresource|replace_subresource|patch_status|replace_status|patch_scale|replace_scale)\(" crates/cluster/src crates/cluster/examples` lists only `access_review.rs` and `object_write.rs`.
4. `crates/app` has no `kube` dependency (test `app_has_no_kube_dependency`), so it can only write through `ClusterConnection::write`.
5. `allow_list_matches_the_operations` pins method, path, query, content type, and body per variant through the fake transport. 0035/0036 connect calls join `object_write.rs` as rows with "Dry-run: no".

## Dry-run

- Every commit is preceded by a `DryRun` of the same request (write-flow.md). `supports_dry_run` is false only for connect verbs (later specs).
- **An admission webhook that rejects dry-run** (HTTP 400 whose message says the webhook does not support dry run) → `DryRunRejected { reason }`, and the commit is **blocked**. A later spec may add an explicit escape; 0030 offers none.

## Write errors

| Variant | From | `Display` |
|---|---|---|
| `WritesBlocked` | policy `Blocked` | `writes are blocked in this debug build (set K8SBOARD_ALLOW_WRITES=1)` |
| `Denied { message }` | 403 | `not permitted: {message}` |
| `NotFound` | 404 | `the object no longer exists` |
| `Conflict { message, managers }` | 409 (`managers` from SSA conflict causes, 0031) | `the object changed since it was read: {message}` |
| `Invalid { message, fields }` | 422 (`details.causes[].field`) | `the change is invalid: {message}` |
| `DryRunRejected { reason }` | 400 on a dry-run naming dry-run support | `an admission webhook does not support dry-run, so the change cannot be checked: {reason}` |
| `OutcomeUnknown` | **Commit** only: timeout, `HyperError`, `Service`, response `SerdeError` (anything after the request may have left) | `no answer in time; the change may have been applied` |
| `Cluster(ClusterError)` | the rest, and every non-`Api` error of a `DryRun` | the `ClusterError` text |

- A `Service` error can also be a pre-send auth failure; treating it as unknown is the safe side.
- **Redaction** (Secret targets): one function runs on the **final** `WriteError` of any variant, keyed on the target kind name through a pure `redact_message(kind_name, ..)`: server message text becomes the status `reason` code plus field paths. No `ObjectKind::Secret` exists yet, so it changes nothing until 0016/0033 add one; the test uses the name `"Secret"`. The audit `error` field uses the redacted text.
- `tracing`: context, operation name, kind, namespace, name, mode, status code, elapsed. Never a body.

## Conflicts (resourceVersion policy)

| Write style | Precondition | On 409 |
|---|---|---|
| Single-field merge patch (cordon, scale, suspend) | none: the patch states the whole intent of one field | shown as Conflict |
| Server-side apply (0031) | `metadata.resourceVersion` of the edited object | Conflict with managers; Force only with a second confirm (C8) |
| Delete (0033) | `Preconditions { uid }` | "a new object with this name exists" |

## Test transport (`fake_api.rs`, `#[cfg(test)]`)

`FakeApi::connection(policy: WritePolicy, respond: impl Fn(&RecordedRequest) -> (u16, String))` builds `kube::Client::new(tower::service_fn(..), "default")` (kube-client 4.2 `client/mod.rs:153`) and `ClusterConnection::from_client(client, context, policy)` (`#[cfg(test)]`). `RecordedRequest { method, path, query, content_type, body }`; `requests()` returns them. `tower-test` is not used (not in `Cargo.lock`).
