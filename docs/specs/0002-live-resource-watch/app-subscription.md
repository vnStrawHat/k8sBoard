# 0002 · App subscription contract (tokio to GPUI)

[Back to index](README.md)

This file designs the app side. Implementation lands in 0003 (`crates/app/src/cluster_runtime.rs`). It amends the 0001 [async contract](../0001-cluster-read-only/async-contract.md): **the crate batches**, and the app calls `cx.notify()` once per received update.

## Runtime ownership

- `main` builds the tokio runtime as 0001 describes (`multi_thread`, 2 workers, `thread_name("k8sboard-kube")`, `enable_all`) **before** GPUI starts, and keeps the `Runtime` on its stack until `run` returns.
- The GPUI global holds only a `tokio::runtime::Handle`:
  ```rust
  pub(crate) struct ClusterRuntime { handle: tokio::runtime::Handle }
  impl Global for ClusterRuntime {}
  ```
- Everything from `cluster` (`open`, queries, watch streams) is polled on that runtime. The GPUI main thread only awaits handles and channels.

## Bridge types

```rust
/// A tokio task that is aborted when dropped. Awaiting it yields the task output.
pub(crate) struct RuntimeTask<T>(tokio::task::JoinHandle<T>);
impl<T> Future for RuntimeTask<T> { type Output = Result<T, tokio::task::JoinError>; /* delegate */ }
impl<T> Drop for RuntimeTask<T> { fn drop(&mut self) { self.0.abort() } }

/// A live watch feeding one GPUI entity. Dropping it stops both halves.
pub(crate) struct WatchSubscription { _pump: RuntimeTask<()>, _receiver: gpui_kit::Task<()> }

impl ClusterRuntime {
    pub(crate) fn spawn<F>(&self, future: F) -> RuntimeTask<F::Output>
    where F: Future + Send + 'static, F::Output: Send + 'static;

    /// Generic over the item type `U` since 0004 (pod log updates use it too).
    pub(crate) fn subscribe<V: 'static, U: Send + 'static>(
        &self,
        updates: impl Stream<Item = U> + Send + 'static,
        cx: &mut Context<V>,
        apply: impl Fn(&mut V, U, &mut Context<V>) + 'static,
        on_closed: impl FnOnce(&mut V, &mut Context<V>) + 'static,
    ) -> WatchSubscription;
}
```

## `subscribe` steps

1. `let (sender, mut receiver) = tokio::sync::mpsc::channel(1);`
2. Pump, spawned on tokio: poll `updates` and `sender.send(update).await` each item. Stop when the send fails (receiver gone).
3. Receiver, spawned with `cx.spawn(async move |this, cx| …)`: for each `receiver.recv().await`, call `this.update(cx, |view, cx| { apply(view, update, cx); cx.notify(); })`. Stop when the entity is gone. On `None` (the stream ended for any reason), call `on_closed(view, cx)` and `cx.notify()` if the entity is still alive. Watch owners use it to mark the list as stopped.
4. Return both handles in a `WatchSubscription`.

| Concern | Rule |
|---|---|
| Notify rate | one `cx.notify()` per update; updates are already batched at 100 ms |
| Back-pressure | capacity 1. A slow UI pauses the pump, not memory: at most one queued and one in-flight snapshot per watch. A very long stall can turn the resume into a 410 relist, which the crate handles silently |
| Poller | the pump runs on tokio, as it must: the crate's batching timer is a `tokio::time::sleep_until` inside the stream |
| Cancel | drop the `WatchSubscription`. The `RuntimeTask` aborts the pump, which drops the stream and the HTTP watch. The dropped GPUI task stops the receiver |
| Cluster switch | drop the whole session, and with it every subscription |
| Namespace switch | replace only the pods subscription |
| Panics | a `JoinError` ends the pump; the receiver sees `None`. The session treats a closed watch before `Snapshot` as `Failed("watch stopped")` |
| Main thread | never awaits network futures directly, only the channel and `RuntimeTask` |

## One-shot calls

Use `runtime.spawn(async move { connection.server_version().await })` and `.await` the `RuntimeTask` inside `cx.spawn`. Keep the `RuntimeTask` in the owning entity while it runs, so dropping the entity aborts it, for example a superseded access review.

## Why tokio `mpsc` and not `watch`

The channel carries `Failed` as well as `Snapshot`, and a watch channel would drop a `Failed` that is followed quickly by a snapshot. Capacity 1 gives the same memory bound.
