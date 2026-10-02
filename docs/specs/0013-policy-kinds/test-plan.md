# 0013 · Test plan

[Back to index](README.md). Unit tests are offline and deterministic; fixtures are k8s-openapi structs with `..Default::default()`. Names are binding (AC 2). Each test checks one behavior.

## Step 1 — `crates/cluster`

| Module | Tests |
|---|---|
| `network_policy.rs` | `absent_pod_selector_selects_everything`, `default_types_isolate_ingress_and_egress_with_rules`, `listed_type_without_rules_denies_all`, `peers_map_pods_namespaces_and_ip_blocks`, `port_defaults_to_tcp_and_keeps_range` |
| `autoscaler_tests.rs` | `min_replicas_defaults_to_one`, `resource_utilization_metric_reads_current`, `current_matched_by_type_and_name_not_order`, `unmatched_metric_has_no_current`, `metric_without_target_value_is_dropped`, `target_is_kind_and_name`, `current_value_uses_target_variant`, `conditions_keep_scaling_limited` |
| `resource_quota.rs` | `items_from_status_hard_with_used`, `items_fall_back_to_spec_hard`, `scopes_include_scope_selector` |
| `disruption_budget.rs` | `null_selector_is_none_and_empty_selects_everything`, `disruption_state_no_pods`, `disruption_state_allowed`, `disruption_state_blocked_by_unhealthy_pods`, `disruption_state_blocked_without_room`, `sync_failed_wins_over_no_pods`, `stale_status_from_observed_generation` (missing generation → not stale) |
| `quantity.rs` | `quantity_ratio_mixes_units` (`180Gi`/`192Gi`, `9k`/`1k`, `500m`/`1`), `quantity_ratio_rejects_zero_and_junk` |
| `event_tests.rs` | `failed_create_selector_filters_warning_failed_create` |
| `access_review.rs` | `all_checks_cover_distinct_permissions` (new length), `policy_checks_use_their_api_groups`, `check_display_matches_kubectl_wording` (4 new texts) |
| `object_yaml_tests.rs` | `policy_kinds_have_names_and_scope` |

## Step 2 — NetworkPolicies, PDBs, shared model

| Module | Tests |
|---|---|
| `network_policy_rows.rs` | `network_policy_row_cells_match_column_count`, `all_pods_selector_reads_all_pods`, `peer_text_namespace_by_name_label`, `peer_text_pods_in_all_namespaces`, `peer_text_ip_block_with_except`, `ports_text_single_range_named_and_all`, `deny_all_ingress_note`, `not_isolated_egress_has_no_section`, `rule_without_peers_reads_any_source` |
| `policy_rows_tests.rs` | `pdb_row_cells_match_column_count`, `pdb_allowed_cell_tone_by_state`, `pdb_status_singular_and_plural`, `pdb_sync_failed_status`, `pdb_stale_status_note`, `pdb_sections_in_order` |
| `kind_diagnosis_tests.rs` | `pdb_blocks_drain_with_unhealthy_pods` (W7 text: 2 of 3, minAvailable 2), `pdb_blocks_drain_without_room` (maxUnavailable 0), `pdb_allowed_has_no_box`, `pdb_no_pods_has_no_box`, `pdb_sync_failed_blocks_drain` |
| `kind_join_tests.rs` | `joined_column_indices_name_their_columns` (adds `NETWORK_POLICY_AFFECTS`), `network_policy_affects_matching_pods_of_its_namespace`, `network_policy_selecting_no_pods_warns`, `network_policy_without_pods_keeps_builder_status` |
| `live_sections_tests.rs` | `selected_pods_unhealthy_first`, `null_selector_selects_no_pods` |
| `resource_kind.rs` | existing invariants pass for the new kinds (`plural_slugs_round_trip`, `labels_round_trip`, `object_kinds_round_trip`, `every_kind_ends_with_a_right_aligned_age_column`) |
| `navigation.rs` | `enabled_items_are_pods_nodes_and_explorer_kinds` (adds NetworkPolicies, PDBs) |

## Step 3 — HPAs, ResourceQuotas, Namespace Quota

| Module | Tests |
|---|---|
| `policy_rows_tests.rs` | `hpa_row_cells_match_column_count`, `metric_text_utilization_value_and_unknown`, `above_target_by_utilization_and_ratio`, `above_target_unknown_for_other_variant`, `at_max_from_scaling_limited_too_many_replicas`, `scaling_disabled_is_scaled_to_zero`, `hpa_status_order`, `hpa_metrics_section_bars`, `quota_row_cells_match_column_count`, `quota_columns_prefer_requests_items`, `quota_measure_by_resource_suffix` (`hugepages-2Mi` and `requests.hugepages-1Gi` are bytes), `quota_tone_thresholds`, `quota_status_names_highest_item` |
| `kind_diagnosis_tests.rs` | `hpa_metrics_unavailable_by_reason` (`FailedGetResourceMetric`, `FailedGetExternalMetric`, `InvalidMetricSourceType`), `hpa_cannot_scale_by_reason` (`FailedGetScale`, `FailedUpdateScale`), `hpa_scaling_inactive_other_reason`, `hpa_scaling_disabled_has_no_box`, `hpa_at_max_replicas`, `hpa_scaling_normally_has_no_box`, `quota_at_limit` |
| `live_sections_tests.rs` | `scaling_events_newest_first_rescale_only`, `blocked_creations_match_quota_name_exactly` (`compute-quota` does not match `compute-quota-2`), `blocked_creations_include_failed_quota` |
| `related_objects.rs` | `related_subject_per_kind` (adds ResourceQuotas, Namespaces) |
| `cluster_session_tests.rs` | `related_list_ignores_other_variant` (new variants), `denied_quota_subject_does_not_start` |
| `namespace_rows.rs` | `namespace_row_has_quota_section` |
| `resource_actions_tests.rs` | `hpa_menu_has_go_to_target`, `go_to_target_disabled_without_screen` (target kind `Rollout`) |
| `navigation.rs` | enabled list adds HPAs, ResourceQuotas |

## Live checks (coder-lite, UAT `readonly@Monitor`)

1. Step 1: `probe --watch-seconds 5 --counts`; fill [decisions.md](decisions.md) "UAT probe"; AC7 credential script reports 0.
2. Steps 2–3: run `<plural>` and `<plural>-drawer` for each allowed kind into `.tmp/ui-shots/`; record which kinds had objects. A kind with no objects gets its empty screen only; its logic relies on unit tests.
3. Step 3: `namespaces-drawer` shows the Quota section (a quota or "No ResourceQuota").

## ui-verifier (steps 2–3)

Screens: `networkpolicies`, `networkpolicies-drawer`, `poddisruptionbudgets`, `poddisruptionbudgets-drawer`, `horizontalpodautoscalers`, `horizontalpodautoscalers-drawer`, `resourcequotas`, `resourcequotas-drawer`, `namespaces-drawer`, light; one drawer in dark. Check against W7: column order, Affects and Allowed tones, sentence rows, bars colored by tone, WHY boxes above the first section, Go to target link styling, disabled mutating items with "Read-only mode", Delete last, sidebar items enabled or locked with reason, no hardcoded colors.
