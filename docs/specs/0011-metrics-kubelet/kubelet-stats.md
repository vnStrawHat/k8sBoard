# 0011 · Cluster: kubelet stats through the node proxy

[Back to index](README.md) · Step 1 · Modules: `kubelet_stats.rs` (new) + `kubelet_stats_tests.rs`, `cadvisor_text.rs` (new, [cadvisor-disk-io.md](cadvisor-disk-io.md)), `pod.rs`, `lib.rs`, `Cargo.toml`, `examples/probe.rs`

## Path allow-list (private)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KubeletPath { StatsSummary, MetricsCadvisor }   // 0019 adds node logs here
impl KubeletPath {
    fn subresource(self) -> &'static str;   // "proxy/stats/summary", "proxy/metrics/cadvisor"
    fn action(self) -> &'static str;        // "reading kubelet stats", "reading kubelet cAdvisor metrics"
}
/// DNS-1123 subdomain: 1–253 chars; dot-separated labels of 1–63 chars from [a-z0-9-],
/// each starting and ending alphanumeric.
fn is_node_name(name: &str) -> bool;
impl ClusterConnection {
    async fn kubelet_text(&self, node: &str, path: KubeletPath) -> Result<String, ClusterError>;
    async fn kubelet_lines(&self, node: &str, path: KubeletPath)
        -> Result<impl AsyncBufRead + use<>, ClusterError>;   // callers pin it
}
```

- **`is_node_name` is load-bearing.** kube's `Request::get_subresource` only rejects an empty name and does not percent-encode it, so `a/../../x`, `a?x`, or `a#b` would change the proxied path. The check runs first in both functions; nothing else stands between a name and the URL.
- Then the request is built inline with `kube::core::Request::new("/api/v1/nodes").get_subresource(path.subresource(), node)` and sent with `client.request_text` / `client.request_stream` inside `self.run(path.action(), …)`. The `http` type is inferred, never named: no `http` dependency. No other code builds a proxy path.
- An invalid name or a build error → `ClusterError::UnexpectedResponse { action, source: "invalid node name" }` (fixed text).
- `client.request` (typed JSON) is not used: on a decode error it traces the whole body.

