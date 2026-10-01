# 0001 · Test plan (pods and pod status)

[Back to index](README.md) · Conventions: [test-plan.md](test-plan.md#conventions)

## `src/pod_tests.rs`

| Test | Pins |
|---|---|
| `containers_list_init_containers_then_main_in_spec_order` | ordering |
| `init_container_with_restart_policy_always_is_sidecar` | `Sidecar` |
| `init_container_without_restart_policy_always_is_init` | `Init` for `None` and for `"OnFailure"` |
| `container_without_status_is_not_reported` | `NotReported`, not ready, 0 restarts |
| `container_state_maps_waiting_running_terminated` | each state; empty reason → `None` |
| `terminated_state_precedes_running_and_waiting` | precedence |
| `last_termination_carries_reason_exit_code_and_signal` | `OOMKilled`, exit 137; zero signal → `None` |
| `negative_restart_count_clamps_to_zero` | defensive conversion |
| `pod_summary_reads_namespace_name_node_and_creation_time` | metadata mapping; status, ready, restarts come from `pod_display` |

## `src/pod_status_tests.rs` (kubectl 1.32 parity)

| Test | Pins |
|---|---|
| `running_pod_with_ready_containers_is_running` | `Running`, ready `n/n` |
| `pending_pod_without_statuses_is_pending` | `Pending`, ready `0/n` |
| `missing_phase_is_unknown` | `Unknown` (deliberate divergence) |
| `pod_reason_overrides_phase` | `Evicted` |
| `scheduling_gated_condition_shows_scheduling_gated` | `SchedulingGated` |
| `waiting_crash_loop_back_off_overrides_phase` | `CrashLoopBackOff` |
| `waiting_image_pull_back_off_overrides_phase` | `ImagePullBackOff` |
| `waiting_container_creating_overrides_phase` | `ContainerCreating` |
| `terminated_oom_killed_shows_oom_killed` | `OOMKilled` |
| `succeeded_pod_with_completed_containers_shows_completed` | `Completed` |
| `terminated_without_reason_shows_exit_code` | `ExitCode:1` |
| `terminated_without_reason_with_signal_shows_signal` | `Signal:9` |
| `first_failing_container_in_spec_order_supplies_reason` | reverse-loop semantics |
| `completed_with_running_container_and_ready_condition_is_running` | `Running` |
| `completed_with_running_container_without_ready_condition_is_not_ready` | `NotReady` |
| `deleting_running_pod_is_terminating` | `Terminating` |
| `deleting_succeeded_pod_keeps_completed` | terminal phase is not `Terminating` (1.30+ guard) |
| `deleting_pod_with_node_lost_reason_is_unknown` | `Unknown` |
| `init_progress_shows_first_incomplete_index_over_total` | `Init:1/3` = `Progress { first_incomplete: 1, total: 3 }`, sidecars counted in the total |
| `init_waiting_reason_shows_init_prefix` | `Init:CrashLoopBackOff` |
| `init_waiting_pod_initializing_shows_progress` | `Init:0/2` |
| `init_terminated_failure_shows_init_reason` | `Init:Error` |
| `init_terminated_without_reason_shows_init_exit_code_or_signal` | `Init(Reason(ExitCode(1)))` = `Init:ExitCode:1`; `Init(Reason(Signal(9)))` = `Init:Signal:9` |
| `started_sidecar_does_not_block_init_progress` | started sidecar skipped; the next init container decides |
| `initialized_condition_uses_main_container_status` | `Initialized=True` overrides a stale init status |
| `ready_total_counts_main_and_sidecar_containers` | total = main + sidecars; classic init excluded |
| `ready_counts_started_ready_sidecars` | sidecar adds to `ready` |
| `running_container_that_is_not_ready_is_not_counted` | ready `0/1`, status `Running` |
| `restarts_sum_sidecar_and_main_after_initialization` | classic init restarts excluded |
| `restarts_sum_init_containers_while_initializing` | kubectl's initializing branch |
| `status_reason_from_api_maps_known_text_and_keeps_unknown` | `"OOMKilled"` → `OomKilled`; `"Foo"` → `Other` |
| `status_display_matches_kubectl_text` | `OOMKilled`, `Init:1/3`, `Init:CrashLoopBackOff`, `Signal:9`, `ExitCode:1`, `NotReady`, `Terminating`, `Other` verbatim |
