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
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum WriteMode { DryRun, Commit }  // the app wraps it (write-flow.md)
/// `created_name`: the server-chosen name of a create with `generateName` (0032 Trigger now, Re-run); `uid`: the uid of
/// a created or replaced object (0037 node-shell pod cleanup). Both are `None` on a dry-run and when not applicable.
#[derive(Clone, Debug, PartialEq, Eq)] pub struct WriteOutcome { pub mode: WriteMode, pub elapsed: Duration,
    pub effect: WriteEffect, pub created_name: Option<String>, pub uid: Option<String> }
/// What the server did. 0030 `Patched`; 0032 `Created`; 0031 adds `Replaced`; 0033 `Deleted`, `DeletionPending`.
#[derive(Clone, Debug, PartialEq, Eq)] pub enum WriteEffect { Patched, Created }
#[derive(Clone, Debug)] pub struct ChangedField { pub path: Cow<'static, str>, pub value: Option<String> } // None = not recorded
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum WritePolicy { Allowed, Blocked }

impl WriteRequest {
    pub fn new(target: ObjectRef, operation: WriteOperation) -> Option<Self>; // None: kind does not fit, or name/namespace is not a DNS-1123 subdomain
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
    /// any request when the connection's policy is `Blocked`. Patch, create, and replace set
    /// `fieldManager=k8sboard` in both modes, and `DryRun` adds `dryRun=All` to the query. Delete
    /// (0033) is the exception: `DeleteOptions` has no field manager, and its dry-run is `"dryRun":["All"]` in the body.
    pub async fn write(&self, request: &WriteRequest, mode: WriteMode) -> Result<WriteOutcome, WriteError>;
}
```

- **Kill switch**: `ClusterConnection` gains `write_policy: WritePolicy`, set in `open` from `WritePolicy::resolve(cfg!(debug_assertions), std::env::var("K8SBOARD_ALLOW_WRITES").ok().as_deref())`. Debug builds (every agent, coder, ui-verifier run) cannot write unless a human sets the variable; agent runs never set it. The screenshot build turns on the cluster crate's `block-writes` feature, which forces `Blocked` whatever the variable says (`WritePolicy::of_build`). Tests inject the policy instead of touching the environment.
- **Target connection (0027 review):** every mutating action takes its `ClusterConnection` and its `ClusterGuard` from the row's or cursor's cluster slot, never from an implicit "current session"; `write` and `guard_for` take the target explicitly.
- Transport: `Api::<DynamicObject>` from `ClusterConnection::object_api(&ObjectRef)` (shared with `object_yaml`), `PatchParams { dry_run, field_manager: Some(FIELD_MANAGER.into()), ..Default::default() }` (it also has `field_validation`), `Patch::Merge(json)`.
- **Uncordon sends `false`, not `null`**: the same body shape as cordon, so the dry-run, the confirm summary, and the audit show one explicit value. `unschedulable` is `omitempty`, so the stored Node is the same as after a `null` delete.
- Timeout: a private `run_raw(action, future) -> Result<Result<T, kube::Error>, Elapsed>` around `tokio::time::timeout(REQUEST_TIMEOUT, ..)`, because `run` classifies errors before the `Status` can be read.
- `AccessCheck::PatchNodes` = `("patch", "", "nodes", None, false)`; `ALL` grows by 1 (counts are relative: the code has 29 today, and other specs add theirs).
- `ObjectKind::ALL` (every variant, in declaration order) and `ObjectKind::resource(self) -> &'static str` (plural API resource, `deployments`) back the per-kind checks of 0031 (`Update(kind)`) and 0033 (`Delete(kind)`); they land with whichever of the two merges first (no production user in 0030 itself).
- **C8 note (amended):** there is no server-side apply and no Force path. Single-field actions are merge patches (0032 Roll back and taint edits are noted per operation); 0031 replaces with a `resourceVersion` precondition.
- Manual `Debug`: `WriteOperation` → `SetNodeSchedulable`; `WriteRequest` → `WriteRequest { operation: SetNodeSchedulable, kind: Node, namespace: None, name: "wk-04" }`. Never a body or field value.

## Allow-list (the controlled 0001 grep)

