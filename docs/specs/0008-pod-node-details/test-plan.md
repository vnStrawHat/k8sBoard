# 0008 · Test plan

[Back to index](README.md). **S** is the step. Tests are deterministic and offline; cluster fixtures are k8s-openapi structs (as in `pod_tests.rs`); each test checks one behavior.

## `crates/cluster` (step 1)

| File | Test | Checks |
|---|---|---|
| `pod_tests.rs` | `pod_summary_reads_status_message_cut_at_limit` | phase Failed: trimmed; 1 KiB + `…`; empty → None |
| `pod_tests.rs` | `status_message_kept_only_for_failed_or_evicted` | Running pod with a message → None; reason Evicted → Some; phase Failed → Some |
| `pod_status_tests.rs` | `status_reason_reads_new_image_and_create_errors` | `from_api` and `Display` round trip for the three new variants |
| `pod_tests.rs` | `pod_conditions_keep_reason_and_message` | |
| `pod_tests.rs` | `waiting_state_keeps_message` | |
| `pod_tests.rs` | `container_reads_digest_pull_policy_and_started` | `docker-pullable://x@sha256:ab` → `sha256:ab`; `sha256:cd` kept; other text → None |
| `pod_tests.rs` | `spec_fields_are_filled_without_a_status` | ports, probes, env present for a container with no status |
| `pod_tests.rs` | `terminated_message_is_not_kept` | a distinctive termination message is absent from `format!("{summary:?}")` |
| `container_spec_tests.rs` | `resources_union_requests_and_limits_in_order` | cpu, memory, ephemeral-storage, then by name; one-sided entries |
| `container_spec_tests.rs` | `probe_actions_map_each_handler` | HTTP (scheme, port name or number, path default `/`), TCP, gRPC, exec, none → Unknown |
| `container_spec_tests.rs` | `probe_defaults_period_and_threshold`, `probe_defaults_timeout_to_one_second` | absent → 10 and 3; timeout 1 |
| `container_spec_tests.rs` | `http_probe_path_drops_query_and_headers` | `/ready?token=SECRETQ` → `/ready`; header value absent from `Debug` |
| `container_spec_tests.rs` | `exec_probe_command_is_not_kept` | distinctive command text absent from `Debug` |
| `container_spec_tests.rs` | `env_keeps_names_and_sources_never_values` | every `EnvSource` variant; a distinctive literal value absent from `Debug` of the whole `PodSummary` |
| `container_spec_tests.rs` | `env_from_keeps_source_and_prefix` | |
| `container_spec_tests.rs` | `mounts_resolve_volume_sources` | ConfigMap, Secret, PVC, emptyDir, hostPath, projected, downwardAPI, nfs → Other, missing volume → Other; readOnly, subPath |
| `workload_tests.rs` | existing template port tests pass unchanged through `container_ports` | |
| `event_tests.rs` | `field_path_names_the_container` | containers, initContainers, ephemeralContainers; `spec.volumes`, `implicitly required container api`, and empty → None |
| `node_tests.rs` | `node_conditions_keep_status_reason_message_and_transition` | `Unknown` and odd text → `Unknown` |
| `node_tests.rs` | `node_conditions_ignore_heartbeat` | two nodes differing only in `lastHeartbeatTime` are equal summaries |
| `node_tests.rs` | `node_addresses_keep_api_order` | |
| `node_tests.rs` | `node_system_info_reads_node_info` | absent nodeInfo → empty strings |
| `node_tests.rs` | `node_resources_union_order_and_drop_zero_hugepages` | |
| `node_tests.rs` | `node_labels_are_terms_and_annotations_are_absent` | a distinctive annotation value absent from `Debug` |

## `crates/app`

| S | File | Test |
|---|---|---|
| 2 | `pod_diagnosis_tests.rs` | one test per rule, named `diagnosis_p0_terminating` … `diagnosis_c8_not_ready` (P0–P3, C1–C8); C1 and C5 cover `InvalidImageName` and `CreateContainerError`, and an unknown waiting reason gives no box |
| 2 | `pod_diagnosis_tests.rs` | `diagnosis_prefers_bad_over_warn_then_container_order` |
| 2 | `pod_diagnosis_tests.rs` | `diagnosis_suffix_counts_other_problem_containers` (healthy, 1 other, 2 others) |
| 2 | `pod_diagnosis_tests.rs` | `healthy_and_completed_pods_have_no_diagnosis` |
| 2 | `pod_diagnosis_tests.rs` | `probe_result_table` (every row, incl. Terminating → Inactive and liveness/readiness → WaitingForStartup while a set startup probe has not passed) |
| 2 | `pod_diagnosis_tests.rs` | `probe_failures_match_reason_container_kind_and_run` (other container, other kind, before start, `errored` wording) |
| 2 | `pod_diagnosis_tests.rs` | `probe_failures_use_newest_event_count` (two matches with counts 7 and 3, newest 3 → `Failing { failures: 3 }`) |
| 2 | `status_tone_tests.rs` | `new_reasons_are_bad` (the three variants toned Bad) |
| 2 | `pod_diagnosis_tests.rs` | `next_retry_from_back_off_message` (`5m0s`, `40s`, `1h0m0s`, past → None, bad text → None, not CrashLoopBackOff → None) |
| 2 | `container_detail_tests.rs` | `resource_text_forms`, `resource_label_names`, `probe_summary_text_forms`, the `*_probe_chips_*` tests |
| 2 | `container_detail_tests.rs` | `env_summary_groups_sources_and_caps_at_three`, `mount_summary_first_and_more` |
| 2 | `container_detail_tests.rs` | `env_rows_list_env_from_first_with_targets` (ConfigMap target in the pod namespace; Secret target None; literal text) |
| 2 | `container_detail_tests.rs` | `mount_rows_text_and_targets`, `last_run_text_needs_both_times` |
| 2 | `table_selection_tests.rs` | `of_object_maps_pods_nodes_and_kinds` (ReplicaSet namespaced, Namespace cluster-scoped, Event and unknown kind → None) |
| 2 | `resource_actions_tests.rs` | `kubectl_command_quotes_only_unsafe_parts` (`readonly@Monitor` bare; a space and a `'` quoted) |
| 2 | `drawer.rs` | `drawer_state_starts_on_info` |
| 3 | `kind_row.rs` | `owns_pod_on_node_in_any_namespace` |
| 3 | `status_tone_tests.rs` | `node_condition_tone_table` (Ready and pressure × True/False/Unknown, a custom type) |

## Live checks (coder-lite, UAT `readonly@Monitor`)

1. A pod drawer shows Node and Controlled by as links that open the node and the ReplicaSet/DaemonSet/StatefulSet/Job.
2. Containers → Info shows digest, pull policy, ports with a disabled Forward, resources, and probes for a pod that has them; Env and Mounts list names and sources; no literal env value appears anywhere outside the YAML tab's `Env values` toggle.
3. If UAT has a non-ready or restarting pod, its WHY box text is recorded; if not, the report says so.
4. A node drawer shows conditions, addresses, system info, resources, pods, and labels. Copy kubectl command pastes a command that `kubectl` accepts (read-only `describe`).
5. The 0001 AC7 credential script reports 0 for the app log of the session.

## ui-verifier

Screens `pod-drawer`, `pod-containers` (expanded, Info), `node-drawer`, light and dark: WHY box (when present) above the Pod section, sub-tab bar under the container header, sections readable at 420 px and 640 px, no clipped labels, links in the theme link color. Expected behavior, not defects: the container sub-tab persists across pods (a new pod opens on the last-used sub-tab), and event-based WHY text and probe results appear about 250 ms after selection plus the events list load.
