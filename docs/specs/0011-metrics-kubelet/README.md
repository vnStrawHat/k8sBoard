# 0011 — Metrics II: kubelet stats, Network and Disk I/O charts, PVC usage (read-only)

Status: draft for advisor review. Crates: `crates/cluster` (step 1), `crates/app` (steps 2–3). Requires 0010 merged (sampler, `poll_updates`, `FeedStatus`, fine/coarse rings, `UsageChart`, Monitor tab). Wireframes: W4c (Network receive/transmit, Disk I/O read/write, source note), W7 (node `mon` net/disk, PVC "Used" and Usage bars, data from kubelet stats). UAT: `get nodes/proxy` allowed.

## Goal

- Read two fixed kubelet paths through the API server node proxy: `stats/summary` (pod and node network counters, PVC volume stats) and `metrics/cadvisor` (container disk read/write byte counters).
- Turn counters into bytes/s rates with reset handling; keep them in 0010-style fine/coarse rings.
- Monitor tab: **Network** and **Disk I/O** charts (two series, legend) for Pod, container, Node, and the five workload kinds; Table view columns; source note.
- Store the newest PVC usage (used, capacity, inodes) per claim for 0014/0021; first consumer: PVC rows of the container Mounts sub-tab.

## Non-goals

Filesystem usage charts for containers, `rootfs`/`logs` stats, node ephemeral-storage bars (not in the wireframe); PVC/PV kind screens (0014); Overview Volumes bar (0021); node logs through the proxy (0019); Prometheus; kube `gzip` feature (open item 1); any other kubelet path.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | `crates/cluster`: `kubelet_stats.rs` (allow-list, summary structs, watch-driven poll), `cadvisor_text.rs` (disk I/O lines), `PodSummary.host_network`, `serde` dependency, probe `--kubelet-seconds` | 1, 2, 3, 4 |
| 2 | App data: `kubelet_history.rs` (on 0010's `history_rings.rs`), `kubelet_metrics.rs` (demand, targets, feed), session wiring, `sync_kubelet_demand`, PVC usage in Mounts | 1, 2, 3, 5, 6 |
| 3 | App Monitor: rate format, two-series legend and notices in `UsageChart`, Network/Disk models, Table view columns, disk demand, screenshots | 1, 2, 7, 8 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale; Disk I/O source choice; budget; ceilings |
| [kubelet-stats.md](kubelet-stats.md) | step 1: path allow-list, types, summary mapping, poll round, probe |
| [cadvisor-disk-io.md](cadvisor-disk-io.md) | step 1: streaming Prometheus-text reader for two metric families |
| [kubelet-history.md](kubelet-history.md) | steps 2–3: rate rule, record, series, disk state, PVC store |
| [kubelet-session.md](kubelet-session.md) | step 2: demand, targets, gate, feed lifecycle, Mounts consumer |
| [monitor-charts.md](monitor-charts.md) | step 3: rate format, chart changes, Network/Disk models, notices, Table view |
| [files-to-touch.md](files-to-touch.md) | modules per crate and step, doc updates |
| [test-plan.md](test-plan.md) | unit tests per step, live checks, ui-verifier checklist |

## Acceptance criteria

- [ ] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`. `Cargo.lock` changes only by the `serde` line under `k8sboard-cluster` (no new package or feature).
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline.
- [ ] 3. Read-only: requests are SSAR `create` plus GETs. Every `proxy/` path literal in `crates/cluster/src` sits in `KubeletPath`; no path is built from user input; node names pass `is_node_name` first. No kube type in a public signature; the crate spawns no task.
- [ ] 4. On UAT, `probe --kubelet-seconds 40` prints ≥ 2 rounds with node, pod, PVC counts and one disk I/O line; byte sizes come from the app's debug trace; results are copied into [decisions.md](decisions.md) "UAT probe".
- [ ] 5. Memory and demand bounds proven by tests: rings ≤ 240/288, unseen series freed, scope change drops other namespaces, targets ≤ 10 summary and ≤ 3 disk nodes, one subscription per session, empty targets make no request, early rounds only for added nodes.
- [ ] 6. A PVC mount row of a pod on UAT (if any PVC exists) shows `{used} of {capacity} used ({pct})`; without stats it is unchanged.
- [ ] 7. Screenshots `pod-monitor`, `node-monitor`, `deployments-monitor` show four cards (CPU, Memory, Network, Disk I/O), two-series legends, rates in `KB/s`/`MB/s`, and the source note; ui-verifier reports no high-severity defect against W4c and W7.
- [ ] 8. Denied `nodes/proxy`, a failed node (summary or disk), a host-network pod, a pod without disk series, and partial node coverage each show their notice (unit tests in `monitor_data_tests.rs`).

## Open items

1. If the UAT probe shows cAdvisor bodies above ~5 MB per node, propose the kube `gzip` feature (adds decompression crates; needs user approval per C6).
2. 0014 decision 9: drawer demand only. The PVC table adds no demand; on clusters of at most 10 Ready nodes every node is polled anyway, and above that the Used column fills for claims on polled nodes only.
3. Node-level disk I/O depends on cAdvisor's root (`id="/"`) series, which cgroup v2 hosts may omit; the probe records it.
