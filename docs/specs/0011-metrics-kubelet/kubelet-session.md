# 0011 · App: demand, targets, and the kubelet feed

[Back to index](README.md) · Step 2 (disk demand in step 3) · Modules: `kubelet_metrics.rs` (new) + `kubelet_metrics_tests.rs`, `cluster_metrics.rs`, `cluster_session.rs`, `app_shell.rs`, `container_detail.rs`, `screenshot.rs`

## Types (`kubelet_metrics.rs`)

```rust
pub(crate) const SUMMARY_NODE_LIMIT: usize = 10;   // always-on threshold and demand cap (decision 11)
pub(crate) const DISK_IO_NODE_LIMIT: usize = 3;    // decision 12
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum KubeletSubject { Pod { namespace: String, name: String }, Node(String), Workload(PodOwner) }
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct KubeletDemand { pub(crate) subject: Option<KubeletSubject>, pub(crate) wants_disk_io: bool }
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NodeShare { pub(crate) node: String, pub(crate) pods: usize }
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct NodeErrors { pub(crate) summary: Option<String>, pub(crate) disk_io: Option<String> }
/// Feed of a live session; dropping it stops the poll.
pub(crate) struct KubeletFeed {
    pub(crate) history: KubeletHistory,
    pub(crate) status: FeedStatus,                          // 0010's enum
    pub(crate) node_errors: BTreeMap<String, NodeErrors>,   // last round
    demand: KubeletDemand,
    targets: tokio::sync::watch::Sender<KubeletTargets>,    // `targets()` reads `borrow()`
    subscription: Option<WatchSubscription>,
}
/// Nodes running the subject's pods (not `Done`), most pods first, then by name; a Node subject is itself.
pub(crate) fn subject_nodes(subject: &KubeletSubject, pods: &[PodSummary]) -> Vec<NodeShare>;
pub(crate) fn kubelet_targets(demand: &KubeletDemand, nodes: &[NodeSummary], pods: &[PodSummary]) -> KubeletTargets;
pub(crate) fn kubelet_error_text(error: &ClusterError) -> String;
```

`ClusterMetrics` (0010) gains `pub(crate) kubelet: KubeletFeed`.

## Targets (pure)

| Input | `summary_nodes` | `disk_io_nodes` |
|---|---|---|
| Ready nodes ≤ `SUMMARY_NODE_LIMIT` | every Ready node | first `DISK_IO_NODE_LIMIT` demanded Ready nodes, only when `wants_disk_io` |
| Ready nodes > limit | first `SUMMARY_NODE_LIMIT` demanded Ready nodes | same |
| no subject | as above with no demanded nodes | empty |

"Demanded" = `subject_nodes` filtered to Ready nodes (`NodeReadiness::Ready`), order kept. Both lists end sorted by name, so equal demand gives equal targets.

## Gate

0010's `nodes_gate(access, check)` with `AccessCheck::GetNodeProxy` (already in `AccessCheck::ALL`): denied → `not allowed to get nodes/proxy: {reason}`.

## Lifecycle (`cluster_session.rs`)

| Event | Effect |
|---|---|
| `update_metrics_feeds` (0010) | gate `Wait` → `Checking`; `Off(reason)` → drop the subscription, `Unavailable(reason)`; `Poll` → `refresh_kubelet_targets`, then subscribe once if none: `poll_kubelet_stats(targets.subscribe())`; status `Waiting` unless `Live`/`Interrupted` |
| `set_kubelet_demand(demand)` | store; `refresh_kubelet_targets`. Takes no `cx` and never notifies (it runs from `render`) |
| pods or nodes snapshot | `refresh_kubelet_targets` |
| `refresh_kubelet_targets` | compute; `targets.send_if_modified` (a no-op when equal). The poll loop reads the newest value each round and starts early only for new nodes (decision 13). One subscription lives for the whole session |
| `Snapshot(round)` | `history.record(Timestamp::now(), &round, live.pods.items(), &live.scope)`; `node_errors` rebuilt: `summary` from `Err` summaries, `disk_io` from `Some(Err)` disk reads, both through `kubelet_error_text`; `Live` |
| `Failed(error)` | 0010 rule (`Interrupted` after a tick, else `Failed`) with `kubelet_error_text` |
| stream closed | `Failed("kubelet polling stopped unexpectedly")` |
| `set_scope` | `history.retain_scope(&scope)`; targets refresh with the new pods snapshot |
| session dropped | everything drops (the sender closes, the stream ends) |

| `kubelet_error_text` | Text |
|---|---|
| `Api { code: 503, .. }` | `the kubelet does not answer through the API server proxy` |
| `Api { code: 404, .. }` | `the kubelet does not serve this endpoint` |
| anything else | `error_text(error)` |

The subscribe callback borrows `live.metrics.kubelet` mutably and `live.pods`, `live.scope` immutably (disjoint field paths, as 0010). Kubelet polls are not in `watch_count()` and never set `has_problem()`.

## Demand from the shell (`app_shell.rs`)

`fn sync_kubelet_demand(&mut self, cx)` runs in `render` right after `sync_yaml_view` and calls `set_kubelet_demand` only when the value differs from the session's: (Pod and Node subjects come from the drawer key alone; only a Workload reads its row's `related_pods`, for kinds with `has_monitor`.)

| Open drawer | `subject` | `wants_disk_io` (step 3) |
|---|---|---|
| Pod | `Pod { namespace, name }` | tab Monitor, or tab Containers with container sub-tab Monitor |
| Node | `Node(name)` | tab Monitor |
| `has_monitor` kind with `related_pods` | `Workload(owner)` | tab Monitor |
| none, or any other kind | `None` | false |

Step 2 always sends `wants_disk_io: false`.

## Mounts consumer (`container_detail.rs`, step 2)

- `ContainerDetailInput` gains `pub(crate) kubelet: Option<&'a KubeletHistory>`.
- `mount_rows`: for `VolumeSource::PersistentVolumeClaim { claim }` whose `pvc_usage(namespace, claim)` has `used` and a non-zero `capacity`, the row gets a second, muted line under the source: `{used} of {capacity} used ({percent})` with 0010's `Measure::Bytes.format_pair(used, capacity, " of ")` and `format_percent`, e.g. `83 of 100Gi used (83%)`. The source text stays the claim alone (`pvc/data-kafka-0`, truncated with its tooltip), so the usage is never cut off by a long claim name. The line is toned like the usage bars: Warn from 80 %, Bad from 90 % (`usage_tone`). Otherwise the row is unchanged. Decision: the first layout appended ` · {usage}` to the source, which the 0.6/0.4 column split truncated at the default drawer width (screenshots of UAT postgres `coroot-0`).

## Screenshot settle (`screenshot.rs`, step 3)

Monitor screens also wait for `is_metrics_settled(kubelet.status, kubelet.history.tick_count(), 2)` (two rounds give the first rate).

## Logging

`tracing::warn!` once per feed transition into `Failed`/`Interrupted`/`Unavailable`; per-node errors only at `debug`.
