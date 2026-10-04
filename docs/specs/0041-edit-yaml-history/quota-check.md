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
    /// The smallest headroom left after the change among the growing resources.
    Fits { quota: String, resource: QuotaResource, left: u64 },
    /// Every item the change exceeds.
    Exceeds(Vec<QuotaShortfall>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuotaShortfall { pub quota: String, pub resource: QuotaResource, pub needed: u64, pub left: u64 }

/// `None` for a kind without pods to count (only Deployment, StatefulSet, DaemonSet count).
pub(crate) fn workload_demand(kind: ObjectKind, object: &Value) -> Option<WorkloadDemand>;
pub fn quota_check(change: &DemandChange, quotas: &[ResourceQuotaSummary]) -> QuotaCheck;
```

`EditPreview` gains `pub demand: Option<DemandChange>`: `build_preview` sets it from the stripped `fresh` (before) and the dry-run `response` (after) when both give `Some` and they differ. Quantities are not masked by any 0007 rule, so the masked trees serve too.

## `workload_demand`

| Part | Rule |
|---|---|
| Multiplier | Deployment, StatefulSet: `spec.replicas`, default 1. DaemonSet: `status.desiredNumberScheduled` of the object, default 0 (the dry-run answer keeps the status). Other kinds: `None` |
| Pod template | `spec.template.spec` |
| Per pod, per resource | `max(sum(containers) + sum(sidecar inits), max(each regular init + sidecar inits started before it))`; a sidecar init has `restartPolicy: Always` |
| Quantities | `resources.requests` / `resources.limits` `cpu` via `CpuAmount::parse`, `memory` via `ByteAmount::parse` (`quantity.rs`); missing or unparsable = 0 |
| Totals | `pods = multiplier`; each resource = per pod × multiplier, saturating |

`ponytail:` steady state only; surge pods, `spec.overhead`, and LimitRange defaults are ignored (decision 4).

## `quota_check`

1. Growth per resource: `after − before` when positive; none positive → `NotAffected`.
2. Quotas: only those with empty `scopes`. Items mapped by name: `pods`, `count/pods` → `Pods`; `requests.cpu`, `cpu` → `RequestsCpu`; `requests.memory`, `memory` → `RequestsMemory`; `limits.cpu` → `LimitsCpu`; `limits.memory` → `LimitsMemory`. Others ignored. An item without `used` (not synced) or with an unparsable quantity is skipped.
3. Per matching item with growth `g`: `left = hard − used` (saturating). `g > left` → a `QuotaShortfall { needed: g, left }`; else `left − g` is a headroom candidate.
4. Any shortfall → `Exceeds` (quota then resource order); else the smallest headroom → `Fits`; no matching item → `NotAffected`.

## App (step 3)

- Quotas: the ResourceQuotas `ConditionFeed` of the active session (`IssueFeeds`), items of the edited namespace (`KindObject::ResourceQuota`). A read-only accessor `IssueFeeds::condition(kind) -> Option<&ConditionFeed>` and `ConditionFeed::off_reason()` are added (twins of `condition_mut`, `off`).
- `PassedPreview` gains `quota: QuotaLine`, computed when the dry-run passes:

| State | Side panel line (after the Rollout check) | Tone | Dialog warning |
|---|---|---|---|
| `demand` is `None` or `NotAffected` | none | — | none |
| Feed off | `Quota not checked: {reason}` | muted | none |
| Feed loading | `Quota not checked: quotas are still loading` | muted | none |
| `Fits` | `Namespace quota OK ({left} {unit} left)` | success | none |
| `Exceeds` | per shortfall `Quota {name}: {resource} needs {needed} more, {left} left` | warning | the same texts |

- Units (`usage_format.rs` helpers): memory as binary bytes (`22Gi`), CPU as cores or millicores (`1.5 cores`, `250m`), pods as a count (`4 pods`). Resource names in shortfalls: `requests.cpu`, `requests.memory`, `limits.cpu`, `limits.memory`, `pods`.
- The line is computed once per passed preview (feeds change slowly); a later preview recomputes it. Apply is never disabled by it (decision 11).
- `--screen edit-yaml-diff` fixture: `Fits { quota: compute-quota, RequestsMemory, left: 22Gi }` so it reads like W10.
