# 0022 · RBAC layer (steps 4a, 4b)

[Back to index](README.md) · Amendment of 2026-10-03 (audit gap 8), against main `a50264c`. **Read-only: no new mutating calls**; the new feeds are `list`/`watch` through the existing `ResourceKind::watch_rows`. Wireframe: W11 toolbar chip `RBAC` (ghosted today: `topology_view.rs` `topology-chip-rbac`, disabled, `Not shown in this version`); W7 ServiceAccounts/RoleBindings/ClusterRoleBindings notes (bound roles, cluster-admin flag). Decisions 42–51 in [decisions.md](decisions.md).

## What the layer shows

Who the namespace's workloads run as, and what that grants: `workload → ServiceAccount → (Cluster)RoleBinding → (Cluster)Role`.

| Node (`TopologyKind`, new) | Feed (`TOPOLOGY_FEED_KINDS`, +4) | Drawn when |
|---|---|---|
| `ServiceAccount` | `ServiceAccounts` (namespace) | a pod of the namespace runs as it (`access_bindings::pod_account`), following the config-ref rule (decision 8) |
| `RoleBinding` | `RoleBindings` (namespace) | it names a drawn ServiceAccount as a **direct** subject |
| `ClusterRoleBinding` | `ClusterRoleBindings` (cluster-wide list) | the same |
| `Role` | `Roles` (namespace) | a drawn binding refers to it |
| `ClusterRole` | none (decision 45) | a drawn binding refers to it; `NodeLook::Plain`, never checked |

- Joins reuse `BindingIndex::build(&BindingLists { role_bindings, cluster_role_bindings })`. The feeds hold `KindRow`s, so each graph build collects **cloned** `BindingSummary` values from `KindObject::Binding` rows into two `Vec`s and borrows them as `BindingLists`; the RoleBindings come from the namespace feed, the ClusterRoleBindings only after the namespace filter below. The copy is bounded by `RAW_LIMIT`; `BindingLists` is not generalized.
- Drawn edges use `bound_roles(namespace, account)` entries with `group.is_none()`; group entries are not drawn, they only count in the caption (decision 44). The cluster-admin check reads `roles_held(namespace, account)` (`access_bindings.rs`), groups included, as the W7 boxes do.
- Captions: `ServiceAccount` (`· +{n} via groups` when group bindings reach it; `· token off` when `automount_token == Some(false)`), `RoleBinding`, `ClusterRoleBinding`, `Role · {k} rules`, `ClusterRole`.
- Keys: every node has a `ResourceKey` (all five kinds have screens). A click on a node whose row is not in the feeds (`ClusterRole`) reveals it on its screen instead of opening the drawer over the graph (`LiveCluster::row_of` is `None`).

## Edges

```rust
pub(crate) enum Relation { Owns, RoutesTo, Mounts, Access }   // + Access
```

| From → to | Relation | Aggregation |
|---|---|---|
| workload or pod → ServiceAccount | `Access` | to the top visible workload, like config refs (decision 12); a pod without a visible owner keeps its own edge |
| ServiceAccount → binding | `Access` | one per (account, binding) |
| binding → Role / ClusterRole | `Access` | one per binding |

Legend: a fourth entry `access` (`LEGEND` in `topology_canvas.rs`, the export legend too).

## Filter and feeds

- `KindFilter::Rbac` (label `RBAC`); `KindFilter::ALL` gains it; new `KindFilter::DEFAULT` = the four of today. `TopologyFilter::everything()` is renamed `TopologyFilter::initial()` and starts from `DEFAULT`: **RBAC is off by default** (decision 42).
- The toolbar loop over `KindFilter::ALL` draws the RBAC chip like the others; the disabled `topology-chip-rbac` button goes. Tooltip `Show service accounts, bindings, and roles`.
- `TopologyKind::filter()` maps the five new kinds to `Rbac`; `TopologySubject::wants` starts their feeds only while the chip is on. `open_count()` ≤ 14.
- `feed_plan` (Known and denied → Off) applies unchanged: refs of an Off or failed kind draw `Unchecked` (decision 4).
- `RAW_LIMIT` counts the RBAC rows like every feed, except that ClusterRoleBindings count **only those that name an account of the drawn namespace** (direct, or through `system:serviceaccounts` / `system:serviceaccounts:{ns}` / `system:authenticated`), filtered before counting. The full cluster-wide list stays in the feed's memory only (decision 50).
- `ponytail:` a namespace change restarts every feed (`SubjectChange::Restart`), so the cluster-wide ClusterRoleBindings list is fetched again although it did not change; upgrade path: keep that one feed across `Restart` when the RBAC chip stays on.

## Checks (`CheckRule`, +3; `topology_checks.rs`)

| Rule | When | Node | Tone | Text | Chip label (1 / n) |
|---|---|---|---|---|---|
| `MissingServiceAccount` | a pod's account is not listed (ServiceAccounts Ready) | `Missing { ServiceAccount }` | Bad | `{owner} runs as missing ServiceAccount {name}.` | `1 missing ServiceAccount` / `{n} missing ServiceAccounts` |
| `MissingRole` | a RoleBinding names a Role that is not listed (Roles Ready) | `Missing { Role }` | Warn | `RoleBinding {b} grants missing Role {r}.` | `1 binding to a missing Role` / `{n} bindings to missing Roles` |
| `ClusterAdminAccount` | `roles_held(namespace, account)` of a drawn account has an `is_cluster_admin` role, directly or through a group (`system:serviceaccounts`, `system:serviceaccounts:{ns}`, `system:authenticated`), as `broad_admin` flags it on W7 | the binding when it is drawn (direct subject), else the account | Warn | `ServiceAccount {sa} has cluster-admin through {binding}.` for a ClusterRoleBinding (cluster wide); `ServiceAccount {sa} has cluster-admin in namespace {ns} through {binding}.` for a RoleBinding to `cluster-admin` (still counted, but its reach is the namespace); a group grant adds ` (group {group})` before the period | `1 account with cluster-admin` / `{n} accounts with cluster-admin` |

`ClusterRole` refs are never checked (no feed; decision 45). The checks follow visible nodes and Problems only as today.

## Layout (`topology_layout.rs`)

- `Placement::AccessRow` (new) for the five kinds and their `Missing` ghosts: a row under the config row of the band (under the columns when the band has no config row), `CONFIG_GAP` below it.
- Slot rule (extends layout.md step 5): a ServiceAccount wants the slot of its first source's column; a binding wants its account's slot + 1; a role its binding's slot + 1. A taken slot moves right; past slot 3 the row wraps, as the config row does. With `previous`, the order is kept and new ids are appended (pin 3 holds).
- Edges into the access row run bottom center → top center like mounts; within the row, left → right.

## Colors (0022b colors.md, extended)

| Item | Token |
|---|---|
| `KindHue::Access` (all five kinds) | `cyan_light` (the last free hue; red and yellow stay reserved) |
| `Relation::Access` edge | `cyan_light`, rest alpha 0.8, `relation_stroke` width 1.5, dash (2, 3) |

The 0003 AC4 color-literal grep stays clean; the SVG export maps the same tokens (`svg_dash_per_relation` gains the fourth).

## Keys

- A node click puts the cursor (`ClusterObject` of the cluster in view) on the account, binding, or role, so `RowAction` keys work as on their tables (Y, E, Del, Copy name). No key changes.

## Not in this layer

User and Group subjects as nodes; RoleBindings in other namespaces that name this namespace's ServiceAccounts (not drawn); ClusterRole rules or aggregation; Secrets listed in `ServiceAccount.secrets`; "who can" queries (0023 owns them).
