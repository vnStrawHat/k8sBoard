# 0002 — Live resource watch (cluster crate)

Status: approved for implementation (user decisions and advisor review applied). Crate: `crates/cluster`. Builds on [0001](../0001-cluster-read-only/README.md). Implemented before [0003](../0003-app-shell-pods-nodes/README.md).

## Goal

Give the app live, read-only data for pods, nodes, and namespaces, with one watch per kind per connection. Each watch is a `Stream` of full, ordered **snapshots** of domain summaries, batched to at most one every ~100 ms. Under the hood, kube's `watcher` does the initial list and the watch, relists on 410, and retries with backoff. The crate still spawns nothing. The app drives each stream on its tokio runtime and forwards updates into GPUI ([app-subscription.md](app-subscription.md)).

## Non-goals

- **Events (moved to [0006](../0006-events/README.md)).** `watch_events`, `EventSummary`, the store cap, and message truncation live there. 0006 decision 7 supersedes the `watch events` permission check: only `ListEvents` gates the sidebar item.
- kube `reflector::Store` and shared reader handles. The store lives inside the stream, and the app keeps the last snapshot.
- Diffs or incremental updates to the UI. Snapshots are the only output.
- Watches for other kinds, label or field selectors, metric values, and logs.
- Detecting stalled watches (half-open TCP). See open items.
- Proxy support (still bypassed; see 0001) and multi-cluster aggregation.
- Any change to `crates/app`. The GPUI wiring is designed here and implemented in 0003.

## Files

| File | Contents |
|---|---|
| [watch-api.md](watch-api.md) | public API: `WatchUpdate` and the `watch_*` methods |
| [watch-pipeline.md](watch-pipeline.md) | summary store, batching, errors and 410, backoff, cancellation, memory |
| [app-subscription.md](app-subscription.md) | runtime-to-GPUI contract (amends the 0001 async contract) |
| [files-to-touch.md](files-to-touch.md) | modules, `lib.rs`, Cargo, probe `--watch-seconds` |
| [test-plan.md](test-plan.md) | deterministic tests with fake streams and paused time, plus the live probe check |

## Acceptance criteria

- [x] 1. The quality gate passes (`fmt`, `clippy -D warnings`, `test --workspace`) with no new `#[allow]`.
- [x] 2. Every test named in [test-plan.md](test-plan.md) exists under that name and passes. None of them touches a network.
- [ ] 3. `lib.rs` matches [files-to-touch.md](files-to-touch.md). No `kube`, `k8s_openapi`, or `kube::runtime` type appears in a public signature.
- [ ] 4. The read-only guard from 0001 AC4 still holds, extended to the new files, and `kube` features still exclude `ws`. — superseded by 0030 (write allow-list) and 0036 (kube now built with ws)
- [x] 5. The crate never calls `tokio::spawn`, `spawn_blocking`, or `block_on` (scoped grep of `crates/cluster/src`).
- [ ] 6. Live check (coder-lite): `probe -- --kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --watch-seconds 5` prints one line per kind. Expected: pods about 104, nodes 4, namespaces 20. No credential appears (0001 AC7 script).
- [x] 7. `crates/app` is unchanged.

## Decisions

1. Output is full snapshots, not diffs: about 100 pods at up to 10 Hz is trivial, and the UI just swaps rows.
2. Batching lives in the crate stream (a `sleep_until` inside the stream, testable with paused time). This supersedes the 0001 line "the app batches events".
3. Only summaries are kept, never raw objects. That is the memory bound, and no cap is needed for pods, nodes, or namespaces.

## Open items

1. Stall detection: a watch on a half-open connection can go silent, because the client has no read timeout. Revisit if users see frozen data, for example by restarting the watch when no event arrives within N minutes.
2. Watch health is event-driven. After a `Failed`, the UI's "interrupted" state clears on the next event, not on a silent resume ([watch-pipeline.md](watch-pipeline.md)).
