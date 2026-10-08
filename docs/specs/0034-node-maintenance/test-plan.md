# 0034 · Test plan

[Back to index](README.md). All tests are offline. **No test, probe, or agent run sends a write to any cluster**; agent runs never set `K8SBOARD_ALLOW_WRITES`. The app crate has no kube dependency, so app tests drive the pure plan and state machine with `WriteError` values; the wire format is pinned in the cluster crate through the 0030 `FakeApi`.

## Step 1 — cluster crate (fake transport)

| Test | Checks |
|---|---|
| `allow_list_matches_the_operations` (extended) | rows for `EvictPod`, `SetNodeTaints`, `SetNodeLabels`: method, path, query, content type, exact body |
| `evict_sends_a_policy_v1_eviction_with_uid` | `POST /api/v1/namespaces/payments/pods/api-1/eviction`, `application/json`, `deleteOptions.preconditions.uid`, `gracePeriodSeconds: 30`; key is `deleteOptions` (camel case) |
| `evict_omits_grace_for_pod_default` | no `gracePeriodSeconds` key |
| `evict_dry_run_sends_dry_run_all` | query `dryRun=All&fieldManager=k8sboard` |
| **`evict_429_is_too_many_requests`** | simulated 429 `Status`: message `Cannot evict pod as it would violate the pod's disruption budget.`, cause `DisruptionBudget` / `The disruption budget api-pdb needs 2 healthy pods and has 2 currently`, no `retryAfterSeconds` → `TooManyRequests { retry_after: None, message: <cause> }` |
| `evict_429_still_processing_carries_ten_seconds` | cause `The disruption budget api-pdb is still being processed by the server.`, `retryAfterSeconds: 10` → `retry_after: Some(10 s)`; a 429 without causes falls back to the status message |
| `evict_429_on_dry_run_is_too_many_requests` | same on `DryRun`, never `Cluster(..)` |
| `commit_429_is_never_outcome_unknown` | 429 on `Commit` → `TooManyRequests` |
| `too_many_requests_message_is_the_first_cause` | `map_status` change: a 429 with causes → the first cause message for every operation (the merged mapping used the status message) |
| `evict_uid_conflict_is_conflict` | 409 → `Conflict` |
| **`evict_201_with_failure_status_is_an_error`** | HTTP 201 with body `{"kind":"Status","status":"Failure","code":500,"message":"This pod has more than one PodDisruptionBudget…"}` → `Cluster(..)` with that text; a 201 `Failure` with code 429 → `TooManyRequests` |
| `evict_success_reports_created` | 201 `Success` → `effect: Created`, `created_name: None` |
| `taints_send_full_list_with_resource_version` | body has `metadata.resourceVersion` and every taint in order; `timeAdded` kept |
| `no_taints_send_an_empty_list` | `"taints":[]`, never `null` |
| `labels_set_and_remove_keys` | `{"metadata":{"labels":{"team":"infra","old":null}}}`; unchanged keys absent |
| `drain_pods_use_the_node_field_selector` | `GET /api/v1/pods?fieldSelector=spec.nodeName%3Dwk-04` (paged `limit`/`continue` like `list_all`) |
| `drain_pod_flags` | mirror annotation, emptyDir volume, Succeeded/Failed, `deletionTimestamp`, DaemonSet controller |
| `node_for_edit_reads_taints_labels_and_version` | fixture node → `NodeEdit` |
| `access_check_create_pods_eviction` | SSAR `create`, `""`, `pods`, `eviction`, namespaced; Display `create pods/eviction` |
| `blocked_policy_blocks_evictions` | `WritePolicy::Blocked` → `WritesBlocked`, zero requests |
| `manual_debug_shows_names_only` (extended) | no uid, taint, label, or grace value in `{:?}` |

## Step 2 — node edits (`node_edits_tests.rs`, `row_selection` tests)

`system_taints_are_read_only`, `kubelet_labels_are_read_only_but_node_role_is_editable`, `taint_intent_keeps_system_rows_and_time_added`, `adding_no_execute_is_destructive`, `removing_no_execute_is_change`, `label_intent_sends_only_changes`, `duplicate_keys_are_rejected`, `no_changes_disables_review`, `bulk_cordon_is_a_batch_and_skips_already_cordoned`, `bulk_cordon_needs_every_dry_run`, `bulk_uncordon_all_skipped_reason`, `edit_labels_header_needs_one_ticked_node`. Window tests: `taint_conflict_retry_reopens_fresh_with_notice`, `editor_review_opens_the_confirm_dialog`.

## Step 3a — drain plan and dialog (`drain_plan_tests.rs`)

| Test | Checks |
|---|---|
| `verdict_table` | each row of drain-dialog.md, in order (mirror first; terminating before finished; **finished before DaemonSet**, so a finished DaemonSet pod is evicted) |
| `pending_pods_skip_the_pdb_row` | a pending pod matched by a blocked PDB, or by two PDBs → `Evict(None)` |
| `passing_dry_run_downgrades_blocked_and_waits` | dialog state: local `Blocked`/`Waits` + dry-run `Ok` → `Dry-run accepted` |
| `second_pod_of_a_one_allowed_budget_waits` | rank 1 → `Allows`, rank 2 → `Waits` |
| `blocked_budget_reuses_disruption_state` | `UnhealthyPods`, `NoRoom`, `SyncFailed` → `Blocked` |
| `two_budgets_refuse_the_pod` | `Refused`, and `drain_blocker` names the pod |
| `unticked_option_blocks_drain` | `Needs(..)` → blocker text names the option |
| `option_counts_ignore_the_checkbox_state` | counts from pods |
| `preview_puts_blocked_pods_first` | order Refused, Blocked, Needs, Waits, Allows, Evict, Terminating, skips |

