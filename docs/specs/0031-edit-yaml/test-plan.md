# 0031 · Test plan

[Back to index](README.md). All tests run offline on fixtures and the 0030 `FakeApi` transport. **No test, probe, or agent run sends a mutating verb (dry-run included) to any cluster.** Agent runs never set `K8SBOARD_ALLOW_WRITES`. Fixture values are distinctive strings (`s3cr3t-env`, `c2VjcmV0`), so tests can assert they are absent.

## Step 0

`masked_yaml_output_is_unchanged` (every 0007 fixture byte-equal to the captured text); all 0007 tests unmodified.

## Step 1 (pure unless noted)

| Test | Checks |
|---|---|
| `editor_text_round_trips_to_the_same_value` | Deployment, ConfigMap (`"yes"`, `"1.0"`, `"0755"`, `"null"`, `"~"`, `"1e3"`, `\|-` and `\|+` blocks), Secret, CronJob: `to_yaml_text` → `from_str` gives the same `Value` |
| `unquoted_leading_zero_is_decimal_and_flagged` | `defaultMode: 0755` parses as 755; `leading_zero_lines` = that line; quoted `"0755"` is not flagged |
| `base_strips_server_fields`, `base_masks_like_the_yaml_tab`, `edit_header_names_the_placeholder_rule`, `secret_header_says_data_is_locked` | |
| `syntax_error_has_line_and_column`, `non_mapping_is_rejected`, `identity_fields_cannot_change`, `typed_server_fields_are_ignored` | |
| `secret_data_change_is_refused` | changed value, added key, removed key, added `stringData` |
| `secret_label_change_is_allowed` | |
| `unmatched_placeholder_names_its_path` | renamed env var keeping `<hidden>` → `…containers[api].env[DB_PASS2].value` |
| `placeholder_survives_reordering` | containers and uniquely named env entries reordered → `Ok` |
| `duplicate_env_names_match_by_index` | two `env` items named `X`: matched by index; paths read `env[0]`, `env[1]` |
| `unchanged_text_is_no_changes`, `changed_paths_use_names_and_quoted_keys` | |
| `update_check_is_lazy_not_in_all`, `review_checks_reviews_the_given_list` (`FakeApi`) | `Update(Deployment)` → `update deployments`; one SSAR per given check |
| `kind_access_runs_once_per_kind_and_scope` (`cluster_session_tests.rs`) | second screen show → no new review; scope change → cleared |
| `edit_base_reads_one_object` (`FakeApi`) | one `GET` |

## Step 2 (`FakeApi`)

| Test | Checks |
|---|---|
| `replace_dry_run_shape` | `GET`, then `PUT /apis/apps/v1/namespaces/payments/deployments/api?dryRun=All&fieldManager=k8sboard`, `application/json` |
| `replace_commit_has_no_dry_run` | `?fieldManager=k8sboard` |
| `body_carries_base_resource_version_and_uid` | even when the user typed others |
| `body_has_no_status_or_server_metadata` | no `status`, `managedFields`, `generation`, `creationTimestamp` |
| `request_body_never_holds_placeholders` | each fixture × each mask rule: no body contains `<hidden>`, `<hidden, changed>`, `<hidden, moved>`, or `# Values shown as` |
| `placeholders_restore_server_values` | env literal, annotation (top level and template), Secret data |
| `secret_put_keeps_its_data` | |
| `index_matched_placeholder_is_marked_moved` | duplicate env names reordered with `<hidden>` → `after` holds `<hidden, moved>`, `checks` has `Moved` |
| `stale_base_is_a_conflict_without_put` | exactly one recorded request |
| `server_409_is_a_conflict`, `server_422_lists_fields_verbatim`, `secret_errors_are_redacted`, `debug_policy_blocks_before_the_get` | |
| `preview_masks_both_sides` | changed env literal → `<hidden, changed>`; no raw value; neither text starts with a `#` header |
| `preview_lists_field_changes`, `changes_are_capped` | |
| `preview_checks` | Rollout (Deployment, StatefulSet `OnDelete`), `StaleLastApplied` when the annotation exists, none for a plain ConfigMap |
| `request_rejects_a_foreign_edit`, `replace_access_check_is_update`, `manual_debug_shows_names_only` | |

## Step 3 (app, no server write)

`diff_rows_fold_unchanged_runs`, `diff_rows_number_both_sides`, `edit_yaml_is_not_shipped_in_step_3` (gate → `Comes in a later version`), `editor_marks_dirty_on_change`, `env_toggle_disabled_while_dirty`, `apply_while_clean_does_nothing`, `ctrl_s_runs_local_checks_only`, `local_error_stays_on_editor_tab`, `format_sorts_keys_and_keeps_header`, `navigation_with_changes_asks_to_discard`, `cluster_switch_with_changes_asks_to_discard`, `ctrl_s_is_bound_in_yaml_edit`, `screen_edit_yaml_diff_parses`. Window tests use fixtures; there is no connection call and no recorded `PUT`.

## Step 4

| Test | Checks |
|---|---|
| `ctrl_s_runs_the_preview_and_shows_diff`, `stale_preview_needs_a_new_check` | |
| `ctrl_s_while_running_does_nothing` | a second press during `Running` sends no request and opens no dialog |
| `second_ctrl_s_opens_the_confirm_dialog`, `dialog_lists_paths_and_check_warnings` | |
| `commit_success_closes_the_editor`, `commit_conflict_shows_the_banner`, `invalid_lists_fields_in_the_side_panel` | |
| `rebase_keeps_user_changes_on_the_new_base` | replicas 3 → 5 by the user + a label added on the server → both |
| `rebase_keeps_a_concurrent_list_item_change` | user edits `containers[api]` memory; the server changed `containers[sidecar].image` → both in the result |
| `rebase_user_value_wins_on_both_changed`, `rebase_removed_key_stays_removed` | |
| `rebase_reports_unreachable_paths` | the user edited `containers[old]`, the server removed it → `unreachable` lists the path; banner line shown |
| `rebase_lists_server_changes` | `server_changed` = the server's paths; the side panel shows them |
| `audit_records_paths_only` (`audit_log_tests.rs`) | `Edit YAML`, no `value` keys |
| `preview_never_reaches_audit_or_notice` | no preview text in the audit line or the notification |
| `commit_rechecks_the_row_cluster` | 0030 pattern |

## Live check (coder-lite, UAT, read-only, denied path only; debug build, step 4)

`--kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --config-dir .tmp/config-0031-<case>`; never `K8SBOARD_ALLOW_WRITES`.

1. Probe the `update` SSAR of every editable kind. If any is **allowed**, stop and report.
2. Open Deployments (lazy check runs) → `Edit YAML` disabled with `Not permitted: update deployments`; E → notice; ConfigMap `Edit` likewise.
3. `RUST_LOG=kube=trace`: only `GET`/watch and SSAR `POST`; no `PUT`, `PATCH`, `DELETE`. Counts only; never copy headers or bodies.

## ui-verifier (screenshot build, step 3)

`edit-yaml-diff`, light and dark: W10 header with `resourceVersion`, Diff tab selected, red and green rows with signs, a folded row, `2 changes` with `3 → 5` and `512Mi → 1Gi`, Checks (dry-run OK, rollout warning), footer `Dry-run OK · 412 ms`, `Cancel` / `Apply…` + `Ctrl S`. Defects: color literals, clipped paths without a tooltip, no fold row.
