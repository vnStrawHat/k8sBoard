# 0022 · RBAC layer, as built (steps 4a, 4b)

[Back to index](README.md). Built on main `52f8f87`, branch `spec-0022-rbac`. Read-only: no new request kind and no `WriteOperation`; `crates/cluster` is untouched.

## What was built

- `topology_access.rs` (new, tests in `topology_access_tests.rs`): `names_namespace_account` (the rule that picks which ClusterRoleBindings count), `AccessBindings` (cloned `BindingSummary` values for `BindingIndex::build`; `BindingLists` unchanged), `ClusterAdminGrant`, `cluster_admin_grant` (reads `roles_held`, so groups count).
- `topology_graph.rs`: five new `TopologyKind`s appended at the end of the enum (existing node order is unchanged), `Relation::Access`, `KindFilter::Rbac` + `DEFAULT`, `TopologyFilter::initial()`, `counted_rows` for `RAW_LIMIT`, the account ref in `add_config_refs`, `add_access_edges`, `role_node`.
- `topology_checks.rs`: `MissingServiceAccount`, `MissingRole`, `ClusterAdminAccount`; coverage names for the new feeds.
- `topology_layout.rs`: `Placement::AccessRow`, `place_access_row`; `topology_route.rs`: an access edge is a curve only within a row.
- `topology_feeds.rs`: `TOPOLOGY_FEED_KINDS` 15 (ServiceAccounts, RoleBindings, Roles, ClusterRoleBindings, and ClusterRoles for the drawer of a ClusterRole node only: `TopologyKind::of_resource_kind` knows it, `feed_rows` leaves it out of the graph build). `cluster_session.rs` needed no change: `topology_feed` already starts any kind through `watch_rows`, which ignores the namespace for ClusterRoleBindings.
- `topology_view.rs`: the RBAC chip is a normal chip (the disabled button is gone), `card_click` (decision 51), `set_rbac`. `--screen topology-rbac`.
- Colors: `KindHue::Access` and `Relation::Access` use `cyan_light`; the legend and the SVG export have a fourth `access` entry.

## Deviations and choices

1. Access nodes take their band from the workload that leads to them, not from their own labels (`group: None`, then `spread_groups`). Operator-made bindings carry the operator's app label, which put the access row far from the app in the first UAT screenshot.
2. A Role caption reads `Role · 1 rule` / `Role · {k} rules` (singular for one).
3. The three access rules replace the `kind_diagnosis` boxes on access nodes (`diagnosis_check` skips them), so the chip counts exactly the rules of rbac-layer.md. The row tones (a Role that grants everything, a cluster-admin binding) still come from the row builders.
4. With the Workload chip off, an account stays drawn while a binding connects it; an account with no binding is dropped like an unreferenced config object.
5. A ServiceAccount or binding node is drawn even when its own feed is Off (as `not checked`), so the other feeds still show the chain.
6. `topology_budget` ceilings are now 500 nodes and 600 edges (measured 500 and 560 with the RBAC rows added): the layer fills `NODE_LIMIT` exactly in that fixture.

## Verification

- Gate: fmt, clippy (workspace and `--features screenshot`), `cargo test --workspace` all pass.
- UAT (`readonly@Monitor`, namespace `keda`, app debug log, counted only): per run 90 `sending request` lines = 56 SelfSubjectAccessReview POSTs + 34 GET reads, 0 PUT, 0 PATCH, 0 DELETE, no `write finished` line. Watches are not in that log.
- `keda` against the list screens: ServiceAccounts 4 (3 drawn; `default` runs no pod), RoleBindings 1 (drawn), Roles 1 (drawn), ClusterRoleBindings 82 cluster-wide (4 of them name `keda` accounts or groups, and are drawn). The click on a node and the drawer were not driven live.
- Status bar on UAT: "Watching 23 resource types" with the RBAC chip off, "27" with it on (+4).
- Screenshots: `.tmp/ui-shots/v93-topology-rbac-light.png` and `-dark.png`.
