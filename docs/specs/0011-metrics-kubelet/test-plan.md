# 0011 · Test plan

[Back to index](README.md). **S** is the step. Deterministic and offline; one behavior per test. JSON and Prometheus text fixtures are inline strings with made-up names.

## Unit tests

| S | File | Test | Checks |
|---|---|---|---|
| 1 | `kubelet_stats_tests.rs` | `node_names_follow_dns_subdomain_rules` | `ip-10-0-1-23`, `a.b` yes; ``, `A`, `-a`, `a-`, `a/b`, `a/../x`, `a..b`, `a?x`, `a#b`, `a%2Fb`, 254 chars no |
| 1 | `kubelet_stats_tests.rs` | `kubelet_paths_are_fixed` | `proxy/stats/summary`, `proxy/metrics/cadvisor`, and their `action` texts |
| 1 | `kubelet_stats_tests.rs` | `summary_reads_pod_network_and_uid` | namespace, name, uid, rx, tx, `time` |
| 1 | `kubelet_stats_tests.rs` | `summary_skips_pods_without_pod_ref` | |
| 1 | `kubelet_stats_tests.rs` | `network_prefers_top_level_counters` | top level present → interfaces ignored |
| 1 | `kubelet_stats_tests.rs` | `pod_network_sums_interfaces_except_loopback` | no top level; `eth0` + `net1`, `lo` dropped |
| 1 | `kubelet_stats_tests.rs` | `node_network_skips_virtual_interfaces` | `ens5` kept; `veth1`, `cni0`, `cali9`, `lo` dropped |
| 1 | `kubelet_stats_tests.rs` | `network_without_counters_is_none` | |
| 1 | `kubelet_stats_tests.rs` | `summary_reads_pvc_volumes_only` | `pvcRef` volume kept with bytes and inodes; configmap volume dropped |
| 1 | `kubelet_stats_tests.rs` | `bad_time_keeps_counters` | `sampled_at` `None`, counters kept |
| 1 | `kubelet_stats_tests.rs` | `round_fails_only_when_every_node_fails` | `round_result`: one ok → `Ok`; all failed → first error |
| 1 | `kubelet_stats_tests.rs` | `round_orders_nodes_by_name` | `round_result` sorts |
| 1 | `kubelet_stats_tests.rs` | `poll_reads_the_newest_targets_each_round` | paused time, fake fetch receives the sent targets |
| 1 | `kubelet_stats_tests.rs` | `added_node_starts_an_early_round` | send at 5 s → fetch at 5 s, not 15 s |
| 1 | `kubelet_stats_tests.rs` | `early_rounds_keep_the_minimum_gap` | send 1 s after a round → fetch at 3 s |
| 1 | `kubelet_stats_tests.rs` | `shrinking_targets_wait_for_the_timer` | |
| 1 | `kubelet_stats_tests.rs` | `empty_targets_make_no_request` | no fetch, no item until a non-empty send |
| 1 | `kubelet_stats_tests.rs` | `closed_sender_ends_the_stream` | while idle |
| 1 | `kubelet_stats_tests.rs` | `summary_ignores_unknown_fields` | extra `cpu`, `memory`, `rootfs` objects decode |
| 1 | `cadvisor_text.rs` | `reads_container_read_and_write_lines` | value, labels, timestamp |
| 1 | `cadvisor_text.rs` | `skips_other_families_and_comments` | `# HELP`, `container_cpu_usage_seconds_total{…}` |
| 1 | `cadvisor_text.rs` | `unescapes_label_values` | `\"`, `\\`, `\n` inside `image` do not break `pod` |
| 1 | `cadvisor_text.rs` | `sums_devices_per_container` | two devices → one container entry |
| 1 | `cadvisor_text.rs` | `root_cgroup_is_the_node_series` | `id="/"` → `node` |
| 1 | `cadvisor_text.rs` | `ignores_pause_pod_and_system_cgroups` | `container="POD"`, `container=""`, `/system.slice/…` |
| 1 | `cadvisor_text.rs` | `rejects_malformed_and_non_finite_values` | missing `}`, `NaN`, `-1`, `+Inf` |
| 1 | `cadvisor_text.rs` | `scientific_values_parse` | `1.2345e+06` → 1,234,500 |
| 1 | `pod_tests.rs` | `host_network_defaults_to_false` | and `true` when set |
| 2 | `kubelet_history_tests.rs` | `first_sample_has_no_rate` | |
| 2 | `kubelet_history_tests.rs` | `rate_divides_by_sample_time` | 15,000 B over 15 s → 1,000 B/s |
| 2 | `kubelet_history_tests.rs` | `same_sample_time_repeats_the_last_rate` | |
| 2 | `kubelet_history_tests.rs` | `rate_ignores_backwards_sample_time` | earlier `at` → last rate, state kept |
| 2 | `kubelet_history_tests.rs` | `rate_math_does_not_overflow` | Δ near `u64::MAX` over 1 s saturates |
| 2 | `kubelet_history_tests.rs` | `rate_pair_mean_is_field_wise` | `RingPoint` impls for `u32` and `u64` |
| 2 | `kubelet_history_tests.rs` | `counter_reset_gives_a_gap` | lower value → `None`, next tick rates again |
| 2 | `kubelet_history_tests.rs` | `new_uid_resets_the_counters` | same name, new uid → `None` |
| 2 | `kubelet_history_tests.rs` | `pod_rates_saturate_at_u32` | |
| 2 | `kubelet_history_tests.rs` | `failed_node_records_none_this_tick` | other nodes recorded |
| 2 | `kubelet_history_tests.rs` | `rings_never_exceed_fine_ticks` | 300 rounds → 240 |
| 2 | `kubelet_history_tests.rs` | `pods_outside_scope_are_not_stored` | |
| 2 | `kubelet_history_tests.rs` | `summary_pod_with_host_network_is_not_stored` | no network rings or counter; its PVCs and disk series still kept |
| 2 | `kubelet_history_tests.rs` | `retain_scope_drops_pods_and_pvcs` | |
| 2 | `kubelet_history_tests.rs` | `pvc_usage_keeps_the_newest_sample` | RWX claim under two pods |
| 2 | `kubelet_history_tests.rs` | `unseen_pvc_is_dropped` | after 240 ticks |
| 2 | `kubelet_metrics_tests.rs` | `small_clusters_poll_every_ready_node` | 3 Ready + 1 NotReady → 3 |
| 2 | `kubelet_metrics_tests.rs` | `large_clusters_poll_only_demanded_nodes` | 12 nodes, subject on 2 → 2; none → empty |
| 2 | `kubelet_metrics_tests.rs` | `summary_demand_caps_at_ten_by_pod_count` | |
| 2 | `kubelet_metrics_tests.rs` | `disk_targets_need_disk_demand_and_cap_at_three` | |
| 2 | `kubelet_metrics_tests.rs` | `targets_are_sorted_for_equality` | |
| 2 | `kubelet_metrics_tests.rs` | `subject_nodes_skip_done_and_unscheduled_pods` | |
| 2 | `kubelet_metrics_tests.rs` | `kubelet_errors_are_worded` | 503, 404, other |
| 2 | `container_detail_tests.rs` | `pvc_mount_shows_usage` | `pvc/data · 83 of 100Gi used (83%)` |
| 2 | `container_detail_tests.rs` | `pvc_mount_without_stats_is_unchanged` | also zero capacity |
| 3 | `kubelet_history_tests.rs` | `pod_disk_total_sums_containers` | |
| 3 | `kubelet_history_tests.rs` | `container_scope_network_is_the_pods` | |
| 3 | `kubelet_history_tests.rs` | `owner_network_skips_host_network_pods` | they add nothing to the sum |
| 3 | `kubelet_history_tests.rs` | `disk_io_state_follows_the_newest_sample` | `NotSampled`, `Sampled { has_root }` |
| 3 | `usage_format.rs` | `format_rate_uses_decimal_units` | table in monitor-charts.md |
| 3 | `usage_chart_tests.rs` | `nice_max_floors_rates_at_one_kb` | |
| 3 | `monitor_data_tests.rs` | `charts_are_cpu_memory_network_disk` | titles, series names, `Rate` |
| 3 | `monitor_data_tests.rs` | `collecting_until_two_ticks` | |
| 3 | `monitor_data_tests.rs` | `not_ready_and_failed_nodes_have_notices` | |
| 3 | `monitor_data_tests.rs` | `host_network_pod_has_no_network_series` | |
| 3 | `monitor_data_tests.rs` | `workload_coverage_notice_counts_nodes` | `Covers pods on 3 of 7 nodes` |
| 3 | `monitor_data_tests.rs` | `node_without_root_disk_io_has_notice` | |
| 3 | `monitor_data_tests.rs` | `pod_without_disk_series_has_notice` | node `Sampled`, pod absent; container variant |
| 3 | `monitor_data_tests.rs` | `workload_without_disk_series_has_notice` | only when every owned pod lacks one |
| 3 | `monitor_data_tests.rs` | `node_errors_split_summary_and_disk` | summary error on Network only, disk error on Disk only |
| 3 | `monitor_data_tests.rs` | `kubelet_cards_end_at_the_kubelet_tick_without_metrics` | metrics ticks → metrics end; none → kubelet newest tick |
| 3 | `monitor_data_tests.rs` | `rows_join_the_nearest_kubelet_tick` | ±7.5 s; outside → `None` |

