# 0041 · Test plan

[Back to index](README.md) · All tests offline; no test talks to a real cluster.

## Step 1 · cluster crate

| Test | File | Checks |
|---|---|---|
| `deployment_revisions_lists_the_namespace_once` | replica_set tests (`FakeApi`) | one `GET /apis/apps/v1/namespaces/{ns}/replicasets`, no other request (query `labelSelector=app%3Dapi`) |
| `deployment_revisions_empty_selector_sends_nothing` | same | `""` and `<invalid>` → `Ok(vec![])`, zero requests |
| `deployment_revisions_keeps_only_controlled_sets` | same | another Deployment's set, an orphan, and a non-controller owner are dropped |
| `deployment_revisions_refuses_other_kinds` | same | a Service `ObjectRef` → error, zero requests |
| `template_writer_is_the_newest_template_owner` | `deployment_tests.rs` | two entries owning `f:spec.f:template`: the later `time` wins |
| `template_writer_ignores_status_and_non_template_entries` | same | `subresource: status`, `scale`, and an entry owning only `f:metadata` are skipped |
| `template_writer_is_none_without_managed_fields` | same | `None` |
| `demand_counts_replicas_default_one` | `quota_demand_tests.rs` | no `replicas` → ×1 |
| `demand_sums_containers_and_sidecars` | same | two containers + one `restartPolicy: Always` init |
| `demand_takes_a_larger_init_container` | same | a regular init larger than the containers sets the per-pod value |
| `demand_uses_desired_scheduled_for_daemon_sets` | same | `status.desiredNumberScheduled: 4` |
| `demand_skips_unparsable_quantities` | same | `cpu: lots` → 0 |
| `demand_request_defaults_to_limit` | `limits.memory: 1Gi` and no request → `requests_memory` = 1Gi; both missing → 0 |
| `demand_is_none_for_other_kinds` | same | ConfigMap, Pod → `None` |
| `check_not_affected_when_nothing_grows` | same | equal or smaller demand |
| `check_fits_reports_smallest_fraction` | same | `requests.cpu` 1 of 40 cores left vs `requests.memory` 20Gi of 64Gi: CPU (2.5 %) is reported, not the smaller raw number; `left` after the change |
| `check_exceeds_lists_every_shortfall` | same | memory and pods exceeded, order quota then resource |
| `check_reads_cpu_and_memory_aliases` | same | `cpu`, `memory`, `count/pods` |
| `check_skips_scoped_and_unsynced_quotas` | same | `scopes: [BestEffort]`, `used: None` |
| `preview_carries_demand_for_replica_change` | `edit_preview_tests.rs` | replicas 3 → 5 → `Some`, before/after pods 3/5 |
| `preview_has_no_demand_for_label_change` | same | `None` (equal) |
| `preview_daemon_set_demand_reads_status` | DaemonSet `status.desiredNumberScheduled: 4` in fresh and response → pods 4 on both sides (demand computed before the strip) |
| `demand_types_debug_holds_numbers_only` | `quota_demand_tests.rs` | `Debug` of `DemandChange` has no names or text |

## Step 2 · app

| Test | File | Checks |
|---|---|---|
| `revision_list_is_newest_first_with_current_marked` | `revision_diff_tests.rs` | numbers 36, 38, 37, none → 38 (current), 37, 36, none |
| `latest_pair_needs_two_numbered_revisions` | same | one → `None`; two → (newest, previous) |
| `history_tab_only_for_deployments` | `yaml_edit_tests.rs` | Deployment three tabs; Service two |
| `history_selects_previous_revision_on_load` | `revision_history_tests.rs` | `Ready` → `selected` is the previous side, diff entity created |
| `history_current_row_has_no_diff` | same | text `This is the current revision.` |
| `history_denied_sends_nothing` | same | `ListReplicaSets` denied → `Denied`, zero requests |
| `history_single_revision_text` | same | `No earlier revision kept (revisionHistoryLimit)` |
| `history_never_changes_the_editor_text` | `yaml_edit_tests.rs` | text equal before and after tab use |

## Step 3 · app

| Test | File | Checks |
|---|---|---|
| `quota_line_fits_text` | `yaml_edit_tests.rs` | `Namespace quota OK (22Gi left)`, success tone |
| `quota_line_exceeds_adds_dialog_warnings` | same | per-shortfall texts in Checks and `WriteIntent.warnings` |
| `quota_line_off_or_loading_is_muted` | same | `Quota not checked: {reason}` |
| `quota_line_absent_when_not_affected` | same | no line |
| `apply_stays_enabled_when_quota_exceeds` | same | Apply enabled after a passed preview with `Exceeds` |

## Step 4 · app

| Test | File | Checks |
|---|---|---|
| `rollout_actor_is_field_manager_in_window` | `recent_changes_tests.rs` | writer 20 s before `last_seen` → `ci-bot`, `FieldManager` |
| `field_manager_actor_tooltip_says_probably` | tooltip `probably ci-bot · last pod-template writer (field manager)` |
| `rollout_actor_falls_back_outside_window` | same | writer 2 min after, or 40 min before → event source |
| `rollout_actor_without_feed_is_event_source` | same | `deployments: None` |
| `other_rows_keep_their_actor` | same | HPA, node, namespace unchanged |
| `deployment_row_click_opens_latest_diff` | shell flow (fake cluster) | message names no listed set → one LIST (with `labelSelector`), dialog opened with (previous, newest) |
| `deployment_row_click_diffs_the_named_set` | same | message `Scaled up replica set api-b to 3` with revisions a=1, b=2, c=3 → (a, b) |
| `change_pair_falls_back_without_predecessor` | `revision_diff_tests.rs` | named set has the lowest number, or is unlisted → `latest_pair` |
| `event_entry_reads_named_replica_set` | `recent_changes_tests.rs` | `Scaled down replica set api-7d9f8c to 0` → `Some("api-7d9f8c")`; other text → `None` |
| `deployment_row_click_denied_shows_notice` | same | notice text, zero requests |
| `single_revision_click_shows_notice` | same | `No earlier revision kept for deployment/{name}` |
| `go_to_deployment_reveals_and_closes` | `revision_diff_tests.rs` or shell flow | reveal called with the key; dialog closed |

## Live and visual (step 4)

- UAT (screenshot build, `readonly@Monitor`, `RUST_LOG=cluster=debug`): open Edit YAML on a Deployment (the gate allows opening the view only where `update` is allowed; on UAT use `--screen edit-yaml-history` plus the drawer Diff and timeline click), then count requests: new ones are `GET …/replicasets` lists and template GETs only; no PUT, PATCH, POST other than SSAR, or DELETE.
- ui-verifier: AC 12 screens, light and dark, against W10 and W3.