| Operation | HTTP | Path | Body | Dry-run | Spec |
|---|---|---|---|---|---|
| SSAR (exists) | POST | `/apis/authorization.k8s.io/v1/selfsubjectaccessreviews` | review | n/a (non-mutating) | 0001 |
| `SetNodeSchedulable` | PATCH (merge) | `/api/v1/nodes/{name}` | `{"spec":{"unschedulable":b}}` | yes | 0030 |
| `ReplaceObject` | GET, then PUT | `{path}/{name}?dryRun=All&fieldManager=k8sboard` (commit: no `dryRun`) | the edited object with the base `resourceVersion` and `uid`; no `status` or other server metadata | yes | 0031 (name rule: the DNS-1123 rule above, except the four RBAC kinds, which accept a path-segment name, 0031 decision 27) |
| `DeleteObject` | DELETE | `{path}/{name}`, no query | `{"propagationPolicy":…,"preconditions":{"uid":…}}` plus `"dryRun":["All"]` on a dry-run; no `fieldManager` | yes (body) | 0033 |
| `ScaleWorkload`, `RestartRollout`, `SetRolloutPaused`, `RollBackDeployment`, `SetCronJobSuspended`, `TriggerCronJob`, `RerunJob` | PATCH (merge; JSON Patch for Roll back), GET + POST for creates | 0032 write-operations.md | 0032 write-operations.md | yes | 0032 |
| `EvictPod`, `SetNodeTaints`, `SetNodeLabels` | POST (eviction), PATCH (merge) | 0034 write-operations.md | 0034 write-operations.md | yes | 0034 |
| `SetHpaReplicaRange`, `ExpandClaim`, `SetDefaultStorageClass` | PATCH (merge) | 0032b operations.md | 0032b operations.md | yes | 0032b |
| `pods/portforward` (connect) | GET + WebSocket upgrade | `/api/v1/namespaces/{ns}/pods/{pod}/portforward?ports={p}` | stream | no | 0035 step 1 (`port_forward.rs`) |
| `pods/exec` (connect) | GET + WebSocket upgrade | `/api/v1/namespaces/{ns}/pods/{pod}/exec?…` | stream | no | 0036 step 1 (`pod_shell.rs`) |
| `AddDebugContainer`, `CreateNodeShellPod`, `DeleteNodeShellPod` | PATCH (strategic) `ephemeralcontainers`, POST pod, DELETE pod | 0037 pod-specs.md | 0037 pod-specs.md | yes, yes, no (delete is commit only) | 0037 step 1 |
| `pods/attach` (connect) | GET + WebSocket upgrade | `/api/v1/namespaces/{ns}/pods/{pod}/attach?…` | stream | no | 0037 step 1 (`debug_shell.rs`) |

