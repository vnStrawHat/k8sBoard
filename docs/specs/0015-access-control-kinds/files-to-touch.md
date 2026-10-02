# 0015 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; nothing lands before its first user (the probe is the crate's first user in step 1). No Cargo change (`k8s_openapi::api::rbac::v1` is in `v1_32`).

## `crates/cluster` (step 1)

| File | Change |
|---|---|
| `src/service_account.rs` (new) + tests in module | `ServiceAccountSummary`, `CloudIdentity`, `CloudProvider` (allowlist of three keys), `watch_service_accounts` |
| `src/role.rs` (new) + tests in module | `RoleSummary`, `RbacRule`, `grants_everything`, `is_aggregated`, `is_built_in`, `watch_roles`, `watch_cluster_roles` |
| `src/role_binding.rs` (new) + `role_binding_tests.rs` | `BindingSummary`, `RoleRef`, `RoleKind` (with `Other`), `Subject`, `SubjectKind`, `SubjectMatch`, `BroadGroup`, `broad_group`, `binds_service_account`, `has_service_account_subject`, `watch_role_bindings`, `watch_cluster_role_bindings` |
| `src/access_review.rs` | 5 checks, `ALL`, tests |
| `src/object_yaml.rs` (+ tests), `src/object_count.rs` | 5 `ObjectKind`s and arms |
| `src/lib.rs` | modules and exports |
| `examples/probe.rs` | 5 watch lines, `USAGE` |

## `crates/app`

| S | File | Change |
|---|---|---|
| 2 | `src/resource_kind.rs` | `Roles`, `ClusterRoles`, `RoleBindings`, `ClusterRoleBindings` statics, `ALL`, `watch_rows`; cluster-scoped test |
| 3 | `src/resource_kind.rs` | `ServiceAccounts` |
| 2 | `src/kind_row.rs` | `KindObject::{Role, Binding}`, `LiveContent::{RoleBindings, RoleSubjects}` |
| 3 | `src/kind_row.rs` | `KindObject::ServiceAccount`, `LiveContent::{BoundRoles, ServiceAccountPods}` |
| 2 | `src/access_bindings.rs` (new) + tests in module | text helpers, `binding_key`, `role_key`, `BindingLists`, `BindingIndex` (`build`, `bindings_of_role`) |
| 3 | `src/access_bindings.rs` | `BoundRole`, `BindingIndex::bound_roles` (`by_account`, `by_group`) |
| 2 | `src/access_rows.rs` (new) + `access_rows_tests.rs` | `role_row`, `cluster_role_row`, `role_binding_row`, `cluster_role_binding_row`, `rules_table` |
| 3 | `src/access_rows.rs` | `service_account_row` |
| 2 | `src/cluster_session.rs` (+ tests) | `CompanionLists::Bindings`, `CompanionUpdate` and `CompanionKind` variants, `companion_plan` arms, readiness (no renames: 0012 has the general companion) |
| 2 | `src/kind_join.rs` (+ tests) | Roles and ClusterRoles arms, indices |
| 3 | `src/kind_join.rs` | ServiceAccounts arm (bound roles, used by, status) with the pods-by-account `HashMap`, pods trigger |
| 2 | `src/live_sections.rs` (+ tests) | `RoleBindings`, `RoleSubjects` |
| 3 | `src/live_sections.rs` | `BoundRoles`, `ServiceAccountPods` |
| 2 | `src/kind_diagnosis.rs` (+ tests) | `DiagnosisInputs.bindings`; VERY BROAD, REVIEW |
| 3 | `src/kind_diagnosis.rs` | CLUSTER ADMIN |
| 2 | `src/table_filter.rs`, `src/table_view.rs`, `src/kind_table.rs`, `src/node_table.rs`, `src/pod_table.rs` (+ tests) | `FilterPreset::HideSystem`, `default_filter`, `in_preset` arms |
| 2 | `src/workspace.rs` | Hide system toggle |
| 2 | `src/resource_actions.rs` (+ tests) | Go to role |
| 2, 3 | `src/main.rs` | `mod access_bindings; mod access_rows;` |

## Docs (with the last step)

- `docs/roadmap/inventory-kinds.md`: the five kinds → Done (cloud identity included); "can do" → 0023.
- `docs/roadmap/gap-plan-read-only.md` 0015 entry: "cloud identity annotations" → "cloud identity from three allowlisted keys"; "can do" moved to the 0023 entry (decision 12).
- `docs/roadmap/cross-cutting.md` UAT table: RBAC kinds probe results.
- `docs/roadmap/README.md`: status row; 0001 `probe-example.md`: watch line list.