## Public types (`kubelet_stats.rs`)

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KubeletTargets { pub summary_nodes: Vec<String>, pub disk_io_nodes: Vec<String> }
#[derive(Debug)]
pub struct NodeKubeletStats {
    pub node: String,
    pub summary: Result<KubeletSummary, ClusterError>,
    /// `None` when the node is not a disk I/O target this round.
    pub disk_io: Option<Result<DiskIoSample, ClusterError>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KubeletSummary { pub network: Option<NetworkCounters>, pub pods: Vec<PodKubeletStats> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PodKubeletStats { pub namespace: String, pub name: String, pub uid: String,
    pub network: Option<NetworkCounters>, pub volumes: Vec<PvcUsage> }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkCounters { pub sampled_at: Option<jiff::Timestamp>, pub rx_bytes: u64, pub tx_bytes: u64 }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PvcUsage { pub namespace: String, pub claim: String, pub sampled_at: Option<jiff::Timestamp>,
    pub used: Option<ByteAmount>, pub capacity: Option<ByteAmount>, pub available: Option<ByteAmount>,
    pub inodes_used: Option<u64>, pub inodes: Option<u64> }

impl ClusterConnection {
    /// Polls the newest `targets` value every `METRICS_INTERVAL` (decision 13). Snapshot
    /// items are ordered by node. Never ends; dropping the stream stops it.
    pub fn poll_kubelet_stats(&self, targets: tokio::sync::watch::Receiver<KubeletTargets>)
        -> impl Stream<Item = WatchUpdate<NodeKubeletStats>> + Send + 'static;
}
```

`DiskIoSample` is in [cadvisor-disk-io.md](cadvisor-disk-io.md); `ByteAmount` is 0010's.

## Summary decoding (private)

Private `#[derive(serde::Deserialize)]` structs, `#[serde(rename_all = "camelCase")]`, every field `Option` (lists `#[serde(default)]`); unknown fields are skipped by serde, so only what is listed is allocated: `SummaryDoc { node: Option<NodeDoc>, pods: Vec<PodDoc> }`, `NodeDoc { network }`, `PodDoc { pod_ref, network, volume: Vec<VolumeDoc> }`, `PodRefDoc { name, namespace, uid }`, `NetworkDoc { time, rx_bytes, tx_bytes, interfaces: Vec<InterfaceDoc> }`, `InterfaceDoc { name, rx_bytes, tx_bytes }`, `VolumeDoc { time, pvc_ref: Option<PvcRefDoc>, used_bytes, capacity_bytes, available_bytes, inodes_used, inodes }`.

`fn kubelet_summary(doc: SummaryDoc) -> KubeletSummary` (pure). A body that fails to decode → `UnexpectedResponse` with the fixed source `"kubelet summary could not be decoded"` (serde text can quote the body).

| Output | Rule |
|---|---|
| `KubeletSummary.network` | `node.network` through `network_counters(doc, InterfaceRule::Node)` |
| `PodKubeletStats` | `podRef` `namespace`, `name`, `uid` all present, else the pod is skipped; `network` through `InterfaceRule::Pod` |
| `network_counters` | `time` RFC 3339 (bad → `None`); top-level `rxBytes` and `txBytes` both present → them; else the sum over `interfaces` whose name passes the rule (Pod: not `lo`; Node: decision 22); none → `None` |
| `PvcUsage` | each `volume` with `pvcRef.name` and `pvcRef.namespace`; bytes via `ByteAmount::from_bytes` |

## Poll loop (private core, paused-time tests)

```rust
const MIN_ROUND_GAP: Duration = Duration::from_secs(3);
fn poll_targets<F, Fut>(targets: watch::Receiver<KubeletTargets>, fetch: F)
    -> impl Stream<Item = WatchUpdate<NodeKubeletStats>> + Send + 'static
where F: FnMut(KubeletTargets) -> Fut + Send + 'static,
      Fut: Future<Output = Result<Vec<NodeKubeletStats>, ClusterError>> + Send + 'static;
```

`stream::unfold` over `{ targets, fetch, failures, last: Option<KubeletTargets> }`:

1. Read `targets.borrow_and_update().clone()`. Empty `summary_nodes` → await `changed()` only (no timer, no item); a closed sender ends the stream.
2. Fetch, yield `Snapshot` (failures = 0) or `Failed` (failures + 1); remember the targets as `last`.
3. Wait: `select!` between `sleep(next_delay(failures))` (0010) and `changed()`. A change wakes an early round only when the new targets hold a node or disk node absent from `last`, and never sooner than `MIN_ROUND_GAP` after the previous round ended; a shrink keeps sleeping (the next round reads it).

`poll_kubelet_stats` = `poll_targets(targets, move |t| kubelet_round(connection.clone(), t))`.

## Round (private)

- `kubelet_round(connection, targets)`: `stream::iter(summary_nodes).map(node_stats).buffer_unordered(KUBELET_CONCURRENCY = 4)`, collected, then the pure `round_result(Vec<NodeKubeletStats>) -> Result<…>`: sort by node; every summary `Err` → the first node's error (decision 14); else `Ok`. A disk node outside `summary_nodes` is ignored.
- `node_stats`: summary and cAdvisor with `futures::join!`. Summary: `kubelet_text(node, StatsSummary)`, `serde_json::from_str::<SummaryDoc>`, `kubelet_summary`. Errors classify as usual (`Api { code: 503 }` for an unreachable kubelet).
- Tracing: `tracing::debug!(node, pods, pvcs, bytes)` per summary and `(node, containers, bytes)` per cAdvisor body. Success bodies are never traced; note that kube itself logs non-`Status` error bodies at warn (kubelet error pages hold no secrets).

## Other changes

- `PodSummary.host_network: bool` from `spec.hostNetwork` (absent → `false`), doc "The pod uses the node's network namespace; its network stats are the node's."
- `Cargo.toml`: `serde.workspace = true` (derive). No new package; `Cargo.lock` gains only the `serde` entry in `k8sboard-cluster`'s dependency list.
- `lib.rs`: `mod cadvisor_text; mod kubelet_stats;`; exports `ContainerDiskIo`, `DiskIoCounters`, `DiskIoSample`, `KubeletSummary`, `KubeletTargets`, `NetworkCounters`, `NodeKubeletStats`, `PodKubeletStats`, `PvcUsage`.

## Probe (`examples/probe.rs`)

`--kubelet-seconds <n>`: targets every Ready node for the summary and the first Ready node for disk I/O, polls for `n` s, and prints per node per round `kubelet {node}: 31 pods, 4 PVCs, network rx 1.2 GB tx 0.8 GB`, for the disk node `disk io: 58 containers, node root series yes`, or `kubelet {node} failed: {error}`. Byte sizes come from the debug trace (live check 1). No kubeconfig data, PVC names, or image names are printed.
