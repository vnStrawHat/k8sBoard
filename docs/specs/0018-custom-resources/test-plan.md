# 0018 · Test plan

[Back to index](README.md). Unit tests are offline and deterministic; fixtures are `serde_json::json!` values (cert-manager Certificate, Argo CD Application, Strimzi KafkaTopic, a `ClusterSecret`). One behavior per test.

## Step 1a — CRDs, column paths, matcher

| File | Tests |
|---|---|
| `column_path_tests.rs` | `parses_dotted_fields`, `parses_quoted_keys_with_dots`, `parses_index_and_wildcard`, `parses_filter_with_spaces_and_both_quotes` (`[?(@.type == "Ready")]`, `[?(@.type=='Ready')]`), `rejects_unsupported_syntax` (`{.spec.x}`, `$.spec.x`, `spec.x`, `.*`, `..`, `[0:2]`, `[0,1]`, `[-1]`, `!=`, `<`, `&&`, `=~`, integer and boolean literals, `range`, empty), `filter_selects_first_matching_item`, `missing_filter_field_never_matches`, `wildcard_returns_first_item`, `metadata_paths_read_the_metadata_root`, `condition_status_path_is_detected` (Ready status yes, Ready message no), `string_column_stringifies_scalars`, `object_values_are_absent_in_string_columns`, `integer_column_rejects_strings_and_floats`, `number_column_keeps_json_text`, `date_column_parses_rfc3339_only`, `secret_like_last_field_is_hidden`, `password_format_columns_are_hidden`, `long_text_is_cut_at_200_chars` |
| `custom_resource_definition_tests.rs` | `crd_summary_reads_names_scope_and_versions`, `singular_defaults_to_lowercase_kind`, `printer_columns_keep_order_type_and_support` (unknown type → String; unsupported path flagged), `password_format_is_read`, `condition_status_columns_are_flagged`, `printer_column_new_parses_its_path`, `schema_outline_lists_spec_and_status_fields`, `schema_outline_caps_at_40_and_counts_omitted`, `missing_schema_gives_empty_outline`, `crd_state_follows_conditions`, `preferred_version_is_highest_priority_served` (`v1` over `v2beta1` over `v1alpha3`; not-served `v2` skipped), `no_served_version_has_no_preference`, `resource_names_group_version_kind_plural_scope`, `crd_watch_config_pages_most_recent` (page size 20), `lean_crd_decoding_ignores_deep_schema` |
| `object_yaml_tests.rs` | `crd_kind_has_name_and_cluster_scope`, `client_secret_and_api_key_are_hidden`, `secret_refs_stay_readable` |
| `storage_class.rs` tests | `secret_like_parameters_are_hidden`, `harmless_parameters_are_shown` (0014, now through `is_secret_key`, results unchanged) |
| `access_review` tests | `crd_check_targets_apiextensions_cluster_scope` |

## Step 1b — custom objects, masking, access, counts

| File | Tests |
|---|---|
| `custom_object_tests.rs` | `summary_keeps_metadata_labels_and_phase`, `summary_has_one_value_per_column`, `unsupported_columns_are_absent`, `conditions_capped_at_10_with_messages_cut_at_300`, `fields_flatten_spec_and_status_to_depth_3`, `fields_skip_status_conditions`, `scalar_arrays_join_five_then_count`, `object_arrays_report_item_count`, `deep_objects_report_field_count`, `fields_cap_at_60_and_count_omitted`, `null_and_empty_values_are_skipped`, `objects_watch_config_pages_most_recent` (page size 50), `fields_watch_selects_by_name`, `cluster_scoped_resources_use_one_api`, `counts_use_one_cluster_wide_list`, `certificate_fixture_matches_worked_example` |
| `object_yaml_tests.rs` | `custom_ref_requires_matching_scope`, `custom_yaml_hides_secret_like_keys` (S3), `secret_like_kinds_hide_all_scalars_but_status` (S2: `ClusterSecret` `data` hidden, `status` readable), `url_userinfo_is_hidden` (S4), `urls_without_userinfo_are_unchanged`, `custom_masking_counts_toward_header`, `builtin_yaml_is_unchanged_by_custom_rules`, `custom_summary_debug_lacks_secret_values` |
| `access_review` tests | `custom_review_asks_list_only`, `custom_review_runs_per_namespace_for_several`, `custom_review_ignores_scope_for_cluster_resources`, `first_custom_denial_wins` (pure request building and folding) |
| `resource_watch_tests.rs` | `closure_summarizer_captures_runtime_data` |

## Step 2a — `KindApi` and the Crds kind

| File | Tests |
|---|---|
| `resource_kind.rs` tests | existing round-trip tests include `Crds`; `builtin_kinds_have_an_access_check`, `object_ref_builds_builtin_refs`, `crds_have_no_own_watch` |
| `crd_rows_tests.rs` | `crd_row_cells_match_column_count`, `crd_status_follows_state`, `versions_section_marks_preferred_storage_and_deprecated`, `printer_columns_section_marks_unsupported`, `schema_section_renders_outline_and_omitted` |
| `cluster_session_tests.rs` | `crd_watch_waits_for_a_non_denying_review`, `crd_snapshot_feeds_the_crds_explorer`, `open_watch_count_stays_within_3n_plus_5` |

## Step 2b — custom kinds, tables, sidebar

