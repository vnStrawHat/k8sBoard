# 0002 · Test plan

[Back to index](README.md)

Rules: no network, no cluster, no real `watcher`. The tests drive `batch_updates` with fake sources:

- `futures::stream::iter(..)` for scripted sequences;
- `futures::channel::mpsc::unbounded()` when timing between items matters, or when the source must stay open.

Time is `#[tokio::test(start_paused = true)]`, so `BATCH_WINDOW` elapses instantly and deterministically. Fixtures are k8s-openapi `Pod` objects from a local `pod(namespace, name)` builder. The 0001 pod fixture helpers are private to `pod_tests.rs`. Errors are built as `watcher::Error::WatchError(Box::new(Status { code, .. }))` and `watcher::Error::InitialListFailed(kube::Error::Api(..))`.

## `resource_watch_tests.rs`: store (sync `#[test]`)

| Test | Verifies |
|---|---|
| `store_snapshot_is_ordered_by_namespace_then_name` | key order matches `list_pods` |
| `store_apply_of_equal_summary_reports_unchanged` | dirty only on a real change |
| `store_delete_of_missing_key_reports_unchanged` | |
| `store_keeps_old_items_visible_during_init` | `begin_init` plus `init_apply` do not alter `snapshot()` |
| `store_finish_init_replaces_items_and_drops_vanished` | relist removes objects absent from the new list |

## `resource_watch_tests.rs`: batching (`start_paused`)

| Test | Verifies |
|---|---|
| `no_snapshot_before_init_done` | `Init` plus `InitApply`s, then the source stays open: nothing is emitted |
| `init_done_emits_one_snapshot_with_all_items` | content and order |
| `empty_initial_list_emits_empty_snapshot` | the UI can show an "empty" state |
| `burst_within_window_emits_one_snapshot` | 50 `Apply`s with no time gap produce one `Snapshot` holding the final state |
| `changes_in_separate_windows_emit_separate_snapshots` | advance 150 ms between applies |
| `unchanged_apply_emits_nothing` | equal summary |
| `delete_event_emits_snapshot` | `Delete` of a synced pod goes through `batch_updates` to a `Snapshot` without it |
| `gone_410_is_silent_and_relist_replaces_items` | `WatchError(410)`, then `Init…InitDone` without one pod: no `Failed`, and the snapshot lacks that pod |
| `failure_flushes_pending_snapshot_before_failed` | after sync: `Snapshot`, then `Failed` |
| `failure_during_initial_list_emits_failed_without_snapshot` | `Init`, `InitApply`, then a 403: only `Failed` is emitted, no partial snapshot; a later `InitDone` then emits the full snapshot |
| `forbidden_maps_to_cluster_error_forbidden` | a 403 `WatchError` becomes `Failed(ClusterError::Forbidden { action: "watching pods", .. })` |
| `event_after_failure_reemits_snapshot_even_if_unchanged` | recovery clears the UI error |
| `retry_failure_after_sync_emits_failed_without_snapshot` | after sync: failure, then `Init` (retry), then another failure. Only `Failed`, `Failed` is emitted, and the `Init` stages but emits no snapshot |
| `source_end_flushes_pending_changes` | |
| `dropping_stream_drops_source` | with an mpsc fake, `sender.is_closed()` is true after the watch stream is dropped |

## `resource_watch_tests.rs`: `watch_error` (sync)

| Test | Verifies |
|---|---|
| `watch_error_watch_410_is_none` | `WatchError(410)` |
| `watch_error_initial_list_410_is_none` | `InitialListFailed(Api 410)`, an expired continue token |
| `watch_error_maps_watch_start_and_watch_failed` | `WatchStartFailed` and `WatchFailed` (non-410) are classified with `classify_error`, e.g. a 403 becomes `Forbidden` |
| `watch_error_no_resource_version_is_unexpected_response` | |

## Live check (coder-lite, not part of `cargo test`)

```bash
cargo run -p k8sboard-cluster --example probe -- --kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --watch-seconds 5
```

Report the three `watch …` lines verbatim, then run the 0001 AC7 credential-leak script over the output.

- Expect about 104 pods, 4 nodes, and 20 namespaces.
- `watch nodes` and `watch namespaces` were not probed by 0001. If either fails with `Forbidden`, report it verbatim.
