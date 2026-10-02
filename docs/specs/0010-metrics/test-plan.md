# 0010 · Test plan

[Back to index](README.md). **S** is the step. Tests are deterministic and offline; each checks one behavior. Poll tests use paused tokio time (`test-util`, like the Batcher tests) and a fake `fetch`.

## Unit tests

| S | File | Test | Checks |
|---|---|---|---|
| 1 | `quantity.rs` | `parses_plain_and_fractional_numbers` | `2`, `0.5`, `.5`, `5.` exactly |
| 1 | `quantity.rs` | `parses_decimal_suffixes` | `250m` → 250,000,000 n; `1234567n`; `1k` → 1000 B; `1G` |
| 1 | `quantity.rs` | `parses_binary_suffixes` | `512Mi` → 536,870,912 B; `16384256Ki`; `1.5Gi` |
| 1 | `quantity.rs` | `parses_exponents` | `129e6`; `1E3` = 1000; `1E` = exa; `1e-3` cores = 1,000,000 n |
| 1 | `quantity.rs` | `rounds_sub_unit_values_up` | `1.5n` → 2 n; `1.4n` → 2 n; `0.4` bytes → 1; zero stays 0 |
| 1 | `quantity.rs` | `long_fraction_zeros_do_not_overflow` | `1.` + 45 zeros + `Ki` = 1024 B |
| 1 | `quantity.rs` | `rejects_malformed_text` | ``, `-1`, `1.2.3`, `5x`, `Mi`, `1 Gi` |
| 1 | `quantity.rs` | `rejects_exponent_with_suffix` | `1e3Ki`, `1.5e2m` |
| 1 | `quantity.rs` | `rejects_values_beyond_u64` | `20Ei` bytes; a 39-digit mantissa |
| 1 | `resource_metrics_tests.rs` | `pod_metrics_reads_metrics_server_json` | a real-shaped item (`timestamp`, `window`, labels, `1234567n`, `12345Ki`): namespace, name, `sampled_at`, two containers in order |
| 1 | `resource_metrics_tests.rs` | `pod_metrics_skips_unparsable_containers` | missing `usage.memory` or bad text drops only that container |
| 1 | `resource_metrics_tests.rs` | `pod_metrics_without_containers_is_none` | |
| 1 | `resource_metrics_tests.rs` | `node_metrics_reads_usage_and_timestamp` | bad `timestamp` → `sampled_at` None, usage kept |
| 1 | `resource_metrics_tests.rs` | `concat_namespaces_keeps_namespace_order` | `[a1, a2] + [b1]` |
| 1 | `resource_metrics_tests.rs` | `concat_namespaces_wraps_the_failing_namespace` | several → `ClusterError::Namespace { namespace: "b" }`; a single unnamed api → the error as-is |
| 1 | `resource_metrics_tests.rs` | `next_delay_backs_off_and_caps` | 15, 30, 60, 120, 120 s |
| 1 | `resource_metrics_tests.rs` | `poll_fetches_at_once_then_every_interval` | fetches at 0 s, 15 s, 30 s |
| 1 | `resource_metrics_tests.rs` | `poll_backs_off_then_resets_on_success` | fail, fail, ok → waits 30 s, 60 s, then 15 s |
| 1 | `resource_metrics_tests.rs` | `poll_reports_failures_and_keeps_polling` | `Failed` then `Snapshot` |
| 1 | `connection_tests.rs` | `scoped_dynamic_apis_builds_one_api_per_namespace` | All → one unnamed; Several → one per name, in order |
| 1 | `access_review.rs` | `metrics_checks_target_the_metrics_group` | group `metrics.k8s.io`; pods namespaced, nodes not; `ALL.len() == 21` |
| 1 | `access_review.rs` | `display_names_the_metrics_group_only` | `list pods.metrics.k8s.io`; `list pods` and `list deployments` unchanged |
| 2 | `usage_format.rs` | `cpu_format_uses_millicores_below_one_core` | `0m`, `<1m`, `310m`, `1 core`, `2.5 cores`, `12 cores` |
| 2 | `usage_format.rs` | `bytes_format_uses_binary_units` | `0B`, `512B`, `2Ki`, `498Mi`, `15.6Gi`, `120Gi`, the 1024 roll-up |
| 2 | `usage_format.rs` | `format_pair_shows_a_shared_unit_once` | `9.8 / 15.8 cores`, `498 of 512Mi`, `310m of 1 core` |
| 2 | `usage_format.rs` | `format_percent_rounds` / `usage_tone_thresholds` | `31%`, `104%`; 0.79 None, 0.8 Warn, 0.9 Bad |
| 2 | `metrics_history_tests.rs` | `rings_never_exceed_fine_capacity` | 300 records → every fine ring ≤ 240 |
| 3 | `metrics_history_tests.rs` | `rings_never_exceed_coarse_capacity` | 6,000 records → 288 coarse points, 288 per ring |
| 3 | `metrics_history_tests.rs` | `coarse_ring_averages_twenty_ticks` | mean of the `Some` values of 20 ticks; all `None` → `None` |
| 2 | `metrics_history_tests.rs` | `new_pod_aligns_to_the_newest_tick` | first seen at tick 5: earlier points `None` |
| 2 | `metrics_history_tests.rs` | `absent_pod_reads_none_at_the_newest_tick` | `latest` None, older points kept |
| 2 | `metrics_history_tests.rs` | `fine_ring_is_freed_after_an_hour_and_pod_removed_after_a_day` | |
| 2 | `metrics_history_tests.rs` | `retain_scope_drops_other_namespaces` | All keeps; Named keeps one namespace |
| 2 | `metrics_history_tests.rs` | `usage_point_saturates` / `node_rings_keep_u64_usage` | 5,000 cores in a container; a 6 TiB node exact |
| 2 | `cluster_metrics_tests.rs` | `pods_gate_waits_while_the_review_runs` | |
| 2 | `cluster_metrics_tests.rs` | `gate_polls_allowed_namespaces_only` | `Several([a, b, c])`, `b` denied → `Poll { scope: Several([a, c]), note: "no access in b" }` |
| 2 | `cluster_metrics_tests.rs` | `pods_gate_is_off_when_no_namespace_is_allowed` | reason names every denied namespace and the first review reason |
| 2 | `cluster_metrics_tests.rs` | `pods_gate_polls_after_a_failed_review` | |
| 2 | `cluster_metrics_tests.rs` | `nodes_gate_follows_the_session_review` | Checking → Wait; denied → Off naming the check; allowed/Unknown → Poll |
| 2 | `cluster_metrics_tests.rs` | `poll_error_text_names_missing_or_broken_metrics_server` | 404, 503, other |
| 2 | `cluster_metrics_tests.rs` | `metrics_settle_after_min_ticks_or_unavailable` | |
| 2 | `node_usage.rs` | `node_usage_divides_by_allocatable` / `node_usage_is_none_without_sample_or_allocatable` | zero allocatable too |
| 2 | `node_usage.rs` | `node_requests_sum_main_and_sidecar_on_the_node` | init and Completed pods excluded |
| 2 | `node_usage.rs` | `node_quantity_text_formats_known_resources` | cpu, memory, hugepages; `pods` and junk as written |
| 2 | `pod_table.rs` | `pod_row_usage_values_sort_as_numbers` / `pods_view_hides_cpu_by_default` | `Number`, `Absent` without usage |
| 2 | `node_table.rs` | `node_row_usage_is_per_mille` | 0.314 → 314 |
| 2 | `pod_drawer_tests.rs` | `container_usage_row_shows_usage_of_limit` | text, tone at 97 %, bar fill and marker |
| 2 | `pod_drawer_tests.rs` | `container_usage_row_without_limit_has_no_bar` | and `None` without usage |
| 3 | `usage_format.rs` | `format_offset_reads_hours_minutes_seconds` | `now`, `-45s`, `-3m 15s`, `-5h 40m` |
| 3 | `metrics_history_tests.rs` | `pod_total_sums_containers` | per point; `None` only when all are `None` |
| 3 | `metrics_history_tests.rs` | `owner_series_keeps_pods_replaced_by_a_rollout` | |
| 3 | `metrics_history_tests.rs` | `owner_series_counts_a_pod_whose_snapshot_arrived_late` | metrics tick before the pods snapshot, then pods arrive → counted |
| 3 | `metrics_history_tests.rs` | `coarse_series_appends_the_fine_tail` | coarse points, then fine ticks newer than the last one; `step` 5 min |
| 3 | `metrics_history_tests.rs` | `oom_marks_are_recorded_once` / `oom_marks_age_out_after_a_day_and_cap_at_32` | |
| 3 | `kind_row.rs` | `owns_matches_namespace_and_controller` | Deployment hash rule still applies |
| 3 | `usage_chart_tests.rs` | `nice_max_steps_and_floors` / `y_max_includes_reference_lines` | |
| 3 | `usage_chart_tests.rs` | `x_at_maps_time_linearly` / `nearest_tick_picks_the_closest_point` | |
| 3 | `usage_chart_tests.rs` | `segments_split_at_none_and_gaps` / `segments_drop_points_before_start` | > 2.5 steps apart splits |
| 3 | `monitor_data_tests.rs` | `pod_total_lines_sum_main_and_sidecar` | init excluded |
| 3 | `monitor_data_tests.rs` | `request_line_is_partial_when_a_request_is_missing` / `limit_line_needs_every_limit` | |
| 3 | `monitor_data_tests.rs` | `container_scope_uses_its_own_lines_and_marks` / `workload_total_is_named_by_pod_count` | `3 pods` |
| 3 | `monitor_data_tests.rs` | `node_requested_line_only_for_all_namespaces` | allocatable always |
| 3 | `monitor_data_tests.rs` | `scope_choices_follow_the_subject` / `stale_part_scope_falls_back_to_total` | |
| 3 | `monitor_data_tests.rs` | `stale_when_the_server_timestamp_lags_two_ticks` / `short_history_marks_the_range` | |
| 3 | `monitor_data_tests.rs` | `rows_are_newest_first_with_oom_flags` | |
| 3 | `drawer.rs` | `drawer_tabs_follow_the_wireframe_order` (updated) | Monitor for Pod, Node, Deployments; none for ConfigMaps, Events |
| 3 | `drawer.rs` | `long_ranges_read_coarse_points` | 15m/1h Fine; 6h/24h Coarse |
| 3 | `resource_kind.rs` | `has_monitor_matches_the_wireframe_kinds` | five kinds; CronJobs no |
| 3 | `launch_options_tests.rs` | `monitor_screens_parse` | `pod-monitor`, `node-monitor`, `deployments-monitor`; `configmaps-monitor` rejected |

