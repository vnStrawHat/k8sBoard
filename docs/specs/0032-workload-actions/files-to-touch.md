# 0032 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; every new item has a production user in its step. Prerequisites: 0030 code (with decisions 30–36), 0028, 0029, 0012 merged. Step 2a-i needs the user's approval (C3).

## Cargo and lint config (step 1)

| File | Change |
|---|---|
| root `Cargo.toml` | kube features gain `"jsonpatch"` (no new package: `json-patch 4.2.0` is locked via kube-runtime; the coder confirms `Cargo.lock` has no new `[[package]]`) |
| `clippy.toml` | `disallowed-methods` gains `kube::Api::restart` (wrong annotation; use `RestartRollout`), `kube::Api::cordon`, `kube::Api::uncordon` (they bypass `SetNodeSchedulable`). Same path form as the 0030 entries; a scratch call proves each fires |

## `crates/cluster` (step 1)

| File | Change |
|---|---|
| `src/object_write.rs` (+ `object_write_tests.rs`) | the seven variants, `new` kind checks, `access_check`, `changed_fields`, manual `Debug` (name only), wire format, the three read-then-send operations, Roll back template guard and 422 → `Conflict`, create 422 → field paths only, `WriteEffect::Created` + `created_name` |
| `src/access_review.rs` | `PatchDeploymentScale`, `PatchStatefulSetScale`, `PatchDeployments`, `PatchStatefulSets`, `PatchDaemonSets`, `PatchCronJobs`, `CreateJobs`; `ALL` + 7; tests |

## `crates/app`

| S | File | Change |
|---|---|---|
| 2a-i | `src/resource_actions.rs` (+ tests) | `Scale(ObjectKind)`, `RestartRollout(ObjectKind)`, `PauseRollout`, `RollBack`, `SuspendCronJob`, `TriggerCronJob`, `RerunJob`; `gate()` by carried kind; `row_block`; state labels; `kind_menu` builds mutating items with `action_item` |
| 2a-i | `src/workload_actions.rs` (new) + `workload_actions_tests.rs` | intent builders (`GuardedIntent`), warnings, previous-revision pick |
| 2a-i | `src/resource_kind.rs` | `KindAction.action` for every W7 workload item |
| 2a-i | `src/keymap.rs` (+ tests), `src/keyboard_navigation.rs` | unit actions `PauseRollout`, `RollBack`, `SuspendCronJob`, `TriggerCronJob`, `RerunJob`; `Scale(_)`/`RestartRollout(_)` in `action_for`/`action_label`; R builds the intent |
| 2a-i | `src/write_flow.rs` | notice `{label}: created {name}` from `created_name` (0030 amendment) |
| 2a-ii | `src/value_popover.rs` (new) + `value_popover_tests.rs` | `ValuePopover`, `ValueForm::Replicas { input }`, `ValueTargets` (named for 0032b's forms from the start); menu `Scale…` and ⇧S open it |
| 2a-ii | `src/keymap.rs`, `src/command_palette.rs`, `src/palette_search.rs` (0029, + tests) | `ScaleCursorRow` on `secondary-enter` in `CommandPalette`, out of `RESERVED_KEYS`; inline argument mode; ⏎ on Scale → ⇧S fallback |
| 2a-iii | `src/live_sections.rs` (+ tests), `src/kind_drawer.rs` | Revisions `Roll back` gated; scroll-to-Revisions entry for menu `Roll back…` |
| 2a-iii | `src/command_palette.rs` | `Roll back to rev {n}` entry |
| 2b | `src/write_flow.rs` (+ tests) | `GuardedKind::Batch`, `BatchPlan`, `BatchItem`, `SkippedItem`, `BatchExtras::None`, `CheckedRow`, `batch_intent`, `MAX_BATCH_ITEMS`, the batch branch of `run_guarded` |
| 2b | `src/confirm_dialog.rs` | list variant (object list, states, counts, `expected_name`) |
| 2b | `src/row_selection.rs` (+ tests), `src/value_popover.rs` | `bulk_actions` → `KindAction`s; gated buttons; `ValueTargets::Ticked` |
| 2a-ii, 2b | `src/launch_options.rs` (+ tests), `src/screenshot.rs` | `--screen scale-popover`, `--screen scale-confirm` (2a-ii), `--screen restart-bulk-confirm` (2b); fixture states, no connection call |
| 2a-i, 2a-ii | `src/main.rs` | `mod workload_actions;`, `mod value_popover;` |

Removed from the first draft: `LiveContent::Replicas`, the drawer replicas input, and their tests.

## Docs

| S | File | Change |
|---|---|---|
| — | `docs/specs/0030-guardrails-write-path/*` | shared amendments, done in this revision (decisions 30–36, allow-list rows, `WriteEffect`, `Batch`, 429, delete exception, no SSA) |
| 2a-i | `docs/specs/0028-keyboard-map/keymap.md`, `row-actions.md` | `Scale(kind)`/`RestartRollout(kind)`, new unit actions, `KindAction` mapping |
| 2a-ii | `docs/specs/0028-keyboard-map/keymap.md`, `docs/specs/0029-command-palette/entries.md` | `secondary-enter` bound in `CommandPalette`; `action_for` covers the new variants; Scale argument and fallback |
| 2b | `docs/roadmap/gap-plan-local-and-mutating.md`, `inventory-*.md` | 0032 done for workloads; object actions → 0032b; Renew → 0018 |
