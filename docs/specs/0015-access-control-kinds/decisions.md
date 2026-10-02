# 0015 · Decisions

[Back to index](README.md). Architect defaults; the user asked not to stop for questions. 0005, 0012, 0013, and 0014 decisions apply unless replaced here.

## Data

| # | Decision | Rationale |
|---|---|---|
| 1 | Three modules: `service_account.rs`, `role.rs` (Role and ClusterRole), `role_binding.rs` (RoleBinding and ClusterRoleBinding); one summary type per pair with `namespace: Option<String>` | the pairs share every field; ClusterRole adds only aggregation |
| 2 | `ServiceAccountSummary` keeps `secrets[].name`, `imagePullSecrets[].name`, `automountServiceAccountToken`, and cloud identities. No Secret is ever requested | secret safety (C1): the SA object holds references only; token values live in Secrets this spec never reads |
| 3 | Roles keep their rules in full (groups, resources, resource names, verbs, non-resource URLs) | rules are not secret (user note); resource names are object names |
| 4 | RBAC matching rules live in the cluster crate: `Subject` namespace defaulting (an SA subject of a RoleBinding without a namespace means the binding's namespace), `BindingSummary::binds_service_account` (direct subject, or groups `system:serviceaccounts` and `system:serviceaccounts:{ns}`), and `Subject::broad_group` | Kubernetes semantics stay in the cluster crate (0005 decision 3); the first two are the authorizer's `appliesTo` rule |
| 5 | `system:authenticated` and `system:unauthenticated` do not count as binding one service account (`binds_service_account`, `bound_roles`), but they do count as broad subjects for REVIEW. In the app, `BindingIndex::everyone_roles` keeps the `system:authenticated` bindings apart, and a service account **holds** (`roles_held`: Bound roles list, cell, status, CLUSTER ADMIN box) those that are cluster-admin, listed last as `· group system:authenticated`. `system:unauthenticated` never reaches an account | they bind everyone; listing `system:basic-user` on every SA is noise, while cluster-admin to everyone is the worst case and every service account holds it |
| 6 | "Full access" = a rule with verb `*` on resource `*` in group `*` and no `resourceNames` (`RbacRule::grants_everything`) | the cluster-admin shape; W7 "1 (all)"; named objects narrow a wildcard rule. Roles that reach full control through `escalate`, `bind`, or `impersonate` are not flagged: that is rule evaluation, which goes to 0023 |
| 7 | Built-in = label `kubernetes.io/bootstrapping=rbac-defaults` (a label, not an annotation) | the API server marks its default roles this way |
| 8 | 5 list `AccessCheck`s (ServiceAccounts, Roles, RoleBindings namespaced; ClusterRoles, ClusterRoleBindings cluster-scoped) | 0005 decision 8; UAT RBAC unknown, the probe records it first |
| 9 | `roleRef.kind` maps explicitly: `Role`, `ClusterRole`, else `RoleKind::Other(String)` (no link, shown as written) | the API rejects other kinds today; an explicit arm keeps a surprise visible instead of guessing |

## App

| # | Decision | Rationale |
|---|---|---|
| 10 | One **Bindings** companion (0014 `CompanionLists`): RoleBindings (session scope) for Roles; RoleBindings plus ClusterRoleBindings for ClusterRoles and ServiceAccounts; each list gated on its own access check | binding counts, bound roles, and the two-way links need every binding of the screen; companions live only while their kind is shown |
| 11 | Joined cells read one `BindingIndex` and one pods-by-account `HashMap`, each built once per join call; drawers reuse the same pure helpers | O(bindings + pods + rows) per join instead of a scan per row |
| 12 | "Can do" (W7 SA section) moves to 0023 with "Check permissions" (user decision) | it is rule evaluation across bound roles, the same engine as "Who can…" |
| 13 | **Cloud identity kept** (user decision): the summarizer reads only `eks.amazonaws.com/role-arn`, `iam.gke.io/gcp-service-account`, `azure.workload.identity/client-id`; every other annotation is ignored | identifiers (role ARN, GCP SA email, client id), not credentials; an allowlist keeps the "never read annotations" rule for everything else |
| 14 | cluster-admin is detected **by name** on bindings and service accounts (`ClusterRole/cluster-admin`); a wildcard role is detected by its rules on the role itself | bindings carry only the role name (README open item 1) |
| 15 | REVIEW fires for cluster-admin given to a service account or a broad group (`system:serviceaccounts`, `system:serviceaccounts:{ns}`, `system:authenticated`, `system:unauthenticated`); Bad for the last two, else Warn | a group can hand full access to every pod or every caller |
| 16 | **Hide system** (`FilterPreset::HideSystem`: hides names starting `system:`, except rows whose status is Bad, which always stay visible) on ClusterRoles and ClusterRoleBindings, on by default | W7 list buttons; ~70 `system:` objects on a stock cluster; same pattern as Hide inactive (0009 decision 26) |
| 17 | Links: binding → role and service-account subjects; Role → its RoleBindings; ClusterRole → its bindings; ServiceAccount → bound roles and pods. Menus: bindings gain **Go to role** | W7 "two-way links"; `go_to_item` (0013) |
| 18 | Column text follows W7: RoleBindings "Role" as `Role/x`; Bound roles as `role/x, clusterrole/y`; subjects as `sa {ns}/{name}`, `user {name}`, `group {name}` | the wireframe; one subject format everywhere |
| 19 | Rules render as an aligned `Code` table (`apiGroups resources verbs`), at most 200 rules | W7 Roles drawer; long aggregated roles stay bounded |
| 20 | Menus: no read-only actions besides Delete; no list-level top buttons except Hide system and those 0023 adds (Who can…, Check permissions); the W7 `meta` line is not rendered | 0005 decision 22; "Who can…", "Check permissions" are 0023 (shipped there: top buttons and first menu items, 0023 decision 18; "Can do" too) |
| 21 | Screenshot screens: `<plural>` and `<plural>-drawer` for the five kinds; `--filter cluster-admin` for `clusterroles-drawer` and `clusterrolebindings-drawer`; `--filter default` for `serviceaccounts-drawer` | deterministic rows that exist on every cluster |

## Known ceilings

- Joins rerun on every pods batch while ServiceAccounts is shown, and on every bindings snapshot (0012 ceiling).
- **Scope**: RoleBindings come from the session scope, so a ClusterRole's binding count and Bound to list, a ServiceAccount's Bound roles, and CLUSTER ADMIN detection miss RoleBindings in other namespaces; drawers say "Role bindings from {scope}".
- cluster-admin by name misses equivalent custom roles on bindings (decision 14).
- The rules `Code` block wraps lines wider than the drawer, which breaks column alignment for very long group or resource lists (cells are cut at 40 chars to limit it).

## UAT probe (coder-lite fills after step 1)

| Check | Result |
|---|---|
| `list serviceaccounts` / watch line / count | allowed / `watch service accounts: 1 snapshots, last 98 items, 0 failures` / 98 |
| `list roles` / watch line / count | allowed / `watch roles: 1 snapshots, last 28 items, 0 failures` / 28 |
| `list clusterroles` / watch line / count | allowed / `watch cluster roles: 1 snapshots, last 95 items, 0 failures` / 95 |
| `list rolebindings` / watch line / count | allowed / `watch role bindings: 1 snapshots, last 31 items, 0 failures` / 31 |
| `list clusterrolebindings` / watch line / count | allowed / `watch cluster role bindings: 1 snapshots, last 82 items, 0 failures` / 82 |

AC4 credential check: the 0001 AC7 script was not run, because auto mode blocks token extraction from the kubeconfig. It is covered instead by `kubeconfig_tests::debug_output_never_contains_credentials` and `tests/connection.rs::connection_debug_hides_credentials`, and by a code audit: the new probe lines print only counts and error summaries through `tally_source`, and the new summaries never copy Secret data: a service account keeps secret and image-pull-secret names only and the three allowlisted cloud-identity annotation values (`service_account_summary_keeps_names_only`, `cloud_identity_reads_only_allowlisted_keys`, `other_annotations_are_ignored`); no code path requests a Secret.
