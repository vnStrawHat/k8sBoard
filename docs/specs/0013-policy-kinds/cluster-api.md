# 0013 · Cluster crate: four kinds, access, probe

[Back to index](README.md) · Step 1 · The 0001/0002 rules apply: summaries only, no kube types in public signatures, no spawned task. Common fields (`namespace`, `name`, `created_at`, `labels`) as in 0005; annotations are never read. Selectors use 0012's `cluster::Selector` (`Selector::of`, `matches`, `terms`, `selects_everything`).

## NetworkPolicy (`network_policy.rs`)

```rust
pub struct NetworkPolicySummary { /* common */ pub pod_selector: Selector,
    pub ingress: PolicyDirection, pub egress: PolicyDirection }
pub enum PolicyDirection { NotIsolated, Allowed(Vec<PolicyRule>) } // Allowed(empty) = deny all
pub struct PolicyRule { pub peers: Vec<PolicyPeer> /* empty = any peer */, pub ports: Vec<PolicyPort> /* empty = all ports */ }
pub enum PolicyPeer {
    /// `namespaces: None` = the policy's own namespace; `pods: None` = every pod there.
    Pods { namespaces: Option<Selector>, pods: Option<Selector> },
    IpBlock { cidr: String, except: Vec<String> },
}
pub struct PolicyPort { pub protocol: String /* TCP default */, pub port: Option<String> /* number or name */, pub end_port: Option<u16> }
```

- `spec.podSelector` absent → empty selector (all pods).
- Isolation (API defaulting, applied when `policyTypes` is empty): Ingress always; Egress when `egress` is non-empty. A listed type with no rules → `Allowed(vec![])`; an unlisted type → `NotIsolated`.
- A peer with neither selector nor `ipBlock` is skipped. All types derive `Clone, Debug, PartialEq, Eq`.

## HPA (`autoscaler.rs`, autoscaling/v2)

```rust
pub struct HorizontalPodAutoscalerSummary { /* common */ pub target: ControllerRef, pub min_replicas: u32 /* default 1 */,
    pub max_replicas: u32, pub current_replicas: u32, pub desired_replicas: u32, pub metrics: Vec<HpaMetric>,
    pub conditions: Vec<WorkloadCondition>, pub last_scaled_at: Option<jiff::Timestamp> }
pub struct HpaMetric { pub name: String, pub source: MetricSource, pub target: MetricValue, pub current: Option<MetricValue> }
pub enum MetricSource { Resource, ContainerResource { container: String }, Pods, Object { object: String /* kind/name */ }, External }
/// One shape for a target and a current value (decision 5).
pub enum MetricValue { Utilization(u32), AverageValue(String), Value(String) }
```

- `name`: the resource name (`cpu`, `memory`) or `metric.name`. `target` from the spec target's `type` and its value; a missing value → the metric is dropped.
- `current`: the `status.currentMetrics` entry with the same source type, name, and container/object (decision 5). Its value uses the target's variant when present (`averageUtilization`, `averageValue`, `value`); else the first present in that order; none → `None`.
- Conditions use 0012 `WorkloadCondition` (with `message`): `AbleToScale`, `ScalingActive`, `ScalingLimited`.

## ResourceQuota (`resource_quota.rs`, core/v1)

```rust
pub struct ResourceQuotaSummary { /* common */ pub items: Vec<QuotaItem>, pub scopes: Vec<String> }
pub struct QuotaItem { pub resource: String, pub hard: String, pub used: Option<String> }
```

`items` from `status.hard` (falls back to `spec.hard` before the controller syncs), key order; `used` from `status.used`. `scopes` = `spec.scopes` then `scopeSelector` expressions as `{scopeName} {operator} ({values})`.

## PDB (`disruption_budget.rs`, policy/v1)

```rust
pub struct PodDisruptionBudgetSummary { /* common */ pub min_available: Option<String>, pub max_unavailable: Option<String>,
    pub selector: Option<Selector> /* None selects no pods */, pub current_healthy: u32, pub desired_healthy: u32,
    pub expected_pods: u32, pub disruptions_allowed: u32, pub unhealthy_pod_eviction_policy: Option<String>,
    pub conditions: Vec<WorkloadCondition>,
    /// `status.observedGeneration` < `metadata.generation`: the counts describe an older spec.
    pub is_status_stale: bool }
pub enum DisruptionState { NoPods, Allowed(u32), Blocked(BlockCause) }
pub enum BlockCause { SyncFailed, UnhealthyPods, NoRoom }
impl PodDisruptionBudgetSummary { pub fn disruption_state(&self) -> DisruptionState; }
```

`disruption_state`, first match:

1. condition `DisruptionAllowed` false with reason `SyncFailed` → `Blocked(SyncFailed)` (the controller could not compute the budget, so the eviction API refuses);
2. `expected_pods == 0` → `NoPods`;
3. `disruptions_allowed > 0` → `Allowed(n)`;
4. `current_healthy < expected_pods` → `Blocked(UnhealthyPods)`;
5. else `Blocked(NoRoom)` (the budget needs every pod).

`is_status_stale` is false when either generation is missing.

## Other additions

| Item | Change |
|---|---|
| `quantity.rs` (0010) | `pub fn quantity_ratio(numerator: &str, denominator: &str) -> Option<f64>`: both parse, denominator non-zero |
| `event.rs` | `watch_failed_creates(&self, namespace: &str)`: field selector `type=Warning,reason=FailedCreate`, `event_limit()`, action `watching failed creates` |
| `access_review.rs` | `ListNetworkPolicies` (networking.k8s.io), `ListHorizontalPodAutoscalers` (autoscaling), `ListResourceQuotas` (""), `ListPodDisruptionBudgets` (policy); all namespaced; appended to `ALL` |
| `object_yaml.rs` | `ObjectKind::{NetworkPolicy, HorizontalPodAutoscaler, ResourceQuota, PodDisruptionBudget}` + `name()` + `api_resource` arms |
| `object_count.rs` (0012) | the four `ObjectKind` arms |
| `lib.rs` | modules; export the summaries, `PolicyDirection`, `PolicyRule`, `PolicyPeer`, `PolicyPort`, `HpaMetric`, `MetricSource`, `MetricValue`, `QuotaItem`, `DisruptionState`, `BlockCause`, `quantity_ratio` |

## Probe

`--watch-seconds`: four lines after `endpointslices`: `network policies`, `horizontal pod autoscalers`, `resource quotas`, `pod disruption budgets`. Access and `--counts` lines grow with `ALL` and `ObjectKind`. Update `USAGE` and 0001 `probe-example.md`.