| File | Tests |
|---|---|
| `custom_kind_tests.rs` | `labels_follow_kind_and_plural`, `badges_use_the_next_capital`, `only_established_served_crds_become_kinds`, `kinds_sort_by_group_then_label`, `cached_definitions_are_reused` (`std::ptr::eq`, also across a cache hand-over), `changed_columns_make_a_new_kind`, `equality_short_circuits_on_the_same_pointer`, `columns_follow_printer_column_types`, `no_printer_columns_give_age_only`, `built_in_columns_extend_cert_manager_certificates` (Expires before Age, rule `Expiry`), `built_in_columns_do_not_apply_to_other_crds`, `only_custom_kinds_lack_an_access_check` |
| `custom_rows_tests.rs` | `custom_row_cells_match_kind_columns`, `condition_status_cells_are_toned`, `hidden_values_read_hidden`, `negative_integers_are_text`, `expiry_dates_get_the_expiry_rule`, `ready_condition_sets_status`, `available_is_used_without_ready`, `failing_condition_warns_even_when_ready`, `phase_is_info_without_conditions`, `no_status_without_conditions_or_phase` |
| `kind_table` / `kind_drawer` tests | `expiry_date_tone_follows_tls_thresholds` (64 days none, 6 days Warn, past Bad), `future_dates_read_in` |
| `cluster_session_tests.rs` | `custom_gate_denied_fails_the_list_with_reason`, `custom_gate_allowed_starts_the_watch`, `custom_gate_error_starts_the_watch_uncached`, `scope_change_clears_custom_gates`, `custom_denied_reason_names_scope`, `custom_kind_cache_moves_to_the_next_session` |
| `navigation.rs` tests | `custom_kinds_group_by_api_group`, `crds_item_resolves_through_label`, `denied_custom_gate_locks_the_item`, `no_groups_without_a_ready_crd_list` |
| `launch_options_tests.rs` | `parses_custom_screen_with_suffixes`, `custom_names_with_dashes_keep_their_dashes`, `crds_slugs_parse_through_plural` |
| `app_shell` / `screenshot` tests | `custom_screen_remaps_to_the_changed_kind`, `removed_custom_kind_returns_to_crds` (`remapped_screen`), `custom_launch_waits_for_the_crd_list` |

## Steps 3–5

| File | Tests |
|---|---|
| `live_sections.rs` tests | `conditions_rows_tone_by_type_and_reason`, `status_fields_render_entries_and_omitted_note`, `secret_name_fields_link_to_secrets`, `field_list_states_render_notes` |
| `kind_diagnosis.rs` tests | `custom_box_reports_ready_false`, `custom_box_reports_failing_condition`, `custom_box_absent_when_healthy` |
| `related_objects.rs` tests | `custom_rows_have_a_fields_subject`, `fields_subject_carries_namespace_for_namespaced_kinds` |
| `resource_actions_tests.rs` | `browse_instances_opens_the_custom_kind`, `browse_instances_disabled_without_a_kind`, `custom_menus_have_no_read_only_actions` |
| `kind_join.rs`, `cluster_session_tests.rs` (4) | `crd_instances_index_names_the_column`, `crd_instances_fill_from_counts`, `missing_counts_stay_absent`, `custom_counts_refresh_after_30s_and_survive_scope_change`, `denied_kinds_are_not_counted` |
| `namespace.rs`, `namespace_rows.rs` (5) | `deletion_conditions_keep_true_known_types`, `deleting_since_reads_deletion_timestamp`, `deletion_messages_cut_at_500`, `remaining_resources_parse_content_and_finalizers`, `unparsed_messages_become_notes`, `stuck_box_after_five_minutes`, `stuck_box_is_bad_with_failure_conditions`, `no_box_while_recently_terminating` |

## Live checks (coder-lite, UAT `readonly@Monitor`)

1. Probe `--crds` (1a), then `--crds --watch-seconds 10 --yaml` (1b); copy results into [decisions.md](decisions.md).
2. If CRDs exist: open the first Established one; check rows, columns, a drawer, the YAML header, the Events tab. If none: the CRDs empty state and an empty Custom Resources group.
3. `open_watch_count` from the status bar while a custom kind and its drawer are shown.

## ui-verifier checklist (W7 CRDs, Certificates, Namespaces)

- Sidebar: Custom Resources › CRDs plus collapsed API groups; the shown kind's group open; counts muted.
- CRDs table columns and drawer sections; custom table printer columns, toned Ready cell, Expires tone (Certificates), Name qualified by namespace.
- Drawer: status box title and tone, Conditions, Status, Spec, links; YAML tab with `<hidden>` where rules apply.
- **Accepted deviations, not defects**: decision 20 (API-group submenus instead of a flat list), 27 (values, not schema descriptions), 30 (full CRD name plus Group column), the extra Certificates Status column, no meta lines, no Show remaining resources item (Renew now is step 6), Remaining resources without object names.

## Step 6 — Certificate Renew now

The tests are listed in [renew-now.md](renew-now.md) (cluster: `certificate_renewal_tests.rs`, `object_write_certificate_tests.rs`; app: `resource_actions_tests.rs`, `app_shell_write_tests.rs`). Added when built: `app_shell_certificate_tests.rs` (key flow over a fake server, held Enter, session change at commit, lock, other CRD version, header state), `launch_options_tests.rs` (`renew-confirm`), `custom_kind_tests.rs` (predicates).

Live (UAT, no cert-manager): `RUST_LOG=cluster=debug` on the screenshot build shows 57 `reviewing access` lines on `--screen overview` (56 of `ALL`, one cluster-wide each, plus the pod-metrics review), no `write finished` line, and no Renew row. Screenshots: `--screen renew-confirm --theme light|dark` (`v101-*`).