## Live checks (coder-lite, UAT `readonly@Monitor`)

1. Step 1: `probe --kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --kubelet-seconds 40` → ≥ 2 rounds per node and one disk line. Byte sizes: after step 3, run the app on `pod-monitor` with `RUST_LOG=cluster::kubelet_stats=debug` and read the `bytes` fields. Fill the "UAT probe" table. Nothing from the kubeconfig in any output.
2. Step 1: the 0001 read-only grep (`create|replace|patch|delete|exec|attach|portforward`) finds only the SSAR `create`; `grep -rn "proxy/" crates/cluster/src` finds only the `KubeletPath` literals.
3. Step 2: open a pod drawer on a pod with a PVC (if UAT has one) → Mounts shows usage.
4. Step 3: screenshots `pod-monitor`, `node-monitor`, `deployments-monitor`, light and dark.

## ui-verifier checklist (step 3)

W4c: four cards in the order CPU, Memory, Network, Disk I/O; two per row when expanded; Network and Disk I/O with two lines (`chart_1`, `chart_2`), a legend (`receive`/`transmit`, `read`/`write`), no area fill, `KB/s`/`MB/s` axis labels, `-15m`/`now`; notices muted; the source note names kubelet. W7: node and Deployments Monitor show the same four cards. No hardcoded colors.
