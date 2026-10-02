# 0023 — RBAC and policy analysis (read-only)

Status: amended after the advisor review (must-fix 1–2, should-fix 3–9, nice-to-haves 10–15), HEAD `56f360c`; architect defaults (the user asked not to stop for questions). Crates: `crates/cluster` (steps 1a, 1b), `crates/app` (steps 2–4). Requires 0013 (NetworkPolicy summaries), 0015 (RBAC summaries, `access_bindings.rs`), 0012 (`Selector`, `reveal`, `Live`) merged. Wireframes: W7 `k("Roles")`, `k("ClusterRoles")` (Who can…), `k("ServiceAccounts")` (Check permissions, Can do), `k("NetworkPolicies")` (Test traffic); W11 RBAC chip stays disabled (decision 16).

## Goal

- **Who can…**: `<verb> <resource> [in ns]` → subjects grouped with the granting binding and role, computed client-side from one RBAC snapshot (Roles, ClusterRoles, both binding kinds).
- **Check permissions**: verbs × resources table for **You** (SelfSubjectRulesReview per namespace) or for a ServiceAccount, user, or group (client-side evaluator); ad-hoc "can … ?" (SSAR for You, evaluator otherwise).
- **ServiceAccount "Can do"**: drawer chips from the same evaluator (0015 decision 12).
- **Test traffic**: source (pod, labels, or IP) → destination pod + port/protocol, evaluated client-side against NetworkPolicies; allowed/denied with the deciding rules.
- Every result states what it is computed from and what it cannot see (caveats).

## Non-goals

SubjectAccessReview for other subjects (needs `create subjectaccessreviews`, decision 3); impersonation; SelfSubjectReview ("who am I"); API discovery (built-in resource table instead, decision 9); CNI-specific policies (Calico, Cilium), AdminNetworkPolicy, Services as traffic targets; Topology RBAC overlay; any mutation.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1a | Cluster, RBAC: `rbac_evaluation.rs`, `rbac_snapshot.rs` (`read_rbac` with partial coverage), SSRR + request SSAR in `access_review.rs`, probe `--analysis` RBAC lines. **Run the UAT probe**, fill [decisions.md](decisions.md) "UAT probe" | 1–4 |
| 1b | Cluster, traffic: `network_policy_traffic.rs`, `read_network_policies`, probe `network policies` line; fill its probe row | 1–4 |
| 2 | App: `access_query.rs`, `RbacState` in the session, **Who can…** dialog, buttons and menus on Roles/ClusterRoles, `--screen who-can` | 1, 2, 5–8 |
| 3 | App: `permission_table.rs`, **Check permissions** dialog, SA **Can do** section, buttons and menus, two screens | 1, 2, 5–8 |
| 4 | App: **Test traffic** dialog, NetworkPolicies button and menu, `--screen test-traffic`; full ui-verifier run | 1, 2, 5–8 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale, ceilings, UAT probe table |
| [rbac-evaluation.md](rbac-evaluation.md) | step 1a: request and identity types, rule matching, `who_can`, `rules_of`, `decide`, snapshot fetch, SSRR, request SSAR |
| [traffic-evaluation.md](traffic-evaluation.md) | step 1b: NetworkPolicy semantics, endpoints, verdict, ports, CIDR, policy fetch |
| [app-model.md](app-model.md) | steps 2–4: session state, async contract, query parser, permission table, entry points, launch screens |
| [dialogs.md](dialogs.md) | steps 2–4: dialog layouts, texts, caveats, Can do section |
| [files-to-touch.md](files-to-touch.md) | modules per step, doc updates |
| [test-plan.md](test-plan.md) | table tests per evaluator, app tests, live checks, ui-verifier |

## Acceptance criteria

- [ ] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`, no new dependency, no `Cargo.lock` change.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline.
- [ ] 3. Read-only: the 0001 AC4 grep finds only the SSAR and SSRR `create` calls (both `Api::<SelfSubject…Review>::all`); every new request is `list`. No kube or k8s-openapi type in a public signature; the crate spawns no task.
- [ ] 4. On UAT the probe `--analysis` prints the snapshot counts, coverage, an SSRR rule count, and a Who-can grant count; results in [decisions.md](decisions.md). The 0001 AC7 credential script reports 0.
- [ ] 5. No network I/O on the GPUI thread: fetches run on `ClusterRuntime`, results arrive through `cx.spawn`; each dialog view is created once in `open_*` (the dialog builder only clones it, review); closing a dialog drops (aborts) its in-flight request.
- [ ] 6. Every result panel shows its source line and caveats ([dialogs.md](dialogs.md)); evaluator results say "computed from RBAC objects" / "computed from NetworkPolicies"; every coverage gap shows its Warn line.
- [ ] 7. On UAT: Who can `get secrets` lists subjects with binding links that reveal the binding; Check permissions (You) shows the SSRR table; a ServiceAccount drawer shows Can do; Test traffic gives a verdict for two pods.
- [ ] 8. The 0003 AC4 color-literal grep is clean; the step's screenshots exist; the ui-verifier reports no high-severity defect.

## Open items

1. "Who can" across every namespace at once (today: one namespace or cluster-wide grants only).
2. Users' authenticator groups are unknown; evaluating a user counts only bindings to the name and `system:authenticated`.
3. `get namespaces/<ns>` through a RoleBinding in `<ns>` (the authorizer treats the namespace object as inside itself) is not modeled.
4. A Topology RBAC overlay needs a wireframe first (decision 16).
