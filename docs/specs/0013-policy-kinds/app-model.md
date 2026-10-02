# 0013 · App: kind specs, row model, joins, related subjects, menus

[Back to index](README.md) · Steps 2–3 · Modules: `resource_kind.rs`, `kind_row.rs`, `kind_table.rs`, `kind_drawer.rs`, `kind_diagnosis.rs`, `kind_join.rs`, `live_sections.rs`, `related_objects.rs`, `cluster_session.rs`, `resource_actions.rs`. Everything not listed works unchanged for a new `ResourceKind` (sidebar gating, counts, Events and YAML tabs, toolkit, screenshots).

## `KindSpec` statics (`resource_kind.rs`)

| Variant (S) | label | object | singular / plural | badge | access | read-only actions | delete label |
|---|---|---|---|---|---|---|---|
| `NetworkPolicies` (2) | NetworkPolicies | NetworkPolicy | networkpolicy / networkpolicies | Np | `ListNetworkPolicies` | — | Delete policy… |
| `PodDisruptionBudgets` (2) | PDBs | PodDisruptionBudget | poddisruptionbudget / poddisruptionbudgets | Pd | `ListPodDisruptionBudgets` | — | Delete PDB… |
| `HorizontalPodAutoscalers` (3) | HPAs | HorizontalPodAutoscaler | horizontalpodautoscaler / horizontalpodautoscalers | Hp | `ListHorizontalPodAutoscalers` | Edit min / max… | Delete HPA… |
| `ResourceQuotas` (3) | ResourceQuotas | ResourceQuota | resourcequota / resourcequotas | Rq | `ListResourceQuotas` | Edit | Delete quota… |

All namespaced, `NameColumn::Flexible`, `has_labels: true`, no port-forward. Appended to `ALL` in that order. `watch_rows` arms: `rows(update, network_policy_row)` and so on. Columns are in the two kind files.

## Row model (`kind_row.rs`)

```rust
pub(crate) enum KindObject { /* 0012 … */ NetworkPolicy(NetworkPolicySummary), PodDisruptionBudget(PodDisruptionBudgetSummary),
    HorizontalPodAutoscaler(HorizontalPodAutoscalerSummary), ResourceQuota(ResourceQuotaSummary) }
pub(crate) enum LiveContent { /* 0012 … */ SelectedPods /* 2 */, ScalingEvents, BlockedCreations, NamespaceQuotas /* 3 */ }
```

Bars, toned quantity cells, and `percent` are 0012's (`DetailRow::Bar`, `KindCell::Quantity { tone }`); nothing in the row model is renamed.

## W7 parts not rendered

- The W7 drawer `meta` line (e.g. "web · targets deployment/frontend") is not rendered: the subtitle stays `status · namespace · created` (0005); the same facts appear as section fields.
- No list-level top buttons: Test traffic (0023), Edit limits and New (0031–0032) are not shown (inventory note: only read-only list buttons ship with a kind).

## Diagnosis (`kind_diagnosis.rs`, 0012)

`kind_diagnosis(object, inputs)` gains arms: PDB **BLOCKS DRAIN** (step 2), HPA **METRICS UNAVAILABLE / CANNOT SCALE / AT MAX REPLICAS** and quota **AT QUOTA** (step 3). Rules in the kind files. These rules read only the object, so they fire while pods load.

## Joins (`kind_join.rs`, step 2)

`join_rows` arm `NetworkPolicies`: index pods by namespace once (0012), then per row `affected = pods_in_ns.filter(|p| policy.pod_selector.matches(&p.labels)).count()`. Writes column `NETWORK_POLICY_AFFECTS` and the status ([network-policy-and-pdb.md](network-policy-and-pdb.md)). Pods not ready → `Absent` and the builder status. Triggered by explorer and pods snapshots (0012 trigger list gains NetworkPolicies).

## Live content (`live_sections.rs`)

| Content (S) | Source |
|---|---|
| `SelectedPods` (2) | `live.pods` of the PDB namespace matching `selector` |
| `ScalingEvents` (3) | `live.object_events` of the drawer (0006), reason `SuccessfulRescale` |
| `BlockedCreations` (3) | related `Events` list, message names the quota |
| `NamespaceQuotas` (3) | related `ResourceQuotas` list |

Rows and empty texts are in the kind files.

## Related subjects (step 3; `related_objects.rs`, `cluster_session.rs`)

```rust
pub(crate) enum RelatedSubject { /* 0012 … */
    QuotaRejections { namespace: String, quota: String },   // ResourceQuotas row
    NamespaceQuotas { namespace: String },                  // Namespaces row
}
pub(crate) enum RelatedList { /* … */ Events(LiveList<EventSummary>), ResourceQuotas(LiveList<ResourceQuotaSummary>) }
```

| Subject | Stream | Gate |
|---|---|---|
| `QuotaRejections` | `watch_failed_creates(namespace)` | none when `ListEvents` is `Known` denied; the section says "Not permitted: list events" |
| `NamespaceQuotas` | `watch_resource_quotas(NamespaceScope::Named(ns))` | none when `ListResourceQuotas` is `Known` denied; "Not permitted: list resourcequotas" |

`related_subject` builds `QuotaRejections` from the row's namespace and name, and `NamespaceQuotas` from the Namespaces row's name (its object stays `Plain`). Same lifecycle, debounce, and settle as 0012 (`follow_drawer_subjects`). `RelatedUpdate` gains the two variants. The 0012 watch bound (`3N + 4`) holds: these kinds have no companion.

## Menus (`resource_actions.rs`)

- `kind_menu` unchanged for NetworkPolicies, PDBs, ResourceQuotas (read-only actions from the spec table).
- HPAs: **Go to target** after View YAML. `ResourceKey::of_owner(namespace, &target)`; `None` → disabled "No screen for {kind}". It reuses 0012's Go to owner item builder, generalized to `go_to_item(label, Option<ResourceKey>, reason, shell)`.
