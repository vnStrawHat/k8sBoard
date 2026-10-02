# 0013 · Decisions

[Back to index](README.md). Architect defaults; the user asked not to stop for questions. 0005 and 0012 decisions apply unless replaced here.

## Data

| # | Decision | Rationale |
|---|---|---|
| 1 | One module and one `watch_<kind>s(scope)` per kind, as 0005 decision 5; HPAs read `autoscaling/v2`, PDBs `policy/v1`, NetworkPolicies `networking.k8s.io/v1` | stable versions on 1.29 (UAT) and on every supported cluster |
| 2 | Selectors use 0012's `cluster::Selector` (typed, `matchExpressions`, binary search on key-ordered labels); no second matcher | one label matcher in the project (amended 0012) |
| 3 | `PolicyDirection { NotIsolated, Allowed(Vec<PolicyRule>) }` per direction, computed with the API defaulting rule | "deny all" (`Allowed(vec![])`) and "not restricted" become different states, not a missing list |
| 4 | HPA target reuses `ControllerRef` (kind, name) | same shape; `ResourceKey::of_owner` (0012) builds the link with no new code |
| 5 | One `MetricValue { Utilization, AverageValue, Value }` for both target and current; currents match by source type, name, and container/object | one shape to compare; the API does not promise equal order in `currentMetrics` |
| 6 | `cluster::quantity_ratio(numerator, denominator) -> Option<f64>` on 0010's private exact parser | quotas and HPA value metrics compare quantities of any unit (`9k / 1k`, `180Gi / 192Gi`) |
| 7 | `DisruptionState { NoPods, Allowed(n), Blocked(SyncFailed \| UnhealthyPods \| NoRoom) }` is computed in the cluster crate from PDB status; `SyncFailed` first | 0034's drain preview needs the same rule; a budget the controller cannot compute refuses every eviction |
| 8 | Quota "Blocked creations" = Warning `FailedCreate` events of the namespace whose message names the quota; one drawer-scoped related watch (`watch_failed_creates`) | a quota rejects at admission, so the blocked pods never exist; only controllers record the failure as an event |
| 9 | Namespace drawer **Quota** section from a related watch of that namespace's quotas | W7 Namespace drawer; inventory "quota section 0013"; one small server-scoped list |
| 10 | 4 list `AccessCheck`s (0005 decision 8); denied kinds disabled by the existing `kind_availability` | same rule as every kind |
| 11 | No annotation is read by any new summary | user rule; none of the four kinds needs one |

## App

| # | Decision | Rationale |
|---|---|---|
| 12 | Affects (NetworkPolicies) is a cross-list cell joined on the main thread (0012 decision 18); Selected pods (PDBs) is paint-time Live content | pods live on the main thread; no second pods watch |
| 13 | Bars, toned quantity cells, and `kind_diagnosis.rs` come from the amended 0012; this spec renames nothing | no churn in 0012 code |
| 14 | No PDB "Show selected pods" menu item; the drawer's Selected pods section lists them with click-through | the Pods filter has no namespace chip and no `in`/`notin` label queries; the section covers the need |
| 15 | HPA `ScalingActive` false with reason `ScalingDisabled` is Done "Scaled to zero" with no box; WHY titles follow the condition reason (`FailedGet…Metric`, `InvalidMetricSourceType` → METRICS UNAVAILABLE; `FailedGetScale`, `FailedUpdateScale` → CANNOT SCALE) | a target scaled to 0 by hand is intended; the reason says what failed, not the condition type |
| 16 | Quota tone: ≥ 100 % Bad, ≥ 90 % Warn, else none (`quota_tone`); `hugepages-*` quantities are bytes | a quota blocks only at 100 %; W7 colors 92 % as warning; hugepages are byte quantities |
| 17 | HPA "at max" = condition `ScalingLimited` true with reason `TooManyReplicas`; Bad in Replicas, Metrics, status, and the AT MAX box | the controller's own verdict that the metrics want more than `maxReplicas` (W7 note) |
| 18 | NetworkPolicy peers become sentences in the app (`namespace ingress`, `pods app=worker in all namespaces`, `10.0.0.0/8 except 10.1.0.0/16`) | wording is app-side (0005 decision 3) |
| 19 | Menus: W7 mutating items disabled "Read-only mode"; HPAs gain **Go to target**; Test traffic, Show in Topology, Edit YAML omitted. No list-level top buttons; the W7 drawer `meta` line is not rendered | 0005 decision 22; Test traffic and Topology are later specs; the subtitle stays 0005's |
| 20 | Kubectl columns where W7 matches kubectl; Age is kept last on every kind | the `every_kind_ends_with_a_right_aligned_age_column` invariant |
| 21 | Screenshot screens: `<plural>` and `<plural>-drawer` per kind plus `namespaces-drawer`; `--filter` picks a row with data; empty-state shots when UAT has none | 0005 decision 25; 0012 decision 29 |

## Known ceilings

- Affects reruns on every pods batch while NetworkPolicies is shown (0012 ceiling).
- Scaling events and Blocked creations show only events the API server still keeps (about one hour by default) and that fit under the 0006 event cap; pods created directly (not by a controller) leave no event.
- Quota usage is the quota controller's `status.used`, which can lag new pods by its resync period.
- Healthy pods are judged by the Ready condition only; the PDB controller also skips terminating pods (the status counts stay authoritative). A stale `observedGeneration` is shown as a note, not hidden.

## UAT probe (coder-lite fills after step 1)

| Check | Result |
|---|---|
| `list networkpolicies` / watch line / count | |
| `list horizontalpodautoscalers` / watch line / count | |
| `list resourcequotas` / watch line / count | |
| `list poddisruptionbudgets` / watch line / count | |
