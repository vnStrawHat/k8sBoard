# 0015 · Test plan

[Back to index](README.md). Unit tests are offline and deterministic; fixtures are k8s-openapi structs with `..Default::default()`. Names are binding (AC 2).

## Step 1 — `crates/cluster`

| Module | Tests |
|---|---|
| `service_account.rs` | `service_account_summary_keeps_names_only` (fixture references carry a distinctive `uid` and `resourceVersion`, the object has a non-allowlisted annotation value and an `eks.amazonaws.com/role-arn` ARN; `format!("{summary:?}")` contains the ARN and none of the others), `cloud_identity_reads_only_allowlisted_keys` (all three providers, in order; empty value skipped), `other_annotations_are_ignored` (`kubernetes.io/enforce-mountable-secrets`, a look-alike `eks.amazonaws.com/role-arn-x`), `automount_token_is_optional`, `empty_secret_names_are_dropped` |
| `role.rs` | `rules_keep_groups_resources_names_verbs_urls`, `grants_everything_needs_all_three_wildcards`, `cluster_role_aggregation_selectors`, `built_in_from_bootstrapping_label`, `role_has_no_aggregation` |
| `role_binding_tests.rs` | `binding_reads_role_ref_and_subjects`, `service_account_subject_defaults_to_binding_namespace`, `cluster_binding_service_account_without_namespace_never_matches`, `binds_service_account_direct`, `binds_service_account_through_service_account_groups`, `authenticated_group_does_not_bind`, `unknown_subject_kind_is_dropped`, `role_kind_maps_role_cluster_role_and_other`, `broad_groups_recognized` (the four groups; `system:masters` is not broad) |
| `object_yaml_tests.rs` | `access_kinds_have_names_and_scope` |
| `access_review.rs` | `all_checks_cover_distinct_permissions` (new length), `rbac_checks_use_rbac_group_and_scope`, `check_display_matches_kubectl_wording` |

## Step 2 — Roles, ClusterRoles, bindings

| Module | Tests |
|---|---|
| `access_rows_tests.rs` | `role_row_cells_match_column_count`, `cluster_role_row_cells_match_column_count`, `role_binding_row_cells_match_column_count`, `cluster_role_binding_row_cells_match_column_count`, `rules_cell_marks_all`, `role_status_order`, `binding_role_cell_kubectl_style`, `cluster_admin_to_service_account_is_warn`, `cluster_admin_to_authenticated_is_bad`, `cluster_admin_to_user_is_ok`, `other_role_kind_cell_as_written`, `subjects_text_formats`, `rules_table_aligns_columns`, `rules_table_prints_empty_group_and_resource_names`, `rules_table_lists_non_resource_urls_last`, `rules_table_caps_at_200` |
| `access_bindings.rs` | `role_text_and_binding_text`, `binding_key_per_binding_kind`, `role_is_bound_only_in_its_namespace`, `cluster_role_bound_by_both_kinds`, `role_key_per_role_kind` (`Other` → None), `index_skips_other_role_kinds` |
| `kind_join_tests.rs` | `joined_column_indices_name_their_columns` (adds `ROLE_BINDINGS`, `CLUSTER_ROLE_BINDINGS`), `binding_counts_per_role`, `wildcard_role_bindings_warn`, `bindings_absent_until_companion_ready`, `denied_cluster_role_bindings_keep_cells_absent` |
| `live_sections_tests.rs` | `role_bindings_rows_link_back`, `role_subjects_service_accounts_first_with_review` |
| `kind_diagnosis_tests.rs` | `cluster_role_very_broad_names_first_service_account`, `cluster_role_very_broad_without_bindings`, `role_very_broad_names_namespace`, `cluster_binding_review_single_and_many`, `role_binding_review_names_namespace`, `review_for_broad_groups` (Bad for authenticated and unauthenticated, Warn for service-account groups), `view_binding_has_no_box` |
| `cluster_session_tests.rs` | `companion_plan_per_kind` (adds Roles: role bindings only; ClusterRoles: both; one denied list), `bindings_companion_applies_each_list`, `open_watch_count_counts_several_related_and_companion` (Bindings companion N + 1; ServiceAccounts at N = 5 gives 19) |
| `table_view_tests.rs`, `kind_table.rs` | `hide_system_is_default_for_cluster_roles_and_bindings`, `hide_system_hides_system_prefix_only`, `hide_system_ignores_pods_and_nodes` |
| `resource_actions_tests.rs` | `binding_menu_has_go_to_role` |
| `navigation.rs` | enabled list adds the four kinds |

## Step 3 — ServiceAccounts

| Module | Tests |
|---|---|
| `access_rows_tests.rs` | `service_account_row_cells_match_column_count`, `binding_service_account_subject_is_a_link` (a `Field` in step 2, when ServiceAccounts has no screen yet), `service_account_secrets_section_names_only`, `cloud_identity_section_after_bound_roles`, `no_cloud_identity_section_when_empty`, `automount_default_text` |
| `access_bindings.rs` | `bound_roles_direct_and_group_sorted`, `bound_roles_ignore_other_accounts`, `index_built_once_serves_every_account` (one `build`, lookups for three accounts) |
| `kind_join_tests.rs` | `service_account_bound_roles_cell`, `service_account_cluster_admin_warns`, `service_account_used_by_counts_pods`, `pod_without_service_account_counts_for_default`, `cluster_admin_through_service_accounts_group_warns` |
| `live_sections_tests.rs` | `bound_roles_rows_via_binding_and_group`, `bound_roles_note_names_scope`, `service_account_pods_by_name` |
| `kind_diagnosis_tests.rs` | `service_account_cluster_admin_through_cluster_binding`, `service_account_admin_of_namespace` |
| `navigation.rs` | enabled list adds ServiceAccounts |

## Live checks (coder-lite, UAT `readonly@Monitor`)

1. Step 1: `probe --watch-seconds 5 --counts`; fill [decisions.md](decisions.md) "UAT probe"; AC7 credential script reports 0. Never print kubeconfig data.
2. Step 2: `roles`, `roles-drawer`, `clusterroles` (Hide system on), `clusterroles-drawer --filter cluster-admin`, `rolebindings`, `rolebindings-drawer`, `clusterrolebindings`, `clusterrolebindings-drawer --filter cluster-admin`. AC 6: follow a RoleBinding's role link and back.
3. Step 3: `serviceaccounts`, `serviceaccounts-drawer --filter default`. Confirm by review that no Secret request appears in the debug trace.

## ui-verifier (steps 2–3)

Screens above in light; `clusterroles-drawer` and `serviceaccounts-drawer` in dark. Check against W7: column order; Warn tones on cluster-admin cells and wildcard binding counts; Rules code block aligned and wrapping; Hide system toggle on by default; VERY BROAD / REVIEW / CLUSTER ADMIN boxes above the first section; links styled as links in both directions; Secrets section shows names and the "never read" note; Cloud identity rows when the account has an allowlisted key; Delete last and disabled; no hardcoded colors.
