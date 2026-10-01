# 0011 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; nothing lands before its first user (dead-code rule).

## Cargo

One change: `crates/cluster/Cargo.toml` gains `serde.workspace = true` (the workspace entry already has `derive`; `serde` and `serde_derive` are already in `Cargo.lock`, which gains only that dependency line under `k8sboard-cluster`). kube `client` (`request_text`, `request_stream`, `kube::core::Request`), `serde_json`, `futures` (`AsyncBufReadExt`, `buffer_unordered`), `tokio` (`time`, `sync::watch`), and `jiff` are already dependencies.

## `crates/cluster` (step 1)

| File | Change |
|---|---|
| `src/kubelet_stats.rs` (new) + `kubelet_stats_tests.rs` | `KubeletPath` (`subresource`, `action`), `is_node_name`, `kubelet_text`, `kubelet_lines`, public types, private `*Doc` structs, `kubelet_summary`, `network_counters`, `poll_targets`, `kubelet_round`, `round_result`, `node_stats`, `read_disk_io`, `poll_kubelet_stats` |
| `src/cadvisor_text.rs` (new) | `DiskIoCounters`, `ContainerDiskIo`, `DiskIoSample`, `ContainerKey`, `disk_io_line`, `DiskIoBuilder`; tests in module |
| `src/connection.rs` | `REQUEST_TIMEOUT` → `pub(crate)` if needed |
| `src/resource_metrics.rs` (0010) | `next_delay` → `pub(crate)` if private to the module |
| `src/pod.rs` (+ `pod_tests.rs`) | `PodSummary.host_network` |
| `Cargo.toml` | `serde.workspace = true` |
| `src/lib.rs` | `mod cadvisor_text; mod kubelet_stats;` and the exports |
| `examples/probe.rs` | `--kubelet-seconds <n>`; usage text |

## `crates/app`

| S | File | Change |
|---|---|---|
| 1 | any `PodSummary { … }` literal in tests | add `host_network: false` |
| 2 | `src/kubelet_history.rs` (new) + `kubelet_history_tests.rs` | `RatePair` (+ `RingPoint` impls), `PodKey`, `KubeletHistory`, `next_rate`, `record`, `pvc_usage`, `retain_scope`, `tick_count` (step 3: `RateKind`, `RateSeries`, `DiskIoState`, `pod_rates`, `owner_rates`, `node_rates`, `span`, `newest_tick`, `disk_io_state`, `has_disk_series`); uses 0010's `history_rings.rs` unchanged |
| 2 | `src/kubelet_metrics.rs` (new) + `kubelet_metrics_tests.rs` | `SUMMARY_NODE_LIMIT`, `DISK_IO_NODE_LIMIT`, `KubeletSubject`, `KubeletDemand`, `NodeShare`, `NodeErrors`, `KubeletFeed` (watch sender), `subject_nodes`, `kubelet_targets`, `kubelet_error_text` |
| 2 | `src/cluster_metrics.rs` | `ClusterMetrics.kubelet`; calls 0010's `nodes_gate(access, GetNodeProxy)` |
| 2 | `src/cluster_session.rs` | `set_kubelet_demand`, `refresh_kubelet_targets` (`send_if_modified`), one subscription per session, snapshot/scope hooks |
| 2 | `src/app_shell.rs` | `sync_kubelet_demand` (subject only); passes `kubelet` to the container detail |
| 2 | `src/container_detail.rs` (+ tests) | `ContainerDetailInput.kubelet`; PVC usage text in `mount_rows` |
| 2 | `src/main.rs` | module declarations |
| 3 | `src/usage_format.rs` | `Measure::Rate` |
| 3 | `src/usage_chart.rs` (+ tests) | `chart_2`, legend, `notice`, rate floor |
| 3 | `src/monitor_data.rs` (+ tests) | `MonitorInput.kubelet`, `.nodes`; Network/Disk models; notices; window end; `MonitorRow` columns |
| 3 | `src/monitor_tab.rs` | body by both feeds, kubelet Alert, Table view columns, note |
| 3 | `src/app_shell.rs` | `wants_disk_io` in `sync_kubelet_demand` |
| 3 | `src/screenshot.rs` | monitor screens wait for the kubelet feed too |

## Docs (with the last step)

| File | Change |
|---|---|
| `docs/roadmap/inventory-screens.md` | W4c-2 → Done (0011); O3 note "Volumes data from 0011" |
| `docs/roadmap/inventory-kinds.md` | Monitor tab row → Done (or Partial if a kind lacks data); PVCs "Used % and inodes 0011" → "data ready (0011), UI 0014" |
| `docs/roadmap/README.md` | status table: Drawer Monitor tab → Done (0010, 0011) |
| `docs/roadmap/gap-plan-read-only.md` (line 22, 0011 entry) | "only while a consumer is visible" → "always on for ≤ 10 Ready nodes, else while a drawer needs the node; cAdvisor only while a Monitor tab shows (≤ 3 nodes)" |
| `docs/roadmap/cross-cutting.md` | UAT table: nodes/proxy row "verified by 0011 probe" with the probe sizes |
| `docs/specs/0011-metrics-kubelet/decisions.md` | "UAT probe" table filled after step 1 |
