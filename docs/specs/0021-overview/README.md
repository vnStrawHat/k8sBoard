# 0021 — Overview dashboard (W3, read-only)

Status: amended after the advisor review (should-fix 1–6, nice-to-haves 7–13), HEAD `66da2e8`. Crates: `crates/app`, plus **one function** in `crates/cluster` (`watch_change_events`, step 2). Wireframe: W3 (pins 1–4), sidebar top item Overview. Requires 0010 and 0011 (merged). Step 3 requires **0020 step 1b**; step 4 requires **0019 step 3**. Applies C1, C9, C11, C13.

## Goal

- An **Overview** screen that answers, in W3 order: what is broken, how much room is left, what just changed.
- **Header**: `{context} · Kubernetes {version} · {region}`, `Last 15 min ▾`, Export report. A **stats line** sits under it (nodes ready/total, pods running/total, namespaces). It was **user-requested and is not in W3**, so the ui-verifier must not flag it.
- **Capacity**: three-layer bars (used, requested, allocatable) for CPU and Memory, plus Pods and Volumes bars.
- **Nodes heatmap**: one card per node with its CPU and memory shares, tinted from 80 %. NotReady nodes are outlined; a click reveals the node.
- **Recent changes**: Deployment rollouts and scaling plus HPA rescales (selected server-side), node readiness transitions, joined nodes, and new namespaces.
- **Needs attention**: the first 6 issues of the 0020 board in the W3 row anatomy (pill, `ns / name · container c`, cause, one read-only action). With it, Overview becomes the **default landing** screen.

## Non-goals

Any mutation (W3 "Fix image" → 0032); diff on change click (W10 → 0031); managedFields "who" and ConfigMap edits; per-node bars (the Nodes screen has them); a separate certificate tile (certificates arrive as 0020 issues); memory coloring of the heatmap; Prometheus history; multi-cluster Overview (0027); persisting the range choice (0024).

## Implementation steps

| Step | Scope | Needs | ACs |
|---|---|---|---|
| 1 | `Screen::Overview`, sidebar item enabled, header and stats line, Capacity, Nodes heatmap, layout, `--screen overview` | 0010, 0011 | 1–7 |
| 2 | `watch_change_events` (two selectors), change feed in the session, Recent changes panel, range dropdown, footnote | step 1 | 1–5, 9 |
| 3 | Needs attention panel; default landing; `--window-width`; `overview-narrow` screenshot | step 2, 0020 step 1b | 1–5, 8, 10, 11 |
| 4 | Export report (Markdown through the save dialog); shared `file_export.rs` with 0019 | step 3, 0019 step 3 | 1, 2, 12 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions, each with a one-line rationale |
| [layout.md](layout.md) | screen wiring, header, panels, responsive layout, interactions, state views |
| [capacity-and-nodes.md](capacity-and-nodes.md) | pure models: headline and stats, the capacity row enum, heatmap cells; `CapacityBar`; ceilings |
| [attention.md](attention.md) | Needs attention: the 0020 contract, row anatomy, action button |
| [recent-changes.md](recent-changes.md) | change feed (cluster function, session async, watch math), model, window, rows |
| [export-report.md](export-report.md) | step 4: report content, save flow, the export state shared with 0019 |
| [files-to-touch.md](files-to-touch.md) | modules per step, prerequisites, doc updates |
| [test-plan.md](test-plan.md) | unit tests per step, live checks, ui-verifier checklist |

## Acceptance criteria

- [x] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`; no `Cargo.toml`/`Cargo.lock` change.
- [x] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline.
- [x] 3. Read-only: the only new requests are the two change-events `list`/`watch` (step 2). The 0001 grep still finds only the SSAR `create`. No kube type appears in a public signature, and the cluster crate spawns no task.
- [x] 4. The 0003 AC4 color-literal grep is clean; bars and cells use theme tokens and `tone_color` only.
- [x] 5. No new always-on watch: with Overview hidden, the status-bar watch count equals the 0020 count. While Overview is visible it adds `2 × scope_multiplicity` (step 2 onward).
- [x] 6. On UAT, CPU and Memory allocatable equal the sums from `kubectl --context readonly@Monitor get nodes` allocatable (read-only `get`). Used matches the Nodes screen bars of the same tick, within rounding. Checked with the probe (`allocatable totals`: 82.0 cores, 101.25 GiB, 440 pods), which equals the panel.
- [ ] 7. On UAT, the heatmap has one cell per node. A click reveals that node with its drawer. A NotReady node, if any, is outlined.
- [ ] 8. On UAT, Needs attention equals the first 6 rows of the Issues screen. View logs opens the dock on the issue's pod; a row click reveals the target.
- [x] 9. On UAT, Recent changes lists the window's `ScalingReplicaSet` events (spot-check against `kubectl get events --field-selector reason=ScalingReplicaSet`), and the footnote shows. UAT had no ScalingReplicaSet event in the 15 min window, so the empty text and the footnote were checked.
- [x] 10. The app starts on Overview without `--screen`; `--screen pods` still starts on Pods.
- [ ] 11. Screenshots `overview` (1320 px, four panels) and `overview-narrow` (`--window-width 1000`, one column) exist. The ui-verifier reports no high-severity defect against W3.
- [x] 12. Export writes only after the dialog confirms. The report lists every issue, coverage, capacity, nodes, and the window's changes. Paths are never traced.

## Open items

1. Closed by [0041](../0041-edit-yaml-history/README.md): the "who" of a Deployment rollout (the field manager, inferred) and the click-to-diff of Deployment rows are built; ConfigMap rows were dropped (user, 2026-10-03).
2. StatefulSet and DaemonSet rollouts emit no clean event; add them from ControllerRevisions if wanted.
3. Aggregates are computed per render. Cache them per snapshot only if profiling on a large cluster shows a cost.
4. Clusters with a longer `--event-ttl` could offer 6 h and 24 h ranges.
