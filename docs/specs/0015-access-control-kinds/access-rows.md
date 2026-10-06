# 0015 · Access rows: columns, status, sections, live content, boxes

[Back to index](README.md) · Module: `access_rows.rs` (new) + `access_rows_tests.rs`; `live_sections.rs`, `kind_diagnosis.rs`. Common rules (Name first, Age last, `Absent` = "—") are 0005's. Helpers (`role_text`, `subject_text`, `binding_text`) are in [app-model.md](app-model.md).

## Columns and status (widths px; `r` right-aligned; `[j]` = joined)

| Kind (S) | Columns after Name | Status (first match) |
|---|---|---|
| Roles (2) | Rules 70 r · Bindings 90 r [j] · Age | grants everything → Warn "Full access"; 0 rules → Warn "No rules"; else Ok "{n} rules" ("1 rule") |
| ClusterRoles (2) | Rules 90 r · Aggregated 100 · Bindings 90 r [j] · Age | grants everything → Warn "Full access"; aggregated → Ok "Aggregated · {n} rules"; 0 rules → Warn "No rules"; else Ok "{n} rules" |
| RoleBindings (2) | Role 220 · Subjects 300 · Age | broad admin (below) → Bad "cluster-admin to everyone" for `system:authenticated` or `system:unauthenticated`, else Warn "cluster-admin to service accounts"; no subjects → Warn "No subjects"; else Ok "{n} subjects" ("1 subject") |
| ClusterRoleBindings (2) | ClusterRole 200 · Subjects 300 · Age | same as RoleBindings |
| ServiceAccounts (3) | Bound roles 280 [j] · Used by 90 r [j] · Age | builder Ok "Service account"; join: bound to `clusterrole/cluster-admin` → Warn "Cluster admin"; pods > 0 → Ok "{n} pods"; 0 → Done "No pods" |

| Cell | Rule |
|---|---|
| Rules | `Text("{n}")`, plus ` (all)` when the role grants everything |
| Aggregated | `Yes` / `No` |
| Bindings [j] | `Quantity { n, n, tone }`: Warn when the role grants everything and n > 0, else `None`; companion not ready or denied → `Absent` |
| Role (RoleBindings) | `Text("{RoleKind}/{name}")`, e.g. `Role/api-reader`, `ClusterRole/view`; toned as the status when "broad admin"; `Other` kinds as written |
| ClusterRole (ClusterRoleBindings) | `Text(name)`; toned under the same rule |
| Subjects | `subject_text` joined `, `; none → `Absent` |
| Bound roles [j] | unique `role_text` joined `, `; Warn-toned when it includes `clusterrole/cluster-admin`; none → `Text("—")` muted; companion not ready → `Absent` |
| Used by [j] | `Quantity { "{n} pods" ("1 pod"), n, None }`; pods not ready → `Absent` |

"Broad admin" = `role` is `ClusterRole/cluster-admin` and some subject is a ServiceAccount or has a `broad_group()` (decisions 14, 15).

## Rules table (pure, `access_rows.rs`)

```rust
pub(crate) fn rules_table(rules: &[RbacRule]) -> String;
```

- Header `apiGroups  resources  verbs`; resource rules first, columns padded to the widest cell (each cell cut at 40 chars with `…`), two spaces apart.
- Group `""` prints as `""`; lists join with `, `; resource names append ` ({names})`.
- Non-resource rules after, one line each: `nonResourceURLs: {urls}  verbs: {verbs}`.
- At most 200 rules, then `… {n} more rules`. A line wider than the drawer wraps and loses alignment (decisions ceiling).

## Sections

