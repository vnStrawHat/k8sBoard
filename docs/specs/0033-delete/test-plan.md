# 0033 · Test plan

[Back to index](README.md). All tests run offline on fixtures and the 0030 `FakeApi` transport. **No test, probe, or agent run sends a delete (dry-run or commit) to any cluster.** Agent runs never set `K8SBOARD_ALLOW_WRITES`.

## Step 1 — cluster crate (`FakeApi`)

| Test | Checks |
|---|---|
| `delete_dry_run_shape` | `DELETE /api/v1/namespaces/payments/pods/api-x`, no query, `application/json`, body `{"dryRun":["All"],"propagationPolicy":"Background","preconditions":{"uid":"u-1"}}` |
| `delete_commit_has_no_dry_run` | no `dryRun`, no `fieldManager`, no `resourceVersion` in the preconditions |
| `propagation_as_str_matches_the_body` | for each variant, the recorded body's `propagationPolicy` equals `as_str()` |
| `uid_precondition_conflict`, `missing_object_is_not_found` | 409 → `Conflict`; 404 → `NotFound` |
| `object_with_deletion_timestamp_is_pending` | `Left` with `deletionTimestamp` and finalizers `["a","b"]` → `DeletionPending` |
| `status_response_is_deleted`, `object_without_deletion_timestamp_is_deleted` | |
| `secret_delete_keeps_no_data` | the response data `c2VjcmV0` appears nowhere in the outcome or its `Debug` |
| `debug_policy_blocks_delete` | `WritesBlocked`, zero requests |
| `request_rejects_an_empty_uid`, `manual_debug_hides_the_uid`, `delete_changed_fields_record_propagation` | |
| `identity_reads_metadata_only` | `Accept` is PartialObjectMetadata; only uid, finalizers, `deletionTimestamp` kept |
| `identity_without_uid_is_unexpected` | fixed text |
| `owns_dependents_table` | six owner kinds true; Pod and ConfigMap false |
| `delete_check_text_and_group`, `lazy_checks_are_distinct_permissions` | `delete deployments`, group `apps`; not in `ALL` |
| `kind_access_includes_delete` (`cluster_session_tests.rs`) | first screen show reviews `Update` (editable) and `Delete` |

## Step 2 — single delete

| Test | Checks |
|---|---|
| `scope_is_the_row_when_unchecked`, `delete_label_names_kind_and_count` | |
| `warnings_follow_the_kind_table` | Namespace, Node, StatefulSet texts; ConfigMap none |
| `pv_warning_follows_reclaim_policy` | `Delete` → warning; `Retain` → none |
| `pvc_warning_uses_the_bound_volume` | bound PV `Delete` → named warning; PV not loaded → the "If…" text |
| `uncontrolled_pod_warns`, `finalizer_lines`, `dependents_text_per_owner_kind` | |
| `pod_pending_without_finalizers_reads_grace_period` | `{label}: terminating (grace period)`; a Deployment → `terminating` |
| `delete_gate_order` | shipped → `Checking permissions…` (lazy) → `Not permitted: delete pods` → `{cluster} is read-only` |
| `delete_always_opens_a_dialog` | both tiers, single and bulk |
| `single_type_name_expects_the_object_name` | TypeName → `expected_name` = `api-x` |
| `identity_404_shows_already_deleted`, `identity_error_stops_before_the_dialog` | no `run_guarded`; zero `DELETE` |
| `single_delete_is_a_one_item_batch` | `GuardedKind::Batch` with one item, `BatchExtras::Delete` |
| `propagation_change_rebuilds_items_and_reruns` (window) | Orphan → `Running`; new items carry `Orphan` |
| `held_enter_does_not_delete`, `uid_conflict_notice` | |
| `delete_records_propagation_only` (`audit_log_tests.rs`) | key allow-list; Secret target: redacted error, no value |
| `del_key_runs_start_delete`, `cmd_backspace_is_bound_on_macos` (`keymap_tests.rs`, `cfg(target_os = "macos")` on the expectation only) | |
| `screen_delete_confirm_parses` | |

## Step 3 — multi-select (0032 batch rules)

`scope_is_the_checked_set_for_a_checked_row`, `scope_caps_at_50`, `scope_refuses_two_clusters`, `bulk_type_name_expects_the_cluster_name`, `bulk_dry_runs_are_sequential_in_order` (recorded order), `bulk_needs_every_dry_run_to_pass` (one 403 → Apply disabled), `bulk_not_found_is_already_gone`, `bulk_commit_stops_on_blocked` (lock toggled after item 3 → 3 commits, `stopped: …`), `bulk_commit_continues_after_one_failure`, `bulk_commit_survives_dialog_close` (close during commits → all items sent, final notification), `bulk_audit_one_line_per_object_with_the_note`, `bulk_summary_notice_counts`, `selection_bar_offers_delete_on_every_screen`, `screen_delete_bulk_confirm_parses`.

## Live check (coder-lite, UAT, read-only, denied path only; debug build)

`--kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --config-dir .tmp/config-0033-<case>`; never `K8SBOARD_ALLOW_WRITES`.

1. Probe the `delete` SSAR of every kind. If any is **allowed**, stop and report.
2. Pods: after the lazy check, `Delete pod…` is disabled with `Not permitted: delete pods`; Del → notice; two ticked rows → the selection bar `Delete…` is disabled with the same reason; a Deployment and a Namespace likewise.
3. `RUST_LOG=kube=trace`: only `GET`/watch and SSAR `POST`; no `DELETE`, and no metadata GET (the gate stops first). Counts only; never copy headers or bodies.

## ui-verifier (screenshot build)

| Screen | Expect |
|---|---|
| `delete-confirm`, light and dark | PROD badge, `Delete deployment on …?`, one-row list, `Dependents` radio with consequences, finalizer line, `Type the object name to confirm`, note, `Back` / danger `Delete` |
| `delete-bulk-confirm` | STG badge, scrollable list with `passed` rows, controller warning, no typed field, danger `Delete 12 of 12` |
| `pods-selected` (0009) | `Delete…` (danger) last before ✕ |

Defects: color literals, a non-danger primary, clipped names without an ellipsis.
