# 0030 · Test plan

[Back to index](README.md). All tests are offline. **No test, probe, or agent run sends a write to any cluster**; UAT is read-only and stays so, and agent runs never set `K8SBOARD_ALLOW_WRITES`. File tests use `std::env::temp_dir()`.

## Step 1 — cluster crate (`object_write_tests.rs`, `FakeApi`, `#[tokio::test]` with `test-util`)

| Test | Checks |
|---|---|
| `allow_list_matches_the_operations` | `PATCH /api/v1/nodes/wk-04`, `application/merge-patch+json`, body `{"spec":{"unschedulable":true}}` |
| `uncordon_sends_false` | body `{"spec":{"unschedulable":false}}`, never `null` |
| `dry_run_sends_dry_run_all_and_field_manager` | query has `dryRun=All` and `fieldManager=k8sboard` |
| `commit_sends_field_manager_without_dry_run` | `fieldManager=k8sboard`, no `dryRun` |
| `debug_build_blocks_writes_without_opt_in` | policy from `WritePolicy::resolve(true, None)` → `WritesBlocked`; **zero** recorded requests |
| `write_policy_resolve_table` | release → Allowed; debug + `None`/`"0"`/`"true"` → Blocked; debug + `"1"` → Allowed |
| `request_rejects_a_kind_that_does_not_fit` | `SetNodeSchedulable` on a Pod `ObjectRef` → `None` |
| `access_check_matches_the_operation` | → `AccessCheck::PatchNodes` → SSAR `patch`, `""`, `nodes`, no namespace |
| `forbidden_becomes_denied`, `missing_object_becomes_not_found` | 403, 404 |
| `conflict_keeps_message_and_managers` | 409 with causes → `Conflict` |
| `invalid_lists_field_paths` | 422 `details.causes[].field` → `Invalid { fields }` |
| `webhook_dry_run_rejection_blocks` | 400 "does not support dry run" on `DryRun` → `DryRunRejected { reason }` |
| `commit_timeout_is_outcome_unknown` | never-answering service + `tokio::time::pause`/`advance` past `REQUEST_TIMEOUT` → `OutcomeUnknown` |
| `commit_transport_error_is_outcome_unknown` | the service returns an error (`Service`) on `Commit` → `OutcomeUnknown` |
| `dry_run_errors_are_never_outcome_unknown` | same timeout and service error on `DryRun` → `Cluster(..)` |
| `redaction_applies_to_secret_kinds_only` | pure `redact_message("Secret", ..)` keeps the status reason and field paths, drops the message; `"Node"` unchanged |
| `changed_fields_name_paths_and_values` | `spec.unschedulable` = `"true"` |
| `manual_debug_shows_names_only` | `format!("{:?}")` of request and operation: operation name, kind, namespace, name; no `unschedulable`, no body |

`access_review` tests: `all_checks_cover_distinct_permissions` (count + 1), `patch_nodes_check_is_cluster_scoped`.

Clippy check (coder-lite, once): a scratch `api.delete(..)` in a non-excepted cluster file fails `cargo clippy -D warnings`; reverted after.

## Step 2a — pure guard and gate

`write_guard_tests.rs`: `confirm_mode_defaults_per_environment` (PROD → `TypeName`; STG, DEV, LOCAL → `Click`), `confirm_step_table` (every mode × risk row of guardrails.md), **`non_prod_tiers_always_show_a_dialog`** (STG, DEV, LOCAL, and an unknown context name classified STG, × `Change`/`Destructive` → `DialogConfirm::Click`, never a run without a dialog), `confirm_mode_serializes_kebab_case` (`type-name`, `click`), `lock_at_open_follows_the_profile`, `app_has_no_kube_dependency` (`include_str!("../Cargo.toml")`: no line starting with `kube`).

`resource_actions_tests.rs`: `gate_order_table` (exact texts), `read_only_actions_ignore_the_lock`, `rbac_reason_wins_over_the_lock`, and **`gate_and_confirm_use_the_rows_cluster`**: guard A = `dev-1`, DEV, `Locked`; guard B = `prod-eu-1`, PROD, `Unlocked`; both allow `patch nodes`. Cordon on an A row → `dev-1 is read-only`; on a B row → `Enabled`, `confirm_step` → `TypeName { expected: "prod-eu-1" }`, badge env PROD. Swapping which guard is "primary" in the fixture changes nothing.

`cluster_registry` tests: `confirm_round_trips`, `settings_keys_are_the_allow_list` (gains `confirm`). `settings_window_tests.rs`: `confirm_select_stores_the_mode`, `confirm_auto_clears_the_value`, `pages_follow_w2_order` (Safety at 5).

## Step 2b — lock, badge, key, unlock dialog

`keymap_tests.rs` (0028): `toggle_read_only_is_bound_in_window`, `enter_is_suppressed_in_write_confirm`. Window tests (`app_shell_tests.rs`): `ctrl_shift_r_locks_at_once`, `unlocking_prod_asks_for_the_typed_name`, `unlocking_non_prod_asks_for_a_click` (dialog with the focused `Unlock` button), `badge_shows_the_lock_state`, `lock_toggle_does_not_write_settings`, `reconnect_bumps_the_generation`.

