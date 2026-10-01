# 0002 · Public watch API

[Back to index](README.md) · Modules: `src/resource_watch.rs`, plus the `watch_*` methods in their domain modules

## `WatchUpdate`

```rust
/// One item of a resource watch stream.
#[derive(Debug)]
pub enum WatchUpdate<T> {
    /// The complete current set, ordered by (namespace, name). Cluster-scoped kinds
    /// use the name only, which matches the `list_*` order from 0001.
    Snapshot(Vec<T>),
    /// The watch failed and is retrying with backoff. The last snapshot stays the
    /// best-known state. Never sent for an expected 410 relist.
    Failed(ClusterError),
}
```

- The first `Snapshot` marks the end of the initial list. An empty result sends `Snapshot(vec![])`.
- No `Clone`, because `ClusterError` is not `Clone`. Items are moved to one consumer.

## Watch methods

These are plain `fn`s, not `async`: nothing happens until the stream is polled. Each stream owns a client clone (cheap) and is `Send + 'static`.

```rust
impl ClusterConnection {
    // src/pod.rs        action: "watching pods"
    pub fn watch_pods(&self, scope: NamespaceScope)
        -> impl Stream<Item = WatchUpdate<PodSummary>> + Send + 'static;
    // src/node.rs       action: "watching nodes"
    pub fn watch_nodes(&self)
        -> impl Stream<Item = WatchUpdate<NodeSummary>> + Send + 'static;
    // src/namespace.rs  action: "watching namespaces"
    pub fn watch_namespaces(&self)
        -> impl Stream<Item = WatchUpdate<NamespaceSummary>> + Send + 'static;
}
```

- Each method builds its `Api<K>` and calls the private `summary_watch` ([watch-pipeline.md](watch-pipeline.md)) with the existing per-object conversion: `pod_summary`, `node_summary`, or `namespace_summary`.
- Pod `Api` construction for a `NamespaceScope` moves into a private `pods_api(&self, scope)`, which `list_pods` also uses.
- `Stream` is `futures::Stream`. The app already depends on `futures`.
- The streams must be polled by a tokio-driven task: the batching timer is a `tokio::time::sleep_until` inside the stream, not a spawned task.

## Unchanged from 0001

- `ClusterConnection` stays the only entry point, and `kube::Client` stays private.
- `classify_error` becomes `pub(crate)` so watch errors classify the same way as one-shot calls.
- No `read_timeout` on the client. Watches rely on it, and the server closes each watch after `timeoutSeconds` (kube default 290 s), after which the watcher resumes.

## Future (not in this spec)

`watch_events(scope)` with `EventSummary` comes with the Events/Overview screen spec. It reuses `summary_watch` and adds a store cap ([README](README.md) non-goals).
