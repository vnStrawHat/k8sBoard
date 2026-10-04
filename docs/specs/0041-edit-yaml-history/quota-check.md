# 0041 · Quota check

[Back to index](README.md) · Steps 1 (cluster) and 3 (app) · Decisions 4, 5, 11. Wireframe W10 side panel `Namespace quota OK (22Gi left)`, note 2 ("quota").

## Cluster crate: `quota_demand.rs` (new, pure) + `quota_demand_tests.rs`

```rust
/// Steady-state pod resources of a workload, in base units (nanocores, bytes, pods).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkloadDemand { pub pods: u64, pub requests_cpu: u64, pub requests_memory: u64,
    pub limits_cpu: u64, pub limits_memory: u64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DemandChange { pub before: WorkloadDemand, pub after: WorkloadDemand }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuotaResource { Pods, RequestsCpu, RequestsMemory, LimitsCpu, LimitsMemory }
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuotaCheck {
    /// Nothing grows, or no unscoped quota limits a resource that grows: no line.
    NotAffected,
    /// The tightest item after the change: the smallest `left / hard` among the growing resources
    /// (a fraction, so CPU nanocores, bytes, and pod counts compare fairly).
    Fits { quota: String, resource: QuotaResource, left: u64, hard: u64 },
    /// Every item the change exceeds.
    Exceeds(Vec<QuotaShortfall>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuotaShortfall { pub quota: String, pub resource: QuotaResource, pub needed: u64, pub left: u64 }

/// `None` for a kind without pods to count (only Deployment, StatefulSet, DaemonSet count).
pub(crate) fn workload_demand(kind: ObjectKind, object: &Value) -> Option<WorkloadDemand>;
pub fn quota_check(change: &DemandChange, quotas: &[ResourceQuotaSummary]) -> QuotaCheck;
```

`EditPreview` gains `pub demand: Option<DemandChange>`: `build_preview` computes it **first, from the unstripped** `fresh` (before) and dry-run `response` (after), because `strip_server_fields` at the top of `build_preview` removes `status` and a DaemonSet needs `status.desiredNumberScheduled`. It is set when both give `Some` and they differ. Quantities are not masked by any 0007 rule, so the masked trees serve too.

## `workload_demand`

| Part | Rule |
|---|---|
| Multiplier | Deployment, StatefulSet: `spec.replicas`, default 1. DaemonSet: `status.desiredNumberScheduled` of the object, default 0 (the dry-run answer keeps the status). Other kinds: `None` |
| Pod template | `spec.template.spec` |
| Per pod, per resource | `max(sum(containers) + sum(sidecar inits), max(each regular init + sidecar inits started before it))`; a sidecar init has `restartPolicy: Always` |
| Quantities | `resources.requests` / `resources.limits` `cpu` via `CpuAmount::parse`, `memory` via `ByteAmount::parse` (`quantity.rs`). A missing `requests.X` with a `limits.X` uses the limit: the Kubernetes rule "If you specify a limit for a resource, but do not specify any request, and no admission-time mechanism has applied a default value for that resource, then Kubernetes copies the limit you specified and uses it as the requested value" ([Resource Management for Pods and Containers](https://kubernetes.io/docs/concepts/configuration/manage-resources-containers/#requests-and-limits)). The API server applies it to Pods, not to templates, so `workload_demand` applies it itself. Both missing, or unparsable = 0 |
| Totals | `pods = multiplier`; each resource = per pod × multiplier, saturating |

`ponytail:` steady state only; surge pods, `spec.overhead`, and LimitRange defaults are ignored (decision 4).

## `quota_check`

1. Growth per resource: `after − before` when positive; none positive → `NotAffected`.
2. Quotas: only those with empty `scopes`. Items mapped by name: `pods`, `count/pods` → `Pods`; `requests.cpu`, `cpu` → `RequestsCpu`; `requests.memory`, `memory` → `RequestsMemory`; `limits.cpu` → `LimitsCpu`; `limits.memory` → `LimitsMemory`. Others ignored. An item without `used` (not synced) or with an unparsable quantity is skipped.
3. Per matching item with growth `g`: `left = hard − used` (saturating). `g > left` → a `QuotaShortfall { needed: g, left }`; else `(left − g, hard)` is a headroom candidate (an item with `hard = 0` and `g > 0` is a shortfall).
4. Any shortfall → `Exceeds` (quota then resource order); else the candidate with the smallest `(left − g) / hard` (compared as `(left − g) × other_hard < other_left × hard` in `u128`, no floats) → `Fits`; no matching item → `NotAffected`.

## App (step 3)

- Quotas: the ResourceQuotas `ConditionFeed` of the active session (`IssueFeeds`), items of the edited namespace (`KindObject::ResourceQuota`). A read-only accessor `IssueFeeds::condition(kind) -> Option<&ConditionFeed>` and `ConditionFeed::off_reason()` are added (twins of `condition_mut`, `off`).
- `PassedPreview` gains `quota: QuotaLine`, computed when the dry-run passes:

| State | Side panel line (after the Rollout check) | Tone | Dialog warning |
|---|---|---|---|
| `demand` is `None` or `NotAffected` | none | — | none |
| Feed off | `Quota not checked: {reason}` | muted | none |
| Feed loading | `Quota not checked: quotas are still loading` | muted | none |
| `Fits` | `Namespace quota OK ({left} {resource} left)`, e.g. `22Gi requests.memory left` | success | none |
| `Exceeds` | per shortfall `Quota {name}: {resource} needs {needed} more, {left} left` | warning | the same texts |

- `{left}` and `{needed}` (`usage_format.rs` helpers): memory as binary bytes (`22Gi`), CPU as cores or millicores (`1.5`, `250m`), pods as a plain count (`4`). `{resource}` in both lines is the quota name of the item: `requests.cpu`, `requests.memory`, `limits.cpu`, `limits.memory`, `pods` (so `4 pods left`, `250m requests.cpu left`).
- The line is computed once per passed preview (feeds change slowly); a later preview recomputes it. Apply is never disabled by it (decision 11).
- `--screen edit-yaml-diff` fixture: `Fits { quota: compute-quota, RequestsMemory, left: 22Gi, hard: 64Gi }`, read `Namespace quota OK (22Gi requests.memory left)` (W10 draws `22Gi left`; the resource name is added so CPU and pod lines read clearly).