| Kind | Overview body |
|---|---|
| Roles | WHY · **Rules** `Code(rules_table)` or `Note("No rules")` · **Bindings** `Live(RoleBindings)` · Labels |
| ClusterRoles | WHY · **Cluster role**: Built-in (Yes/No), Aggregated (Yes/No) · **Aggregation** (only when aggregated): one `Chips(terms)` per selector, then `Note("Rules are combined from ClusterRoles with these labels")` · **Rules** · **Bound to** `Live(RoleSubjects)` · Labels |
| RoleBindings, ClusterRoleBindings | WHY · **Role**: Kind, Name (`Link` → `role_key`, else `Text`) · **Subjects**: per subject `Field { kind, Mono "{ns}/{name}" or name }`; a ServiceAccount subject is a `Link` to its ServiceAccounts row once that kind exists (step 3; `of_object` gives `None` before); none → `Note("No subjects")` · Labels |
| ServiceAccounts | WHY · **Bound roles** `Live(BoundRoles)` · **Cloud identity** (only when `cloud_identities` is non-empty): per entry `Field { provider.label(), Mono value }` · **Used by** `Live(ServiceAccountPods)` · **Secrets**: per `secrets` name `Field { name, "token reference" }`, per `image_pull_secrets` name `Field { name, "image pull secret" }`, Automount token (`Yes` / `No` / `Default (yes)`), then `Note("Secret contents are never read")`; no names → `Note("No secret references")` · Labels |

Secret names are plain `Mono` labels, not links (the Secrets kind is 0016).

## Live content (`live_sections.rs`)

| Content | Rows (max) | Not ready / denied / empty |
|---|---|---|
| `RoleBindings` (Roles) | `BindingIndex::bindings_of_role`, by name: `binding_text` link (mono) · muted `→ {subjects joined}` (50) | "Loading bindings…" / "Not permitted: list rolebindings" / "Not bound" |
| `RoleSubjects` (ClusterRoles) | for each binding of the role, each subject: `subject_text` · muted `via {binding_text}`; the row reveals the binding (`binding_key`); Warn "review" after ServiceAccount subjects when the role grants everything; ordered service accounts first, then text (50). Then muted "Role bindings from {scope}" | same, plus "Not permitted: list clusterrolebindings" |
| `BoundRoles` (ServiceAccounts) | `BindingIndex::bound_roles`: `role_text` link (to `role_key`, plain when `None`) · muted `via {binding_text}` plus ` · group {g}` for group matches (50). Then muted "Role bindings from {scope}" | "Loading bindings…" / denial note / "No roles bound" |
| `ServiceAccountPods` | pods of the namespace using the account (None reads `default`), by name: name link · status label toned (50) | 0005 pods notes / "Not used by any pod" |

## Boxes (`kind_diagnosis.rs`; Warn unless noted)

| Title | Kind | When | Text |
|---|---|---|---|
| VERY BROAD | ClusterRoles | grants everything | `Grants every verb on every resource.` + with bindings ready: ` Bound to {n} subjects` + `, including sa {ns}/{name}` for the first service-account subject + `.` |
| VERY BROAD | Roles | grants everything | `Grants every verb on every resource in {namespace}.` |
| REVIEW | ClusterRoleBindings | broad admin, service-account subjects | one SA: `Service account {ns}/{name} has full access to the cluster. Consider a namespaced Role instead.`; k > 1: `{k} service accounts, including {ns}/{name}, have full access to the cluster.` |
| REVIEW | RoleBindings | broad admin, service-account subjects | `Service account {ns}/{name} has full access to namespace {namespace}. Consider a narrower Role.` (k > 1 as above, "namespace {namespace}") |
| REVIEW (Bad for `Authenticated`, `Unauthenticated`) | both binding kinds | broad admin, group subject (checked before the SA rows) | `Group {g} gives {who} full access to {the cluster | namespace {ns}}.` with {who}: `Authenticated` "every signed-in user and service account", `Unauthenticated` "anonymous requests", `AllServiceAccounts` "every service account", `NamespaceServiceAccounts(ns)` "every service account in {ns}" |
| CLUSTER ADMIN | ServiceAccounts | a bound role (direct or through its groups) is `clusterrole/cluster-admin` | through a ClusterRoleBinding: `This service account has full access to the cluster through {binding_text}.`; only RoleBindings: `This service account has full access to namespace {binding ns} through {binding_text}.` |

No pod link in these boxes. CLUSTER ADMIN sees only RoleBindings of the session scope plus every ClusterRoleBinding (decisions ceiling).

## Bound roles cell (walk H16)

The ServiceAccounts Bound roles cell names the account's own roles first, then folds the roles it only reaches through a group (`system:serviceaccounts`, `system:serviceaccounts:{ns}`) into `+N via group` (N distinct roles). A cluster-admin grant through a group is never folded: it is named and toned Warn. The drawer lists every role, own first, with `· group {name}` on the group ones.
