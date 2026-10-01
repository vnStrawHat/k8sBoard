# 0009 · Test plan

[Back to index](README.md). **S** is the step. Tests are deterministic and offline; each checks one behavior. Stream tests use `futures::stream::iter` and `block_on`.

## Unit tests

| S | File | Test | Checks |
|---|---|---|---|
| 1 | `namespace.rs` | `of_namespaces_normalizes_to_all_named_or_several` | none → All; one → Named; `[b, a, b]` → `Several([a, b])` |
| 1 | `namespace.rs` | `namespaces_lists_picked_names` | All empty; Named one; Several in order |
| 1 | `resource_watch_tests.rs` | `merge_waits_for_every_input` | no item until both inputs are settled (snapshot or failure) |
| 1 | `resource_watch_tests.rs` | `merge_concatenates_in_input_order` | `[a1, a2] + [b1]` |
| 1 | `resource_watch_tests.rs` | `merge_reemits_with_latest_of_each_input` | a second snapshot of input 1 keeps input 0's latest |
| 1 | `resource_watch_tests.rs` | `merge_emits_partial_snapshot_then_failure` | input 1 fails before data: snapshot of input 0, then `Failed` wrapped in `ClusterError::Namespace` naming it; its next snapshot re-emits both |
| 1 | `resource_watch_tests.rs` | `merge_coalesces_snapshots_within_a_window` | two input snapshots inside one `BATCH_WINDOW` → one merged emit (paused tokio time, like the Batcher tests) |
| 1 | `connection_tests.rs` | `scoped_apis_builds_one_api_per_namespace` | All → one unnamed; Several → one per name, in order |
| 1 | `navigation.rs` | `kind_availability_names_the_picked_namespaces_for_several` | `in a, b` |
| 1 | `resource_watch_tests.rs` | `merge_limit_keeps_newest_in_order` | limit 2 of 3 drops the oldest, order kept |
| 1 | `access_review.rs` | `all_of_allows_only_when_every_report_allows` | first denial reason kept |
| 1 | `pod_tests.rs` | `pod_summary_reads_labels_in_key_order` | |
| 1 | `cluster_session_tests.rs` | `namespaces_label_lists_two_then_counts_the_rest` | `a, b`; `a, b +3` |
| 2 | `table_filter_tests.rs` | `text_matches_name_namespace_columns_and_labels` | one row per source |
| 2 | `table_filter_tests.rs` | `text_ignores_ascii_case_and_surrounding_spaces` | |
| 2 | `table_filter_tests.rs` | `unhealthy_keeps_warn_bad_and_info` | Ok and Done dropped |
| 2 | `table_filter_tests.rs` | `equals_compares_a_column_value` | Absent never equals |
| 2 | `table_filter_tests.rs` | `label_tests_follow_kubectl_semantics` | exists, `=`, `!=` (absent key passes `!=`) |
| 2 | `table_filter_tests.rs` | `parse_label_queries_reads_comma_lists` | `label:a=b,c!=d,e` |
| 2 | `table_filter_tests.rs` | `parse_label_queries_rejects_bad_input` | no prefix, empty key, empty part |
| 2 | `table_filter_tests.rs` | `filter_parts_combine_with_and` | text + chip (step 3 test adds a preset) |
| 2 | `table_sort.rs` | `natural_cmp_orders_digit_runs_numerically` | `7 < 10`, `pod-2 < pod-10`, `v1.9 < v1.10`, `007 = 7` then tiebreak |
| 2 | `table_sort.rs` | `next_sort_cycles_ascending_descending_none` | and another column restarts at Ascending |
| 2 | `table_sort.rs` | `absent_sorts_last_in_both_directions` | |
| 2 | `table_sort.rs` | `status_sorts_by_severity_then_text` | |
| 2 | `table_sort.rs` | `age_ascending_puts_youngest_first` | |
| 2 | `table_sort.rs` | `span_uses_now_for_running_items` | |
| 2 | `table_view_tests.rs` | `rebuild_without_filter_or_sort_is_identity` | |
| 2 | `table_view_tests.rs` | `rebuild_sort_is_stable_on_ties` | equal keys keep source order |
| 2 | `table_view_tests.rs` | `row_of_and_item_index_round_trip` | |
| 2 | `table_view_tests.rs` | `clear_filter_keeps_sort_and_hidden_columns` | |
| 3 | `table_view_tests.rs` | `reset_filter_restores_the_default` | ReplicaSets back to HideInactive |
| 2 | `table_view_tests.rs` | `add_chips_dedupes` | |
| 2 | `table_view_tests.rs` | `rebuild_keeps_sort_on_a_hidden_column` | |
| 2 | `table_layout.rs` | `layout_columns_skips_hidden_and_maps_logical` | |
| 2 | `table_layout.rs` | `layout_columns_never_hides_the_flexible_column` | |
| 2 | `table_layout.rs` | `layout_columns_gives_spare_width_to_flexible` | replaces the per-delegate width tests |
| 2 | `pod_table.rs`, `node_table.rs`, `kind_table.rs` | `{pod,node,kind}_row_values_follow_columns` | one assertion per column variant |
| 2 | `table_selection_tests.rs` | `list_row_index_searches_the_view` | hidden subject → `Some(None)` |
| 2 | `launch_options_tests.rs` | `filter_flag_is_parsed` | |
| 3 | `node_summary.rs` | `node_counts_group_ready_not_ready_and_cordoned` | cordoned NotReady counted twice |
| 3 | `node_summary.rs` | `versions_sort_by_count_and_pick_common` | tie → higher version |
| 3 | `node_summary.rs` | `role_counts_skip_nodes_without_roles` | |
| 3 | `kind_table.rs` | `hide_inactive_drops_done_rows` | |
| 3 | `table_view_tests.rs` | `replica_sets_start_with_hide_inactive` | `default_filter`; other screens empty |
| 3 | `table_filter_tests.rs` | `view_pods_on_node_replaces_the_pod_filters` | `TableFilter::on_node(name)` has only the Node chip |
| 3 | `table_filter_tests.rs` | `filter_similar_replaces_an_existing_reason_chip` | `TableFilter::set_equals(column, title, value)`; other chips kept |
| 3 | `cluster_session_tests.rs` | `paused_flow_holds_newest_snapshot` | list rows unchanged; held replaced |
| 3 | `cluster_session_tests.rs` | `paused_flow_applies_failures` | interruption set |
| 3 | `cluster_session_tests.rs` | `resume_applies_held_snapshot` | |
| 3 | `resource_actions_tests.rs` | `filter_similar_disabled_without_reason` | |
| 3 | `resource_actions_tests.rs` | `view_pods_on_node_counts_pods_on_that_node` | |
| 4 | `namespace_picker.rs` | `open_copies_scope_into_draft` | |
| 4 | `namespace_picker.rs` | `toggle_refuses_past_max` | |
| 4 | `namespace_picker.rs` | `applied_scope_is_none_when_empty_or_unchanged` | |
| 4 | `launch_options_tests.rs` | `namespace_flag_accepts_a_comma_list_up_to_five` | six → error |
| 4 | `cluster_session_tests.rs` | `initial_scope_keeps_a_requested_several_scope` | |
| 4 | `namespace_picker.rs` | `open_records_the_anchor` | the other trigger stays closed |
| 5 | `table_view_tests.rs` | `rebuild_prunes_checked_to_visible_rows` | |
| 5 | `table_view_tests.rs` | `set_all_checked_touches_visible_rows_only` | |
| 5 | `table_view_tests.rs` | `check_range_checks_visible_rows_from_the_anchor` | both directions; no anchor acts as toggle |
| 5 | `row_selection.rs` | `bulk_actions_follow_the_wireframe` | Nodes three actions; Pods none |

