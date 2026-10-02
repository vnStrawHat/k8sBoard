# 0013 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; nothing lands before its first user (the probe is the crate's first user in step 1). No Cargo change: k8s-openapi `v1_32` has `autoscaling::v2`, `policy::v1`, `networking::v1`; the kit has `Progress`, `Alert`. `Selector`, `DetailRow::Bar`, `KindCell::Quantity { tone }`, and `kind_diagnosis.rs` come from 0012.

## `crates/cluster` (step 1)

| File | Change |
|---|---|
| `src/network_policy.rs` (new) + tests in module | summary types, `network_policy_summary`, `watch_network_policies` |
| `src/autoscaler.rs` (new) + `autoscaler_tests.rs` | HPA summary, metric matching, `watch_horizontal_pod_autoscalers` |
| `src/resource_quota.rs` (new) + tests in module | `ResourceQuotaSummary`, `QuotaItem`, `watch_resource_quotas` |
| `src/disruption_budget.rs` (new) + tests in module | PDB summary, `DisruptionState`, `BlockCause`, `disruption_state`, `watch_pod_disruption_budgets` |
| `src/quantity.rs` | `quantity_ratio` |
| `src/event.rs` (+ `event_tests.rs`) | `watch_failed_creates`, pure `failed_create_selector()` |
| `src/access_review.rs` | 4 checks, `ALL` length, tests |
| `src/object_yaml.rs`, `src/object_count.rs` | 4 `ObjectKind` variants and arms |
| `src/lib.rs` | modules and exports ([cluster-api.md](cluster-api.md)) |
| `examples/probe.rs` | 4 watch lines, `USAGE` |

## `crates/app`

| S | File | Change |
|---|---|---|
| 2 | `src/resource_kind.rs` | `NetworkPolicies`, `PodDisruptionBudgets` statics, `ALL`, `watch_rows` arms; `only_namespaces_is_cluster_scoped` keeps passing (all four are namespaced) |
| 3 | `src/resource_kind.rs` | `HorizontalPodAutoscalers`, `ResourceQuotas` |
| 2 | `src/kind_row.rs` | `KindObject::{NetworkPolicy, PodDisruptionBudget}`, `LiveContent::SelectedPods` |
| 3 | `src/kind_row.rs` | `KindObject::{HorizontalPodAutoscaler, ResourceQuota}`, `LiveContent::{ScalingEvents, BlockedCreations, NamespaceQuotas}` |
| 2 | `src/kind_diagnosis.rs` (+ tests; 0012 module) | BLOCKS DRAIN |
| 3 | `src/kind_diagnosis.rs` | HPA and quota boxes |
| 2 | `src/network_policy_rows.rs` (new) + tests in module | `network_policy_row`, `peer_text`, `ports_text` |
| 2 | `src/policy_rows.rs` (new) + `policy_rows_tests.rs` | `pod_disruption_budget_row` |
| 3 | `src/policy_rows.rs` | `horizontal_pod_autoscaler_row`, `resource_quota_row`, metric and quota helpers |
| 2 | `src/kind_join.rs` (+ tests) | NetworkPolicies arm, `NETWORK_POLICY_AFFECTS`; join trigger list |
| 2 | `src/live_sections.rs` (+ tests) | `SelectedPods` |
| 3 | `src/live_sections.rs` | `ScalingEvents`, `BlockedCreations`, `NamespaceQuotas` |
| 3 | `src/related_objects.rs`, `src/cluster_session.rs` (+ tests) | two subjects, `RelatedList`/`RelatedUpdate` variants, access gate |
| 3 | `src/namespace_rows.rs` | Quota section |
| 3 | `src/resource_actions.rs` (+ tests) | `go_to_item`; HPA Go to target |
| 2, 3 | `src/main.rs` | `mod` lines |

## Docs (with the last step)

- `docs/roadmap/inventory-kinds.md`: NetworkPolicies (T, Dr), HPAs, ResourceQuotas, PDBs → Done; Namespaces "quota section" → Done (LimitRange open).
- `docs/roadmap/cross-cutting.md` UAT table: the four kinds' probe results.
- `docs/roadmap/README.md`: status row for 0013.
- 0001 `probe-example.md`: watch line list.
