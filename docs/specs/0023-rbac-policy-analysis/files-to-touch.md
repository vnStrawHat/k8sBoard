# 0023 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; nothing lands before its first user (the probe is the crate's first user in steps 1a and 1b). No Cargo change: `k8s_openapi::api::authorization::v1` (SSRR) is already used for SSAR; CIDR math uses `std::net`.

## `crates/cluster`

| S | File | Change |
|---|---|---|
| 1a | `src/rbac_evaluation.rs` (new) + `rbac_evaluation_tests.rs` | `AccessRequest`, `RequestTarget`, `ResourceRequest`, `Identity`, `GrantNames`, `Grant`, `EffectiveRule`, `rule_match`, `who_can`, `rules_of`, `decide` |
| 1a | `src/rbac_snapshot.rs` (new) + tests in module | `RbacSnapshot`, `RbacCoverage`, `NamespaceCoverage`, `covers`, `read_rbac(fallback)` (per-list Forbidden handling) |
| 1a | `src/role.rs`, `src/role_binding.rs` (0015) | summarizers `pub(crate)` for `read_rbac` |
| 1a | `src/access_review.rs` (+ tests) | `RulesReview`, `review_rules`, `review_request`; one attributes builder shared with `review_one` |
| 1b | `src/network_policy_traffic.rs` (new) + `network_policy_traffic_tests.rs` | traffic types, `evaluate_traffic`, `cidr_contains` |
| 1b | `src/network_policy.rs` (0013) | `read_network_policies`; summarizer `pub(crate)` |
| 1a, 1b | `src/lib.rs` | modules; export the new public types (1b: `evaluate_traffic` and its types) |
| 1a, 1b | `examples/probe.rs` | `--analysis` RBAC lines (1a), network policies line (1b); `USAGE` |

## `crates/app`

| S | File | Change |
|---|---|---|
| 2 | `src/access_query.rs` (new) + tests in module | `parse_request`, `parse_subject`, `who_can_prefill`, `BUILT_IN_RESOURCES`, errors and hints |
| 2 | `src/cluster_session.rs` (+ tests) | `RbacState`, `LiveCluster::rbac`, `request_rbac`, `refresh_rbac`, reset on scope change |
| 2 | `src/who_can_view.rs` (new) + tests in module | view, result grouping (`who_can_groups`) and `coverage_notes`, both pure |
| 2 | `src/app_shell.rs` | `open_who_can`; launch screen hook |
| 2 | `src/workspace.rs` | `Who can…` header buttons (Roles, ClusterRoles) |
| 2 | `src/resource_actions.rs` (+ tests) | `Who can…` first menu item for Roles and ClusterRoles |
| 2 | `src/launch_options.rs` (+ tests), `src/screenshot.rs` | `who-can` |
| 3 | `src/permission_table.rs` (new) + tests in module | `permission_table`, `can_do_chips` |
| 3 | `src/permissions_view.rs` (new) + tests in module | view, `RequestState`, answer text (pure helpers tested) |
| 3 | `src/kind_row.rs`, `src/access_rows.rs`, `src/live_sections.rs` (+ tests) | `LiveContent::CanDo`, section order, `can_do_rows` |
| 3 | `src/app_shell.rs` | `open_permissions`; SA drawer subject → `request_rbac` |
| 3 | `src/workspace.rs`, `src/resource_actions.rs` (+ tests) | `Check permissions` button and menu item |
| 3 | `src/launch_options.rs`, `src/screenshot.rs` | `check-permissions`, `account-permissions` |
| 4 | `src/traffic_test_view.rs` (new) + tests in module | view, `TrafficForm`, `traffic_defaults`, verdict rows (pure helpers tested) |
| 4 | `src/network_policy_rows.rs` (0013) | `peer_text`, `ports_text` → `pub(crate)` |
| 4 | `src/app_shell.rs`, `src/workspace.rs`, `src/resource_actions.rs` (+ tests) | `open_traffic_test`, `Test traffic` button, `Test traffic…` menu item |
| 4 | `src/launch_options.rs`, `src/screenshot.rs` | `test-traffic` |
| 2–4 | `src/main.rs` | `mod` lines |

## Docs (with the last step)

- `docs/roadmap/inventory-kinds.md`: NetworkPolicies Test traffic, ServiceAccounts Check permissions and Can do, Roles/ClusterRoles Who can… → Done.
- `docs/roadmap/gap-plan-read-only.md` 0023 entry: "Check permissions for a ServiceAccount" → "for You (SSRR) or any subject (client-side)".
- `docs/roadmap/README.md` status row; 0001 `README.md` AC4 grep note (SSAR and SSRR `create`); 0001 `probe-example.md` (`--analysis`).
- 0013 `decisions.md` 19 and 0015 `decisions.md` 20: note "Test traffic", "Who can…", "Check permissions" shipped in 0023.
- 0022: no change (RBAC chip stays disabled, decision 16).