## Live checks (coder-lite, UAT `readonly@Monitor`)

| S | Check |
|---|---|
| 1 | probe `--namespace A,B`: pods count = count(A) + count(B); AC7 credential script reports 0 |
| 2 | `--filter kube` and `--filter label:k8s-app=kube-dns` on Pods: `N of M match`, chip shown; `/` right after start focuses the filter |
| 2 | **release** build with `RUST_LOG=k8sboard=trace`: record the max `rebuild_view` `elapsed_us` on Pods and on Events (2,000 rows); within decision 1. Debug builds are slower and do not count |
| 3 | Events: Pause stream, wait 30 s, Resume shows newer rows; Nodes summary chip counts equal `kubectl get nodes` |
| 4 | `--namespace kube-system,default`: both namespaces' pods; title `ns: default, kube-system`; adding a namespace the user cannot list (if any) shows the others plus a banner |

## ui-verifier screenshots (light and dark)

| S | Screen | Compare with |
|---|---|---|
| 2 | `pods --filter kube`, `pods --filter label:k8s-app=kube-dns`, `deployments`, `events` | W4 header and filter bar, W7 filter bar with Columns ▾ |
| 3 | `nodes`, `replicasets`, `events` | W5 summary row and skew tint, W7 top buttons |
| 4 | `pods --namespace kube-system,default` | W4 Namespace chips, title bar `ns: a, b` |
| 5 | `pods-selected`, `nodes-selected` | W5 selection bar: Cordon, Uncordon, Drain… disabled |
