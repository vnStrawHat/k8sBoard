# 0045 · Test plan

[Back to index](README.md). All tests are offline. Headless tests use the 0026/0027 fixture: tokio runtime kept alive, `ClusterRuntime` global, writes off, two Live slots from `ClusterSession::live_fixture` with `seed_pods` and `seed_nodes` (decision 14); both clusters seed a pod `shop/api-0` in CrashLoopBackOff so the same issue exists twice. Names are binding (AC 2).

## Step 1 — Issues

| Module | Tests |
|---|---|
| `issue_table_tests.rs` | `issues_merge_in_board_order_across_clusters` (critical of B before warning of A), `ties_keep_slot_order`, `cluster_column_only_in_multi`, `cluster_column_cell_reads_slot_label`, `leaving_multi_drops_cluster_sort_and_hidden`, `user_sort_wins_over_board_order` |
| `issue_board.rs` tests | `sum_summaries_skips_unknown_slots`, `sum_summaries_is_none_until_one_is_known`, `sum_keeps_partial_and_worst` |
| `navigation.rs` tests | `issue_counts_merge_by_screen`, `issues_tooltip_lists_each_cluster` |
| `app_shell_multi_tests.rs` (headless) | `same_issue_in_two_clusters_is_two_rows`, `issue_click_reveals_in_its_own_cluster`, `issue_menu_open_uses_row_cluster`, `issue_menu_view_logs_uses_row_connection`, `boards_are_visible_on_every_slot_on_issues`, `issues_header_counts_all_clusters` (`2 clusters · 2 issues`), `title_flag_sums_clusters`, `failed_slot_keeps_banner_on_issues`, `single_mode_issues_unchanged` |

## Step 2 — Overview

| Module | Tests |
|---|---|
| `overview.rs` tests | `merge_attention_orders_across_slots`, `merge_attention_keeps_slot_index`, `merge_changes_newest_first`, `stats_parts_sum_ready_lists`, `stats_parts_dash_until_any_ready`, `single_slot_stats_text_unchanged` |
| `overview_report.rs` tests | `multi_report_has_a_section_per_cluster`, `multi_report_lists_not_connected_slots`, `multi_report_holds_no_server_url` |
| `app_shell_multi_tests.rs` (headless) | `overview_feeds_run_on_every_slot` (watch count = single + 2 per extra slot while shown; back on leave), `attention_click_reveals_in_its_own_cluster`, `heat_cell_reveals_node_in_its_cluster`, `change_row_reveals_in_its_cluster`, `export_canceled_when_view_changes`, `overview_header_reads_cluster_count`, `single_mode_overview_unchanged` |
| budget | `overview_merge_budget` (5 × 1,000 pods × 60 nodes; debug ≤ 80 ms; prints time) |

## Step 3 — Topology

| Module | Tests |
|---|---|
| `app_shell_multi_tests.rs` (headless) | `topology_draws_primary_by_default`, `topology_cluster_choice_sets_session`, `switching_topology_cluster_stops_old_feeds` (old slot's watch count drops to its closed value before the new one grows), `topology_node_click_selects_in_topology_cluster`, `row_key_on_topology_node_uses_topology_cluster`, `show_in_topology_switches_cluster`, `released_topology_cluster_falls_back_to_primary`, `single_switch_clears_topology_cluster` |
| `resource_actions_tests.rs` | `show_in_topology_enabled_for_other_cluster`, `show_in_topology_disabled_outside_scope` (kept) |
| `topology_view_tests.rs` | `set_session_keeps_graph_for_same_session` (kept), `set_session_resets_for_other_session` |
| `launch_options.rs` tests | `screen_overview_multi_parses`, `screen_issues_multi_parses`, `screen_topology_multi_parses` |
| `screenshot.rs` tests | `multi_screens_settle_when_slots_are_ready_or_failed` |
| budget | `issues_merge_budget` (5 × 400 issues; debug ≤ 80 ms; prints time) |

Removed tests that asserted the old primary-only rule (for example a `Showing {label} only` or `Topology draws only the primary cluster` text) are replaced by the tests above, not kept.

## Live checks (coder-lite, read-only)

`--kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --config-dir .tmp/config-0045 --view readonly@Monitor,<fixture ctx>`; never print the kubeconfig.

1. `--screen issues-multi`: UAT issues with the Cluster column, the fixture banner, header `2 clusters · {n} issues` counts UAT only.
2. `--screen overview-multi`: one Capacity block, merged attention, the fixture banner; the status-bar watch count matches budget.md.
3. `--screen topology-multi`: the Cluster dropdown lists both; picking the fixture shows its error view; picking UAT again redraws the graph.
4. Budget measurement steps of [budget.md](budget.md).

## ui-verifier (screenshot build; writes off)

| Screen | Expect |
|---|---|
| `issues-multi`, light and dark | Cluster column last with badges (W1), header count, no `Showing … only` line |
| `overview-multi` | cluster chips in Needs attention, headed Capacity and Nodes blocks (W3 layout kept), header `2 clusters` |
| `topology-multi` | `Cluster:` dropdown left of Fit, graph of the primary (W11) |
