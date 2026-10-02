# 0020 — Issues engine and Issues screen (read-only)

Status: amended after the advisor review (must-fix 1–4, should-fix, nice-to-haves), HEAD `dc4ed69`. **Step 1a starts on committed code plus 0011; step 2 after 0012–0018 merge** ([files-to-touch.md](files-to-touch.md)). Crates: `crates/cluster` (one field), `crates/app`. Wireframes: W3 "Needs attention", W2 title bar `⚑ 4`, sidebar `Issues 4` (red), anatomy note "counts and error counts". Settles C13; applies C1, C4 (live session only), C11.

## Goal

- A pure **issues engine** that turns live state into problems: severity, reason, plain cause, object link, count, age; WHY rules reused (0008, 0012–0018); grouping per workload; one issue per object; generic grace periods.
- **Always-on feeds** with a stated cost: core (pods, nodes, namespaces, metrics, Warning events) and condition feeds (rollouts, jobs, HPAs, PDBs, quotas, claims, TLS certificates) for every scope, as compact summaries. Missing feeds make coverage **Partial**, and the UI says which.
- **Issues screen** (toolkit table, quick filter, click reveals the object's drawer), **sidebar issue counts**, **title-bar flag button**.

## Non-goals

Any mutation (W3 "Fix image" → 0032); snooze, acknowledge, persistence (first-seen does not survive an app restart), notifications; severity chips and menu filters; Overview cards (0021); config checks such as Services matching no pods, unused objects, RBAC breadth (0022/0015); cordoned nodes as issues; StatefulSet/ReplicaSet/CronJob/Ingress/PV rules; multi-cluster counts (0027); a drawer on the Issues screen; Prometheus rules; measured CPU throttling.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1a | `PodCondition.changed_at`; `DiagnosisCause`; `pod_workload`; engine (`issue.rs`, `issue_rules.rs`, `issue_board.rs` with grace); core feeds (pods, nodes, metrics, volume usage; the Namespaces feed comes with NamespaceStuck in step 2), Warning events, board and tick in `ClusterSession`, `subscribe_silent`; sidebar counts; title-bar button (tooltip only) | 1–6, 9 |
| 1b | Issues screen and table, menu, reveal, button click, Issues item enabled, `--screen issues`, screenshots | 1, 2, 7, 9, 10 |
| 2 | Condition feeds (`condition_plan`, All-scope above 2 namespaces), `issue_kind_rules.rs` incl. NamespaceStuck, coverage labels, watch count; budget measurements; roadmap updates; full ui-verifier run | 1–10 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale; budget measurement table |
| [engine.md](engine.md) | model types, pipeline (evaluate, group, dedupe, grace, sort), board, constants |
| [rules.md](rules.md) | pod, node, volume-usage, and event rules; `DiagnosisCause` |
| [kind-rules.md](kind-rules.md) | workload, HPA, PDB, quota, PVC, namespace, certificate rules; evaluation order |
| [feeds.md](feeds.md) | rule → feed map, feeds, coverage, silent subscription, board and tick, watch count |
| [issues-screen.md](issues-screen.md) | sidebar, title bar, screen, table, menu, launch, 0021 contract |
| [files-to-touch.md](files-to-touch.md) | modules per step, prerequisites, doc updates |
| [test-plan.md](test-plan.md) | unit tests per step, live checks, ui-verifier checklist |

## Acceptance criteria

- [ ] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`; no `Cargo.lock` change.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline.
- [ ] 3. Read-only: new requests are `list`/`watch` only; the 0001 grep still finds only the SSAR `create`; no kube type in a public signature; the cluster crate spawns no task.
- [ ] 4. Secret safety: issue modules contain no `tracing::` call; TLS summaries hold no secret values (0016); coverage and causes never include credentials.
- [ ] 5. Rules are pure (`issue_rules.rs`, `issue_kind_rules.rs`, `issue_board.rs` take no GPUI context); evaluation runs at most once per `ISSUE_TICK`; it notifies only on change, or on `TIME_REFRESH` while Issues is visible.
- [ ] 6. `issue_evaluation_budget` (release) ≤ 4 ms on the dev machine; the number is in [decisions.md](decisions.md).
- [ ] 7. On UAT the Issues screen lists live problems; each Critical row matches `kubectl` state; clicking a row reveals the object with its drawer.
- [ ] 8. Idle RSS after 5 min on UAT (scope All, all feeds) < 150 MB; watch count equals `open_watch_count`; three namespaces run condition feeds as All-scope watches showing only in-scope rows.
- [ ] 9. The 0003 AC4 color-literal grep is clean; tones come from `tone_color`.
- [ ] 10. The step's screenshots exist; the ui-verifier reports no high-severity defect against W2, W3 (content), and the sidebar.

## Open items

1. While the explorer shows a fed kind, its watch runs twice (decision 9); share the condition feed's list with the explorer if the budget is tight.
2. Every TLS Secret counts, used or not; filter by Ingress/pod use once 0022 builds the reference graph. The cluster-wide secrets watch shows in audit logs; opt-out lands with 0024 settings (decision 11).
3. Back navigation from a revealed object to Issues (0028 keyboard map).
4. Events expire after 1 h (API TTL), so event-only issues (FailedCreate) clear even if the cause persists; condition feeds cover rollouts and Jobs.
5. The watch bound is `4N + 12` except at N = 2, where per-namespace condition watches give 28; switching N = 2 to All-scope would make it uniform at the cost of a cluster-wide RBAC need.
