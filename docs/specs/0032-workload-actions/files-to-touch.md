# 0032 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; every new item has a production user in its step. Baseline: main `2c7dc08`. Step 1 needs only the merged 0030 step 1; 2a-i onward needs 0030 steps 2b + 4 (in flight). C3: Approved by the user on 2026-10-02 (one approval for all mutating specs).

## Cargo and lint config (step 1)

| File | Change |
|---|---|
| root `Cargo.toml` | kube features gain `"jsonpatch"` (no new package: `json-patch 4.2.0` is locked via kube-runtime; the coder confirms `Cargo.lock` has no new `[[package]]`) |
| `clippy.toml` | `disallowed-methods` gains `kube::Api::restart` (wrong annotation; use `RestartRollout`), `kube::Api::cordon`, `kube::Api::uncordon` (they bypass `SetNodeSchedulable`); none is on main yet. Same path form and `reason` text as the merged entries; a scratch call proves each fires |

## `crates/cluster` (step 1)

| File | Change |
|---|---|
| `src/object_write.rs` (+ `object_write_tests.rs`) | the seven variants (arms appended to `send`, `Debug`, `access_check`, `changed_fields`, `supports_dry_run`), `new` kind checks, per-operation effect seam, wire format, the three read-then-send operations (reads through `self.run`), Roll back template guard and 422 → `Conflict`, create 422 → field paths only, `WriteEffect::Created` + `created_name` |
| `src/access_review.rs` | `PatchDeploymentScale`, `PatchStatefulSetScale`, `PatchDeployments`, `PatchStatefulSets`, `PatchDaemonSets`, `PatchCronJobs`, `CreateJobs`; `ALL` 36 → 43; tests |

## `crates/app`

| S | File | Change |
|---|---|---|
| 2a-i (first commit) | `src/resource_actions.rs`, `src/keyboard_navigation.rs`, `src/palette_search.rs`, `src/command_palette.rs`, `src/resource_kind.rs` (+ tests) | `RowAction`, `RowAction::key_action`, `ResourceAction::row_action`, `subject_action → Option`, `key_availability_of(RowAction, ..)`, `run_row_key(RowAction)`, `ROW_ACTIONS: [RowAction; _]`, `PaletteTarget::RowAction(RowAction)`, `row_action_icon`; no behavior change |
| 2a-i | `src/resource_actions.rs` (+ tests) | `Scale(ObjectKind)`, `RestartRollout(ObjectKind)`, `PauseRollout`, `RollBack`, `SuspendCronJob`, `TriggerCronJob`, `RerunJob`; `gate()` by carried kind (`Mutating`, shipped); `row_block`, `row_action_item`; state labels; `kind_menu` builds mutating items without `on_click` |
| 2a-i | `src/workload_actions.rs` (new) + `workload_actions_tests.rs` | intent builders (`GuardedIntent`), warnings, previous-revision pick |
| 2a-i | `src/resource_kind.rs` | `KindAction::keyed` with the kind's `ObjectKind` for every W7 workload item |
| 2a-i | `src/keymap.rs` (+ tests), `src/keyboard_navigation.rs` | unit actions `PauseRollout`, `RollBack`, `SuspendCronJob`, `TriggerCronJob`, `RerunJob` (unbound, `on_row_key`); arms `RestartRollout(_)`, `PauseRollout`, `SuspendCronJob`, `TriggerCronJob`, `RerunJob` → `run_guarded` |
| 2a-i | `src/write_flow.rs` (0030 step 4) | notice `{label}: created {name}` from `created_name` (if 0030 step 4 did not already) |
| 2a-ii | `src/value_popover.rs` (new) + `value_popover_tests.rs` | `ValuePopover`, `ValueForm::Replicas { input }`, `ValueTargets`; the `Scale(_)` arm opens it |
| 2a-ii | `src/keymap.rs`, `src/command_palette.rs`, `src/palette_search.rs` (0029, + tests) | `ScaleCursorRow` on `secondary-enter` in `Command > Input`, out of `RESERVED_KEYS`; inline argument mode; ⏎ on Scale → the ⇧S arm |
| 2a-iii | `src/live_sections.rs` (+ tests), `src/kind_drawer.rs` | Revisions `Roll back` gated on the drawer subject's slot; `RollBack` arm opens the drawer scrolled to Revisions |
| 2a-iii | `src/palette_search.rs` | `Roll back to rev {n}` entry |
| 2b | `src/write_flow.rs` (+ tests) | `GuardedKind::Batch`, `BatchPlan`, `BatchItem`, `SkippedItem`, `BatchExtras::None`, `BatchFailure`, `CheckedRow`, `batch_intent`, `MAX_BATCH_ITEMS`, the batch branch of `run_guarded` |
| 2b | `src/table_view.rs` (+ tests) | `checked_rows(items)` accessor |
| 2b | `src/confirm_dialog.rs` (0030 step 4) | list variant (object list, states, counts, `expected_name`) |
| 2b | `src/row_selection.rs` (+ tests), `src/value_popover.rs` | `bulk_actions` → `KindAction`s; gated buttons; `ValueTargets::Ticked` |
| 2a-ii, 2b | `src/launch_options.rs` (+ tests), `src/screenshot.rs` | `--screen scale-popover`, `--screen scale-confirm` (2a-ii), `--screen restart-bulk-confirm` (2b); fixture states, no connection call |
| 2a-i, 2a-ii | `src/main.rs` | `mod workload_actions;`, `mod value_popover;` |

## Docs

| S | File | Change |
|---|---|---|
| 2a-i | `docs/specs/0028-keyboard-map/keymap.md`, `row-actions.md` | `RowAction` key layer, `Scale(kind)`/`RestartRollout(kind)`, new unit actions, `KindAction` mapping |
| 2a-ii | `docs/specs/0028-keyboard-map/keymap.md`, `docs/specs/0029-command-palette/entries.md` | `secondary-enter` bound in `Command > Input`; `RowAction::key_action` replaces `ResourceAction::key_action`; Scale argument and fallback |
| 2b | `docs/roadmap/gap-plan-local-and-mutating.md`, `inventory-*.md` | 0032 done for workloads; object actions → 0032b; Renew → 0018 |

## Parallel work (lane W1: 0032 → 0031 steps 3–4 → 0033 → 0032b)

- Step 1 runs now, beside the in-flight 0030 2b/4 (app only) and lane W2's crate steps.
- Merge the 2a-i `RowAction` commit before lane W2 reaches 0036 step 4 (its `OpenShell` arm and gate build on it).
- Disjoint from lane W2: `workload_actions.rs`, `value_popover.rs`, `live_sections.rs`, `kind_drawer.rs`, palette files, `table_view.rs`. Shared, append-only: `object_write.rs`, `access_review.rs`, `resource_actions.rs`, `keyboard_navigation.rs`, `keymap.rs`, `write_flow.rs`, `confirm_dialog.rs`, `row_selection.rs` (0034 Nodes row), `launch_options.rs`, `screenshot.rs`, `main.rs`.
