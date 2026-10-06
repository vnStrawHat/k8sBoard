# 0003 · Test plan

[Back to index](README.md)

Unit tests cover the pure logic, inline (`mod tests`) or in a sibling `*_tests.rs` when large. There are no GPUI render tests in this spec: layout is checked by the ui-verifier with the screenshot hook. No test touches a network. Cluster crate tests are listed in [cluster-additions.md](cluster-additions.md).

## `launch_options.rs`

| Test | Verifies |
|---|---|
| `parses_all_flags` | every flag and value lands in `LaunchOptions` |
| `defaults_without_flags` | screen `Pods`, no theme, no screenshot |
| `unknown_flag_is_error` | |
| `missing_value_is_error` | e.g. a trailing `--context` |
| `invalid_screen_or_theme_is_error` | |
| `help_flag_returns_help` | |
| `kubeconfig_flag_wins_over_env_and_home` | |
| `kubeconfig_env_uses_first_entry` | the env value is built with `std::env::join_paths`, so the test is OS-agnostic |
| `kubeconfig_falls_back_to_home_dot_kube_config` | |
| `kubeconfig_none_when_nothing_available` | |

## `cluster_session.rs` (`LiveList`, `initial_scope`)

| Test | Verifies |
|---|---|
| `live_list_snapshot_from_loading_becomes_ready` | |
| `live_list_failed_before_data_is_failed` | |
| `live_list_failed_after_data_keeps_items_and_sets_interruption` | |
| `live_list_snapshot_clears_interruption` | |
| `initial_scope_uses_requested_namespace` | `--namespace` wins |
| `initial_scope_all_when_list_pods_allowed_everywhere` | |
| `initial_scope_context_namespace_when_list_pods_denied` | |
| `initial_scope_context_namespace_when_review_failed` | |

## `status_tone.rs`

| Test | Verifies |
|---|---|
| `pod_running_all_ready_is_ok` | |
| `pod_readiness_failed_when_running_and_not_all_ready` | the label text is exactly "Readiness failed", tone Warn |
| `pod_running_not_ready_with_waiting_container_keeps_status_text` | the rule needs every main/sidecar container `Running` |
| `pod_bad_reasons_are_bad` | table-driven over the Bad list |
| `pod_init_reason_takes_reason_tone` | `Init:CrashLoopBackOff` is Bad; `Init:1/2` is Info |
| `pod_completed_is_done_and_terminating_is_info` | |
| `node_ready_scheduling_disabled_reads_cordoned_in_warn` | "Cordoned" |
| `node_not_ready_is_bad` | |
| `container_terminated_exit_zero_is_done_nonzero_is_bad` | |

## `age.rs`

| Test | Verifies |
|---|---|
| `age_none_is_dash` | |
| `age_boundaries_seconds_minutes_hours_days` | 59 s → `59s`, 60 s → `1m`, 59 m → `59m`, 60 m → `1h`, 23 h → `23h`, 24 h → `1d` |
| `age_future_timestamp_clamps_to_zero` | |

## `tables` helpers (`pod_table.rs` / `node_table.rs`)

| Test | Verifies |
|---|---|
| `row_index_finds_key_after_reorder` | |
| `row_index_none_when_key_vanished` | |
| `selection_sync_keeps_unchanged_index` | no `set_selected_row` call, so no scroll and no re-emitted `SelectRow` |
| `selection_sync_moves_on_reorder` | |
| `selection_sync_clears_when_subject_deleted` | |
| `taints_cell_shows_first_and_plus_count` | `a:NoSchedule +2` |
| `roles_cell_dash_when_empty` | |

## `resource_actions.rs`

| Test | Verifies |
|---|---|
| `copy_name_always_enabled` | in every `AccessState` |
| `cordon_and_drain_always_disabled_read_only_mode` | reason "Read-only mode" |
| `gated_actions_disabled_while_checking` | |
| `shell_denied_reason_names_access_check` | "Not permitted: create pods/exec" |
| `shell_allowed_still_disabled_in_read_only_mode` | |
| `logs_allowed_disabled_with_later_version_reason` | |
| `node_shell_gated_by_create_pods_exec` | |

## `screenshot.rs`

| Test | Verifies |
|---|---|
| `settled_when_session_failed` | |
| `not_settled_while_target_list_loading` | |
| `drawer_screen_needs_selection_to_settle` | |
| `outcome_exit_codes` | saved = 0, timeout = 3, failed = 1 |
| `pod_drawer_prefers_first_multi_container_pod` | the pure picker over `&[PodSummary]` |

## ui-verifier checklist (after coder and coder-lite)

1. Capture the 10 screenshots plus the error state as in [screenshot-hook.md](screenshot-hook.md).
2. Compare them with the anatomy, W4 (table + overview drawer), W4b (containers, expanded), and W5 (nodes):
   - the drawer overlays the workspace only, and the table width is unchanged;
   - the drawer has no body buttons;
   - status colors are correct in both themes;
   - columns are aligned, with mono numbers right-aligned;
   - the sidebar shows disabled kinds;
   - the status bar is present;
   - names truncate with an ellipsis.
3. Menus cannot be captured headlessly. RBAC gating is covered by `resource_actions` tests and the 0001 probe access matrix; the user spot-checks the right-click menu on UAT.
