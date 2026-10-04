# 0046 · Steps, files, hot spots

[Back to index](README.md). One coder lane, steps in order; the gate is green after each. Each step deletes what it leaves unused, **including `#[cfg(test)]` hooks and test helpers** (no `#[allow(dead_code)]`, no shims). `crates/cluster`, `Cargo.toml`, `Cargo.lock`, `clippy.toml` are not touched. All files under `crates/app/src/`.

## Landing order

After 0039 merges, before the queued lanes (0022 RBAC, 0044, 0029 step 5). Steps 1–4 back to back; step 5 (renames) right after those lanes merge. Record the commit step 1 starts from as the **step-1 base** (write-safety.md reviewer checks).

## Step 1 — tests first (no production change)

| File | Change |
|---|---|
| `app_shell_multi_tests.rs` | **move** the tests marked P in [test-plan.md](test-plan.md) to `app_shell_switch_tests.rs` (with the `seed_pods` / `pod` helpers they need); the file and its `#[path]` mod **stay** until step 2, because `view_connects`, `ViewConnectCheck`, and `released_sessions` are read only there; delete any helper of the file the move leaves unused |
| `app_shell_write_tests.rs` `two_clusters` | becomes "prod-a and stg-b loaded, start on prod-a, `switch_cluster(stg-b)`, go Live"; tests that act on prod-a rows switch back first; add the step-1 safety tests ([write-safety.md](write-safety.md)) |
| `debug_open_tests.rs` `two_clusters` (also used by `node_shell_open_tests.rs`, `node_shell_sweep_tests.rs`) | same pattern; `sweep_runs_on_each_slots_first_live` → `sweep_runs_on_first_live` |
| `port_forward_open_tests.rs`, `shell_open_tests.rs` `two_clusters` | same pattern; `remove_from_view` tests become switch tests or are deleted (test-plan.md) |
| `app_shell_edit_tests.rs`, `app_shell_drain_tests.rs`, `node_shell_cleanup_tests.rs` | `remove_from_view` calls → `switch_cluster` (with the `leaving_work` confirm) |
| `app_shell_delete_tests.rs`, `app_shell_workload_tests.rs`, `app_shell_drain_tests.rs` | delete the headless "rows of two clusters" tests; the pure `batch_requires_one_cluster`, `scope_refuses_two_clusters` stay |
| `app_shell_switch_tests.rs`, `keymap_tests.rs` | `space_ticks_a_row_and_never_confirms` stays until step 2 (it names `ToggleClusterTick`) |

## Step 2 — entry points and lifecycle

| File | Change |
|---|---|
| `app_shell_multi_tests.rs` | delete the file and its mod; with it `view_connects`, `ViewConnectCheck`, `released_sessions` (`app_shell.rs`, `app_shell_session.rs`) |
| `cluster_switcher.rs`, `cluster_switcher_rows.rs` (+ tests) | ticks, checkbox, footer, tick notice, `ToggleClusterTick` gone; `space` → `gpui_kit::NoAction` in both contexts |
| `app_shell.rs` | tick methods, `--view` start branch, `named_clusters`, multi branches of `switch_to`, `view_request`, `view_scope`, `subject_cluster`; `release_all` leaves the forwards alone (decision 17) |
| `app_shell_session.rs` | delete `view_clusters` … `remove_from_view` and `release_slot` (inventory.md); rewrite the module doc |
| `dock.rs` `close_tabs_of`, `edit_yaml_flow.rs` `close_edit_of`, `cluster_view.rs` `ClusterView::remove` | delete (orphans of `release_slot`) |
| `cluster_view.rs` (+ tests) | delete `plan_view`, `ViewPlan`, `TooManyClusters`, `MAX_VIEWED_CLUSTERS`, `reorder` |
| `workspace.rs` (+ tests) | multi body, banners, header count, `Showing {label} only`, `Retry all` |
| `node_shell_sweep.rs` | `show_leftover_notice` drops a notice whose cluster is not active |
| `launch_options.rs`, `screenshot.rs` (+ tests) | `--view`, `pods-multi`, multi settle |
| `leaving_work.rs` | module doc only |
| `keymap_tests.rs`, `app_shell_switch_tests.rs` | `ToggleClusterTick` → `NoAction` assertions (test-plan.md) |

## Step 3 — leaves

