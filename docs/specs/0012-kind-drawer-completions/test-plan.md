# 0012 · Test plan

[Back to index](README.md). Unit tests are offline and deterministic (fixed timestamps; zones from the bundled tzdb). Names are binding (AC 2).

## Step 1a — cron (`crates/cluster`)

| Module | Tests |
|---|---|
| `cron_schedule_tests.rs` | `parses_every_field_form` (`*`, `N`, `N-M`, `*/15`, `5/20`, lists), `question_mark_is_star` (`0 0 ? * 1` = Mondays only), `names_in_ranges_with_step` (`0 0 * jan-jun/2 mon-fri`), `descriptors_expand`, `rejects_bad_fields` (count, range, step 0, `5-1`, `60`, `7` in dow), `numbers_are_ascii_digits_only` (`+5`, `٣`), `tz_prefix_checked_before_trim` (`TZ=UTC 0 0 * * *` and ` CRON_TZ=…`), `unknown_descriptor_is_unsupported`, `star_flag_ignores_stepped_star`, `day_rule_ors_restricted_fields` (`0 0 1 * 1`), `day_rule_ands_when_one_is_star`, `next_after_is_strictly_later` (also from an exact match and from mid-second), `next_after_rolls_over_month_and_year`, `next_after_finds_leap_day` (`0 0 29 2 *`), `impossible_schedule_has_no_next` (`0 0 30 2 *`), `next_runs_use_the_zone` (`0 9 * * *`, `Europe/Berlin`), `spring_forward_skips_the_missing_hour` (`30 2 * * *`, `America/New_York`, March change day: next run is the following day 02:30), `fall_back_repeats_the_hour` (`30 1 * * *`, November change day: two runs one hour apart), `every_parses_go_durations` (`1h30m`, `90s`, `1.5h`, `250ms`, sign, bare `0`; rejects `1d`, `5`, `1h 30m`), `empty_list_parts_are_skipped` (`0,,5`; `,` alone is an error), `anchor_does_not_change_a_calendar_schedule`, `fold_east_of_utc_takes_the_later_occurrence`, `is_every_tells_anchored_schedules_apart`, `day_step_undoes_a_dst_shift_of_midnight` (`0 0 * * 1`, `America/Santiago`, 2025-09-07), `gap_day_keeps_a_later_hour_in_a_west_zone` (`0 1 * * 0`, Santiago), `descriptor_check_comes_before_any_trimming`, `every_runs_from_the_anchor`, `every_without_anchor_has_no_next`, `unknown_zone_is_an_error`, `unset_zone_has_no_name` |
| `cron_job.rs` | `cron_job_summary_parses_timetable`, `invalid_schedule_keeps_error`, `every_schedule_is_anchored_at_last_schedule` |

## Step 1b — other crate work

| Module | Tests |
|---|---|
| `endpoint_slice.rs` | `endpoint_slice_summary_maps_service_ports_and_endpoints`, `endpoint_nil_conditions_default` (ready true, terminating false), `endpoint_without_address_is_dropped`, `non_pod_target_has_no_pod`, `slice_without_service_label_has_no_service` |
| `object_count.rs` | `count_adds_remaining_items` (also asserts `count_params()` has `limit == Some(1)` and `resource_version == None`), `count_without_continue_is_item_count`, `count_with_continue_but_no_remaining_is_unknown` |
| `workload_tests.rs` | `condition_message_is_cut`, `container_port_keeps_host_port` |
| `deployment.rs`, `job.rs`, `stateful_set.rs`, `ingress.rs` | `progress_deadline_defaults_to_600`, `job_keeps_deadline_and_ttl`, `claim_retention_text`, `ingress_paths_name_their_service` |
| `config_map.rs` | `value_preview_kinds` (line, long line, JSON, multi-line text, binary), `values_sorted_by_key`, `crlf_line_endings_stay_a_single_line` |
| `container_spec_tests.rs` | `projected_volume_names_config_maps` |
| `access_review.rs` | `endpoint_slices_check_targets_discovery_group` |
| `selector.rs` | `match_labels_and_expressions_all_hold`, `not_in_and_does_not_exist_hold_for_missing_key`, `empty_selector_selects_everything`, `unknown_operator_matches_nothing`, `terms_match_kubectl_syntax` (moved from `selector_terms_format_labels_and_expressions`), `of_labels_builds_equalities`, `matches_by_key_binary_search` (keys `a` and `a.b`, value with `=`) |

## Step 2 — row model, drawer subjects, Deployments, CronJobs

| Module | Tests |
|---|---|
| `batch_rows_tests.rs` | `cron_job_row_has_next_run_cell`, `suspended_cron_job_has_no_next_run`, `cron_job_sections_start_with_next_runs`, `job_owner_is_a_link`, `job_status_shows_deadline_and_ttl` |
| `workload_rows_tests.rs` | `deployment_row_has_revisions_section`, `replica_set_owner_is_a_link`, rows keep their `object` |
| `live_sections_tests.rs` | `revisions_newest_first_and_current_marked`, `revisions_keep_only_owned_replica_sets`, `image_tag_of_registry_with_port`, `recent_jobs_newest_first_and_owned_only`, `run_label_today_and_later_day` |
| `related_objects.rs` | `related_subject_per_kind`, `deployment_without_selector_has_no_subject` |
| `object_events_tests.rs` | `subject_change_is_generic` (existing cases on a second type) |
| `cluster_session_tests.rs` | `related_list_ignores_other_variant`, `open_watch_count_counts_several_related_and_companion` (replaces `open_watch_count_includes_object_events`: All with nothing open = 3; N = 3 with explorer, related, object events = 2 + 3 + 3 + 2; Namespaces explorer counts 1; 4a adds the slices term) |
| `kind_table.rs` | `next_run_sorts_soonest_first` |
| `table_selection_tests.rs` | `of_owner_maps_controller_kinds` |
| `table_view_tests.rs` | `reveal_clears_a_filter_that_hides_the_target` (clears text, chips, preset only when `row_of(item)` is `None`) |

