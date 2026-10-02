# 0020 · Test plan

[Back to index](README.md). Unit tests are offline and deterministic: fixed `now`, summaries built with struct literals. Names are binding (AC 2). Each test checks one behavior; table-driven tests check one rule over a case table.

## Step 1a

| Module | Tests |
|---|---|
| `pod_tests.rs` (cluster) | `pod_condition_keeps_transition_time` |
| `pod_diagnosis_tests.rs` | `cause_names_the_rule_that_fired` (one case per `DiagnosisCause` variant), `unschedulable_cause_carries_condition_time` |
| `kind_row.rs` | `pod_workload_maps_hashed_replica_set_to_deployment`, `pod_workload_keeps_other_controllers`, `pod_workload_none_without_controller` |
| `issue_rules_tests.rs` | `crash_loop_is_critical_with_view_logs`, `crash_and_exit_onset_is_ready_transition`, `image_pull_is_critical`, `rule_graces` (table: PodUnschedulable, PodNotReady, PodStuck, PodStartup from probe and fallback, NodeNotReady Unknown vs False → expected `grace`), `stuck_creating_quotes_newest_warning_event`, `oom_restart_within_window`, `old_restart_is_quiet`, `restart_text_says_in_total`, `job_pod_exit_left_to_live_job_feed`, `job_pod_exit_reported_without_job_feed`, `old_eviction_is_quiet`, `memory_near_limit_needs_two_samples`, `cpu_cause_does_not_claim_throttling`, `no_limit_no_usage_finding`, `node_pressure_lists_other_pressures`, `cordoned_node_is_not_an_issue`, `volume_almost_full_from_kubelet`, `failed_create_on_replica_set_names_deployment`, `event_burst_needs_count_and_window`, `event_burst_needs_recent_first_seen`, `failed_scheduling_is_never_an_event_issue` |
| `issue_board_tests.rs` | `pods_of_one_deployment_group_with_count`, `group_of_one_shows_the_pod`, `group_representative_is_oldest`, `one_issue_per_object_keeps_rule_order`, `event_on_grouped_pod_is_dropped`, `event_for_deleted_pod_is_dropped`, `missing_feed_skips_its_rules`, `issues_sort_by_severity_then_age`, `since_prefers_onset_over_first_seen`, `grace_holds_findings_back` (table: onset age below / at / above grace, onset None using first-seen), `held_finding_records_first_seen_and_takes_its_slot`, `first_seen_survives_refresh_and_is_pruned`, `refresh_reports_change_only_when_different`, `run_due_dirty_or_time_refresh`, `summary_none_until_pods_and_nodes_ready`, `count_for_screen_uses_reveal_target` |
| `issue_board_tests.rs` | `issue_evaluation_budget` (`#[ignore]`; 1,000 pods with 50 problems, 2,000 events, 1,000 objects; prints elapsed; run with `--release -- --ignored`) |
| `issue_feeds_tests.rs` | `warning_events_slice_per_object`, `warning_events_of_none_until_ready`, `coverage_note_groups_by_state`, `coverage_full_has_no_note`, `limited_feed_is_not_partial`, `core_feed_states_from_lists_and_metrics` |
| `metrics_history_tests.rs`, `kubelet_history_tests.rs` | `latest_container_pair_needs_both_ticks`, `pvc_usages_yield_newest_per_claim` |
| `cluster_session_tests.rs` | `core_updates_mark_board_dirty`, `scope_change_restarts_warning_events`, `retry_keeps_first_seen`, `time_refresh_notifies_only_when_issues_visible` |
| `navigation.rs` | `issue_count_suffix_uses_worst_severity` |

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
