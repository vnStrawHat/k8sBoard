# 0015 — Access-control kinds: ServiceAccounts, Roles, ClusterRoles, RoleBindings, ClusterRoleBindings (read-only)

Status: amended after advisor review (HEAD `81497ba`); architect defaults, the user asked not to stop for questions. Crates: `crates/cluster` (step 1), `crates/app` (steps 2–3). Requires the amended 0012 (`KindObject`, `Live`, joins, the general `KindList.companion`, `Selector`, `kind_diagnosis.rs`, counts, `reveal`), 0013 (`go_to_item`), and 0014 (`companion_plan` arms) merged. Wireframes: W7 `k("ServiceAccounts")`, `k("Roles")`, `k("ClusterRoles")`, `k("RoleBindings")`, `k("ClusterRoleBindings")`. Applies C1, C11.

## Goal

Five new explorer kinds on the 0005 explorer (one `KindSpec`, row builder, lazy watch, list SSAR, Events/YAML tabs, 0009 toolkit, 0012 counts):

- **Roles / ClusterRoles**: rules table, rule and binding counts, Aggregated flag and selectors, built-in flag, **VERY BROAD** box, **Hide system** (ClusterRoles);
- **RoleBindings / ClusterRoleBindings**: role and subjects, **two-way links** (binding → role and service account; role → its bindings), **REVIEW** box when cluster-admin goes to a service account or a broad group, Hide system (ClusterRoleBindings);
- **ServiceAccounts**: **Bound roles** (from bindings), **Cloud identity** (three allowlisted annotation keys), **Used by** pods, secret **names only**, automount flag, **CLUSTER ADMIN** box.

## Non-goals

"Who can…", "Check permissions", and the computed "Can do" list (0023, decision 12); any other annotation (decision 13); creating or editing anything (New, Edit YAML, Delete disabled); reading any Secret object or token; resolving whether a referenced role exists; W7 `meta` facts that need annotations or managedFields ("created by ci-bootstrap"; [app-model.md](app-model.md)).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | `crates/cluster`: `service_account.rs` (with cloud identities), `role.rs`, `role_binding.rs`, 5 `AccessCheck`s, 5 `ObjectKind`s, probe. **Run the UAT probe first** and fill [decisions.md](decisions.md) "UAT probe" | 1, 2, 3, 4 |
| 2 | App: **Roles, ClusterRoles, RoleBindings, ClusterRoleBindings**; `Bindings` companion; binding index; joins; rules table; Hide system; boxes; Go to role | 1, 2, 3, 5, 6, 7, 8 |
| 3 | App: **ServiceAccounts** (bound roles, cloud identity, used by, secrets, box); full ui-verifier run | 1, 2, 3, 5, 6, 7, 8 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale, ceilings, UAT probe table |
| [cluster-api.md](cluster-api.md) | step 1: summaries, cloud identity allowlist, RBAC subject matching, watches, access checks, probe |
| [app-model.md](app-model.md) | steps 2–3: kind specs, W7 parts not rendered, `KindObject`, Bindings companion, binding index, joins, Hide system, menus |
| [access-rows.md](access-rows.md) | steps 2–3: columns, cells, status, sections, live content, rules table, boxes |
| [files-to-touch.md](files-to-touch.md) | modules per step, doc updates |
| [test-plan.md](test-plan.md) | unit tests per step, live checks, ui-verifier checklist |

## Acceptance criteria

- [ ] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`. No `Cargo.lock` change.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline.
- [ ] 3. No kube or k8s-openapi type in a public signature; the crate spawns no task; the 0001 read-only grep finds only the SSAR `create`. **Secret safety**: `ServiceAccountSummary` holds secret names only; no new code path requests a Secret; the only annotations read are the three cloud-identity keys (review plus `service_account_summary_keeps_names_only`, `cloud_identity_reads_only_allowlisted_keys`).
- [ ] 4. On UAT the probe prints 5 new watch, access, and count lines; results are in [decisions.md](decisions.md) "UAT probe". The AC7 credential script reports 0.
- [ ] 5. On UAT each allowed kind shows live rows and a drawer with Overview, YAML, Events tabs; a denied kind is disabled with "Not permitted: list …"; a denied binding list shows "—" cells and the reason in the drawer.
- [ ] 6. On UAT: a RoleBinding's role link opens the role; the role's Bindings list links back; a ServiceAccount used by pods shows them and its bound roles.
- [ ] 7. Watches per session stay at most `3N + 4` (`open_watch_count`: Bindings companion, N = 5 gives 19).
- [ ] 8. The 0003 AC4 color-literal grep is clean; the step's screenshots exist; the ui-verifier reports no high-severity defect against W7.

## Open items

1. Bindings to a custom wildcard ClusterRole are not flagged on the binding or the service account (only the role itself gets VERY BROAD); flagging needs the ClusterRoles list beside the bindings.
2. Binding counts, Bound roles, and CLUSTER ADMIN include RoleBindings of the current scope only; scope All gives the full picture (decisions, ceilings).
