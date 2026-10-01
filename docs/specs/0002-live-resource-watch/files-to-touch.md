# 0002 · Files to touch

[Back to index](README.md)

## Cargo

| File | Change |
|---|---|
| `crates/cluster/Cargo.toml` | add `[dev-dependencies] tokio = { workspace = true, features = ["test-util"] }`, needed for `start_paused` |
| root `Cargo.toml` | none: `kube` already has `runtime`, and `tokio` has `time`, `sync`, and `macros` |

No new runtime dependencies. `kube` features must still exclude `ws`.

## Modules (`crates/cluster/src`)

| File | Change |
|---|---|
| `resource_watch.rs` (new) | `WatchUpdate` (pub), `summary_watch`, `batch_updates`, `SummaryStore`, `ObjectKey`, `watch_error`, `BATCH_WINDOW` |
| `resource_watch_tests.rs` (new) | store and batching tests (`#[path]` sibling, as in 0001) |
| `pod.rs` | `watch_pods`; private `pods_api(&self, scope)`, now also used by `list_pods` |
| `node.rs` | `watch_nodes` |
| `namespace.rs` | `watch_namespaces` |
| `connection.rs` | `classify_error` becomes `pub(crate)`. No other change |
| `lib.rs` | add `mod resource_watch;` and `pub use resource_watch::WatchUpdate;` |

Every other new item is private or `pub(crate)`. The 0001 API-wide rules still hold:

- every public type is `Send + Sync + 'static`;
- no `kube` or `k8s_openapi` type appears in a public signature;
- `WatchUpdate` derives `Debug` only.

## Probe example (`crates/cluster/examples/probe.rs`)

New optional flag: `--watch-seconds <n>` (a positive integer; anything else prints usage and exits 2).

- Runs after all existing sections, and only when given.
- Starts `watch_pods(scope)`, `watch_nodes()`, and `watch_namespaces()` together, with three `tokio::select!` arms in one loop (or `select_all` over a small enum). It stops after `n` seconds with `tokio::time::sleep`.
- Prints one line per kind, counts only:
  ```text
  watch pods: 3 snapshots, last 104 items, 0 failures
  watch nodes: 1 snapshots, last 4 items, 1 failures; last error: <ClusterError Display>
  ```
- Never prints object contents. The run fails (exit 1) only if a kind had 0 snapshots **and** 0 failures (nothing happened at all).
- Update the doc comment, `USAGE`, and [0001 probe-example.md](../0001-cluster-read-only/probe-example.md): add one table row for the flag.
