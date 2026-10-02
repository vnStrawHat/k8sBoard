# 0023 · Decisions

[Back to index](README.md). Architect defaults; the user asked not to stop for questions. 0013 and 0015 decisions apply unless replaced here.

## Data and evaluation

| # | Decision | Rationale |
|---|---|---|
| 1 | Both evaluators live in the **cluster crate** as pure functions over summaries (`rbac_evaluation.rs`, `network_policy_traffic.rs`); wording stays in the app | Kubernetes semantics belong to the cluster crate (0005 decision 3, 0015 decision 4); pure code gets table tests |
| 2 | **You** = SelfSubjectRulesReview (one for the picked namespace) for the table and SSAR for "can I …?" | non-mutating review objects, allowed by the project rules; the API server's own answer, covering every authorizer |
| 3 | Any other subject = **client-side RBAC evaluation**, never SubjectAccessReview | SAR needs `create subjectaccessreviews` (cluster-scoped, privileged) and impersonation is a privilege; both are out of a read-only app. The W7 note "SelfSubjectRulesReview" for an SA cannot work: SSRR answers only for the caller |
| 4 | RBAC data is **one snapshot per session** (`RbacSnapshot`: four one-shot `list`s), fetched on first need, refreshed by a button; no new watches | analysis tools are occasional; a snapshot keeps every result consistent and the 3N + 4 watch budget unchanged |
| 5 | **Partial coverage, never a failed snapshot**: a Forbidden ClusterRoles or ClusterRoleBindings list becomes an empty list plus a coverage flag; Roles and RoleBindings are listed cluster-wide, and a Forbidden answer falls back to one list per **known namespace** (the session's namespaces list, else the scope), each list tracked separately; every gap is a Warn line | a restricted account still gets the grants it can see; the scope `All` needs a real namespace list to fall back to; a narrower answer is labeled, not hidden; the per-namespace fallback lists run at most 8 at once, in fallback order |
| 6 | Aggregated ClusterRoles use **rules as listed** | the aggregation controller already fills `rules`; re-aggregating would duplicate it |
| 7 | Matching follows the authorizer: verb `*`, group `*`, resource `*`, `*/sub`, `resourceNames`, nonResourceURL `*` and trailing-`*` prefixes; RoleBindings apply only in their namespace and never to cluster-scoped or non-resource requests | the RBAC authorizer's `RuleAllows` and `appliesTo` |
| 8 | A rule limited by `resourceNames` matched by a request **without a name** is a separate "only for named objects" grant, never "allowed" | the real authorizer denies list/create there; showing it still answers "who could touch this" |
| 9 | Requests are typed as `verb resource[.group][/subresource] [name]` or `verb /url` (the placeholder spells it out); a missing group comes from a **built-in resource table** (v1.29 kinds, with namespaced flag); resource `*` means group `*` | no discovery until 0018; one input is the smallest UI; the flag drops the namespace for cluster-scoped kinds |
| 10 | ServiceAccount identity = user `system:serviceaccount:{ns}:{name}` + groups `system:serviceaccounts`, `system:serviceaccounts:{ns}`, `system:authenticated`; a user = name + `system:authenticated` (`system:anonymous` gets `system:unauthenticated` instead); a group = that group only | the token authenticator's identity; users' other groups are unknowable (open item 2) |
| 11 | Who-can always shows a fixed muted row **group system:masters · always allowed (bypasses RBAC)** | the superuser group short-circuits authorization; leaving it out would understate access |
| 12 | NetworkPolicy evaluation: a direction is isolated when any policy of the pod's namespace selects it for that direction; then the union of rules decides; traffic is allowed only when the source's egress and the destination's ingress both allow | NetworkPolicy v1 semantics |
| 13 | Namespace labels come from the always-on namespaces list; ipBlock is matched against pod IPs (primary `pod_ip`); named ports resolve against the **destination** pod's container ports | the session already has both; named ports always name the destination's port |
| 14 | Policies are fetched per check (`read_network_policies` for the 1–2 namespaces involved), not from the NetworkPolicies screen watch | the tool also opens from other screens and needs namespaces outside the screen's list |

## App

| # | Decision | Rationale |
|---|---|---|
| 15 | The three tools are kit **`Dialog`s** (`window.open_dialog`, 760 px) holding one view entity each, created once in `open_*` before the dialog opens (the per-frame builder only clones it); opened from W7 top buttons and drawer menus | no wireframe for the tool body; a modal leaves the drawer and table intact; `gpui_kit::open_window` already installs the kit `Root` |
| 16 | **Topology RBAC chip stays disabled** (0022 decision 28) | W11 shows it ghosted with no RBAC nodes or layout; Who can and Can do cover the need (YAGNI) |
| 17 | Can do chips: the account's own namespace plus cluster-wide grants, one chip per resource row (`get, list pods`), wildcard row = Warn chip `everything`, max 12 then `+{n} more`; grants that reach it only through `system:authenticated` are left out of the chips, kept in the dialog table | W7 "Can do" chips; basic-user review grants on every account are noise |
| 18 | Entry points: Roles/ClusterRoles top **Who can…** (before Hide system) and first drawer menu item; ServiceAccounts top and menu **Check permissions**; NetworkPolicies top **Test traffic** and menu **Test traffic…** | W7 `top` and `acts`; replaces 0013 decision 19 and 0015 decision 20 for these items |
| 19 | Prefills: Role/ClusterRole menu → first resource rule (`{verb} {resource}[.{group}]`, role namespace or cluster-wide); SA top button → the open drawer's account, else You; policy menu → destination = first pod the policy selects | the obvious next question from each place |
| 20 | Result links (binding, role, service account, pod) close the dialog and `reveal` the target | 0012 navigation; no second navigation path |
| 21 | Screens: `who-can`, `check-permissions`, `account-permissions`, `test-traffic` | ui-verifier needs deterministic dialogs |
| 22 | `decide` and `rules_of` do not model `system:masters` (it bypasses RBAC); `permissions_view` (step 3) **short-circuits**: an identity in that group (a `group system:masters` query) is always allowed, with the Who-can fixed-row wording, and shows no rules table | the evaluator only reads bindings; an empty result for that group would wrongly say denied |

## Known ceilings

- Snapshot staleness: results describe the objects listed at the shown time until Refresh.
- `who_can` and `rules_of` run synchronously on the GPUI thread over an immutable `Rc<RbacSnapshot>`: O(bindings × rules) per Check (`ponytail:` note in code; move to the runtime if a large cluster stalls a frame).
- Named ports in egress rules resolve on the destination pod; when one decides the result the dialog says so (some plugins differ).
- Not modeled: other authorizers (Node, webhook, ABAC), admission, impersonation, users' authenticator groups, `namespaces/<ns>` self-access; CNI policies, AdminNetworkPolicy, host-network pods (warned), dual-stack secondary IPs, Service → pod translation.
- Pods in the traffic pickers come from the session scope only.

## UAT probe (coder-lite fills after steps 1a and 1b)

| Check | Result |
|---|---|
| `read_rbac` coverage (cluster roles, cluster bindings, roles, role bindings) and counts (1a) | all four listed (roles and role bindings `all`); roles 28, cluster roles 95, role bindings 31, cluster role bindings 82 |
| `review_rules` (default namespace): rule count, `incomplete` (1a) | 7 rules, not incomplete |
| `who_can(get secrets, default)` grant count, one per subject and binding (1a) | 22 grants (0 only named); kube-system 25 (1 only named) |
| `read_network_policies` count per probed namespace (1b) | argocd 7, goauthentik 1, default 0, kube-system 0 (8 policies cluster-wide, matching the watch) |
