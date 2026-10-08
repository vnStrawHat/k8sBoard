# 0032b · Test plan

[Back to index](README.md). All tests are offline. **No test, probe, or agent run sends a write to any cluster**; agent runs never set `K8SBOARD_ALLOW_WRITES`.

## Step 1 — cluster crate (`object_write_tests.rs`, 0030 `FakeApi`)

| Test | Checks |
|---|---|
| `allow_list_matches_the_operations` (extended) | one row per operation: method, path, query (`fieldManager=k8sboard`), content type `application/merge-patch+json`, exact body |
| `hpa_range_patches_both_fields` | `PATCH /apis/autoscaling/v2/namespaces/web/horizontalpodautoscalers/frontend-hpa`, `{"spec":{"minReplicas":3,"maxReplicas":20}}` |
| `hpa_range_rejects_zero_and_inverted` | `min 0`, `min 5 max 3` → `WriteRequest::new` is `None` |
| `expand_patches_the_storage_request` | `{"spec":{"resources":{"requests":{"storage":"150Gi"}}}}`, value as typed |
| **`expand_trims_whitespace`** | `" 150Gi "` → stored and sent as `"150Gi"`; `"  "` → `None` |
| `expand_rejects_bad_quantities` | `""`, `abc`, `0`, `-5Gi` → `None` |
| `set_default_true_sets_the_ga_annotation` | body has only the GA key `"true"` |
| `set_default_false_clears_the_beta_annotation` | GA `"false"` and beta `null` |
| `default_class_patch_is_cluster_scoped` | path `/apis/storage.k8s.io/v1/storageclasses/gp3`, no namespace |
| `dry_run_sends_dry_run_all` | each operation's dry-run query has `dryRun=All` |
| `expand_admission_refusal_is_invalid` | 403 with the `PersistentVolumeClaimResize` text (no `is forbidden: User`) → `Invalid`, message kept, never `Denied` |
| `expand_shrink_is_invalid` | 422 with cause field `spec.resources.requests.storage` → `Invalid { fields }` |
| `access_check_matches_each_operation` | SSAR attributes; StorageClasses cluster-scoped |
| `changed_fields_values` | `"3"`, `"20"`, `"150Gi"`, `"true"`/`"false"`; an unset also lists the beta annotation path with value `None` |
| `manual_debug_shows_names_only` (extended) | no replica numbers or sizes in `{:?}` |
| `blocked_policy_blocks_the_new_operations` | `WritePolicy::Blocked` → `WritesBlocked`, zero requests |

`access_review` tests: `all_checks_cover_distinct_permissions` (count + 3).

## Step 2 — HPA

`resource_edits_tests.rs`: `hpa_label_names_the_range`, `hpa_warns_when_max_below_current`, `hpa_warns_when_min_above_current`, `hpa_no_warning_inside_range`. `value_popover` tests: `range_form_validates_min_and_max` (exact texts), `range_form_disabled_when_unchanged`, `range_form_owns_its_inputs`. `row_selection`: `hpa_edit_limits_skips_rows_already_in_range`. Window test: `edit_min_max_opens_the_confirm_dialog`.

## Step 3 — PVC

`resource_edits_tests.rs`: `expand_must_grow` (equal or smaller → disabled with `Must be larger than 100Gi`), `expand_compares_with_the_larger_of_request_and_capacity`, `expand_is_change_with_the_irreversible_warning`, `expand_warnings` (irreversible always; in-progress; file-system pending). `resource_actions_tests.rs`: `expand_row_block_table` (pending, terminating, class without expansion when loaded, nothing when the list is not loaded). `row_selection`: `bulk_expand_skips_claims_already_large_enough`. Window test: `expand_confirm_shows_the_irreversible_warning`.

## Step 4 — Set default

`resource_edits_tests.rs`: `set_default_plan_sets_new_then_unsets_old` (items `[gp3 true, io2 false]`, in that order, `on_failure: Stop`), `set_default_unsets_every_other_default` (two old defaults, name order), `set_default_without_previous_default_is_one_item`, `set_default_disabled_on_the_default`, `retry_plan_omits_the_set_when_target_is_default`, `two_defaults_text_names_the_newer_class` (by `created_at`). `write_flow_tests.rs`: **`set_default_stops_when_the_set_fails`** (item 1 fails → zero further requests recorded, the old default still set, notice `stopped after 0 of 2` + Retry), `partial_default_change_offers_retry` (item 2 fails → Stop notice + Retry), `retry_bypasses_row_block_and_replans_only_the_unsets`, `blocked_after_first_item_stops_the_rest`. `row_selection`: `set_default_needs_exactly_one_ticked_class`.

## Live checks (coder-lite, UAT, read-only, denied path only; debug build)

Always `--kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --config-dir .tmp/config-0032b-<case>`; never `K8SBOARD_ALLOW_WRITES`.

1. Probe the SSARs `patch horizontalpodautoscalers`, `patch persistentvolumeclaims`, `patch storageclasses`. If any is **allowed**, stop and report; the check never presses that action.
2. HPAs, PVCs, StorageClasses: the menu items and selection-bar buttons are disabled with `Not permitted: patch …`.
3. `RUST_LOG=kube=trace`: only GET (list/watch) and SSAR POSTs; counts only, never headers.

## ui-verifier (screenshot build)

| Screen | Seed | Expect |
|---|---|---|
| `hpa-range-popover`, light and dark | fixture HPA `3 / 20`, current 9 | two fields, `Now 9 replicas`, scale-down warning when max is 5 |
| `expand-confirm` | PROD entry, fixture claim `100Gi` → `150Gi` | W10 modal, normal (Change) variant, irreversible warning line, typed-name field |
| `default-class-confirm` | STG entry, fixture classes `gp3` (target), `io2` (default) | list variant with two rows in order, both warnings |

Report color literals, clipped text, and a missing warning tone as defects.
