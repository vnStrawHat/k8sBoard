# 0046 — One cluster at a time (remove multi-cluster mode)

Status: steps 1–4 built 2026-10-03 (step 5, the `slot_*` rename, waits for the queued lanes); drafted 2026-10-03, against main `6662108` (0034 merged); **amended after the opus review** (2 must-fix, 6 should-fix, nits). **User decision 2026-10-03: k8sBoard works on one cluster at a time; multi-cluster mode is removed.** Supersedes the multi-view parts of [0027](../0027-multi-cluster/README.md); drops [0045](../0045-multi-cluster-screens/README.md). Crate: `crates/app` only. **No new Kubernetes call; the write path stays exactly as strict** ([write-safety.md](write-safety.md)). No settings migration. Lands after 0039, before the queued lanes (0022 RBAC, 0044, 0029 step 5); step 5 follows them.

> **Start rule for watched folders (0043).** A file of a watched folder starts a session only as the exact `last_used` the user picked: never through `--context`, `current-context`, or the first-file fallback (`ClusterCatalog::start_kubeconfigs()` has the chain and the registry files only). With nothing to start the shell shows "No cluster selected. Pick one in the switcher." and opens the switcher.

## Goal

- One active cluster. The switcher (click, Enter, Ctrl 1–9), palette `@`, and "Back to" switch it, 0026 break before make, unchanged.
- Delete the multi-view code: switcher ticks and footer, several sessions, merged rows and the Cluster column, `+N`, the "n of m unlocked" badge and its menu, per-slot banners, summed counts, the New-forward cluster picker, `--view`, `--screen pods-multi`.
- Collapse the slot model to one `ActiveSession`; keep the cluster-qualified identifiers that guard writes (`ClusterObject`, `guard_for(&ClusterRef)`, `RowContext`).
- Keep: port forwards across a switch (0035 AC 8, page Cluster column), `leaving_work` before a switch, scope memory, `last_used`.

## Non-goals

- Any change to `object_write.rs`, the clippy write ban, `fresh_enter`, the audit log, tiers, or `WritePolicy`.
- Renaming the `slot_*` helpers in steps 1–4: the rename is step 5, a mechanical commit after the queued lanes merge.
- Rewriting historical specs; they get a superseded note (0027) or a dropped status (0045) only.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Tests first: port every two-cluster fixture to "two loaded, one active, switch"; move the single-cluster tests out of `app_shell_multi_tests.rs` (the file stays until step 2); add the safety tests that need no production change. No production change | 1, 2, 8–10, 12 |
| 2 | Entry points and lifecycle: switcher ticks (Space → `NoAction`), `--view`, `pods-multi`, `view_clusters` … `remove_from_view` with their orphans (`release_slot`, `close_tabs_of`, `close_edit_of`, `ClusterView::remove`), workspace multi body, `plan_view`; delete `app_shell_multi_tests.rs` with its test hooks; leftover notice check | 1–5, 8, 12, 13 |
| 3 | Leaves: title bar, status bar, picker union, sidebar sums, dock and log titles, drawer cluster chip, palette single session, row-menu cluster filter, `RowContext` trimmed and upgrade-checked, `reveal_in_primary`, `scope_live`, `is_multi`, `riskiest` | 1, 2, 6–8 |
| 4 | Model, tables, connect callers: `ClusterView` → `ActiveSession`; one `TableSession` per delegate; merge code gone; `set_session` backup tick clear; every `self.view.*` caller replaced per [inventory.md](inventory.md); New-forward form fixed label; final greps; ui-verifier | 1, 2, 5, 6, 8–11, 14 |
| 5 | After the queued lanes merge: mechanical rename (`slot_session`/`slot_live`/`slot_connection`/`slot_label` → `session_of`/`live_of`/`connection_of`/`label_of`; `new_slot`, `on_slot_changed`, `slot_row_context`, `app_shell_view.rs`) | 1, 2 |

## Files

| File | Contents |
|---|---|
| [inventory.md](inventory.md) | every multi-cluster code path, decision, step; exact replacements for connect and write-path callers |
| [single-session.md](single-session.md) | the model and API after removal, numbered decisions, step-5 renames |
| [write-safety.md](write-safety.md) | write-path invariants, kept identifiers, safety tests |
| [steps.md](steps.md) | files per step, 0034 hot spots, landing order, docs after merge |
| [test-plan.md](test-plan.md) | tests to delete, port, and add; greps; ui-verifier |

## Acceptance criteria

