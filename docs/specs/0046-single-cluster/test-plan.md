# 0046 · Test plan

[Back to index](README.md). All offline, on the 0026 switch fixture (two kubeconfig contexts `prod-a`, `stg-b` over dead ports, `open_switch_fixture`, `go_live`, and the `seed_pods` helper moved from `app_shell_multi_tests.rs`). **P** port (rewrite on one active cluster, same intent), **D** delete, **A** add. Names are final; a ported test may keep its old name when it still reads right.

## Fixture rule (step 1)

Every `two_clusters` / `two_live_clusters` helper becomes: both contexts loaded, start on `prod-a`, `switch_cluster(stg-b)`, `go_live`. A test that acted on `prod-a` rows switches back first. `remove_from_view(c)` becomes `switch_cluster(other)` plus the `leaving_work` confirm where the test expects the dialog.

## `app_shell_multi_tests.rs` (P moves in step 1; the file and the rest go in step 2)

| Do | Tests |
|---|---|
| P → `app_shell_switch_tests.rs` (step 1) | `last_used_is_written_on_live`, `last_used_is_not_written_on_failure`, `last_used_is_written_once_per_session`, `select_all_follows_the_ticks`, `a_closed_drawer_keeps_its_cursor_but_does_not_steer_a_bare_reveal` (→ `a_closed_drawer_keeps_its_cursor`), `the_palette_of_a_single_cluster_adds_no_cluster_label` (→ `palette_resource_entries_have_no_cluster_label`), `the_yaml_key_acts_on_the_cursor_row_of_its_cluster` (→ `the_yaml_key_acts_on_the_cursor_row`), `right_click_menu_action_acts_on_the_clicked_row`, `a_disabled_menu_item_confirmed_by_key_acts_on_the_clicked_row` |
| D (step 2, with the file and its hooks `view_connects`, `ViewConnectCheck`, `released_sessions`) | every other test in the file (apply/plan, fan-out, ticks, Space ticks, footer, header sums, merged rows, cluster-qualified selection/checks/reveal, banners, Retry all, YAML/log/subject/kubelet/drawer per slot, `--view`, watch counts per slot, Filter by this cluster, Cluster column state, sixth tick, pending multi connect, name click keeps session, palette entry of B, palette leaves multi, `gate_and_confirm_use_the_rows_cluster_with_*_primary`) |

## Step 1 — other files

| File | D | P |
|---|---|---|
| `app_shell_write_tests.rs` | — | fixture; `the_lock_chord_acts_on_the_cursor_cluster` → `the_lock_chord_acts_on_the_active_cluster`; every `cordon_on_a_*_row_*` on the cluster it names |
| `debug_open_tests.rs`, `node_shell_open_tests.rs`, `node_shell_sweep_tests.rs` | — | fixture; `the_debug_item_reads_the_gate_of_its_own_cluster` on the active cluster; `sweep_runs_on_each_slots_first_live` → `sweep_runs_on_first_live` |
| `shell_open_tests.rs` | `slot_release_closes_only_its_shells`, `a_view_change_that_keeps_every_shell_asks_nothing` | fixture; `open_shell_uses_the_cursor_slot` |
| `port_forward_open_tests.rs` | `release_slot_keeps_forwards` (covered by `switch_cluster_keeps_forwards`) | fixture; `release_adds_no_leaving_work_line` → `a_switch_with_forwards_asks_nothing`; `start_needs_a_viewed_cluster` → `start_needs_the_active_cluster` (`Open {cluster} first`) |
| `app_shell_drain_tests.rs` | `the_bar_drain_is_off_across_clusters`, `the_bar_drain_refuses_ticks_of_two_clusters_instead_of_dropping_one` | `drain_uses_the_cursor_slot` (active cluster); `slot_release_stops_the_drain` → `a_switch_stops_the_drain` |
| `app_shell_edit_tests.rs` | `releasing_another_cluster_keeps_the_editor` | `release_of_the_edited_cluster_asks_first` → `a_switch_with_an_unsaved_edit_asks_first` (if not a duplicate of `cluster_switch_with_changes_asks_to_discard`, else D); `a_clean_editor_closes_with_its_released_cluster_without_asking` via a switch |
| `node_shell_cleanup_tests.rs` | — | `releasing_a_slot_deletes_only_its_pods` → `a_switch_deletes_the_node_shell_pods_of_the_old_cluster` |
| `app_shell_delete_tests.rs`, `app_shell_workload_tests.rs` | `the_bar_button_is_off_for_rows_of_two_clusters`, `rows_of_two_clusters_have_no_bulk_action` | — (pure `batch_requires_one_cluster`, `scope_refuses_two_clusters` stay) |