## Step 3b — drain run (`drain_run_tests.rs`, simulated time)

| Test | Checks |
|---|---|
| `retry_delay_table` | n = 1..6 with `None` → 5, 10, 20, 30, 30, 30 s; with 10 s → 10, 10, 20, 30 s |
| **`refused_pod_retries_after_backoff`** | `on_write(TooManyRequests { 10 s })` → `Sleep` until 10 s, then `Evict` the same pod; then `Ok` → `Evicted` |
| `poll_marks_gone_by_uid` | uid absent → `Gone`; same name, new uid → `Gone` |
| `not_found_and_uid_conflict_are_gone` | 404, 409 → `Gone` |
| `unknown_outcome_is_retried` | `OutcomeUnknown` → `Refused`, re-evicted after the delay |
| `timeout_makes_the_node_stuck` | `now − node_started ≥ timeout` → `Stuck` with the pods-left count; no further `Evict` |
| `timeout_is_per_node` | node a takes 4 min of a 5 min timeout; node b still gets 5 min from its own `node_started` |
| `poll_runs_every_three_seconds` | `Poll` at most once per 3 s while pods await deletion |
| `failed_pod_does_not_stop_the_others` | `Denied` on one pod; the next pod is still evicted; node ends `Stuck` |
| `cancel_sends_nothing_more` | after `cancel()` every `next_step` is `Finished`; nothing uncordoned |
| `in_flight_result_is_applied_after_cancel` | `on_write` after `cancel()` records the result |
| `blocked_write_stops_the_run` | `CheckedWriteError::Blocked` → Stopped with the text |
| `multi_node_cordons_all_first` | steps `Cordon(a)`, `Cordon(b)` before any `Evict` |
| `stuck_node_stops_later_nodes` | node a `Stuck` → `Finished`; node b never evicted |
| `cordon_failure_stops_the_run` | error on `Cordon(b)` → Stopped; a stays cordoned |
| `new_pod_gets_a_dry_run_first` | a pod first seen at node start → `DryRun` before `Evict` |
| `refusals_are_not_audited` | no audit line for a `TooManyRequests` commit (`checked_write` test, temp config dir) |
| **`node_end_writes_one_summary_line`** | drained, stuck, cancelled, stopped → one `Drain` line each with `evicted`/`refused`/`failed`/`skipped` counts; a node never reached writes none |

Window tests: `drain_uses_the_cursor_slot` (0027 fixture: node of cluster B → guard, connection, tier, and audit of B), `slot_release_stops_the_drain` (`leaving_work` line, then run `stopped`, one summary line per reached node, tab closed, notification kept), `node_menu_items_dispatch_their_keys` (right-clicked node → the D arm runs on it), `drain_dialog_requires_the_node_name_on_prod`, `drain_of_several_nodes_types_the_cluster_name`, `held_enter_does_not_drain`, `skip_pdbs_is_disabled_with_reason`, `cordon_only_sends_no_eviction`, `drain_tab_cannot_close_while_running`, `second_drain_on_the_cluster_is_disabled`, `d_key_opens_the_dialog`, `drain_button_disabled_until_step_3b` (3a), `cordon_only_commits_through_checked_write`.

## Live checks (coder-lite, UAT, read-only, denied path only; debug build)

Always `--kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --config-dir .tmp/config-0034-<case>`; never `K8SBOARD_ALLOW_WRITES`.

1. Probe SSARs `create pods/eviction` and `patch nodes`. If either is **allowed**, stop and report; the check never presses Drain, Cordon, or an editor.
2. Nodes: right-click → `Drain…` disabled `Not permitted: create pods/eviction`; `Edit taints…` / `Edit labels…` disabled `Not permitted: patch nodes`; D and C show `… is unavailable: …`.
3. Tick 2 nodes: `Cordon`, `Uncordon`, `Drain…` disabled with their reasons; header `Edit labels` disabled.
4. `RUST_LOG=kube=trace`: only GET (list/watch) and SSAR POSTs; counts only, never headers.

## ui-verifier (screenshot build)

| Screen | Seed | Expect |
|---|---|---|
| `drain-dialog`, light and dark | PROD entry; fixture plan of W6 (23 pods: 1 blocked, 1 PDB allows 1, 1 unmanaged, 6 DaemonSet) | W6: title, steps, four options with counts, Skip PDBs danger and disabled, grace and timeout, preview order, HEADS UP, typed node name `matches`, `Cancel` · `Cordon only` · `Drain wk-04` |
| `drain-progress` | fixture run: 12 of 23 gone, one refused with retry countdown | dock tab header, progress bar, refused row warn, `Cancel` |

Report color literals, clipped text, and a missing danger tone as defects.