Deferred (user, 2026-10-02): 0038 Helm writes (`helm rollback`, `helm uninstall` through the user's `helm` CLI, call site `helm_command.rs`) are not scheduled; their allow-list row and clippy exception are added only if 0038 is.

Enforcement:

1. **clippy `disallowed-methods`** in the root `clippy.toml`, one entry with a `reason` per path. The coder re-proves every path with a scratch call that must fire the lint (not committed):
   - `kube::Client::{send, request, request_text, request_status, request_stream, request_events}`.
   - Mutating `kube::Api` methods: `create`, `patch`, `replace`, `delete`, `delete_collection`, `create_subresource`, `patch_subresource`, `replace_subresource`, `patch_status`, `replace_status`, `patch_scale`, `replace_scale`, `patch_metadata`, `replace_ephemeral_containers`, `patch_ephemeral_containers`, `evict`, and `entry` (its `OccupiedEntry::commit` creates or replaces inside kube; both are listed). The kube helpers that patch inside kube are listed too: `restart` (wrong annotation, see `RestartRollout`), `cordon`, and `uncordon` (they bypass `SetNodeSchedulable`). Two more helpers write inside kube: `create_token_request` (it mints a ServiceAccount token) and `patch_approval` (it approves a CertificateSigningRequest).
   - `kube::runtime` helpers that write: `events::Recorder::publish`, `finalizer::finalizer`, `wait::delete::delete_and_finalize`.
   - `cluster::ClusterConnection::write` itself (added with 0037): the app sends a write from `checked_write` and from `run_cleanup` only, each with one named allow (table below); the cluster crate allows it on its four `object_write_*tests` modules, which drive `write` directly.
   - Paths that exist only behind a feature or a newer Kubernetes version carry `allow-invalid = true`, because clippy warns about an unreachable path without it: `Client::kubelet_node_{exec, attach, portforward}` (kube's `kubelet-debug` feature), and `Api::{patch_resize, replace_resize}` (Kubernetes 1.33). `Client::connect` and `Api::{exec, attach, portforward}` are real paths since the cluster crate enables kube's `ws` feature (0036 step 1) and carry no `allow-invalid`.
2. **Named exceptions: the canonical list** (single source; other specs point here). Each is one `#[allow(clippy::disallowed_methods)]` on the smallest item with a comment naming its row; no other allow:

   | File | Item | Spec |
   |---|---|---|
   | `access_review.rs` | `post_review` (the SSAR and SSRR `create`, non-mutating) | 0030 |
   | `object_write.rs` | `send`, the `match` that sends each allow-listed operation | 0030 |
   | `kubelet_stats.rs` | `kubelet_text` / `kubelet_lines` (read-only GETs of the 0011 kubelet path allow-list) | 0030 |
   | `secret.rs` | `secret_text` (read-only GET decoded in-crate, 0016) | 0016 |
   | `pod_shell.rs` | `exec` | 0036 |
   | `port_forward.rs` | `portforward` | 0035 |
   | `debug_shell.rs` | `attach` | 0037 |
   | `write_flow.rs` (app) | the `write` call of `checked_write`, the one sender of every confirmed write | 0030 |
   | `write_flow.rs` (app) | the `write` call of `run_cleanup`, the commit-only delete of the app's own node shell pod under a `uid` precondition (no gate, lock, or dialog: it must work after a lock, a switch, and inside the shutdown window) | 0037 |
3. The grep stays as documentation: `grep -rnE "\.(create|patch|replace|delete|delete_collection|exec|attach|portforward|evict|create_subresource|patch_subresource|replace_subresource|patch_status|replace_status|patch_scale|replace_scale)\(" crates/cluster/src crates/cluster/examples` lists only the kube files of that table that have shipped. Shipped so far: `access_review.rs`, `object_write.rs`, `kubelet_stats.rs`, `secret.rs`, `pod_shell.rs` (0036 step 1, `exec`), `port_forward.rs` (0035 step 1, `portforward`), and `debug_shell.rs` (0037 step 1, `attach`).
4. `crates/app` has no `kube` dependency (test `app_has_no_kube_dependency`), so it can only write through `ClusterConnection::write`.
5. `allow_list_matches_the_operations` pins method, path, query, content type, and body per variant through the fake transport. Connect calls do not live in `object_write.rs`: they are rows with "Dry-run: no" whose call sites are `pod_shell.rs` (0036), `port_forward.rs` (0035), and `debug_shell.rs` (0037), each with its own permit.

## Dry-run

- Every commit is preceded by a `DryRun` of the same request (write-flow.md). `supports_dry_run` is false only for connect verbs (0035–0037) and 0037 `DeleteNodeShellPod` (own pod, `uid` precondition, commit only); `run_guarded` shows `DryRunState::NotSupported` for it.
- **An admission webhook that rejects dry-run** (HTTP 400 whose message says the webhook does not support dry run) → `DryRunRejected { reason }`, and the commit is **blocked**. A later spec may add an explicit escape; 0030 offers none.

## Write errors

| Variant | From | `Display` |
|---|---|---|
| `WritesBlocked` | policy `Blocked` | `writes are blocked in this debug build (set K8SBOARD_ALLOW_WRITES=1)` |
| `Denied { message }` | 403 whose message has the RBAC form (`is forbidden: User`) | `not permitted: {message}` |
| `NotFound` | 404 | `the object no longer exists` |
| `Conflict { message, managers }` | 409; also the operation-specific mappings of 0032 (Roll back 422) and 0034. `managers` is always empty: there is no SSA | `the object changed since it was read: {message}` |
| `Invalid { message, fields }` | 422 (`details.causes[].field`); also any **other 403** (an admission plugin or webhook refusal, e.g. `PersistentVolumeClaimResize`), so it never reads "not permitted" | `the change is invalid: {message}` |
| `TooManyRequests { message, retry_after }` | 429 on either mode (PDB eviction, API priority and fairness), and an eviction answered 201 with a `Failure` status of code 429 (0034); never `OutcomeUnknown` | `refused for now: {message}` |
| `DryRunRejected { reason }` | 400 on a dry-run naming dry-run support | `an admission webhook does not support dry-run, so the change cannot be checked: {reason}` |
| `OutcomeUnknown` | **Commit** only: every error except `Api` (the server answered) and the build errors `BuildRequest` and `HttpError` (nothing was sent): timeout, `HyperError`, `Service`, `SerdeError`, `FromUtf8`, and the rest (anything after the request may have left) | `no answer in time; the change may have been applied` |
| `Cluster(ClusterError)` | the rest, and every non-`Api` error of a `DryRun` | the `ClusterError` text |

- A `Service` error can also be a pre-send auth failure; treating it as unknown is the safe side.
- **Redaction** (Secret targets): a status is classified on its raw text first, then `redact_error` runs on the **final** `WriteError` of any variant (every variant that carries server text is listed), keyed on the target kind name through a pure `redact_message(kind_name, ..)`: server message text becomes the status `reason` code plus field paths. No write targets a Secret yet, so the tests use the name `"Secret"`. The audit `error` field uses the redacted text.
- `tracing`: context, operation name, kind, namespace, name, mode, status code, elapsed. Never a body.

## Conflicts (resourceVersion policy)

| Write style | Precondition | On 409 |
|---|---|---|
| Single-field merge patch (cordon, scale, suspend) | none: the patch states the whole intent of one field | shown as Conflict |
| Replace (0031) | base `metadata.resourceVersion` and `uid` | Conflict; Retry rebases. No Force path (C8 note) |
| JSON Patch with a `test` op (0032 Roll back), full-list patch with `resourceVersion` (0034 taints) | the test value / `resourceVersion` | Conflict + Retry |
| Any write answered 429 | — | `TooManyRequests`, shown like Conflict: Retry re-runs the dry-run |
| Delete (0033) | `Preconditions { uid }` | "a new object with this name exists" |

## Test transport (`fake_api.rs`, `#[cfg(test)]`)

`FakeApi::connection(policy: WritePolicy, respond: impl Fn(&RecordedRequest) -> (u16, String))` builds `kube::Client::new(tower::service_fn(..), "default")` (kube-client 4.2 `client/mod.rs:153`) and `ClusterConnection::from_client(client, context, policy)` (`#[cfg(test)]`). `RecordedRequest { method, path, query, content_type, body }`; `requests()` returns them. `tower-test` is not used (not in `Cargo.lock`).
