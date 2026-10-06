# 0032b · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; every new item has a production user in its step. Baseline main `2c7dc08`. Prerequisites: 0030 steps 2b + 4 and 0032 merged (step 1: only 0032 step 1); 0013, 0014 are on main. Steps 3 and 4 add the `ExpandClaim` / `SetDefaultStorageClass` arms the same way as step 2. C3: Approved by the user on 2026-10-02 (one approval for all mutating specs). No Cargo or clippy change.

## `crates/cluster` (step 1)

| File | Change |
|---|---|
| `src/object_write.rs` (+ `object_write_tests.rs`) | `SetHpaReplicaRange`, `ExpandClaim`, `SetDefaultStorageClass`; `new` validation, wire format, `access_check`, `changed_fields`, manual `Debug` (name only) |
| `src/access_review.rs` | `PatchHorizontalPodAutoscalers`, `PatchPersistentVolumeClaims`, `PatchStorageClasses`; `ALL` + 3; tests |

The operations are `pub` through `WriteOperation`, so step 1 has no dead code before the app uses them.

## `crates/app`

| S | File | Change |
|---|---|---|
| 2 | `src/value_popover.rs` (0032, + tests) | `ValueForm::ReplicaRange { min, max }`; validation texts |
| 2 | `src/resource_actions.rs` (+ tests), `src/keymap.rs`, `src/keyboard_navigation.rs` | `EditHpaRange` (gate `PatchHorizontalPodAutoscalers`), its `RowAction`, unbound unit action, and arm (opens the popover on the cursor row) |
| 2 | `src/resource_edits.rs` (new) + `resource_edits_tests.rs` | `hpa_range_intent`, warnings |
| 2 | `src/resource_kind.rs`, `src/row_selection.rs` (+ tests) | `KindAction.action` for `Edit min / max…`; HPAs selection-bar `Edit min / max` |
| 3 | `src/value_popover.rs`, `src/resource_edits.rs`, `src/resource_actions.rs` | `ValueForm::Storage { input }` (trimmed); `expand_intent`; `ExpandClaim` gate and `row_block` |
| 3 | `src/resource_kind.rs`, `src/row_selection.rs` | `Expand…` item; PVCs selection-bar `Expand` |
| 4 | `src/resource_edits.rs`, `src/resource_actions.rs` | `default_class_intent` (two-object `Batch`), `SetDefaultStorageClass` gate and `row_block` |
| 4 | `src/resource_kind.rs`, `src/row_selection.rs`, `src/resource_edits.rs` | `Set as default` item; StorageClasses `Set default` (one ticked); `two_defaults_text` and the Retry re-plan (no `write_flow.rs` change: the generic `Stop` notice and Retry cover the flow) |
| 2–4 | `src/launch_options.rs` (+ tests), `src/screenshot.rs` | `--screen hpa-range-popover` (2), `--screen expand-confirm` (3), `--screen default-class-confirm` (4); fixtures, no connection call |
| 2 | `src/main.rs` | `mod resource_edits;` |

## Docs

| S | File | Change |
|---|---|---|
| — | `docs/specs/0018-custom-resources/README.md` | open item 5: Certificate `Renew now` (done in this revision) |
| — | `docs/specs/0030-guardrails-write-path/write-path.md` | allow-list row for the three operations (done in this revision) |
| 4 | `docs/specs/0014-storage-kinds/README.md` | open item 2 (two defaults) now has an action that resolves it |
| 4 | `docs/roadmap/gap-plan-local-and-mutating.md`, `inventory-kinds.md` | HPA, PVC, StorageClass actions done (0032b); Renew → 0018 follow-up |
