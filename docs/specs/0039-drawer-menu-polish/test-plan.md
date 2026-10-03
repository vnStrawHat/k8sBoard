# 0039 · Test plan

[Back to index](README.md). All tests are offline; no test talks to a cluster. Headless tests run one session through the existing seams `ClusterSession::go_live_for_test` and `set_pods_for_test` (`cluster_session_tests.rs`), in `app_shell_tests.rs`. Names are binding (AC 2).

## Step 1

| Module | Tests |
|---|---|
| `resource_kind` / `resource_actions_tests.rs` | `quota_menu_has_edit_yaml_and_no_stale_edit` |
| `live_sections_tests.rs` | `restart_hint_names_env_readers`, `restart_hint_is_none_for_volume_only`, `restart_hint_skips_cronjob_and_job_owners`, `restart_hint_skips_bare_pod_owners`, `restart_hint_caps_at_three_owners` |
| `resource_actions_tests.rs` | `logs_menu_one_container_is_direct`, `logs_menu_lists_every_container_with_tags`, `logs_menu_disabled_when_logs_not_permitted`, `logs_choice_opens_that_container` (`LogTarget::of_container`, explicit choice), `container_menu_items_in_order`, `container_shell_disabled_when_not_running`, `container_shell_follows_the_gate`, `copy_image_copies_the_spec_image` |
| `app_shell_tests.rs` (headless) | `container_menu_opens_logs_of_that_container`, `logs_submenu_opens_tab_on_the_picked_container` |

## Step 2

| Module | Tests |
|---|---|
| `kind_join_tests.rs` | `last_job_name_uses_unix_minutes`, `last_job_needs_a_schedule`, `last_job_needs_pods_left`, `last_job_ignores_other_namespaces` |
| `resource_actions_tests.rs` | `view_logs_is_offered_on_workload_kinds`, `view_logs_not_offered_on_services`, `cron_job_view_logs_item_has_l_hint`, `cron_job_key_disabled_without_pods_says_why`, `workload_logs_item_has_l_hint` |
| `app_shell_tests.rs` (headless) | `l_on_cron_job_opens_last_job_logs`, `l_on_deployment_opens_workload_logs` |

## Step 3

| Module | Tests |
|---|---|
| `object_yaml_tests.rs` (cluster) | `pod_template_text_drops_the_hash_label`, `pod_template_text_masks_env_by_default`, `pod_template_text_shows_env_when_asked`, `pod_template_text_masks_secret_annotations`, `pod_template_text_masks_secret_annotations_under_template_metadata`, `pod_template_text_without_template_is_err`, `pod_template_yaml_gets_the_replica_set` (`FakeApi` transport: one `GET /apis/apps/v1/namespaces/shop/replicasets/api-7d9f8c`, no other request) |
| `revision_diff.rs` tests | `diff_request_puts_older_left`, `diff_request_when_current_is_older` (after a roll back), `same_templates_say_so`, `equal_texts_with_hidden_env_say_no_visible_difference`, `revision_parses_from_the_annotation_text`, `env_toggle_shown_only_when_hidden`, `subtitle_names_revisions_tags_and_cluster` |
| `live_sections_tests.rs` | `diff_button_only_on_non_current_rows`, `no_diff_without_a_current_revision` |
| `yaml_edit_panels` (existing edit diff tests) | unchanged results after the `diff_row_element` extraction |
| `app_shell_tests.rs` (headless) | `revision_diff_uses_the_session_connection` (opens nothing while not live) |
| `launch_options.rs`, `screenshot.rs` tests | `screen_revision_diff_parses`, `revision_diff_fixture_needs_no_connection` |

## Step 4

| Module | Tests |
|---|---|
| `limit_range.rs` tests (cluster) | `summary_keeps_quantities_as_written`, `summary_orders_limits_as_listed`, `empty_spec_has_no_limits` |
| `access_review` tests (cluster) | `all_checks_cover_distinct_permissions` (count + 1), `list_limit_ranges_attributes` |
| `cluster_session_tests.rs` | `namespace_related_lists_quotas_and_limit_ranges`, `namespace_list_gates_each_follow_their_check`, `denied_limit_ranges_leave_the_quota_stream`, `denied_quotas_leave_the_limit_range_stream`, `both_denied_drop_the_subject`, `limit_range_update_reaches_its_list` |
| `live_sections_tests.rs` | `limit_range_text_lists_each_part`, `limit_range_text_without_limits`, `no_limit_range_note`, `denied_limit_ranges_note`, `denied_quotas_note_with_limit_ranges_listed` |

## Live checks (coder-lite, UAT, read-only)

`--kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --config-dir .tmp/config-0039-<case>`; never print the kubeconfig; never set `K8SBOARD_ALLOW_WRITES`.

1. `--screen namespaces-drawer`: the Quota section shows the LimitRange rows or `No LimitRange`; with `RUST_LOG=kube=trace` the requests are GET (list/watch) and SSAR POSTs only (counts, never headers).
2. `--screen deployments-drawer`: Revisions rows have `Diff` where not current; pressing it (manual, if driven) issues two GETs of ReplicaSets.
3. `--screen cronjobs-menu`: `View logs of last job` enabled or disabled with its reason.
4. `--screen pod-containers`: the container ⋯ menu; Open shell is disabled with `Not permitted: …` on UAT.

## ui-verifier (screenshot build)

| Screen | Expect |
|---|---|
| `pod-containers` | ⋯ button at the right of the container header (W4b) |
| `cronjobs-menu`, `resourcequotas-menu` | `View logs of last job` with hint L; quotas: `Edit YAML`, no disabled `Edit` (W7) |
| `configmaps-drawer` | the restart hint under Used by when an env reader exists |
| `deployments-drawer`, `revision-diff` | `Diff` buttons aligned in their slot; dialog with red/green rows from theme tokens, title and subtitle |
| `namespaces-drawer` | Quota section: quotas then LimitRange rows (W7 Namespaces) |
