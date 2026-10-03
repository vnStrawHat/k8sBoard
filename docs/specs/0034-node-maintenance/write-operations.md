# 0034 · Write operations and reads (cluster crate)

[Back to index](README.md) · Step 1 · Modules: `object_write.rs` (+ tests), `node.rs`, `pod.rs`, `disruption_budget.rs`, `access_review.rs`. Decisions 1–8, 41.

## Verified APIs (kube 4.2, `.cargo-home/registry`)

| API | Where | Use |
|---|---|---|
| `Api::evict(name, &EvictParams)` requires `K: Evict` (only `Pod`) and serializes `"delete_options"` (snake case) | `kube-client-4.2.0/src/api/subresource.rs:628, 637`; `kube-core-4.2.0/src/subresource.rs:110–126` | **not used**: the API server ignores the snake-case key, so grace and uid precondition would be dropped |
| `Api::create_subresource::<I, T>("eviction", name, &PostParams, &body)` | `subresource.rs:84` | eviction with our own body (already disallowed; allowed in `object_write.rs`) |
| `Status.details.retry_after_seconds: u32`, `causes[].{reason, message}`; `Error::Api(Box<Status>)` | `kube-core-4.2.0/src/response.rs:168–208`; `kube-client-4.2.0/src/error.rs` | 429 mapping |
| `ListParams.field_selector` | `kube-core-4.2.0/src/params.rs` | `spec.nodeName=` |

## New operations

```rust
EvictPod { uid: String, grace: GracePeriod },          // Pod
SetNodeTaints { taints: Vec<NodeTaint>, resource_version: String }, // Node
SetNodeLabels { changes: Vec<LabelChange> },           // Node
pub enum GracePeriod { PodDefault, Seconds(u32) }
pub struct LabelChange { pub key: String, pub value: Option<String> } // None removes
```

`NodeTaint` (0003) gains `time_added: Option<jiff::Timestamp>` (stable; no summary churn). Manual `Debug` shows the variant name only.

| Operation | HTTP | Path | Content type | Body |
|---|---|---|---|---|
| `EvictPod` | POST | `/api/v1/namespaces/{ns}/pods/{name}/eviction` | `application/json` | `{"apiVersion":"policy/v1","kind":"Eviction","metadata":{"name":"{name}","namespace":"{ns}"},"deleteOptions":{"preconditions":{"uid":"{uid}"},"gracePeriodSeconds":30}}` (no `gracePeriodSeconds` for `PodDefault`) |
| `SetNodeTaints` | PATCH | `/api/v1/nodes/{name}` | `application/merge-patch+json` | `{"metadata":{"resourceVersion":"{rv}"},"spec":{"taints":[{"key":"dedicated","value":"ingress","effect":"NoSchedule"},{"key":"node.kubernetes.io/unreachable","effect":"NoExecute","timeAdded":"2026-10-02T08:00:00Z"}]}}`; no taints → `"taints":[]` |
| `SetNodeLabels` | PATCH | `/api/v1/nodes/{name}` | `application/merge-patch+json` | `{"metadata":{"labels":{"team":"infra","old-key":null}}}` |

Query: `fieldManager=k8sboard`; dry-run adds `dryRun=All`. The eviction REST handler copies `dryRun` into its delete options and still checks the PDB, so a dry-run returns the real 429.

## RBAC, dry-run, risk, audit

| Operation | `AccessCheck` | SSAR | Dry-run | Risk | `changed_fields()` |
|---|---|---|---|---|---|
| `EvictPod` | `CreatePodEviction` (new) | `create "" pods/eviction`, namespaced | yes | Destructive | `pods/eviction` = `grace 30s` / `grace pod default` |
| `SetNodeTaints` | `PatchNodes` (0030) | `patch "" nodes` | yes | Change; **Destructive** when an added taint is `NoExecute` | `spec.taints` = `dedicated=ingress:NoSchedule, …` or `none` |
| `SetNodeLabels` | `PatchNodes` | `patch "" nodes` | yes | Change | `metadata.labels` = `team=infra; -old-key` |
| bulk Cordon/Uncordon | `PatchNodes` | (0030 operation) | yes | Change | 0030 |

`ALL` grows by 1. Display: `create pods/eviction`. Taint and label values are not secret and are recorded.
## Response and errors

`EvictPod` decodes the response as `Status` (`create_subresource::<Value, Status>`). An answer whose `status` is not `"Success"` is an **error**, mapped from the body's `code` and `message` exactly like an HTTP error of that code: the API server can answer **HTTP 201 with a `Failure` body** (for example a pod matched by two PDBs, code 500). Success → `effect: Created`, `created_name: None`.

| Status (HTTP or body `code`) | Mapping |
|---|---|
| 429 | the merged `TooManyRequests { message, retry_after }` on both modes (never `OutcomeUnknown`: the server refused). `retry_after` = `details.retryAfterSeconds` when > 0, else `None` (as merged). **Changed by 0034**: `message` = first `causes[].message`, else the status message (main: the status message), in `map_status` for every operation |
| 409 on `EvictPod` | `Conflict` (0030): the uid precondition failed, so the pod with that uid is gone |
| 409 on `SetNodeTaints` | `Conflict`: the node changed since it was read; Retry reopens the editor fresh (node-edits.md) |
| 404 | `NotFound` |
| 500 "more than one PodDisruptionBudget" (HTTP or a 201 `Failure` body) | `Cluster(..)` with the server text |

The two 429 shapes of the 1.29 eviction handler (test fixtures use both):

| Cause | Body | `retry_after` |
|---|---|---|
| PDB allows no disruption | message `Cannot evict pod as it would violate the pod's disruption budget.`, cause `DisruptionBudget`: `The disruption budget api-pdb needs 2 healthy pods and has 2 currently`; `retryAfterSeconds` 0 or absent | `None` → the client backoff alone |
| PDB status not yet observed (`observedGeneration` behind) | same message, cause `DisruptionBudget`: `The disruption budget api-pdb is still being processed by the server.`; `retryAfterSeconds: 10` | `Some(10 s)` |

## Reads (GET/LIST only)

```rust
impl ClusterConnection {
    /// Pods on `node`, every namespace: LIST /api/v1/pods?fieldSelector=spec.nodeName={node}, paged.
    pub async fn drain_pods(&self, node: &str) -> Result<Vec<DrainPod>, ClusterError>;
    /// Every PodDisruptionBudget, one-shot (0013 summary).
    pub async fn list_pod_disruption_budgets(&self) -> Result<Vec<PodDisruptionBudgetSummary>, ClusterError>;
    /// GET /api/v1/nodes/{name}: what the editors need, fresh.
    pub async fn node_for_edit(&self, name: &str) -> Result<NodeEdit, ClusterError>;
}
pub struct DrainPod { pub namespace: String, pub name: String, pub uid: String, pub labels: Vec<String>,
    pub controller: Option<ControllerRef>, pub is_mirror: bool /* kubernetes.io/config.mirror */,
    pub has_empty_dir: bool, pub is_finished: bool /* Succeeded | Failed */, pub is_pending: bool, pub is_terminating: bool }
pub struct NodeEdit { pub taints: Vec<NodeTaint>, pub labels: BTreeMap<String, String>, pub resource_version: String }
```

`DrainPod` reads only those fields (no env, no annotations kept beyond the mirror flag). `list_pod_disruption_budgets` reuses `pod_disruption_budget_summary`.
