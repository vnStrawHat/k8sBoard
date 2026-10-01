# 0001 · Async and threading contract

[Back to index](README.md)

This contract is for the app spec. 0001 does not change `crates/app`.

## Runtime ownership

- The `k8sboard` binary builds **one** multi-thread tokio runtime with IO and time enabled, for example `Builder::new_multi_thread().worker_threads(2).thread_name("k8sboard-kube").enable_all()`. It does this before GPUI starts and keeps the runtime alive for the whole process, for example in a GPUI `Global`.
- The cluster crate never creates or enters a runtime, never calls `block_on`, and never calls `tokio::spawn` itself.
- `open` makes kube spawn its client worker on the runtime that polls `open`. If that runtime shuts down, later requests fail with `Unreachable`.

## Where calls run

| Call | Runs on |
|---|---|
| `Kubeconfig::load` (blocking file I/O) | tokio `spawn_blocking` or GPUI's background executor, never the main thread |
| `ClusterConnection::open`, every query | the tokio runtime |
| anything in this crate | never awaited or blocked on from the GPUI main thread |

## Send / Sync / 'static

- `Kubeconfig`, `ClusterConnection`, and all summary and report types are `Clone + Send + Sync + 'static`.
- Query futures are `Send`. They borrow `&self` and take `NamespaceScope` by value.
- Pattern: move a connection clone (cheap) into the task, and await the handle inside `cx.spawn`:
  ```rust
  let task = handle.spawn(async move { connection.list_pods(scope).await });
  ```
  A tokio `JoinHandle` can be awaited from GPUI's executor. A `JoinError` (panic or abort) is reported as a failed load.

## Cancellation and deadlines

- Every public future is cancel-safe: read-only, no locks, no state left behind.
- Dropping a GPUI task does **not** cancel the tokio task. The app calls `JoinHandle::abort()`, or uses an abort-on-drop guard, for superseded loads. Example: the user switches cluster in W1.
- An aborted SSAR may still be evaluated by the server. That is harmless, because SSARs are not persisted.
- Connect is bounded at 10 s, and each one-shot request or list page at 30 s. These values are fixed in 0001; per-cluster settings come with the Settings (W2) spec. Callers may add tighter limits with `tokio::time::timeout`.

## Room for 0002 (watchers and reflectors)

- `ClusterConnection` stays the only entry point, and `kube::Client` stays private.
- 0002 adds methods such as `watch_pods(&self, scope: NamespaceScope) -> impl Stream<Item = …> + Send + 'static` on top of `kube::runtime`, which is already enabled. Streams own a connection clone.
- 0002 reuses the per-object conversions `pod_summary`, `node_summary`, and `namespace_summary` (`pub(crate)`). Never convert whole lists.
- Summary types are `Clone + Eq`, so batched changes can be diffed. Identity is `(namespace, name)` for pods and `name` for cluster-scoped objects. 0002 may add `uid` or `resource_version` if reflector keys need them.
- `NamespaceScope` and `classify_error` are shared, so watch errors classify the same way.
- The client sets no `read_timeout`, so long watch reads survive. The 30 s deadline applies only to `run`.
- The crate still spawns nothing. The app drives the streams on its runtime and batches events before `cx.notify()`.
