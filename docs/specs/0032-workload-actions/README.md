# 0032 — Workload actions

Status: draft, amended after the advisor review (M1–M3, S1–S9, nice-to-haves); **refreshed 2026-10-03 against main `2c7dc08`**. Builds strictly on 0030 as amended (decisions 30–36: `run_guarded`, `checked_write`, `CommitMode::Commit { confirmed }`, `GuardedIntent.warnings`, `WriteEffect`, `created_name`, gate by kind). Owns the one bulk mechanism, `GuardedKind::Batch`, and the `RowAction` key layer (decision 30). Roadmap: gap plan 0032; C3, C8, C10; R2. Wireframes: W7 kind menus and drawers, W9 palette (Scale `Ctrl ⏎`, Roll back to rev 37), keyboard map (R, ⇧S).

**Prerequisites.** Merged: 0012, 0027, 0028, 0029, 0030 steps 1/2a/3 (`object_write.rs` with `WriteOutcome.created_name`, `WriteEffect::Patched`, `TooManyRequests`; `write_guard.rs`; `ActionGate`; `audit_log.rs`). In flight: 0030 steps 2b + 4 (`run_guarded`, `checked_write`, `GuardedIntent`, `confirm_dialog.rs`, session lock); step 1 does not need them, 2a-i onward does. Lane W1, first spec (files-to-touch.md).

**User approval (C3):** Approved by the user on 2026-10-02 (one approval for all mutating specs). Step 1 sends nothing (fake transport); 2a-i adds the first real commits, and 2a-ii, 2a-iii, and 2b add new commit paths. Debug builds still block writes unless `K8SBOARD_ALLOW_WRITES=1` (agents never set it); UAT checks stay denied-path-only.

## Goal

- Seven allow-listed `WriteOperation`s: Scale, Restart rollout, Pause/Resume, Roll back (Deployments), CronJob Suspend/Resume, Trigger now, Job Re-run.
- Menus, keys R and ⇧S, one Scale popover, drawer revision `Roll back` buttons, palette (`Ctrl ⏎` Scale argument), and selection-bar bulk actions, all through the 0030 gate and `run_guarded`.
- `GuardedKind::Batch(BatchPlan)`: the one bulk mechanism, reused by 0033 and 0034.

## Non-goals

- StatefulSet/DaemonSet roll back via ControllerRevision (not in the W7 menus); a drawer replicas input (W7 note 4).
- HPA min/max, PVC expand, StorageClass default → 0032b; Certificate Renew → 0018 open items.
- Delete (0033), Edit YAML (0031), port-forward (0035), cross-cluster bulk, undo.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Cluster crate: operations, per-operation effect seam, `WriteEffect::Created`, `AccessCheck`s, kube `jsonpatch`, clippy additions, fake-transport tests. Can run while 0030 2b/4 are in flight | 1–4 |
| 2a-i | App: first commit the `RowAction` key layer alone (mechanical, merge fast: 0031, 0033, 0036 build on it); then `checked_write` use, `Scale(kind)`/`RestartRollout(kind)`, gate, `row_block`, menus and R for Restart, Pause/Resume, Suspend/Resume, Trigger now, Re-run. Approved by the user on 2026-10-02 (one approval for all mutating specs). | 1, 2, 5–7, 10 |
| 2a-ii | Scale popover (menu, ⇧S, palette fallback), palette `Ctrl ⏎` argument | 1, 2, 8 |
| 2a-iii | Roll back: drawer Revisions buttons, menu, palette entry | 1, 2, 5 |
| 2b | `Batch`, selection-bar buttons, bulk Scale popover; UAT denied path; ui-verifier | 1, 2, 9, 11, 12 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale |
| [write-operations.md](write-operations.md) | per-operation HTTP, body, RBAC, dry-run, risk, audit, errors; verified kube APIs |
| [actions-ui.md](actions-ui.md) | `RowAction` key layer, one entry point, gate by kind, row-state reasons, intents and warnings, menus, keys, Scale popover, palette, Roll back |
| [bulk-write.md](bulk-write.md) | `Batch`: types, sequence, dialog, selection bar (shared with 0033, 0034) |
| [files-to-touch.md](files-to-touch.md) | files per step, Cargo, clippy, doc updates |
| [test-plan.md](test-plan.md) | fake transport, unit and window tests, UAT denied path, ui-verifier |

## Acceptance criteria

- [ ] 1. Quality gate passes, plus the screenshot-feature clippy. No new `#[allow]`; `Cargo.lock` gains no package.
- [ ] 2. Every test named in [test-plan.md](test-plan.md) exists and passes offline; no test talks to a cluster.
- [ ] 3. `allow_list_matches_the_operations` pins method, path, query, content type, and body of every new operation; Trigger now has `controller: true` and no `blockOwnerDeletion`.
- [ ] 4. `ClusterConnection::write` is called only from `checked_write`; clippy forbids `kube::Api::{restart, cordon, uncordon}`.
- [ ] 5. Each action is gated by its own `AccessCheck` for the row's kind through the two-argument `action_availability`, on the row's own slot (never the primary); then `row_block` (paused, revisions, current template). Menu, key, and palette run the same `run_available_row_key` arm.
- [ ] 6. Restart sets `kubectl.kubernetes.io/restartedAt` (whole-second UTC); dry-run and commit bodies are byte-identical.
- [ ] 7. Scale to 0 is `Destructive`, the rest `Change`; creates report `created_name` in the notice and audit.
- [ ] 8. Menu `Scale…`, ⇧S, palette ⏎ and the bulk button open the one Scale popover; `Ctrl ⏎` gives the palette argument; there is no drawer input.
- [ ] 9. Batch: one cluster, ≤ 50, always a dialog, every dry-run must pass, sequential commits through `checked_write`, stop on `Blocked`, one audit line per commit.
- [ ] 10. Audit values never contain a template, env, or Job spec; 422 on creates shows field paths only; 422 on Roll back is `Conflict`.
- [ ] 11. UAT (debug build, no `K8SBOARD_ALLOW_WRITES`): every 0032 item disabled with `Not permitted: {check}`; R and ⇧S show the notice; trace has only GETs and SSAR POSTs.
- [ ] 12. ui-verifier: `--screen scale-popover`, `--screen scale-confirm`, `--screen restart-bulk-confirm` match W7/W10, no high-severity defect.

## Open items

1. The HPA warning on Scale shows only when the HPA list is already loaded (no new list call).
2. `Conflict.managers` is always empty now (0030 amendment); removing the field is a 0030 clean-up for whoever lands first.
3. R2: no write-capable cluster; commits are proven by fake transport until the user provides a disposable cluster.
