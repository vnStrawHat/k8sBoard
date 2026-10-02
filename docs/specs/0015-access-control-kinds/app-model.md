# 0015 · App: kind specs, Bindings companion, binding index, joins, Hide system, menus

[Back to index](README.md) · Steps 2–3 · Modules: `resource_kind.rs`, `kind_row.rs`, `access_bindings.rs` (new, pure), `kind_join.rs`, `live_sections.rs`, `kind_diagnosis.rs`, `cluster_session.rs`, `table_filter.rs`, `table_view.rs`, `kind_table.rs`, `workspace.rs`, `resource_actions.rs`. Anything not listed works unchanged for a new `ResourceKind` (0013 [app-model.md](../0013-policy-kinds/app-model.md)).

## `KindSpec` statics

| Variant (S) | label | object | singular / plural | badge | ns | access | delete label |
|---|---|---|---|---|---|---|---|
| `Roles` (2) | Roles | Role | role / roles | Ro | yes | `ListRoles` | Delete role… |
| `ClusterRoles` (2) | ClusterRoles | ClusterRole | clusterrole / clusterroles | Cr | no | `ListClusterRoles` | Delete cluster role… |
| `RoleBindings` (2) | RoleBindings | RoleBinding | rolebinding / rolebindings | Rb | yes | `ListRoleBindings` | Delete binding… |
| `ClusterRoleBindings` (2) | ClusterRoleBindings | ClusterRoleBinding | clusterrolebinding / clusterrolebindings | Cb | no | `ListClusterRoleBindings` | Delete binding… |
| `ServiceAccounts` (3) | ServiceAccounts | ServiceAccount | serviceaccount / serviceaccounts | Sa | yes | `ListServiceAccounts` | Delete service account… |

No read-only actions (decision 20). `cluster_scoped_kinds_are_listed` (0014) adds ClusterRoles and ClusterRoleBindings.

**W7 parts not rendered**: the drawer `meta` line stays 0005's subtitle. "built-in" is shown as a field from the bootstrapping **label**; "created by ci-bootstrap" (CRB), "IRSA enabled" as meta, and the "built-in" tag on a Bound to row (`group system:masters`) need annotations or managedFields and are not derivable, so they are not shown. No top buttons except Hide system ("Who can…", "Check permissions" are 0023; "New" is 0031).

## Row model (`kind_row.rs`)

- `KindObject::{Role(RoleSummary), Binding(BindingSummary)}` (step 2; shared by each pair), `ServiceAccount(ServiceAccountSummary)` (step 3).
- `LiveContent::{RoleBindings, RoleSubjects}` (step 2), `{BoundRoles, ServiceAccountPods}` (step 3).

## Binding index (`access_bindings.rs`, pure, tests in module)

```rust
pub(crate) fn role_text(role: &RoleRef) -> String;              // "role/x" | "clusterrole/x" | "{kind lowercased}/x"
pub(crate) fn subject_text(subject: &Subject) -> String;        // "sa ns/name" | "user x" | "group y"
pub(crate) fn binding_text(binding: &BindingSummary) -> String; // "rolebinding/x" | "clusterrolebinding/x"
pub(crate) fn binding_key(binding: &BindingSummary) -> ResourceKey;  // RoleBindings or ClusterRoleBindings row
pub(crate) fn role_key(binding: &BindingSummary) -> Option<ResourceKey>; // Role → (binding ns, name); ClusterRole → cluster; Other → None
pub(crate) struct BindingLists<'a> { pub(crate) role_bindings: &'a [BindingSummary], pub(crate) cluster_role_bindings: &'a [BindingSummary] }
pub(crate) struct BoundRole { pub(crate) role: RoleRef, pub(crate) role_key: Option<ResourceKey>,
    pub(crate) binding: ResourceKey, pub(crate) binding_text: String, pub(crate) group: Option<String>,
    pub(crate) is_cluster_wide: bool /* through a ClusterRoleBinding */ }
/// Built once per join call or Live render (decision 11).
pub(crate) struct BindingIndex<'a> {
    by_role: HashMap<(RoleKindKey, Option<&'a str> /* Role namespace */, &'a str), Vec<&'a BindingSummary>>,
    by_account: HashMap<(&'a str, &'a str), Vec<BoundRole>>,     // direct ServiceAccount subjects
    by_group: HashMap<&'a str, Vec<BoundRole>>,                  // "system:serviceaccounts", "system:serviceaccounts:{ns}"
}
impl<'a> BindingIndex<'a> {
    pub(crate) fn build(lists: &BindingLists<'a>) -> Self;       // one pass over both lists
    pub(crate) fn bindings_of_role(&self, role: &RoleSummary) -> &[&'a BindingSummary];
    pub(crate) fn bound_roles(&self, namespace: &str, name: &str) -> Vec<&BoundRole>; // direct, then both groups; sorted by role text, then binding
}
```

