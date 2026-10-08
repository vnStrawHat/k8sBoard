# 0032 · Test plan

[Back to index](README.md). All tests are offline. **No test, probe, or agent run sends a write to any cluster**; agent runs never set `K8SBOARD_ALLOW_WRITES`.

## Step 1 — cluster crate (`object_write_tests.rs`, 0030 `FakeApi`)

| Test | Checks |
|---|---|
| `allow_list_matches_the_operations` (extended) | one row per new operation: method, path, query (`fieldManager=k8sboard`), content type, exact body |
| `scale_patches_the_scale_subresource` | `PATCH …/deployments/api/scale` and `…/statefulsets/kafka/scale`, `{"spec":{"replicas":5}}` |
| `scale_rejects_other_kinds` | `ScaleWorkload` on DaemonSet, ReplicaSet, Job → `WriteRequest::new` is `None` |
| `restart_uses_the_kubectl_annotation` | key `kubectl.kubernetes.io/restartedAt`, value `2026-10-02T09:12:03Z` (no fraction) |
| `restart_dry_run_and_commit_send_the_same_body` | two recorded bodies byte-equal |
| `pause_and_resume_send_booleans`, `suspend_and_resume_send_booleans` | `true` / `false`, never `null` |
| `patches_report_the_patched_effect` | `effect: Patched`, `created_name: None` |
| `rollback_reads_then_sends_a_json_patch` | GET deployment, GET replicaset, PATCH `application/json-patch+json`; ops `test /metadata/uid`, `replace /spec/template`; no `pod-template-hash` in the sent labels |
| `rollback_refuses_a_foreign_replica_set` | owner uid differs → `NotFound`, zero PATCH |
| `rollback_refuses_a_changed_revision` | revision annotation differs → `NotFound`, zero PATCH |
| `rollback_refuses_the_current_template` | equal templates (hash removed) → `Invalid`, zero PATCH |
| **`rollback_uid_test_failure_is_conflict`** | PATCH answered 422 → `Conflict { managers: [] }`, fixed message |
| `trigger_builds_a_job_from_the_template` | POST `…/jobs`; `generateName`, `instantiate: manual`, owner ref `controller: true` with the CronJob uid and **no `blockOwnerDeletion` key**, `spec` = `jobTemplate.spec` |
| `trigger_name_base_fits_the_job_limit` | 60-char CronJob name → base of 50 + `-manual-` |
| `creates_report_the_created_name` | response `metadata.name` → `effect: Created`, `created_name: Some(..)` |
| `rerun_strips_controller_fields` | no `spec.selector`, no `manualSelector`, none of the four controller labels, no owner refs, `spec.suspend: false`; base ≤ 51 |
| `create_422_keeps_field_paths_only` | 422 with causes on Trigger/Re-run → `Invalid { fields }`, message is the fixed text (no server text) |
| `read_failure_before_send_is_never_outcome_unknown` | GET 500 on a Commit of Trigger, Re-run, and Roll back → `Cluster(..)`, zero POST/PATCH (the merged `write_error` would say `OutcomeUnknown`) |
| `access_check_matches_each_operation` | kind × operation → check; SSAR attributes |
| `changed_fields_hold_names_and_numbers_only` | Roll back `rev 37 (api-6c1e2a)`; creates show `generateName` text; no template text |
| `manual_debug_shows_names_only` (extended) | no timestamp, replicas, template, or body in `{:?}` |
| `blocked_policy_blocks_every_new_operation` | `WritePolicy::Blocked` → `WritesBlocked`, zero requests (incl. the GETs) |

`access_review` tests: `all_checks_cover_distinct_permissions` (count + 7), `workload_write_checks_are_namespaced`. Clippy check (coder-lite, once): scratch calls to `Api::restart`, `Api::cordon`, `Api::uncordon` fail `-D warnings`; reverted after.

## Step 2a-i

`RowAction` commit (no behavior change; the 0028/0029 tests pass unchanged apart from types): `subject_action_resolves_the_carried_kind` (`Scale` on a Deployment row → `Scale(Deployment)`, on a DaemonSet → `None`, `OpenShell` on a node → `OpenNodeShell`), `every_resource_action_has_a_row_action`, `every_offered_row_action_maps` (0029, on `RowAction`).

