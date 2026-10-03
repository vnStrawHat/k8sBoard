# 0033 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own, and every new item has a production user in its own step. Baseline main `2c7dc08`. Prerequisites: step 1 after 0031 steps 0–1; step 2 after 0030 steps 2b + 4, 0031 step 3, and 0032 2a-i + 2b (README). **C3: Approved by the user on 2026-10-02 (one approval for all mutating specs).** No Cargo change.

## `crates/cluster` (step 1)

| File | Change |
|---|---|
| `src/object_write.rs` (+ `object_write_tests.rs`) | `WriteOperation::DeleteObject`, `DeletePropagation` (+ `as_str`), `WriteEffect::{Deleted, DeletionPending}` (variants of the 0030 enum), the `delete` arm in the excepted `match`, `changed_fields`, manual `Debug` |
| `src/object_yaml.rs` (+ tests) | `ObjectIdentity`, `ClusterConnection::object_identity` (`get_metadata`), `ObjectKind::owns_dependents` |
| `src/access_review.rs` (+ tests) | `AccessCheck::Delete(ObjectKind)` (lazy, not in `ALL`) |
| `src/lib.rs` | export `DeletePropagation`, `ObjectIdentity` |

## `crates/app`

| S | File | Change |
|---|---|---|
| 1 | `src/cluster_session.rs` (+ tests) | `request_kind_access` also reviews `Delete(kind)` (0031's mechanism) |
| 2 | `src/object_delete.rs` (new) + `object_delete_tests.rs` | `delete_scope`, `DeleteTarget`, `delete_items`, `delete_warnings` (kind table, reclaim policy, controller), finalizer and notice texts, dependents texts, `AppShell::start_delete` |
| 2 | `src/write_flow.rs` (+ tests) | `BatchExtras::Delete` and its arm in the 0032 Batch flow: NotFound → `already_gone`, propagation change rebuilds items and reruns the dry-runs, delete summary parts |
| 2 | `src/confirm_dialog.rs` | `Dependents` `RadioGroup`, warning and finalizer lines, `already gone` line, object-name typed field, danger primary |
| 2 | `src/audit_log.rs` (+ tests) | no format change; test `delete_records_propagation_only` |
| 2 | `src/resource_actions.rs` (+ tests), `src/resource_kind.rs` | `ResourceAction::Delete(ObjectKind)` (unit `Planned` on main): gate `Mutating { Delete(kind) }`, `Destructive`, shipped (single); `subject_action` excludes Helm releases and custom kinds (decision 26); pod and node menus gain the danger last item; kind menus replace the disabled item (Helm releases and custom kinds keep it) |
| 2 | `src/keyboard_navigation.rs` (0028), `src/keymap.rs` (+ tests) | the `Delete(_)` arm → `start_delete(delete_scope(subject, checked))` (on main it shares an empty arm with `EditYaml`); macOS `cmd-backspace` → `Delete` in `WORKSPACE` (`delete` is bound on main) |
| 2 | `src/launch_options.rs` (+ tests), `src/screenshot.rs`, `src/main.rs` | `--screen delete-confirm`; `USAGE`; `mod object_delete;` |
| 3 | `src/row_selection.rs` (+ tests) | `Delete…` danger button on every screen, gated, reason tooltip |
| 3 | `src/resource_actions.rs` | checked-row menu: `Delete {n} {plural}…` on the checked set |
| 3 | `src/launch_options.rs`, `src/screenshot.rs` | `--screen delete-bulk-confirm` |
| 2–3 | palette (0029) | `Delete…` through the same gate |

## Requested from other specs (done: both rows are in 0030 write-path.md and 0032 bulk-write.md on main)

| File | Change |
|---|---|
| `0030-guardrails-write-path/write-path.md` | allow-list row `DeleteObject` (`DELETE {path}/{name}`, body with `propagationPolicy`, `preconditions.uid`, `dryRun` in the body, no query, no `fieldManager`) |
| `0032-workload-actions/bulk-write.md` | `BatchExtras::Delete { propagation, already_gone, warnings }`; NotFound → skipped for delete; "all items must pass"; extras can rebuild items and rerun the dry-runs; `expected_name` honored in the list variant |

## Docs (with the step that ships them)

| S | File | Change |
|---|---|---|
| 2 | `docs/specs/0028-keyboard-map/keymap.md` | Del enabled through the gate; ⌘⌫ bound on macOS (closes 0028 open item 5) |
| 3 | `docs/specs/0009-table-toolkit/row-selection.md` | selection bar gains `Delete…` |
| 3 | `docs/roadmap/gap-plan-local-and-mutating.md`, `inventory-kinds.md`, `inventory-screens.md` | 0033 delete done; Restart pod and Evict still open (0032 / 0034) |

## Parallel work (lane W1, after 0031)

Disjoint from lane W2: `object_delete.rs`, the `BatchExtras::Delete` arm. Shared, append-only: `object_write.rs` (0034 and 0037 add variants in W2), `object_yaml.rs` (`object_identity` next to 0031's `get_object`), `access_review.rs`, `resource_actions.rs` (`node_menu` also changes in 0034 and 0037), `keyboard_navigation.rs`, `keymap.rs`, `row_selection.rs` (0034 Nodes), `confirm_dialog.rs`, `launch_options.rs`, `screenshot.rs`.
