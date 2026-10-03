# 0046 — One cluster at a time (remove multi-cluster mode)

Status: draft, 2026-10-03, against main `6662108` (0034 merged). **User decision 2026-10-03: k8sBoard works on one cluster at a time; multi-cluster mode is removed.** Supersedes the multi-view parts of [0027](../0027-multi-cluster/README.md); drops [0045](../0045-multi-cluster-screens/README.md). Crate: `crates/app` only. **No new Kubernetes call; the write path stays exactly as strict** ([write-safety.md](write-safety.md)). No settings migration.

## Goal

- One active cluster. The switcher (click, Enter, Ctrl 1–9), palette `@`, and "Back to" switch it, 0026 break before make, unchanged.
- Delete the multi-view code: switcher ticks and footer, several sessions, merged rows and the Cluster column, `+N`, the "n of m unlocked" badge and its menu, per-slot banners, summed counts, `--view`, `--screen pods-multi`.
- Collapse the slot model to one `ActiveSession`; keep the cluster-qualified identifiers that guard writes (`ClusterObject`, `guard_for(&ClusterRef)`, `RowContext`).
- Keep: port forwards across a switch (0035 AC 8, page Cluster column), `leaving_work` before a switch, scope memory, `last_used`.

## Non-goals

- Any change to `object_write.rs`, the clippy write ban, `fresh_enter`, the audit log, tiers, or `WritePolicy`.
- Renaming the `slot_*` helpers (`slot_session`, `slot_live`, `slot_connection`): they still mean "the session of this cluster, if open"; a rename would churn ~15 files shared with parallel lanes.
- Rewriting historical specs; they get a superseded note (0027) or a dropped status (0045) only.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Tests first: port every two-cluster fixture to "two loaded, one active, switch"; delete `app_shell_multi_tests.rs` after porting its single-cluster tests; add the safety tests. No production change | 1, 2, 8–10, 12 |
| 2 | Entry points and lifecycle: switcher ticks (Space → `NoAction`), `--view`, `pods-multi`, `view_clusters`/`apply_view`/`remove_from_view`/`retry_cluster`, multi branches of `switch_to`, workspace multi body and banners, `plan_view` and friends | 1–5, 12, 13 |
| 3 | Leaves: title bar, status bar, picker union, sidebar sums, dock and log titles, drawer cluster chip, palette single session, row-menu cluster filter, `reveal_in_primary`, fan-out loops, `RowContext` trimmed, `is_multi`, `riskiest` | 1, 2, 6, 7 |
| 4 | Model and tables: `ClusterView` → `ActiveSession`; one `TableSession` per delegate; merge code, `RowAddress`, `TableRow::cluster`, `session_column` gone; ticks cleared on session change; `primary_cluster` → `active_cluster`; final greps; ui-verifier | 1, 2, 5, 6, 8–11, 14 |

## Files

| File | Contents |
|---|---|
| [inventory.md](inventory.md) | every multi-cluster code path, its decision (delete / simplify / keep), its step |
| [single-session.md](single-session.md) | the model and API after removal, numbered decisions |
| [write-safety.md](write-safety.md) | write-path invariants, why each kept identifier stays, safety tests |
| [steps.md](steps.md) | files per step, 0034 hot spots, docs to update after merge |
| [test-plan.md](test-plan.md) | tests to delete, port, and add; greps; ui-verifier |

## Acceptance criteria

- [ ] 1. Gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`; `Cargo.lock` and `crates/cluster` unchanged.
- [ ] 2. Every test the step adds or ports in [test-plan.md](test-plan.md) exists under that name and passes offline; every test listed for deletion is gone.
- [ ] 3. The switcher has no checkbox, tick notice, or `{n} selected · Clear · View {n} clusters` footer. Click, Enter, Ctrl 1–9, and palette `@` switch the one active cluster (0026 behavior and tests unchanged).
- [ ] 4. Space in the switcher filter or on a row never closes the popover and never switches (`space` → `NoAction` in both switcher contexts).
- [ ] 5. No path can hold two sessions: the final greps in [test-plan.md](test-plan.md) find nothing.
- [ ] 6. No Cluster column on Pods, Nodes, or kind tables; headers, sidebar counts, status bar, namespace picker, dock titles, and palette entries show the single-cluster texts.
- [ ] 7. Title bar: trigger is the active badge + label (no `+N`); the border is the active environment; the lock badge reads `Read-only` / `Unlocked` and a click toggles the active cluster's lock (no per-cluster menu).
- [ ] 8. `guard_for(c)` is `Some` only when `c` is the active cluster and Live. After A → B, every gate on an A-captured row, menu, dialog, or batch is off and sends nothing.
- [ ] 9. Every commit rereads guard, lock, generation, and tier from the active session at commit; a dialog opened on A and confirmed after A → B, or after A → B → A, sends nothing.
- [ ] 10. A switch clears every table's ticks and range anchor; a tick made on A never shows on, or acts on, a same-named row of B.
- [ ] 11. `clippy.toml`, `object_write.rs`, `fresh_enter.rs`, `audit_log.rs`, `write_guard.rs` tiers, and the `Select rows of one cluster` guards are unchanged (`git diff` on them is empty except doc comments).
- [ ] 12. Kept: forwards survive a switch and list their cluster (0035 AC 8); `leaving_work` asks before a switch (shells, batches, drains, node shells, unsaved edit); scope memory and `last_used` behave as in 0026.
- [ ] 13. Settings: no viewed-set field exists or is added; a file with unknown keys or a `Cluster` table pref loads, and the next write drops them. No migration, no version bump.
- [ ] 14. ui-verifier: `switcher`, `pods`, `nodes`, `pods-selected` (light, dark) show no checkbox, footer, `+N`, or Cluster column; no high-severity defect against W1 as amended.

## Open items

1. None needs the user. Defaults taken: keep `ClusterObject` and the `Select rows of one cluster` guards (write safety); keep the `slot_*` helper names (churn).
2. Parallel lanes that touch the same files (0022 RBAC layer, 0039, 0044, 0029 step 5) should rebase after step 4 or land before step 1; see [steps.md](steps.md).