## Live checks (coder-lite, UAT `readonly@Monitor`)

1. Step 1: `probe --kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --metrics-seconds 40` → ≥ 2 pod and ≥ 2 node lines; the pod-metrics count is within 3 of `listed − not running`; access lines show both metrics checks. Nothing from the kubeconfig in the output.
2. Step 1: the 0001 read-only grep (`create|replace|patch|delete|exec|attach|portforward`) still finds only the SSAR `create`.
3. Step 2: screenshots `pods` (Memory column) and `nodes` (bars); three pods' Memory equals the probe's values after formatting (± one tick).
4. Step 3: screenshots `pod-monitor`, `node-monitor`, `deployments-monitor`, light and dark.

## ui-verifier checklist (step 3)

W4: Memory column, "—" for Completed. W5: CPU/Memory bars + percent, yellow at ≥ 80 %. W4b: container Resources bars; sub-tabs `Info · Env · Mounts · Monitor`. W4c: tab order; toolbar (four range buttons, the short one muted, scope, Table view, status); CPU and Memory cards with `now …`, dashed request (muted) and limit (red) lines with labels, `-15m`/`now`, source note. W7: Monitor on Deployments with `All pods ▾`; node drawer "Allocatable used" with `9.8 / 15.8 cores` style. No hardcoded colors.