| File | Change |
|---|---|
| `title_bar.rs` | trigger, border, lock badge for the active cluster only; no dropdown |
| `status_bar.rs`, `namespace_picker.rs`, `navigation.rs` (+ tests) | multi line, union, `SlotCounts` |
| `dock.rs`, `log_tab.rs`, `shell_tab.rs`, `drawer.rs` | `is_multi`, `set_multi`, title suffix, `DrawerCluster` when unused |
| `palette_search.rs`, `command_palette.rs` (+ tests) | one `PaletteSession` |
| `resource_actions.rs`, `cluster_rows.rs` (`RowContext`) (+ tests) | `Filter by this cluster`, `is_primary`, `primary_label`, `is_multi`; menu actions require `row.session.upgrade()` |
| `app_shell.rs`, `write_lock.rs`, `overview.rs`, `issue_table.rs`, `node_heatmap.rs`, `topology_view.rs`, `workspace.rs`, `keyboard_navigation.rs`, `namespace_picker.rs`, `title_bar.rs` | fan-out loops, `reveal_in_primary`, `primary_object`, `context_cluster`, `lock_target`, `scope_live` → `live`, `filter_by_cluster`, `sync_drawer_cluster` |
| `cluster_view.rs`, `environment.rs` | `is_multi`, `riskiest` |

## Step 4 — model, tables, connect callers

| File | Change |
|---|---|
| `active_session.rs` (new), `cluster_view.rs` (deleted), `main.rs` mods | `ActiveSession` |
| `cluster_rows.rs` (+ tests, deleted) → `row_context.rs` | `TableSession`, `RowContext` |
| `pod_table.rs`, `node_table.rs`, `kind_table.rs`, `issue_table.rs`, `table_layout.rs`, `table_view.rs`, `table_selection.rs` (+ tests) | one session; no merge; backup tick clear in `set_session`; `TableRow::cluster`, `RowName.cluster`, `session_column` gone; `clear_all_filters` in `release_all` untouched |
| `debug_open.rs`, `node_shell_open.rs`, `shell_open.rs`, `drain_driver.rs`, `node_shell_cleanup.rs`, `port_forward_page.rs`, `write_lock.rs` | exact replacements in [inventory.md](inventory.md) "Connect and write-path callers"; `label_of` moves to `app_shell.rs` |
| `port_forward_dialogs.rs`, `screenshot.rs` | New-forward form: one fixed cluster, no `Select`; `port-forward-new-fixture` over one cluster |
| `app_shell.rs`, `app_shell_session.rs`, `port_forward_dialogs.rs`, `workspace.rs` | `view` → `active_session`; `primary_cluster` → `active_cluster`; `session_of` body |
| — | final greps, ui-verifier run (test-plan.md) |

## Step 5 — renames (after the queued lanes merge)

Mechanical, no behavior change: the table in [single-session.md](single-session.md) "Step 5 renames", plus test and doc-comment mentions. Gate only.

Also left for this step (single-session leftovers, no behavior change): `open_clusters() -> Vec<ClusterRef>` (one entry at most, an `Option`), `viewed_health -> Vec`, and the `cluster_label` field of `shell_tab.rs`.

## Hot spots with 0034 (merged at `0a1109d`, `6662108`)

| File | Site | Effect |
|---|---|---|
| `drain_dialog.rs` | `start_drain_of_ticked` (~1349), `guard_for` / `live_of` (~577, 605, 1211) | no code change (K2, K3) |
| `drain_driver.rs` | drain start (~105, ~124), `stop_all_drains_now` (~186) | step 4 replacements |
| `node_editor.rs` | `ticked_nodes` "Select rows of one cluster" (~632); `guard_for` (~614, 662, 697) | unchanged |
| `app_shell_drain_tests.rs` | `drain_uses_the_cursor_slot` (~332), `the_bar_drain_is_off_across_clusters` (~824), `slot_release_stops_the_drain` (~1190), `the_bar_drain_refuses_ticks_of_two_clusters_instead_of_dropping_one` (~1424) | step 1: port to a switch, or delete (test-plan.md) |
| `row_selection.rs` `NODE_ACTIONS`, `leaving_work.rs` `drains` | via `bulk_state`; 0034 line | unchanged |

## Docs after merge (orchestrator)

- 0028 `keymap.md` Space row: "`space` → `NoAction` in the switcher (0046); no tick".
- 0026 README: one line "0027 multi view removed by 0046".
- Roadmap rows "removal 0046" → "Removed (0046)". This README: tick ACs.