## Step 3 — audit (`audit_log_tests.rs`)

`audit_keys_are_the_allow_list`, `lock_entry_names_the_guard_cluster`, `secret_kind_records_no_values` (pure `recordable_fields("Secret", ..)`), `config_map_records_paths_only`, `note_is_trimmed_and_capped`, `append_adds_one_line_per_entry`, `append_creates_owner_only_file` (`#[cfg(unix)]`, `mode & 0o077 == 0`), `lock_toggle_appends_a_line` (window test, temp config dir; it waits for `toggle_write_lock` of step 2b). Added: `outcomes_serialize_lowercase`, `a_lock_line_has_no_object_error_or_note_key`, `other_kinds_keep_their_values`, `append_fails_without_a_folder`, `the_log_is_audit_jsonl_in_the_settings_folder`.

## Step 4 — write flow

`write_flow_tests.rs`: `commit_block_table` (guard gone; generation changed; Locked; Running; Failed; Rejected; Differs; all clear → `None`, exact texts), `typed_name_must_match_exactly`, `cordon_label_follows_scheduling`, `cordon_intent_targets_the_node`, `write_entry_records_unknown_outcome`, `write_entry_uses_the_intent_cluster`.

Amendment tests (decisions 30–36): `checked_write_runs_commit_block_before_commit`, `checked_write_dry_run_writes_no_audit`, `blocked_is_never_audited`, `confirmed_needs_a_passed_dry_run_and_match`, `warnings_render_under_changes`, `created_name_in_notice_and_audit`; cluster crate: `too_many_requests_maps_429`, `outcome_reports_patched_effect`, `rbac_403_is_denied`, `admission_403_is_invalid` (a 403 without `is forbidden: User` never reads "not permitted"), `commit_outcome_carries_uid`.

Window tests: `confirm_dialog_enables_apply_after_dry_run_passes`, `rejected_dry_run_keeps_apply_disabled`, `enter_confirms_the_focused_button` (Click tier: the primary button has focus; one non-held Enter → commit), `held_enter_does_not_confirm` (`simulate_event(KeyDownEvent { is_held: true, .. "enter" })` → no commit), `type_name_tier_needs_the_match`, `closing_the_dialog_drops_the_dry_run`, `commit_rechecks_the_row_cluster_lock`.

## Live checks (coder-lite, UAT, read-only, denied path only; debug build)

Always `--kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --config-dir .tmp/config-0030-<case>`, never `K8SBOARD_ALLOW_WRITES`.

1. Record the SSAR for `patch nodes` with the probe. If it is **allowed**, stop and report: the live check never presses Cordon.
2. Nodes: right-click a node → `Cordon` disabled with `Not permitted: patch nodes`; press C → notice `Cordon is unavailable: Not permitted: patch nodes`.
3. `RUST_LOG=kube=trace`: only GET (incl. watch/list) and POST to `selfsubjectaccessreviews`; no PATCH, PUT, DELETE. Counts only; never copy headers.
4. Ctrl Shift R toggles the badge; with a seeded `environment: production` entry the session opens `Read-only` and unlocking asks for `uat-monitor` typed. `audit.jsonl` then holds only `Lock`/`Unlock` lines.

Write-capable cluster (R2): a later, user-run check with `K8SBOARD_ALLOW_WRITES=1` cordons and uncordons one node. Not part of these ACs.

## ui-verifier (screenshot build)

| Screen | Seed | Expect |
|---|---|---|
| `cordon-confirm`, light and dark | entry `environment: production`, `display_name: uat-monitor` | W10 modal: PROD badge title, object row, `spec.unschedulable → true`, `Server dry-run passed · 412 ms`, typed-name field, note checkbox, `Back` / `Cordon` |
| `nodes`, seeded PROD | as above | `Read-only` badge, danger dashed border |
| `nodes`, clean dir | none | `Unlocked` badge, warning dashed border (STG) |

Report color literals, clipped text, and a missing env border as defects. Known deviation: W10 names tiers "dev one click, staging Enter"; decision 9 replaced them.

## Deferred and later

- `guard_for` looks up the slot session of the named cluster (0027). Shell-level tests `gate_and_confirm_use_the_rows_cluster_with_a_production_primary` and `..._with_a_staging_primary` (`app_shell_multi_tests.rs`) check that each viewed cluster answers with its own lock and confirm tier whichever one is primary, and that an unviewed cluster has no guard. The menus, the row keys and the palette build the guard from the row's or cursor's slot session; `active_guard` is gone.
- `ObjectKind::ALL`, `ObjectKind::resource()`, and `commit_outcome_carries_uid` are deferred to 0031 and 0033, which are their first users.
- Added with the security review: `request_rejects_a_name_that_changes_the_path`, `a_non_utf8_answer_to_a_commit_is_outcome_unknown`, `a_secret_rbac_403_is_denied_and_holds_only_the_reason_and_fields`, `a_build_that_blocks_writes_ignores_the_opt_in`, `the_screenshot_build_blocks_writes`, `the_patch_nodes_review_posts_one_review_and_nothing_else`.