`resource_actions_tests.rs`: `gate_reads_the_carried_kind` (`Scale(StatefulSet)` → `Not permitted: patch statefulsets/scale`), `action_availability_stays_two_argument` (compile-level: existing callers unchanged), `gate_order_then_row_block` (Locked wins over paused), `row_block_table`, `state_labels_follow_the_row`, `actions_not_offered_for_other_kinds`, `gate_and_confirm_use_the_rows_cluster` (extended).

`workload_actions_tests.rs`: `scale_to_zero_is_destructive`, `scale_down_warns`, `hpa_warning_only_when_targeting_and_loaded`, `on_delete_warning_for_restart`, `trigger_warnings_follow_policy_and_active_jobs` (Allow, Forbid, Replace × 0/1 active; exact S9 texts), `restart_timestamp_is_whole_seconds`, `previous_revision_is_the_highest_below_current`, `intents_carry_warnings_in_guarded_intent`.

Window tests: `r_restarts_the_cursor_row` (dialog opens; key trigger), `menu_item_dispatches_the_key_on_the_right_clicked_row` (right click on row 3 of slot B, `Restart rollout` → the dialog names row 3 and cluster B), `palette_runs_the_same_arm`, `trigger_notice_names_the_created_job`.

## Step 2a-ii

`value_popover_tests.rs`: `menu_shift_s_and_palette_open_the_same_popover`, `scale_button_disabled_until_a_new_whole_number`, `enter_is_a_key_trigger`, `bulk_popover_starts_empty`. Window tests: `palette_ctrl_enter_argument_mode` (type 5, ⏎ → confirm dialog with `spec.replicas → 5`), `palette_enter_on_scale_falls_back_to_the_popover`, `palette_rejects_non_numbers`, `esc_returns_to_the_list`, `drawer_has_no_replicas_input`. `keymap_tests.rs`: `ctrl_enter_bound_in_palette_only`, `every_offered_row_action_maps` (0029).

## Step 2a-iii

`live_sections_tests.rs`: `revision_roll_back_button_follows_the_gate`, `no_button_on_the_current_revision`. Window tests: `revision_roll_back_opens_the_confirm_dialog`, `menu_roll_back_scrolls_to_revisions`.

## Step 2b — Batch (`write_flow_tests.rs`, `row_selection` tests)

`checked_rows_follow_the_ticks` (`table_view` tests), `batch_requires_one_cluster`, `batch_caps_at_fifty`, `batch_skips_rows_with_a_row_block`, **`batch_apply_needs_every_dry_run_to_pass`**, `batch_dry_runs_are_sequential_and_unaudited`, `batch_continues_after_a_failed_commit`, `batch_stops_when_blocked` (lock between items → rest `Not sent`), `batch_audits_each_commit` (temp config dir, N lines), `batch_list_honours_expected_name`, `bulk_suspend_label_reads_resume_when_all_suspended`, `bulk_actions_follow_the_wireframe`, `bulk_roll_back_is_disabled_with_reason`.

## Live checks (coder-lite, UAT, read-only, denied path only; debug build)

Always `--kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --config-dir .tmp/config-0032-<case>`; never `K8SBOARD_ALLOW_WRITES`.

1. Probe the SSARs of the 7 new checks. If any is **allowed**, stop and report; the check never presses that action.
2. Deployments: menu items disabled with `Not permitted: patch deployments/scale` / `patch deployments`; R and ⇧S show `… is unavailable: …`; revision `Roll back` disabled.
3. Palette `>rest`: Restart disabled with its reason; `Ctrl ⏎` on a Deployment cursor row shows the Scale reason (no argument mode while disabled).
4. Tick 2 CronJobs: `Trigger now` and `Suspend` disabled with `Not permitted: create jobs` / `patch cronjobs`.
5. `RUST_LOG=kube=trace`: only GET (list/watch) and SSAR POSTs; counts only, never headers.

## ui-verifier (screenshot build)

| Screen | Seed | Expect |
|---|---|---|
| `scale-popover`, light and dark | fixture Deployment | popover title, `NumberInput`, `Now 3 desired · 2 ready`, scale-down warning at 1 |
| `scale-confirm` | PROD entry | W10 modal: `Scale deployment api from 3 to 5`, `spec.replicas → 5`, HPA warning, typed-name field |
| `restart-bulk-confirm` | STG entry | list variant: 4 rows passed, `Restart 4`, dry-run line success |

Report color literals, clipped text, and a missing warning tone as defects.
