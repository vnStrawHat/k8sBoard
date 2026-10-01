# 0011 · Decisions

[Back to index](README.md). Architect defaults; the user asked not to stop for questions.

## Sources

| # | Decision | Rationale |
|---|---|---|
| 1 | Network from `GET /api/v1/nodes/{node}/proxy/stats/summary`: pod `network` and node `network` (cumulative `rxBytes`/`txBytes`, sample `time`) | one request per node gives every pod's network namespace counters; containers share it (W4c note 5) |
| 2 | **Disk I/O from `GET /api/v1/nodes/{node}/proxy/metrics/cadvisor`**: `container_fs_reads_bytes_total` and `container_fs_writes_bytes_total` | the Summary API (`stats/v1alpha1`) carries filesystem usage only (`usedBytes`, `capacityBytes`, inodes), no read/write counters; W4c names read/write and "kubelet cAdvisor endpoint through the API server proxy" |
| 3 | cAdvisor cost is accepted only on demand: a Monitor tab is visible, at most **3 nodes** per round, stream-parsed line by line keeping two metric families | the body is Prometheus text of roughly 1–4 MB per node (≈ 18 KB per container); always-on for 50 nodes would be ≈ 10 MB/s; on demand it is ≤ 0.8 MB/s while the user looks, transient memory one line |
| 4 | Rejected: showing filesystem usage instead of Disk I/O | a different metric under the wireframe's "read/write" legend would mislead |
| 5 | Rejected: cAdvisor `container_network_*` for Network | its pod labels differ by runtime (`POD`, empty, pause); the summary is already per pod and much smaller |
| 6 | PVC usage from summary pod `volume[]` entries with `pvcRef` (used, capacity, available, inodes, inodes used) | W7 PVC note: "used from kubelet stats"; free with decision 1 |
| 7 | Fixed allow-list `KubeletPath { StatsSummary, MetricsCadvisor }`; node names must pass `is_node_name` (DNS-1123 subdomain) before a request is built. The check is load-bearing: kube does not percent-encode the name | `nodes/proxy` reaches every kubelet API; a name such as `a/../x` or `a#b` would otherwise change the path |
| 8 | Access: the existing `AccessCheck::GetNodeProxy` from the session report; denied → feed `Unavailable`, never polled | same gate shape as 0010's nodes feed |
| 9 | Private `#[derive(Deserialize)]` structs with `Option` fields for the summary (`serde.workspace` added to the cluster crate) | serde skips unknown fields, so only the listed ones are allocated (no full `Value` tree); no new package in `Cargo.lock` |

## Polling

| # | Decision | Rationale |
|---|---|---|
| 10 | One **round** every 15 s (`METRICS_INTERVAL`) = one tick on one kubelet timeline; nodes of a round are fetched concurrently (4 at a time) | tick alignment lets pods on different nodes sum by index, as in 0010 |
| 11 | Summary targets: **every Ready node while the cluster has ≤ `SUMMARY_NODE_LIMIT` (10) Ready nodes** (always on, history exists when a drawer opens); above it, only the open drawer subject's nodes, at most 10 (most subject pods first). **Deviation** from roadmap 0011 ("only while a consumer is visible"), recorded there | small clusters get 0010-like history for ≈ 20–80 KB/s; large clusters pay only for what is open |
| 12 | Disk targets: the subject's nodes, at most 3 (most subject pods first), only while the Monitor tab or container Monitor sub-tab shows | decision 3 |
| 13 | One subscription per session: `poll_kubelet_stats` takes a `tokio::sync::watch::Receiver<KubeletTargets>` and reads the newest targets each round; the sleep is `select!`ed against `changed()`, and a change starts an early round only when it adds a node, never sooner than 3 s after the last round; empty targets idle without requests | no resubscribe storm while the user clicks through drawers; a newly opened subject still gets data at once |
| 14 | A round fails (`WatchUpdate::Failed`, 0010 backoff) only when every node's summary failed; single-node errors stay in the round | a NotReady node must not stall the others |
| 15 | Not-Ready nodes are never targets | the proxy answers 503 or times out after 30 s |

## Rates and history

| # | Decision | Rationale |
|---|---|---|
| 16 | Rate = Δbytes / Δt between kubelet sample times (`time`, or the cAdvisor line timestamp; arrival time when absent), computed in u128 and saturated | the kubelet's own clock, not poll jitter; mixing a kubelet time with an arrival time skews Δt by up to one scrape lag, accepted because kubelets always send `time` |
| 17 | Δt ≤ 0 or < 1 s → repeat the last rate (no new scrape, or a clock step back); a counter that went down → `None` (reset: restart or new pod); no previous sample → `None` | a gap is honest; one tick without a point after a restart |
| 18 | Pod identity includes `uid`; a new uid under the same name resets the counters | StatefulSet pods reuse names |
| 19 | Points: `RatePair<u32>` bytes/s for pods and containers (saturating at 4 GiB/s), `RatePair<u64>` for nodes; 0010's `history_rings.rs` (fine 240 / coarse 288, freeing, 24 h removal), with `RingPoint` impls | 0010 decision 10 pattern |
| 20 | Only pods in the session scope are stored | bounds memory under a Named scope |
| 21 | Host-network pods (`PodSummary.host_network`) get no network rings at all; the Monitor tab says so and counts them for workloads from the pods list | their counters are the node's; no read-side mask needed |
| 22 | Node network: top-level default interface counters when present, else the sum of interfaces not named `lo`, `veth*`, `cali*`, `cni*`, `flannel*`, `cilium*`, `lxc*`, `docker*`, `tunl*`, `vxlan*`, `kube-*`, `weave*`, `br-*` | kubelet sets the top level only for `eth0` |
| 23 | Node disk I/O: cAdvisor root (`id="/"`) series summed over devices; absent → notice "The kubelet reports no node-level disk I/O" | a sum of pod series would miss system I/O and break on container churn |
| 24 | PVC store keeps the newest `PvcUsage` per (namespace, claim), dropped after 240 unseen ticks | 0014 reads "Used %", 0021 sums volumes |

## Budget

| Item | Cost |
|---|---|
| Pod network rings | (240 + 288) × 12 B + ≈ 200 B ≈ 6.5 KB per stored pod |
| Container disk rings | ≈ 6.5 KB per container on disk nodes (≤ 3 nodes) |
| Nodes, PVCs | ≈ 25 KB per node; ≈ 250 B per PVC |
| Typical (10 nodes, 300 pods, 200 disk containers) | ≈ 3.5 MB steady; worst always-on (10 × 110 pods) ≈ 9 MB; plus 0010's ≈ 14 MB |
| Network | summary ≈ 2–4 KB per pod per round: 300 pods ≈ 60 KB/s; cAdvisor ≤ 3 × 1–4 MB per 15 s while a Monitor tab shows |
| Transient | one summary body as text per in-flight node (≤ 4 × ≈ 0.5 MB) plus the small decoded structs; cAdvisor one line |

## UAT probe (filled by coder-lite after step 1)

| Measure | Value |
|---|---|
| Ready nodes; pods; PVCs with stats | — |
| Summary bytes per node (min / max) | — |
| cAdvisor bytes for one node; disk series kept | — |
| Root `id="/"` disk series present | — |

## Known ceilings

- Clusters above 10 Ready nodes start network history when a drawer opens; upgrade path: raise the limit after the probe.
- Workload disk I/O covers at most 3 nodes; the card says how many.
- Two counter resets inside one interval read as one gap.