## Implemented in 2b and 4 (`spec-0030b`)

Window tests are in `app_shell_write_tests.rs` over two fake API servers (`cluster::fake_api`, the cluster crate's `test-support` feature): `prod-a` is the primary and locked, `stg-b` is unlocked. Pure tests are in `write_flow_tests.rs`.

- Plan names kept: `toggle_read_only_is_bound_in_window`, `enter_is_suppressed_in_write_confirm`, `ctrl_shift_r_locks_at_once`, `unlocking_prod_asks_for_the_typed_name`, `unlocking_non_prod_asks_for_a_click`, `lock_toggle_does_not_write_settings`, `lock_toggle_appends_a_line`, `reconnect_bumps_the_generation`, `commit_block_table`, `typed_name_must_match_exactly`, `cordon_label_follows_scheduling`, `cordon_intent_targets_the_node`, `write_entry_records_unknown_outcome`, `write_entry_uses_the_intent_cluster`, `checked_write_runs_commit_block_before_commit`, `checked_write_dry_run_writes_no_audit`, `blocked_is_never_audited`, `confirmed_needs_a_passed_dry_run_and_match`, `confirm_dialog_enables_apply_after_dry_run_passes`, `rejected_dry_run_keeps_apply_disabled`, `enter_confirms_the_focused_button`, `held_enter_does_not_confirm`, `type_name_tier_needs_the_match`, `closing_the_dialog_drops_the_dry_run`, `commit_rechecks_the_row_cluster_lock`.
- `badge_shows_the_lock_state` is pure (`badge_lock`, `badge_tooltip`); `the_badge_is_drawn_with_a_session` checks the button is on screen.
- The multi-cluster test is `cordon_on_a_staging_row_uses_that_clusters_connection_guard_and_tier`: Cordon on a `stg-b` row while `prod-a` is the primary reaches only `stg-b`'s server (dry-run, then commit), with `stg-b`'s click tier and generation.
- Added: `cordon_on_a_locked_production_row_opens_no_dialog`, `the_debug_policy_blocks_a_write_at_the_dry_run`, `a_cordoned_node_is_uncordoned`, `a_conflict_keeps_the_dialog_with_a_retry_that_checks_again`, `a_refused_commit_with_no_retry_closes_the_dialog`, `a_cordon_commit_appends_an_audit_line_of_the_target_cluster`, `cordon_without_patch_nodes_is_not_permitted_and_sends_nothing`, `the_lock_chord_acts_on_the_cursor_cluster`, `a_session_opens_in_its_profiles_lock_state`.
- Deferred to their specs: `warnings_render_under_changes` (0031–0034 add the first warnings) and `created_name_in_notice_and_audit` (0032).

Live check on UAT, denied path only, debug build, `K8SBOARD_ALLOW_WRITES` unset (2026-10-03): the cluster probe reports `patch nodes denied`; a screenshot run of the Nodes screen with `RUST_LOG=cluster=debug` logged 37 `reviewing access` requests (the review POSTs) and 0 `write finished` lines, so no PATCH, PUT, or DELETE was built. Counts only; the kubeconfig and headers are never copied. The menu and key texts are covered by `cordon_without_patch_nodes_is_not_permitted_and_sends_nothing`.

### After the 2b and 4 security review

- Added: `a_conflict_keeps_the_dialog_with_a_retry_that_checks_again` now also checks that the commit stays blocked and a second confirm sends nothing until Retry has run the dry-run again; `the_retry_button_shows_after_a_failed_check`, `a_dialog_that_passed_its_check_has_no_retry`, `escape_marks_the_dialog_closed`, `a_session_that_is_not_live_offers_no_lock`, `a_menu_that_is_out_of_date_adds_a_warning`, `an_up_to_date_menu_adds_no_warning`, `a_tier_made_stricter_after_opening_applies`, `the_audit_log_of_a_lock_session_holds_only_lock_lines` (step 4 of the live checks, headless: Ctrl Shift R on a production cluster, unlock with the typed name, lock again, and the log holds exactly `Unlock` and `Lock`), and the badge tests `an_open_cluster_wins_over_locked_ones`, `the_lock_notice_names_the_cluster`.
- Live check on a normal debug build (not the screenshot build), UAT, Nodes screen, `K8SBOARD_ALLOW_WRITES` unset, 40 s, requests counted by HTTP method from the client's own span (`RUST_LOG=kube_client::client::builder=debug`; the app pins the broader `kube_client::client` target to `error` on purpose, so `kube=trace` shows no requests): 133 GET, 74 POST (all 74 to `selfsubjectaccessreviews` or `selfsubjectrulesreviews`), 0 PATCH, 0 PUT, 0 DELETE. Methods and counts only; no URL, header, or body was copied.
- Manual check for the user, on the real window against UAT (`--context readonly@Monitor`): (1) right-click a node: `Cordon` is disabled with `Not permitted: patch nodes`; (2) select a node and press C: a notice reads `Cordon is unavailable: Not permitted: patch nodes`; (3) press Ctrl Shift R on a PROD-seeded cluster: the unlock dialog asks for the cluster name, and Back leaves it read-only.
