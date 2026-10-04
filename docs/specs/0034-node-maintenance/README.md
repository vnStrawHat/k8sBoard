# 0034 — Node maintenance

Status: **built, steps 1, 2, 3a, 3b** (see [as-built.md](as-built.md)); amended after the advisor review (M2, M3, S2–S6, nice-to-haves); **refreshed 2026-10-03 against main `2c7dc08`**. Builds strictly on 0030 as amended (decisions 30–36: `checked_write`, `CommitMode::Commit { confirmed }`, `GuardedIntent.warnings`, `WriteEffect`, `TooManyRequests` (merged), 429 audit rule) and on 0032 (`GuardedKind::Batch`, `RowAction`, one entry point). 0030 step 4 ships single-node Cordon/Uncordon (the C arm); this spec does not redo it. Roadmap: gap plan 0034; C3, C8, C10; R2. Wireframes: W5 (node menu, selection bar, Edit labels), W6 (drain dialog), keyboard map (C, D).

**Prerequisites.** Merged: 0009, 0013, 0027, 0028, 0030 steps 1/2a/3. Step 1 (crate) any time after 0032 step 1 (shared `object_write.rs` seam). Step 2: 0030 steps 2b + 4 (in flight), 0032 2a-i + 2b. Step 3b: 0036 step 3a (`Dock`, `DockTab`; no longer optional: the rename lands early in lane W2). Lane W2, last spec.

**User approval (C3):** Approved by the user on 2026-10-02 (one approval for all mutating specs). Step 1 sends nothing (fake transport); steps 2, 3a, and 3b add real commits (bulk cordon, taints, labels; drain-dialog cordon; evictions). Debug builds still block writes unless `K8SBOARD_ALLOW_WRITES=1` (agents never set it); UAT checks stay denied-path-only.

## Goal

- Bulk Cordon / Uncordon from the W5 selection bar (0030 `SetNodeSchedulable` as a 0032 `Batch`).
- Edit taints and Edit labels of one node (`SetNodeTaints`, `SetNodeLabels`).
- Drain (W6): kubectl-flag options with consequences and counts, PDB-aware per-pod preview (0013), grace and timeout, typed node name, Cordon only; eviction (`EvictPod`) with 429 retry and backoff; sequential multi-node drain that stops when stuck; a dock progress tab with Cancel; one audit summary per node.

## Non-goals

- "Skip PodDisruptionBudgets" (`--disable-eviction`, deletes pods): shown disabled until 0033's delete exists (open item 1).
- Bulk taint or label editing; node delete (0033); node shell (0037); resuming a drain after an app restart.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Cluster crate: `EvictPod` (Status-decoded response), `SetNodeTaints`, `SetNodeLabels`, 429 mapping, `drain_pods`, `list_pod_disruption_budgets`, `node_for_edit`, `AccessCheck::CreatePodEviction`; fake-transport tests incl. both 429 shapes and a 201 `Failure` | 1–4 |
| 2 | Bulk cordon/uncordon (`Batch`), Edit taints, Edit labels. Approved by the user on 2026-10-02 (one approval for all mutating specs). | 1, 2, 5, 6, 12 |
| 3a | `drain_plan.rs` (pure), W6 dialog, dry-runs, Cordon only. Approved by the user on 2026-10-02 (one approval for all mutating specs; cordon commits). | 1, 2, 7, 8, 12 |
| 3b | `drain_run.rs` (state machine + driver), evictions, dock tab, audit summary; UAT denied path; ui-verifier. Approved by the user on 2026-10-02 (one approval for all mutating specs). | 1, 2, 9–11, 13 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale |
| [write-operations.md](write-operations.md) | eviction, taints, labels wire format; response decoding; 429 shapes; reads; RBAC; verified kube APIs |
| [node-edits.md](node-edits.md) | bulk cordon, Edit taints and Edit labels dialogs |
| [drain-dialog.md](drain-dialog.md) | W6 dialog: options, verdicts, PDB counts, dry-runs, typed name, Cordon only |
| [drain-run.md](drain-run.md) | cordon-all, eviction loop, backoff, wait, per-node timeout, cancel, multi-node, audit, dock tab, async |
| [files-to-touch.md](files-to-touch.md) | files per step, doc updates |
| [test-plan.md](test-plan.md) | fake transport (429, 201 Failure), pure state-machine tests, window tests, UAT denied path, ui-verifier |
| [as-built.md](as-built.md) | where the code differs from the text, behavior notes, deviations, tests |

## Acceptance criteria

- [x] 1. Quality gate passes, plus the screenshot-feature clippy; no new `#[allow]`; `Cargo.lock` gains no package.
- [x] 2. Every test in [test-plan.md](test-plan.md) exists and passes offline; no test talks to a cluster. Two have no window test and are covered otherwise: `skip_pdbs_is_disabled_with_reason` (the screenshot) and `node_menu_items_dispatch_their_keys` (the items carry the key action; the key tests cover the arms).
- [x] 3. Eviction is `POST …/pods/{name}/eviction` with a `policy/v1` body, `deleteOptions.preconditions.uid`, optional `gracePeriodSeconds`; the response is decoded as `Status` and a non-`Success` body is an error; `kube::Api::evict` is not used.
- [x] 4. 429 maps to 0030 `TooManyRequests` on dry-run and commit; `retry_after` is `None` for "needs N healthy pods" and 10 s for "still being processed".
- [x] 5. Bulk cordon is a 0032 `Batch`: nodes already in the target state are skipped; every dry-run must pass.
- [x] 6. Taint edits send the full list with `metadata.resourceVersion` and keep `timeAdded`; system taints and kubelet labels are read-only; adding `NoExecute` is `Destructive`; a 409 Retry reopens the editor fresh.
- [x] 7. The drain dialog previews every pod with its verdict (finished before DaemonSet; pending skips PDBs); blocked first; a passing dry-run shows `Dry-run accepted`; a missing option disables Drain with a reason.
- [x] 8. Drain always opens the dialog; Destructive; PROD types the node name (one node) or the cluster name (several); every request goes through `checked_write`.
- [x] 9. Evictions retry on 429 with `min(30 s, max(retryAfter, 5 s·2^(n−1)))` until the per-node timeout (`node_started`).
- [x] 10. Cancel sends no new request, keeps nodes cordoned, never undoes an eviction, and records in-flight results.
- [x] 11. Multi-node: cordon all first, drain in order, stop at the first stuck node; one audit summary line per node reached; 429 refusals unaudited.
- [x] 12. UAT (debug build): Drain, bulk Cordon, Edit taints/labels disabled with `Not permitted: …`; C and D show the notice; trace shows only GETs and SSAR POSTs.
- [x] 13. `--screen drain-dialog` matches W6 and `--screen drain-progress` shows the dock tab, no high-severity defect (screenshots `v87-*`, read by the coder; the ui-verifier pass is still to run).

## Open items

1. "Skip PodDisruptionBudgets": done in 0040 (a direct delete per pod through the 0033 `DeleteObject`, typed name in every tier).
2. Bulk label editing from the W5 header button: done in 0040 (one 0032 batch of `SetNodeLabels`).
3. A drain is not resumed after an app restart; nodes stay cordoned and the next drain starts fresh.
4. R2: eviction, PDB 429s, and taints are proven by fake transport and pure tests until a disposable cluster exists.
