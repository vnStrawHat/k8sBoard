# 0023 · Test plan

[Back to index](README.md). Offline and deterministic; fixtures are summaries built in the test (no network). Names are binding (AC 2). Each test checks one behavior; evaluator tests are table-driven where several inputs share one behavior (a `cases` array of `(input, expected)` with the case printed on failure).

## Steps 1a (RBAC) and 1b (traffic) — `crates/cluster`

| Module | Tests |
|---|---|
| `rbac_evaluation_tests.rs` (rule matching) | `verb_wildcard_matches_any_verb`, `verb_must_match_exactly`, `group_wildcard_and_exact`, `core_group_is_empty_string`, `resource_wildcard_matches_subresources_too`, `plain_resource_does_not_match_subresource`, `subresource_matches_exactly`, `star_slash_subresource_matches_any_resource`, `resource_slash_star_is_not_a_wildcard`, `resource_names_any_when_empty`, `resource_names_match_named_request`, `resource_names_other_name_no_match`, `resource_names_without_request_name_is_only`, `non_resource_exact_star_and_prefix` (`/healthz`, `*`, `/apis/*` vs `/apis/apps` yes, `/apis` no; `/apis/**` vs `/apis/apps` yes), `non_resource_rule_never_matches_resource_request`, `resource_rule_never_matches_non_resource_request` |
| `rbac_evaluation_tests.rs` (who can) | `cluster_role_binding_grants_in_every_namespace`, `role_binding_grants_only_in_its_namespace`, `role_binding_never_grants_cluster_wide_request`, `role_binding_to_cluster_role_scopes_rules_to_namespace`, `role_binding_ignores_non_resource_rules`, `role_ref_resolves_role_in_binding_namespace`, `missing_role_grants_nothing`, `other_role_kind_grants_nothing`, `aggregated_cluster_role_uses_listed_rules`, `one_grant_per_subject_and_binding`, `several_rules_merge_names_any_wins`, `only_names_union_sorted`, `grant_order_cluster_bindings_first` |
| `rbac_evaluation_tests.rs` (identity) | `service_account_identity_user_and_groups`, `service_account_matches_direct_subject`, `service_account_matches_service_account_groups`, `service_account_matches_authenticated_group`, `service_account_ignores_other_namespace_group`, `unauthenticated_group_never_applies_to_service_account`, `user_identity_matches_name_and_authenticated`, `anonymous_user_gets_unauthenticated_group`, `group_identity_matches_only_group` |
| `rbac_evaluation_tests.rs` (rules of, decide) | `rules_of_namespace_includes_cluster_and_namespace_bindings`, `rules_of_cluster_wide_uses_cluster_bindings_only`, `rules_of_subject_follows_binding_order`, `rules_of_drops_non_resource_rules_from_role_bindings`, `decide_allows_through_group`, `decide_ignores_only_names_grants`, `decide_empty_when_nothing_grants` |
| `rbac_snapshot.rs` | `coverage_covers_all_or_listed_namespaces`, `covers_uses_narrower_of_roles_and_bindings`, `forbidden_cluster_list_gives_empty_and_flag`, `forbidden_namespace_is_skipped_in_fallback`, `empty_fallback_gives_no_namespaces` (pure helpers that fold list outcomes into lists and coverage) |
| `access_review.rs` | `rules_review_maps_resource_and_url_rules`, `rules_review_incomplete_and_error`, `empty_evaluation_error_is_none`, `request_attributes_for_resource`, `request_attributes_for_non_resource`, `check_attributes_unchanged` (existing `AccessCheck`s through the shared builder) |
| `network_policy_traffic_tests.rs` (isolation) | `no_policies_allow_both_directions`, `policy_not_selecting_pod_does_not_isolate`, `empty_ingress_rules_deny_all`, `ingress_only_policy_does_not_isolate_egress`, `egress_isolation_denies_without_rule`, `union_of_policies_any_rule_allows`, `allowing_lists_every_matching_rule`, `denied_lists_isolating_policies`, `allowed_needs_both_directions` |
| `network_policy_traffic_tests.rs` (peers) | `rule_without_peers_allows_any_source`, `pod_selector_peer_same_namespace_only`, `namespace_selector_peer_uses_namespace_labels`, `namespace_and_pod_selector_both_required`, `empty_namespace_selector_needs_no_lookup`, `unknown_namespace_labels_reported_once`, `ip_block_matches_cidr_and_except`, `ip_block_without_ip_never_matches`, `ipv6_cidr_matches`, `mixed_family_or_invalid_cidr_no_match`, `external_source_skips_egress`, `external_source_matches_only_ip_blocks` |
| `network_policy_traffic_tests.rs` (ports) | `rule_without_ports_allows_any_port`, `port_number_and_protocol_must_match`, `port_none_matches_every_port_of_protocol`, `port_range_with_end_port`, `named_ingress_port_resolves_on_destination`, `named_egress_port_resolves_on_destination`, `named_port_recorded_on_rule_ref`, `request_port_name_resolves`, `unknown_request_port_name_errors` |

Fixtures: a `snapshot()` builder with roles `view`-like and `cluster-admin`-like ClusterRoles, a namespaced `Role`, bindings to an SA, a user, `system:serviceaccounts:{ns}`, and `system:authenticated`; a `policy(name, selector, ingress, egress)` builder and `pod(ns, labels, ip, ports)` endpoints. Extract only once repeated (testing rules).

## Step 2 — Who can