Step 1 also adds the unmarked safety tests of [write-safety.md](write-safety.md) (9 tests, **A**); the ones marked (step 2) or (step 3) land with that step. Any helper or `#[cfg(test)]` hook a step-1 deletion leaves unread is deleted in step 1.

## Step 2

| Do | Tests |
|---|---|
| D | `cluster_switcher_rows_tests.rs` tick tests (`toggle_tick*`, `ticks_differ*`, `is_ticked*`); `cluster_view_tests.rs` plan tests; `cluster_switcher.rs` `tick_notice_line` test; `launch_options_tests.rs` `--view` and `pods-multi` tests; `workspace.rs` banner and `clusters_count_text` tests |
| P | `space_ticks_a_row_and_never_confirms` → `space_never_confirms_in_the_switcher` (the switcher-context binding for `space` is `NoAction`; no `SwitcherConfirm`, no kit `Confirm` wins); `keymap_tests.rs` switcher binding table without `ToggleClusterTick` |
| A | `space_keeps_the_switcher_open` (`app_shell_switch_tests.rs`): Space in the filter and on a row; popover open, active cluster unchanged, no session created |
| A | `enter_always_switches_to_the_highlight` (`app_shell_switch_tests.rs`) |
| A | `a_leftover_notice_landing_after_a_switch_is_dropped` (write-safety.md) |

## Step 3

| Do | Tests |
|---|---|
| D | `title_bar.rs` `trigger_shows_plus_n_and_primary_label`, `trigger_tooltip_lists_the_viewed_clusters`, several-cluster `badge_face` cases; `navigation.rs` `sidebar_*_over_slots`, `sidebar_count_sums_known_slots`; `namespace_picker` union tests; `status_bar` multi tests; `log_tab` / `dock` suffix tests; `palette_search_tests.rs` cluster-label tests; `resource_actions_tests.rs` `Filter by this cluster` and Topology-primary tests; `environment_tests.rs` `riskiest` use; `cluster_view_tests.rs` `border_uses_riskiest`, `riskiest_is_the_max_environment` |
| A | `the_lock_badge_toggles_the_active_cluster` (`app_shell_write_tests.rs`): a click locks at once, unlocking opens the tier dialog; no menu |
| A | `a_menu_built_on_a_does_nothing_after_switching_back` (write-safety.md) |

## Step 4

| Do | Tests |
|---|---|
| D | `cluster_rows_tests.rs` (file); `table_selection_tests.rs` address tests; `table_view_tests.rs` cluster-in-`RowName` tests; `cluster_view_tests.rs` (file) |
| A | `set_session_clears_ticks_and_anchor_on_any_change` (pod delegate: A → B, A → `None` → A); `slot_session_is_none_for_a_cluster_that_is_not_active` (`app_shell_switch_tests.rs`); `the_new_forward_form_shows_the_active_cluster_only` (`port_forward_dialogs_tests.rs`); `quit_stops_a_drain_of_a_cluster_just_left` (`app_shell_drain_tests.rs`) |
| keep green | every step-1 safety test; `unknown_fields_are_ignored`; `switch_cluster_keeps_forwards` |

## Final greps (step 4, must print nothing)

```bash
grep -rnE "ClusterView|ViewSlot|plan_view|ViewPlan|MAX_VIEWED_CLUSTERS|TooManyClusters|view_clusters|remove_from_view|ToggleClusterTick|toggle_tick|ticks_differ|merge_rows|merge_slot_rows|Clustered|RowAddress|CLUSTER_COLUMN|session_column|is_multi\b|set_multi|riskiest\(|reveal_in_primary|primary_cluster|pods-multi|PodsMulti|SlotCounts|Filter by this cluster|of \{total\} unlocked" crates/app/src
grep -rnF -- "--view" crates/app/src
```

`grep -rn "Select rows of one cluster" crates/app/src` must still list the 5 production sites (write-safety.md K3).

## Live and UI (step 4)

- coder-lite: `--screen pods --context readonly@Monitor` on UAT, then a switch to an unreachable fixture and back: no panic, status bar single line, no write sent (denied path only; `K8SBOARD_ALLOW_WRITES` unset).
- ui-verifier: `switcher`, `pods`, `nodes`, `pods-selected`, `port-forward-new-fixture`, light and dark; AC 14.

## Step 5

No new test. Gate only; the step-4 greps plus `grep -rnE "fn (slot_session|slot_live|slot_connection|slot_label|new_slot|on_slot_changed|slot_row_context)" crates/app/src` print nothing.
