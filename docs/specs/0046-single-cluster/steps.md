# 0046 · Steps, files, hot spots

[Back to index](README.md). One coder lane, steps in order; the gate is green after each (each step deletes what it leaves unused; no `#[allow(dead_code)]`, no shims). `crates/cluster`, `Cargo.toml`, `Cargo.lock`, `clippy.toml` are not touched. All files under `crates/app/src/`.

## Step 1 — tests first (no production change)

| File | Change |
|---|---|
| `app_shell_multi_tests.rs` | port the tests marked P in [test-plan.md](test-plan.md) to `app_shell_switch_tests.rs`; delete the file and its `#[path]` mod |
| `app_shell_write_tests.rs` `two_clusters` | becomes "prod-a and stg-b loaded, start on prod-a, `switch_cluster(stg-b)`, go Live"; tests that act on prod-a rows switch back first; add the safety tests ([write-safety.md](write-safety.md)) |
| `debug_open_tests.rs` `two_clusters` (also used by `node_shell_open_tests.rs`, `node_shell_sweep_tests.rs`) | same pattern |
| `port_forward_open_tests.rs`, `shell_open_tests.rs` `two_clusters` | same pattern; `remove_from_view` tests become switch tests or are deleted (list in test-plan.md) |
| `app_shell_edit_tests.rs`, `app_shell_drain_tests.rs`, `node_shell_cleanup_tests.rs` | `remove_from_view` calls → `switch_cluster` (with `leaving_work` confirm) |
| `app_shell_delete_tests.rs`, `app_shell_workload_tests.rs`, `app_shell_drain_tests.rs` | delete the headless "rows of two clusters" tests; the pure `batch_requires_one_cluster`, `scope_refuses_two_clusters` stay |
| `app_shell_switch_tests.rs`, `keymap_tests.rs` | `space_ticks_a_row_and_never_confirms` stays until step 2 (it names `ToggleClusterTick`) |

## Step 2 — entry points and lifecycle

| File | Change |
|---|---|
| `cluster_switcher.rs`, `cluster_switcher_rows.rs` (+ tests) | ticks, checkbox, footer, tick notice, `ToggleClusterTick` gone; `space` → `gpui_kit::NoAction` in both contexts |
| `app_shell.rs` | tick methods, `--view` start branch, `named_clusters`, multi branches of `switch_to`, `view_request`, `view_connects`, `released_sessions`, `view_scope`, `subject_cluster` |
| `app_shell_view.rs` | delete `view_clusters` … `remove_from_view` (inventory.md); rewrite the module doc |
| `cluster_view.rs` (+ tests) | delete `plan_view`, `ViewPlan`, `TooManyClusters`, `MAX_VIEWED_CLUSTERS`, `reorder` |
| `workspace.rs` (+ tests) | multi body, banners, header count, `Showing {label} only`, `Retry all` |
| `launch_options.rs`, `screenshot.rs` (+ tests) | `--view`, `pods-multi`, multi settle |
| `leaving_work.rs` | module doc only |
| `keymap_tests.rs`, `app_shell_switch_tests.rs` | `ToggleClusterTick` → `NoAction` assertions (test-plan.md) |

## Step 3 — leaves

| File | Change |
|---|---|
| `title_bar.rs` | trigger, border, lock badge for the active cluster only; no dropdown |
| `status_bar.rs`, `namespace_picker.rs`, `navigation.rs` (+ tests) | multi line, union, `SlotCounts` |
| `dock.rs`, `log_tab.rs`, `shell_tab.rs`, `drawer.rs` | `is_multi`, `set_multi`, title suffix, `close_tabs_of`, `DrawerCluster` when unused |
| `palette_search.rs`, `command_palette.rs` (+ tests) | one `PaletteSession` |
| `resource_actions.rs`, `cluster_rows.rs` (`RowContext`) (+ tests) | `Filter by this cluster`, `is_primary`, `primary_label`, `is_multi` |
| `app_shell.rs`, `write_lock.rs`, `overview.rs`, `issue_table.rs`, `node_heatmap.rs`, `topology_view.rs`, `workspace.rs` | fan-out loops, `reveal_in_primary`, `primary_object`, `context_cluster`, `lock_target`, `scope_live`, `filter_by_cluster`, `sync_drawer_cluster` |
| `cluster_view.rs`, `environment.rs` | `is_multi`, `riskiest` |

## Step 4 — model and tables

| File | Change |
|---|---|
| `active_session.rs` (new), `cluster_view.rs` (deleted), `main.rs` mods | `ActiveSession` |
| `cluster_rows.rs` (+ tests, deleted) → `row_context.rs` | `TableSession`, `RowContext` |
| `pod_table.rs`, `node_table.rs`, `kind_table.rs`, `issue_table.rs`, `table_layout.rs`, `table_view.rs`, `table_selection.rs` (+ tests) | one session; no merge; `set_session` clears ticks on a cluster change; `TableRow::cluster`, `RowName.cluster`, `session_column` gone |
| `app_shell.rs`, `app_shell_view.rs`, `port_forward_page.rs`, `port_forward_dialogs.rs`, `workspace.rs`, `write_lock.rs` | `view` → `active_session`; `primary_cluster` → `active_cluster`; `slot_session` body |
| — | final greps, ui-verifier run (test-plan.md) |

## Hot spots with 0034 (merged at `0a1109d`, `6662108`)

| File | Site | Effect |
|---|---|---|
| `drain_dialog.rs` | `start_drain_of_ticked` (~1349), `guard_for` / `slot_live` (~577, 605, 1211) | no code change (K2, K3); comment "Ticks of another cluster" may stay |
| `node_editor.rs` | `ticked_nodes` "Select rows of one cluster" (~632); `guard_for` (~614, 662, 697) | unchanged |
| `app_shell_drain_tests.rs` | cursor on the second cluster with a locked primary (~332); `BulkState::Off("Select rows of one cluster")` (~824); `remove_from_view` mid-drain (~1190); `the_bar_drain_refuses_ticks_of_two_clusters_instead_of_dropping_one` (~1424) | step 1: port to a switch, or delete (test-plan.md) |
| `row_selection.rs` `NODE_ACTIONS` bulk cordon | via `bulk_state` | unchanged |
| `leaving_work.rs` `drains` | 0034 line | unchanged |

Other lanes touching the same files: 0022 RBAC layer (`topology_view.rs`), 0039 (`resource_actions.rs`, drawers), 0044 (`dock.rs`, `log_tab.rs`), 0029 step 5 (`palette_search.rs`). Prefer landing 0046 first; else rebase them after step 4.

## Docs after merge (orchestrator)

- 0027 README: Status → superseded line already added; tick nothing new.
- 0028 `keymap.md` Space row: "`space` → `NoAction` in the switcher (0046); no tick".
- 0026 README: one line "0027 multi view removed by 0046".
- Roadmap rows already marked "being removed by 0046" → "Removed (0046)".
- This README: tick ACs.
