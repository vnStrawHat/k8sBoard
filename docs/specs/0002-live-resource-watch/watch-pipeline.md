# 0002 · Watch pipeline: store, batching, errors

[Back to index](README.md) · Module: `src/resource_watch.rs` (tests in `src/resource_watch_tests.rs`)

## Shape

```text
kube::runtime::watcher(api, watcher::Config::default())   // initial list (pages of 500) + watch + 410 relist
  .default_backoff()                                      // kube StreamBackoff: exponential, jittered, capped
  -> batch_updates(events, context, action, summarize)    // SummaryStore + 100 ms batching
  -> impl Stream<Item = WatchUpdate<T>>
```

```rust
const BATCH_WINDOW: Duration = Duration::from_millis(100);

pub(crate) fn summary_watch<K, T>(connection: &ClusterConnection, api: Api<K>,
    action: &'static str, summarize: fn(&K) -> T,
) -> impl Stream<Item = WatchUpdate<T>> + Send + 'static
where K: kube::Resource + Clone + DeserializeOwned + Debug + Send + 'static,
      T: Clone + PartialEq + Send + 'static;

/// The testable core: any source of watcher events works, including fakes.
fn batch_updates<K, T, S>(events: S, context: String, action: &'static str,
    summarize: fn(&K) -> T,
) -> impl Stream<Item = WatchUpdate<T>> + Send + 'static
where S: Stream<Item = Result<watcher::Event<K>, watcher::Error>> + Send + 'static, /* K, T as above */;
```

## `SummaryStore<T>` (private, synchronous, pure)

```rust
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ObjectKey { namespace: String, name: String } // namespace "" when cluster-scoped; from ResourceExt
struct SummaryStore<T> { items: BTreeMap<ObjectKey, T>, pending_init: Option<BTreeMap<ObjectKey, T>> }
```

| watcher event | store call | returns "changed" |
|---|---|---|
| `Init` | `begin_init()`: new empty buffer; `items` stay visible | false |
| `InitApply(k)` | `init_apply(key, summarize(&k))` into the buffer | false |
| `InitDone` | `finish_init()`: the buffer replaces `items` atomically | true |
| `Apply(k)` | `apply(key, summary)` | true only if absent or `!=` the stored value |
| `Delete(k)` | `delete(&key)` | true only if it was present |

- The relist swap drops objects that vanished while disconnected, so no stale rows are left. There is no flicker, because the old `items` serve until `InitDone`.
- `snapshot()` clones the values into a `Vec<T>` in key order.

## Batching algorithm (`batch_updates`)

State: `store`, `is_synced` (the first `InitDone` was seen), `is_dirty`, `is_recovering`, `deadline: Option<tokio::time::Instant>`.

The timer is a `tokio::time::sleep_until(deadline)` polled inside the stream, never a spawned task. It needs tokio `time` and a tokio-driven poller (the app pump, or `#[tokio::test]`).

1. `tokio::select!` on `events.next()` and the sleep, the latter only when a deadline is set. Both are cancel-safe.
2. On `Ok(event)`, update the store. Only `InitDone`, `Apply`, and `Delete` can mark dirty or clear recovery. If such an event changed the store, or is one of these while `is_recovering` is set, set `is_dirty` and clear `is_recovering`. `Init` and `InitApply` only stage the relist buffer and never touch `is_dirty` or `is_recovering`. kube-runtime 4.2 emits `Init` before every list attempt, including backoff retries after a failure, so treating it as a change would make the UI "interrupted" state flap with a stale snapshot. If `is_synced && is_dirty` and there is no deadline, set `deadline = now + BATCH_WINDOW`.
3. When the deadline fires, emit `Snapshot(store.snapshot())`, clear `is_dirty`, and clear the deadline.
4. On `Err(error)`, map it with `watch_error` (below):
   - `None` (expected 410): continue silently.
   - `Some(error)`: flush a pending snapshot only when `is_synced && is_dirty`. A partial initial list is never emitted. Then emit `Failed(error)`, set `is_recovering`, and clear the deadline.
5. When the source ends (fakes only; a real watcher never ends), flush only if `is_synced && is_dirty`, then end.

No snapshot is emitted before the first `InitDone`, so the UI shows "loading" rather than partial data.

## Errors (the 410 mapping is required)

On a 410 Gone, kube's watcher yields an `Err` and resets itself to relist. The 410 can come as `WatchError(status)` during the watch, or as `InitialListFailed(Api 410)` when a list continue token expires. Both must be silent.

```rust
/// None for an expected 410: kube's watcher relists by itself.
fn watch_error(context: &str, action: &'static str, error: watcher::Error) -> Option<ClusterError>;
```

| `watcher::Error` | Result |
|---|---|
| `InitialListFailed(e)`, `WatchStartFailed(e)`, `WatchFailed(e)` | `classify_error(context, action, e)` |
| `WatchError(status)` | `classify_error(context, action, kube::Error::Api(status))` |
| `NoResourceVersion` | `UnexpectedResponse { source: Box::new(error) }` |
| any result equal to `ClusterError::Api { code: 410, .. }` | `None` (logged at `debug`, context and action only) |

`Auth` stays `CredentialsUnavailable` with no detail, as in 0001. Nothing logs objects or headers.

## Backoff, cancellation, memory

- **Backoff:** kube `default_backoff()` (starts near 800 ms, doubles, capped at 30 s, jittered, resets after a healthy period). There is no custom policy. Forbidden or Unauthorized also retry, at the capped rate, so a granted permission is picked up without a restart.
- **Cancellation:** dropping the stream drops the watcher and its HTTP connection. There is nothing else to stop, because the crate owns no tasks. The app aborts its pump task ([app-subscription.md](app-subscription.md)).
- **Back-pressure:** if the UI stalls long enough, the pump stops polling. The server may then expire the resource version, and the resume gets a 410. That is handled: a silent relist, then a fresh snapshot.
- **Memory per watch:**
  - one `BTreeMap` of summaries, plus a transient relist buffer;
  - one raw list page (500 objects) during the initial list;
  - no raw objects or `managedFields` are retained.
- `watcher::Config::default()` keeps `ListSemantic::MostRecent` and paging. Streaming lists are beta on v1.29 and are not used.
