# 0020 · Test plan

[Back to index](README.md). Unit tests are offline and deterministic: fixed `now`, summaries built with struct literals. Names are binding (AC 2). Each test checks one behavior; table-driven tests check one rule over a case table.

## Step 1a

Names below are the tests as built. Four tests of the first plan needed a live connection (`core_updates_mark_board_dirty`, `scope_change_restarts_warning_events`, `retry_keeps_first_seen`, and a notify test on a session); their decisions are tested as pure functions instead, and the dirty marking and the debounced restart are checked live (screenshots).

| Module | Tests |
|---|---|
| `pod_tests.rs` (cluster) | `pod_condition_keeps_transition_time`, `pod_condition_and_waiting_messages_hide_url_userinfo` |
| `node_tests.rs`, `event_tests.rs` (cluster) | `node_condition_message_hides_url_userinfo`, `message_hides_url_userinfo` |
| `container_spec_tests.rs` (cluster) | `probe_defaults_period_and_threshold` (covers `initial_delay_seconds`) |
| `pod_diagnosis_tests.rs` | `cause_names_the_rule_that_fired`, `unschedulable_cause_carries_condition_time` |
| `kind_row.rs` | `pod_workload_maps_hashed_replica_set_to_deployment`, `pod_workload_keeps_other_controllers`, `pod_workload_none_without_controller` |
| `issue_rules_tests.rs` (pods) | `crash_loop_is_critical_with_view_logs`, `crash_and_exit_onset_is_ready_transition`, `image_pull_is_critical`, `other_waiting_reasons_are_critical`, `healthy_gated_and_finished_pods_have_no_finding`, `rule_graces` (table), `startup_grace_adds_the_initial_delay`, `unschedulable_onset_is_the_condition_time`, `stuck_creating_quotes_newest_warning_event`, `oom_restart_within_window`, `old_restart_is_quiet`, `restart_text_says_in_total`, `job_pod_exit_left_to_live_job_feed`, `job_pod_exit_reported_without_job_feed`, `old_eviction_is_quiet`, `memory_near_limit_needs_two_samples`, `cpu_cause_does_not_claim_throttling`, `no_limit_no_usage_finding` |
| `issue_rules_tests.rs` (nodes, volumes, events) | `node_not_ready_reads_the_ready_condition`, `network_unavailable_is_critical`, `node_pressure_lists_other_pressures`, `problem_detector_conditions_are_warnings`, `cordoned_node_is_not_an_issue`, `node_usage_at_the_limit_is_a_warning`, `usage_threshold_is_the_red_bar`, `volume_almost_full_from_kubelet`, `volume_fullness_reads_what_is_left`, `failed_create_on_replica_set_names_deployment`, `job_failure_events_are_warnings`, `event_burst_needs_count_and_window`, `event_burst_needs_recent_first_seen`, `failed_scheduling_is_never_an_event_issue`, `one_event_finding_per_object_with_the_highest_count`, `critical_failed_create_is_not_hidden_by_a_warning_burst`, `job_failure_event_is_not_hidden_by_a_burst`, `event_cause_is_cut_and_single_line` |
| `issue_board_tests.rs` | `pods_of_one_deployment_group_with_count`, `group_of_one_shows_the_pod`, `group_without_a_screen_shows_the_representative_pod`, `group_representative_is_oldest`, `one_issue_per_object_keeps_rule_order`, `event_on_grouped_pod_is_dropped`, `event_for_deleted_pod_is_dropped`, `event_on_a_deployment_pod_group_is_dropped_by_the_workload`, `missing_feed_skips_its_rules`, `issues_sort_by_severity_then_age`, `since_prefers_onset_over_first_seen`, `grace_holds_findings_back` (table), `held_finding_records_first_seen_and_takes_its_slot`, `first_seen_survives_refresh_and_is_pruned`, `first_seen_kept_while_pods_reload`, `first_seen_kept_while_events_reload`, `first_seen_is_dropped_when_the_feed_is_loaded_and_the_problem_is_gone`, `refresh_reports_change_only_when_different`, `an_age_that_moved_is_a_text_only_change`, `run_due_dirty_or_time_refresh`, `summary_none_until_pods_and_nodes_ready`, `a_failed_core_list_is_a_gap_not_endless_checking`, `summary_worst_follows_the_critical_count`, `count_for_screen_uses_reveal_target` |
| `issue_board_tests.rs` | `issue_evaluation_budget` (`#[ignore]`; 1,000 pods with 50 problems, 2,000 events; asserts ≤ 4 ms, so run with `--release -- --ignored`; the 1,000 objects join in step 2) |
| `issue_feeds_tests.rs` | `warning_events_slice_per_object`, `warning_events_of_none_until_ready`, `snapshot_replaces_the_index`, `coverage_note_groups_by_state`, `coverage_full_has_no_note`, `limited_feed_is_not_partial`, `core_feed_states_from_lists_and_metrics`, `volume_usage_is_limited_when_fewer_nodes_are_polled` |
| `issue.rs` | `critical_sorts_before_warning`, `involved_replica_set_is_reported_as_its_deployment`, `target_needs_a_screen_for_the_kind` |
| `metrics_history_tests.rs`, `kubelet_history_tests.rs` | `latest_container_pair_needs_both_ticks`, `pvc_usages_yield_newest_per_claim` |
| `cluster_session_tests.rs` | `time_refresh_notifies_only_when_issues_visible`, `hidden_board_does_not_notify_for_an_age_change`, `open_watch_count_counts_warning_events` |
| `navigation.rs` | `issue_count_suffix_uses_worst_severity`, `a_calm_cluster_shows_no_issue_numbers` |

