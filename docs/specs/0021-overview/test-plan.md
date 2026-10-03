# 0021 · Test plan

[Back to index](README.md). Unit tests are offline and deterministic (fixed `now`, fixture summaries). Each test checks one behavior. Names are binding (AC 2).

## Step 1

| Module | Tests |
|---|---|
| `cluster_capacity.rs` | `cpu_and_memory_sum_used_requested_allocatable`, `requests_are_none_without_all_scope_pods`, `finished_pods_do_not_request`, `used_notes_unsampled_nodes`, `used_is_none_without_node_feed`, `pods_row_counts_pods_that_take_room`, `pods_row_note_without_all_scope`, `volumes_sum_claims_with_capacity`, `volumes_omitted_without_claims`, `volumes_note_limited_polling`, `zero_allocatable_omits_row`, `cpu_label_prints_unit_once`, `label_drops_req_without_requests`, `pods_label_groups_digits`, `volumes_label_shares_unit`, `compute_rows_name_their_ceilings`, `pods_on_unlisted_nodes_do_not_count`, `volumes_skip_shared_filesystem_claims`, `volumes_row_without_feed_shows_dash_and_reason`, `pods_loading_shows_dash_without_scope_note` |
| `usage_format.rs` | `format_shared_prints_unit_once`, `format_shared_keeps_units_that_differ`, `format_shared_never_shares_millicores` |
| `usage_bar.rs` | `capacity_bar_clamps_layers`, `capacity_bar_without_usage_has_no_used_layer` |
| `node_heatmap.rs` | `cells_keep_node_list_order`, `intensity_is_clamped_cpu_ratio`, `not_ready_and_unknown_have_no_intensity`, `missing_sample_has_no_intensity`, `tooltip_names_cpu_memory_status`, `tooltip_marks_cordoned_node` |
| `overview.rs` | `headline_has_context_version_region`, `region_single_value`, `region_counts_several`, `region_absent_without_labels`, `stats_count_ready_nodes`, `stats_count_running_and_not_ready_pods`, `stats_parts_are_none_while_loading` |
| `navigation.rs` | `enabled_items_are_overview_issues_pods_nodes_and_explorer_kinds` (Overview first) |
| `launch_options.rs` | `screen_overview_parses` |
| `screenshot.rs` | `overview_waits_for_node_metrics_and_kubelet` |

## Step 2

| Module | Tests |
|---|---|
| `event_tests.rs` | `change_event_selectors_name_kind_and_reason`, `watch_events_selector_unchanged` |
| `recent_changes_tests.rs` | `rollout_event_becomes_deployment_entry`, `rescale_event_reads_hpa`, `window_cuts_by_last_seen`, `one_hour_window_keeps_older_events`, `event_without_last_seen_is_skipped`, `actor_is_source_before_on`, `node_ready_transition_in_window`, `node_unknown_reads_stopped_reporting`, `joined_node_hides_its_ready_transition`, `namespace_created_in_window`, `newest_first_ties_by_object`, `targets_resolve_through_resource_key`, `count_kept_for_aggregated_events`, `none_inputs_contribute_nothing` |
| `cluster_session_tests.rs` | `open_watch_count_includes_change_events` (hidden +0, All +2, two namespaces +4), `change_feed_denied_by_known_review`, `change_feed_starts_while_checking_or_unknown` |
| `screenshot.rs` | `overview_waits_for_change_feed` |

## Step 3

| Module | Tests |
|---|---|
| `overview.rs` | `object_line_names_namespace_pod_container`, `object_line_counts_grouped_pods`, `object_line_cluster_object_has_no_namespace`, `view_logs_action_for_crash`, `pod_target_reads_see_why`, `secret_target_reads_open_secret` (`Open Secret`), `no_target_has_no_action`, `attention_shows_at_most_six` |
| `resource_actions_tests.rs` | `logs_launch_denied_without_log_access`, `logs_launch_needs_containers` (only if `logs_launch` is added) |
| `launch_options.rs` | `default_screen_is_overview`, `screen_pods_still_parses`, `window_width_parses`, `window_width_out_of_range_is_an_error` |
| `screenshot.rs` | `overview_waits_for_issue_summary` |

## Step 4

| Module | Tests |
|---|---|
| `file_export.rs` | the moved 0019 file-name tests (expecting `-`), plus `export_file_name_uses_extension`, `export_file_name_replaces_path_unsafe_chars` (`readonly@Monitor:a/b` → `readonly-Monitor-a-b`) |
| `overview_report.rs` | `report_lists_every_issue_not_only_six`, `report_has_coverage_note_when_partial`, `report_says_no_issues_when_empty`, `report_capacity_and_nodes_tables`, `report_changes_use_window_label`, `cell_escapes_pipes_and_newlines`, `report_has_no_server_or_user` |

## Live checks (coder-lite; UAT; `--context readonly@Monitor`; `--config-dir .tmp/…` once it exists)

1. Allocatable sums (AC 6): sum the output of `kubectl --context readonly@Monitor --kubeconfig monitor-uat-readonly.yml get nodes -o jsonpath='{range .items[*]}{.status.allocatable.cpu} {.status.allocatable.memory}{"\n"}{end}'` and compare it with the Capacity labels. This is a read-only `get`; never print the kubeconfig.
2. Heatmap (AC 7): the cell count equals the node count. Clicking one cell opens the Nodes screen with the drawer on that node.
3. Recent changes (AC 9): compare `kubectl … get events -A --field-selector reason=ScalingReplicaSet` for the last 15 min with the panel. If UAT has none, record "no rollouts in window" and check the empty text and the footnote.
4. Watch count (AC 5): the status-bar count on Pods vs Overview differs by exactly `2 × scope_multiplicity`.
5. Needs attention (AC 8, step 3): compare with the first 6 rows of the Issues screen; click View logs once.

## ui-verifier checklist (step 1: `overview`; step 2: `overview`; step 3: `overview`, `overview-narrow`; step 4: `overview` with Export)

- The stats line under the header is user-requested and not in W3; do not flag it.
- `--screen overview` at 1320 px: two columns at ≈ 1.5 : 1, with Needs attention top-left, Capacity top-right, Nodes bottom-left, and Recent changes bottom-right (W3 pins 1–4). Before step 3, row 1 holds only Capacity.
- `--screen overview --window-width 1000` (step 3): one column, in the order Needs attention, Capacity, Nodes, Recent changes. No horizontal overflow; rows truncate with an ellipsis.
- Capacity: CPU and Memory show three layers (used darker than requested), the legend matches, labels are right-aligned mono with one unit (`104 used · 131 req · 168 cores`).
- Heatmap: square cells, intensity varies, NotReady outlined in the Bad tone, header `{n} · colored by CPU`.
- Recent changes: time | text | who columns, and the muted footnote `… events kept ~1 h by the API server`.
- Needs attention rows: the pill in a 118 px column, a mono object line, a muted cause, and one ghost button (`View logs`, `See why`, `Open Secret`). The count pill is toned.
- No hardcoded colors; both light and dark themes (`--theme light|dark`) are legible.