- [x] 1. Gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`, after every step (no `#[cfg(test)]` hook or helper left unread). No new `#[allow]`; `Cargo.lock` and `crates/cluster` unchanged.
- [x] 2. Every test the step adds or ports in [test-plan.md](test-plan.md) exists under that name and passes offline; every test listed for deletion is gone.
- [x] 3. The switcher has no checkbox, tick notice, or `{n} selected · Clear · View {n} clusters` footer. Click, Enter, Ctrl 1–9, and palette `@` switch the one active cluster (0026 behavior and tests unchanged).
- [x] 4. Space in the switcher filter or on a row never closes the popover and never switches (`space` → `NoAction` in both switcher contexts).
- [x] 5. No path can hold two sessions: the final greps in [test-plan.md](test-plan.md) find nothing.
- [x] 6. No Cluster column on Pods, Nodes, or kind tables; headers, sidebar counts, status bar, namespace picker, dock titles, and palette entries show the single-cluster texts.
- [x] 7. Title bar: trigger is the active badge + label (no `+N`); the border is the active environment; the lock badge reads `Read-only` / `Unlocked` and a click toggles the active cluster's lock (no per-cluster menu).
- [x] 8. `guard_for(c)` is `Some` only when `c` is the active cluster and Live. After A → B, every A-captured row, menu, dialog, batch, delete start, node-shell create, and leftover notice sends or opens nothing; a menu built on A acts only if its `RowContext.session` still upgrades, so A → B → A leaves it off.
- [x] 9. Every commit rereads guard, lock, generation, and tier from the active session at commit; a dialog opened on A and confirmed after A → B, or after A → B → A, sends nothing.
- [x] 10. A switch clears every table's ticks and range anchor (`release_all` → `clear_all_filters`, kept; `set_session` clears too on any change of the session entity, including to and from `None`); a tick made on A never shows on, or acts on, a same-named row of B.
- [x] 11. `clippy.toml`, `object_write.rs`, `fresh_enter.rs`, `audit_log.rs`, `write_guard.rs` tiers, and the `Select rows of one cluster` guards are unchanged (`git diff` from the step-1 base is empty except doc comments).
- [x] 12. Kept: forwards survive a switch, list their cluster, and keep their last lock state and accepting new local connections (0035 AC 8; decision 17); `leaving_work` asks before a switch (shells, batches, drains, node shells, unsaved edit); scope memory and `last_used` behave as in 0026; the quit path stops every running drain, whatever its cluster.
- [x] 13. Settings: no viewed-set field exists or is added; a file with unknown keys or a `Cluster` table pref loads, and the next write drops them. No migration, no version bump.
- [x] 14. ui-verifier: `switcher`, `pods`, `nodes`, `pods-selected`, `port-forward-new-fixture` (light, dark) show no checkbox, footer, `+N`, Cluster column, or cluster picker; no high-severity defect against W1 as amended.

## Open items

1. Closed 2026-10-03 (coordinator decision): forwards of a cluster you leave keep the 0035 behavior; no lock on leave ([single-session.md](single-session.md) decision 17).
2. Kept by default: `ClusterObject` and the `Select rows of one cluster` guards (write safety).

## As built (steps 1–4)

- Four WIP commits on branch `spec-0046` (base `52f8f87`): tests first, entry points and lifecycle, leaves, model and tables. Gate and `--features screenshot` clippy green after each.
- `slot_session` filters on the active cluster, so every `slot_*` helper and `guard_for` keep their signatures; `guard_for` is untouched. `guarded(row, item)` in `resource_actions.rs` makes a menu inert once its `RowContext.session` no longer upgrades.
- Deviation: `the_new_forward_form_shows_the_active_cluster_only` lives in `port_forward_open_tests.rs` (it needs the shell fixture) and checks that a prefill of another cluster opens nothing; the fixed label itself is checked in the `port-forward-new-fixture` screenshots (kit `h_flex` ids are not findable in the test frame).
- Test fix: `the_lock_badge_toggles_the_active_cluster` clears the notifications before its second click: the lock notice sits over the badge while it animates in, and made the test flaky.
- Screenshots: `.tmp/ui-shots/v92-*` (pods, switcher, forward-new; light and dark).
- Review fixes (5th WIP commit): Secret Reveal and every Copy entry go through `secret_action_item`, which is `guarded`; every View logs entry goes through `open_logs`, which checks `row.session.upgrade()`; Browse instances, Filter similar and Open URL items are `guarded`; `TabPlan.generation` makes a node shell or debug create that answers after A → B → A delete its pod instead of opening a tab.