| Module | Tests |
|---|---|
| `access_query.rs` | `trims_and_lowercases_verb`, `empty_parts_are_errors` (`.apps`, `pods/`, `pods.`, `/log`), `star_resource_means_star_group`, `parses_verb_and_resource`, `parses_group_suffix`, `parses_subresource`, `parses_object_name`, `infers_group_from_built_in_table`, `unknown_resource_assumes_core_with_hint`, `cluster_scoped_drops_namespace_with_hint`, `parses_non_resource_url`, `url_with_name_is_error`, `empty_and_missing_target_errors`, `too_many_words_is_error`, `wildcards_pass_through`, `built_in_table_has_unique_plurals`, `who_can_prefill_uses_first_resource_rule`, `who_can_prefill_skips_url_rules` |
| `cluster_session_tests.rs` | `rbac_trigger_table` (pure `starts_fetch(&RbacState, RbacTrigger)`: Request starts from Idle/Failed only; Refresh from Ready/Failed; never while Loading), `rbac_resets_on_scope_change` |
| `who_can_view.rs` | `groups_grants_by_subject`, `orders_broad_groups_users_groups_accounts`, `only_named_grants_go_to_second_group`, `headline_counts_full_subjects`, `headline_warns_on_broad_group`, `coverage_notes_per_gap` (each of the four rows, none when fully covered) |
| `resource_actions_tests.rs` | `role_menus_start_with_who_can` |
| `launch_options_tests.rs` | `parses_analysis_screens` (adds steps 3–4 values as they land) |

## Step 3 — Check permissions, Can do

| Module | Tests |
|---|---|
| `access_query.rs` | `parses_subject_forms` (you, empty, sa, serviceaccount, system:serviceaccount, user, group), `invalid_subject_is_error` |
| `permission_table.rs` | `expands_groups_times_resources`, `wildcard_verb_fills_all_cells_and_other`, `unknown_verbs_go_to_other_sorted`, `resource_names_make_names_cell`, `all_wins_over_names`, `everything_row_first`, `core_group_sorts_first`, `subresource_rows_separate`, `url_rules_merge_by_url`, `caps_at_300_rows`, `chip_text_lists_verbs_in_column_order`, `chip_all_verbs_and_all_resources`, `chip_named_suffix`, `chip_everything_is_warn`, `chips_cap_at_12_with_more` |
| `permissions_view.rs` | `answer_you_allowed`, `answer_you_denied_with_reason`, `answer_you_denied_without_reason`, `answer_other_via_first_grant`, `answer_other_denied`, `incomplete_note_text` (with and without evaluation error), `you_namespaces_fall_back_to_scope` |
| `live_sections_tests.rs` | `can_do_excludes_authenticated_only_grants`, `can_do_loading_and_failed_lines`, `can_do_coverage_warning` |
| `access_rows_tests.rs` | `service_account_sections_put_can_do_after_cloud_identity` |
| `resource_actions_tests.rs` | `service_account_menu_starts_with_check_permissions` |

## Step 4 — Test traffic

| Module | Tests |
|---|---|
| `traffic_test_view.rs` | `defaults_destination_selected_by_policy`, `defaults_without_policy_use_first_two_pods`, `defaults_port_first_main_container_port_or_80`, `labels_input_sorted_into_terms`, `invalid_ip_is_rejected`, `verdict_rows_per_direction_state`, `unknown_port_name_text`, `host_network_warning`, `missing_ip_warning`, `named_egress_port_warning`; extras: `defaults_source_falls_back_to_another_namespace`, `defaults_with_one_pod_have_no_source`, `defaults_without_pods_have_no_destination`, `empty_labels_input_is_no_labels`, `label_keys_and_values_are_trimmed`, `label_without_equals_is_rejected`, `label_with_empty_key_is_rejected`, `duplicate_label_keys_are_rejected`, `destination_ports_include_sidecar_ports`, `ip_is_read_in_canonical_form`, `port_input_reads_numbers`, `port_input_reads_names`, `port_input_rejects_empty_and_out_of_range`, `verdict_rows_for_a_direction_no_policy_limits`, `verdict_rows_for_an_external_source`, `unknown_namespace_warning`, `outcome_denies_without_an_ingress_rule_and_names_the_namespace`, `outcome_with_two_namespaces_and_a_port_name` |
| `resource_actions_tests.rs` | `network_policy_menu_starts_with_test_traffic` |

## Live checks (coder-lite, UAT `readonly@Monitor`)

1. Steps 1a, 1b: `probe --context readonly@Monitor --analysis` (plus `--namespace` of a namespace with pods); fill [decisions.md](decisions.md) "UAT probe"; AC7 credential script reports 0. Never print kubeconfig data.
2. Step 2: `--screen who-can`; Check `get secrets` and `list nodes` (cluster-scoped hint); follow one binding link.
3. Step 3: `--screen check-permissions`, `account-permissions --filter default`, `serviceaccounts-drawer --filter default` (Can do).
4. Step 4: `--screen test-traffic` and `networkpolicies-drawer` → menu Test traffic… (empty-state wording when UAT has no policies).

## ui-verifier (steps 2–4)

Screens above in light; `who-can` and `test-traffic` in dark. Check: dialog centered over the shell, 760 px, scrolls; inputs on one row; headline, system:masters row, subject groups and grant lines aligned; links styled; Warn/Bad tones only on broad groups, everything rows, Denied; permission table columns aligned with `✓`/`names`; Can do chips under Cloud identity with the note; source line and caveats present and muted in every result; no hardcoded colors.