## Step 3 — diagnosis and pod sections

| Module | Tests |
|---|---|
| `kind_diagnosis_tests.rs` | `deployment_stalled`, `deployment_replica_failure`, `deployment_not_ready_names_pod`, `deployment_rollout_without_unhealthy_pod_has_no_box`, `paused_deployment_has_no_box`, `daemon_set_node_missing`, `daemon_set_nodes_missing_plural`, `daemon_set_pod_not_ready`, `daemon_set_without_pod_on_nodes`, `daemon_set_misscheduled`, `job_backoff_limit_with_exit_code`, `job_deadline_exceeded`, `job_failed_other_reason`, `job_retrying`, `no_box_while_pods_load`, `warming_up_pod_is_a_normal_rollout`, `container_cause_names_the_pod_status` |
| `workload_rows_tests.rs` | `daemon_set_rollout_by_node_starts_with_bars`, `daemon_set_port_shows_host_port`, `stateful_set_claims_show_retention`, `retention_short_form_drops_the_field_names`, `replica_set_template_shows_hash_and_image` |
| `kind_row.rs` | `percent_rounds_and_clamps` |
| `related_pods.rs` | `pod_row_detail_per_owner`, `ordinal_detail_lists_claims_of_the_pod`, `attempts_newest_first_with_exit_code` |
| `resource_actions_tests.rs` | `replica_set_menu_has_go_to_owner`, `go_to_owner_disabled_without_owner` |

## Step 4a — joins and Services

| Module | Tests |
|---|---|
| `kind_join_tests.rs` | `joined_column_indices_name_their_columns`, `service_endpoints_all_ready`, `service_endpoints_partial`, `service_matches_no_pods`, `external_name_has_no_endpoints`, `selector_less_service_without_slices_warns`, `slices_without_service_label_are_ignored`, `dual_stack_counts_once`, `terminating_excluded_from_total`, `denied_slices_keep_builder_status` |
| `network_rows_tests.rs`, `kind_diagnosis_tests.rs` | `service_row_has_endpoints_placeholder`; `service_no_matching_pods`, `service_no_ready_endpoints` |
| `live_sections_tests.rs`, `cluster_session_tests.rs` | `endpoint_rows_single_and_multi_port`; `services_start_endpoint_companion_unless_denied`, `companion_plan_per_kind` |

## Step 4b — ConfigMaps, Namespaces, Ingresses

| Module | Tests |
|---|---|
| `kind_join_tests.rs` | `config_map_users_from_env_env_from_volume_projected`, `owner_mapping_deployment_cronjob_bare_pod`, `used_by_cell_shows_first_and_more`, `namespace_load_sums_requests_of_active_pods`, `namespace_outside_scope_is_absent` |
| `network_rows_tests.rs` | `ingress_urls_https_for_tls_and_wildcard`, `ingress_urls_skip_wildcard_hosts_and_regex_paths`, `ingress_backend_is_a_link` |
| `config_map_rows_tests.rs`, `live_sections_tests.rs` | `config_map_sections_are_live_data_and_used_by`; `config_map_data_prefers_previews` |
| `resource_actions_tests.rs` | `open_url_disabled_without_host`, `open_url_submenu_for_several` |

## Step 5 — counts

`cluster_session_tests.rs`: `counts_start_once_per_scope_after_review`, `navigation_refresh_waits_30_seconds`, `scope_change_clears_counts`, `denied_kinds_are_not_counted`; `navigation.rs`: `live_count_wins_over_counted`.

## Live checks (coder-lite, UAT `readonly@Monitor`)

1. Steps 1a/1b: probe `--watch-seconds 5 --counts`; fill [decisions.md](decisions.md) "UAT probe"; AC7 credential script reports 0.
2. Steps 2–4b: the AC 7 checks; record which kinds had data (UAT may lack CronJobs or LoadBalancers; then rely on unit tests).
3. Step 4b: a namespace with known pods: Pods column equals the Pods screen count for that namespace.

## ui-verifier (steps 2–5)

Screens: `deployments-drawer`, `cronjobs`, `cronjobs-drawer`, `daemonsets-drawer`, `statefulsets-drawer`, `jobs-drawer`, `services`, `services-drawer`, `ingresses-drawer`, `configmaps`, `configmaps-drawer`, `namespaces`, plus the sidebar on any screen (step 5). Use `--filter` to pick a row with data. **ConfigMaps on UAT: run `configmaps` and `configmaps-drawer` only with `--filter kube-root-ca.crt`** (public CA data; other ConfigMaps may hold values that must not land in screenshots). Check against W7: section names and order, bars, toned endpoint states, WHY box placement, disabled Roll back, links styled as links, no hardcoded colors.