- `by_role`: a RoleBinding naming a `Role` keys on its own namespace; any binding naming a `ClusterRole` keys on `None`; `Other` kinds are not indexed. So a Role is named only by RoleBindings of its namespace; a ClusterRole by ClusterRoleBindings and RoleBindings of any namespace in scope.
- A list that is not loaded or not watched is an empty slice; callers check readiness first.

## Bindings companion (step 2, `cluster_session.rs`)

New variants of 0012's general companion (nothing renamed):

```rust
pub(crate) enum CompanionLists { /* … */
    Bindings { role_bindings: Option<LiveList<BindingSummary>>, cluster_role_bindings: Option<LiveList<BindingSummary>> } }
enum CompanionUpdate { /* … */ RoleBindings(WatchUpdate<BindingSummary>), ClusterRoleBindings(WatchUpdate<BindingSummary>) }
pub(crate) enum CompanionKind { /* … */ Bindings { with_cluster_role_bindings: bool } }
```

| Explorer kind | `role_bindings` | `cluster_role_bindings` |
|---|---|---|
| Roles | `watch_role_bindings(scope)` | `None` (not needed) |
| ClusterRoles, ServiceAccounts | `watch_role_bindings(scope)` | `watch_cluster_role_bindings()` |

- Each list is `None` when its check is `Known` denied; `companion_plan` returns `Denied` only when every needed list is denied. The two streams merge with `futures::stream::select` into one subscription.
- Ready = every needed list is `Some` and `Ready`; a denied list gives "—" cells and the note "Not permitted: list rolebindings" (or clusterrolebindings).
- `OpenWatches.companion` = N (+ 1 with ClusterRoleBindings). ServiceAccounts: `2 + N + N + (N + 1) + 1 = 3N + 4`, 19 at N = 5; no related subject for these kinds.

## Joins (`kind_join.rs`)

| Kind (S) | Columns | Built once per call | Triggers |
|---|---|---|---|
| Roles (2) | `ROLE_BINDINGS` | `BindingIndex` | explorer, companion |
| ClusterRoles (2) | `CLUSTER_ROLE_BINDINGS` | `BindingIndex` | explorer, companion |
| ServiceAccounts (3) | `ACCOUNT_BOUND_ROLES`, `ACCOUNT_USED_BY` + status | `BindingIndex`; `HashMap<(&str ns, &str account), usize>` from pods (`service_account` None reads `default`) | explorer, companion, pods |

Each row is then lookups only. Bound roles needs the companion ready; Used by needs pods ready; each falls back to `Absent` alone. Cells and status: [access-rows.md](access-rows.md).

## Hide system (step 2; `table_filter.rs`, `table_view.rs`, `kind_table.rs`, `workspace.rs`)

- `FilterPreset::HideSystem`; `KindRow::in_preset(HideSystem)` = `!row.name.starts_with("system:")`; Pods and Nodes rows return true.
- `default_filter`: ClusterRoles and ClusterRoleBindings start with `preset: Some(HideSystem)`.
- `render_header_actions`: those two screens get `toggle_button("hide-system", "Hide system", is_on)`, tooltip "Hide objects named system:*", built like Hide inactive. `Clear filters` turns it off; `reveal` clears it when it hides the target (0012 decision 27).

## Diagnosis and menus

- `DiagnosisInputs` gains `bindings: Option<&BindingIndex>` (ready companion only). Arms: Role and ClusterRole **VERY BROAD**, bindings **REVIEW** (step 2), ServiceAccount **CLUSTER ADMIN** (step 3). Texts in [access-rows.md](access-rows.md).
- RoleBindings and ClusterRoleBindings: **Go to role** after View YAML via `go_to_item("Go to role", role_key(binding), "No screen for this role")` (step 2).
