# 0019 · Steps 2a/2b: streams, merge, status, legend

[Back to index](README.md) · Modules: `crates/cluster/src/pod_log.rs` (one field), `log_tab.rs`. Targets and membership: [workload-targets.md](workload-targets.md).

## Cluster crate exception (step 2a)

```rust
pub struct LogRequest { pub namespace: String, pub pod: String, pub container: String,
    pub source: LogSource, /// Lines of history requested at open.
    pub tail_lines: u32 }
```

- `log_params`: `tail_lines: Some(i64::from(request.tail_lines))`; the private `LOG_TAIL_LINES` constant is removed. Nothing else in the crate changes.
- Callers: app `POD_TAIL_LINES: u32 = 1000` (pod tabs and initial workload members), `LATE_JOIN_TAIL_LINES: u32 = 50` (members joining after the merge flush); `examples/probe.rs` passes 1000.

## Tab state (`log_tab.rs`)

```rust
enum LogSubject { Pod(PodTarget),
    Workload { target: WorkloadTarget, session: WeakEntity<ClusterSession>, members: Vec<String>,
               offered: Vec<String>, selected: Vec<String>, is_frozen: bool, _pods_observer: Subscription } }
struct TabStream { pod: String, container: String, prefix: SharedString /* short, on screen */,
    full_prefix: SharedString /* `{pod}/{container}`, Export */, color_slot: usize,
    is_member: bool, state: LogStreamState, _stream: Option<WatchSubscription>, _grace: Option<Task<()>> }
struct Staging { lines: Vec<SourcedLine>, bytes: usize, _timer: Task<()> }
// LogTab gains: subject, streams: Vec<TabStream> (index = SourceId), staging: Option<Staging>,
//               pod_slots: Vec<String> (color slot per pod name, join order)
const MERGE_WINDOW: Duration = Duration::from_secs(2);
const LEAVE_GRACE: Duration = Duration::from_secs(10);
```

- A pod tab has exactly one `TabStream` (SourceId 0) and no prefix column; Previous, picker, and status behave as in 0004.
- `restart_stream` (Reconnect, Previous, container change) drops every stream, clears buffer, streams, slots, and staging, re-runs `sync_members` from an empty member list, and arms `Staging`.

## Membership sync (workload only)

`sync_members(&mut self, cx)` runs in `new`, from the session observer, and after any stream closes:

1. Session gone, not live, or `pods.ready_items()` is `None` → return (frozen).
2. `!scope_covers(&live.scope, namespace)` → `is_frozen = true`, return; else `is_frozen = false`.
3. `ranked_pods` → (2b) refresh `offered`; on the first sync `selected` = the first member's `default_container` (2a uses only this) → `member_change(members, ranked, join_slots(members.len(), live_streams, selected.len()))`.
4. `left`: each stream of the pod → `is_member = false`, arm `_grace` (`LEAVE_GRACE` timer, then `_stream = None`, state `Ended`). The server usually closes it first, delivering the final lines.
5. `joined` (**rejoin rule**): first drop any existing streams of that pod name (`_stream = None`, state `Ended`; their buffered lines stay), then reuse the name's color slot from `pod_slots` (or append a new one), then start one stream per selected container present in the pod. Tail: `POD_TAIL_LINES` while `staging` is `Some`, else `LATE_JOIN_TAIL_LINES`.
6. Notify only when something changed.

Pods not admitted for lack of slots wait; the next sync (any pod update or any stream close) retries.

## Streams and merge

- Start: `connection.pod_logs(LogRequest { …, tail_lines })` → `runtime.subscribe(updates, cx, move |tab, u, cx| tab.apply_update(id, u, cx), move |tab, cx| tab.close_stream(id, cx))`; `id: SourceId` is captured by copy. `close_stream` marks the state, clears `_stream`, and calls `sync_members`.
- `Lines(v)` → `SourcedLine`s. While `staging` is `Some`: append; flush early when `bytes > 8 MiB`. Else `buffer.push` + scroller splice/append (0004).
- Timer: `cx.spawn(async move |tab, cx| { cx.background_executor().timer(MERGE_WINDOW).await; tab.update(cx, |tab, cx| tab.flush_staging(cx)) })`.
- `flush_staging`: `sort_staged` (stable by `line.timestamp`, `None` first) → one `push`. Later joiners request 50 lines and append on arrival.
- Pod tabs never stage.

## Status, tone

| Streams | Tab dot | Status text (`{count}` as 0004) |
|---|---|---|
| none (no members yet) | Info | `Waiting for pods of {label}` (also the body text) |
| some Connecting, none Streaming | Info | `Opening {n} streams…` |
| any Streaming | Ok | `Streaming · {p} pods · {count}` (`Paused · …` when scrolled up) |
| all Failed | Bad | `Failed · {count}`, Alert with the first failure message |
| otherwise (all Ended or mixed Ended/Failed) | Done | `Streams ended · {count}` |

Pure `fn workload_tone(states: impl Iterator<Item = &LogStreamState>) -> StatusTone`. Tab label = `{label}`.

## Legend and container picker (step 2b)

- Legend row above the toolbar (step 3 limits it to the Full layout): one chip **per pod name** with a color dot (`[chart_1..chart_5][slot % 5]`), `short`, and a status dot; a failed stream → tooltip with the error text; all of the name's streams non-member → muted `deleted`. `is_frozen` → muted note `Pod changes are not followed outside the namespace filter`.
- Picker: ghost button `Containers: api, worker ▾`, `menu_with_check` per offered name; toggling restarts all streams; the last checked name cannot be unchecked.

## Async contract

| Concern | Rule |
|---|---|
| Thread | membership and buffer work on the main thread; HTTP on the tokio runtime (0004) |
| Stream budget | every live `TabStream` (member or leaver in grace) counts against `MAX_WORKLOAD_STREAMS` (20); joins wait for a free slot |
| Leavers | keep streaming up to `LEAVE_GRACE` (10 s), then are dropped; a close before that frees the slot at once |
| Notify | one per `Lines` batch per stream; observer syncs are O(pods) |
| Cancel | closing the tab drops streams, grace timers, observer, and the merge timer; `close_all` on context switch (0004) |
| Back-pressure | channel capacity 1 per stream (0004); staging bounded by 8 MiB |
| Ordering | exact only for lines staged in the 2 s window; afterwards arrival order (≤ ~100 ms batch skew), and kubelet timestamps come from each node's clock, so cross-node skew also shows (README open item 1) |
