# 0010 · App: access gates and session feeds

[Back to index](README.md) · Step 2 · Modules: `cluster_metrics.rs` (new) + `cluster_metrics_tests.rs`, `cluster_session.rs`, `screenshot.rs`

## Types (`cluster_metrics.rs`)

```rust
/// Both metrics feeds of a live session. Dropping it stops the review and both polls.
pub(crate) struct ClusterMetrics {
    pub(crate) pods: MetricsFeed<PodUsageHistory>,
    pub(crate) nodes: MetricsFeed<NodeUsageHistory>,
    pod_review: PodReview,                      // Running { _task } | Done(Result<Vec<NamespaceAccess>, String>)
}
pub(crate) struct MetricsFeed<H> {
    pub(crate) history: H,
    pub(crate) status: FeedStatus,
    /// Namespaces left out for lack of access, e.g. `no access in web`; pods feed only.
    // Step 3: the Monitor tab is its first reader. Step 2 logs the pods gate note instead.
    pub(crate) note: Option<String>,
    subscription: Option<WatchSubscription>,    // Some exactly while polling
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FeedStatus {
    Checking,                 // an access review still runs
    Waiting,                  // polling, no sample yet
    Live,
    Interrupted(String),      // the last poll failed; older ticks stay
    Failed(String),           // polling, no sample ever; retrying with backoff
    Unavailable(String),      // denied: never polled
}
pub(crate) enum PodsGate { Wait, Poll { scope: NamespaceScope, note: Option<String> }, Off(String) }
pub(crate) enum NodesGate { Wait, Poll, Off(String) }
pub(crate) fn pods_gate(review: Option<&Result<Vec<NamespaceAccess>, String>>, scope: &NamespaceScope) -> PodsGate;
pub(crate) fn nodes_gate(access: &AccessState, check: AccessCheck) -> NodesGate;   // 0011 passes GetNodeProxy
/// The text shown for a failed poll (decision 6).
pub(crate) fn poll_error_text(error: &ClusterError) -> String;
```

`LiveCluster` gains `pub(crate) metrics: ClusterMetrics`.

## Gates

| `pods_gate` input | Result |
|---|---|
| review running (`None`) | `Wait` |
| `Err(_)` (review failed) | `Poll { scope, note: None }` (like access `Unknown`) |
| every namespace allowed (or the single All/Named review) | `Poll { scope, note: None }` |
| some allowed | `Poll { scope: NamespaceScope::of_namespaces(allowed), note: Some("no access in {denied, …}") }` |
| none allowed | `Off("not allowed to list pods.metrics.k8s.io in {denied, …}")` plus `: {reason}` of the first denial when present; All reads `in all namespaces` |

| `nodes_gate(access, ListNodeMetrics)` (the check in the session report) | Result |
|---|---|
| `AccessState::Checking` | `Wait` |
| `Known`, denied | `Off("not allowed to {check}")` (`list nodes.metrics.k8s.io`) plus `: {reason}` |
| `Known` allowed, or `Unknown` | `Poll` |

| `poll_error_text` | Text |
|---|---|
| `ClusterError::Api { code: 404, .. }` | `metrics-server is not installed: the cluster does not serve metrics.k8s.io` |
| `Api { code: 503, .. }` | `metrics.k8s.io does not answer: {error_text}` |
| anything else | `error_text(error)` |

## Lifecycle

| Event | Effect |
|---|---|
| `LiveCluster::start` | `ClusterMetrics::start`: spawn `review_namespaces(ListPodMetrics, &scope)` on the runtime (a `cx.spawn` stores it and calls `update_metrics_feeds`); both feeds `Checking` |
| pod review done, `finish_access_review` | `ClusterSession::update_metrics_feeds(cx)` |
| `update_metrics_feeds` | per feed from its gate: `Wait` → a running poll continues, a stopped feed shows `Checking`; `Off(reason)` → drop the subscription, `Unavailable(reason)`; `Poll` → subscribe if none (pods: `poll_pod_metrics(gate scope)`, step 2 only logs `note`, step 3 stores it in `MetricsFeed.note` once the Monitor tab reads it; nodes: `poll_node_metrics()`), status `Waiting` unless `Live`/`Interrupted` |
| `set_scope` | drop the pods subscription, `pods.history.retain_scope(&scope)`, pods `Checking`, start a new pod review. The nodes feed keeps running |
| pods `Snapshot(items)` | `pods.history.record(jiff::Timestamp::now(), &items, live.pods.items())`; `Live` |
| nodes `Snapshot(items)` | `nodes.history.record(now, &items)`; `Live` |
| `Failed(error)` | `Interrupted(poll_error_text)` when `tick_count() > 0`, else `Failed(poll_error_text)`; polling continues |
| stream closed | `Failed("metrics polling stopped unexpectedly")` |
| session dropped, context switch | everything drops; a new session starts empty |

- The subscribe callbacks borrow `live.metrics.pods` mutably and `live.pods` immutably: two fields of `LiveCluster`, written as direct field paths so the borrows stay disjoint.
- `subscribe` notifies after each update, so the visible table rebuilds (0009) once per tick.
- Feeds are polls: they are not in `watch_count()` and do not set `has_problem()`; their state shows in the Monitor tab and as "—" in tables.

## Reading

| Reader | Uses |
|---|---|
| Pods table, container bars, pod/workload Monitor | `live.metrics.pods` (`history`, `status`, `note`) |
| Nodes table, node drawer, node Monitor | `live.metrics.nodes` |
| Screenshot hook | `is_metrics_settled(status, ticks: u64, min_ticks: u64) -> bool`: `Unavailable`/`Failed`, or `ticks >= min_ticks`. `pods`/`nodes` wait for 1 tick of their feed; monitor screens for 2 (≈ 15 s, inside the 30 s settle timeout) |

## Logging

`error_text` is the only text kept (crate errors are secret-free). `tracing::warn!` once per transition into `Failed`/`Interrupted`/`Unavailable`; never per retry, never values.