## Step 1b

| Module | Tests |
|---|---|
| `issue_table.rs` | `issue_row_values_match_columns`, `object_cell_appends_container`, `count_cell_hides_one`, `quick_filter_matches_kind_and_cause` |
| `navigation.rs` | `issues_item_opens_issues_screen` (update `enabled_items_are_pods_nodes_and_explorer_kinds`) |
| `launch_options_tests.rs` | `screen_issues_parses` |

## Step 2

| Module | Tests |
|---|---|
| `issue_kind_rules.rs` | `rollout_stalled_is_critical`, `kind_rules_ignore_pod_derived_boxes` (D3/S1/S2 never fire with `pods: &[]`), `kind_graces` (table: DaemonSet S3/S4 and Job J4 → `ROLLOUT_GRACE`; D1, J1 → none), `job_backoff_limit_finding`, `hpa_at_max_is_capped_at_warning`, `pdb_blocks_drain_is_warning`, `quota_near_limit_names_first_item`, `at_quota_wins_over_near_limit`, `pvc_pending_needs_warning_event`, `pvc_pending_quiet_without_warning_feed`, `pvc_lost_is_critical`, `stuck_namespace_uses_kind_diagnosis`, `cert_expiring_uses_w3_wording`, `cert_expired_is_critical`, `unparsed_certificate_is_not_an_issue`, `title_becomes_sentence_case` |
| `issue_board_tests.rs` | `pod_group_hides_rollout_stalled_of_same_deployment`, `rollout_stalled_without_pod_problems_shows` |
| `issue_feeds_tests.rs` | `condition_plan_uses_scope_up_to_two_namespaces`, `condition_plan_uses_all_scope_above_two`, `condition_plan_waits_for_review`, `condition_plan_off_when_denied`, `all_scope_feed_drops_rows_outside_scope`, `forbidden_all_scope_feed_turns_off` |
| `cluster_session_tests.rs` | `open_watch_count_with_condition_feeds` (N = 1, 2, 3) |

## Live checks (coder-lite, UAT `readonly@Monitor`)

1. `--screen issues --screenshot .tmp/0020/issues.png`: the list loads; the status bar watch count matches `open_watch_count`.
2. Every Critical row is cross-checked with `kubectl get pods -A` / `get nodes` (read-only), names only in the report.
3. `--namespace a,b,c`: condition feeds run as All-scope watches; only rows of a, b, c appear; a 403 shows `not permitted cluster-wide`.
4. Budget: idle RSS after 5 min at scope All, before and after; the UAT Jobs count from the probe `count` line; `issue_evaluation_budget` in release. Numbers go into [decisions.md](decisions.md).
5. Read-only grep (0001) unchanged; `rg "tracing::" crates/app/src/issue*.rs` finds nothing.

## ui-verifier checklist

- Title bar: flag button between Read-only and Settings, toned, with the count (W2 `⚑ 4`).
- Sidebar: Issues item shows the toned total; Pods/Nodes/kinds show a toned issue count before the muted total; Overview and Topology disabled.
- Issues screen (1b): header, coverage text, columns and widths as [issues-screen.md](issues-screen.md); Critical rows first; no hardcoded colors.
- Clicking a pod issue opens Pods with its drawer and WHY box; a Secret issue opens the Secrets drawer (step 2).
