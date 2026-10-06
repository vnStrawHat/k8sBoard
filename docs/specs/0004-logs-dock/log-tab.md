# 0004 · Log tab: toolbar, states, rows, opening, async

[Back to index](README.md) · Modules: `log_tab.rs`, `resource_actions.rs`, `cluster_runtime.rs`

## Model

```rust
pub(crate) struct LogTarget { pub(crate) namespace: String, pub(crate) pod: String,
    pub(crate) containers: Vec<ContainerSummary>, pub(crate) initial_container: String }
impl LogTarget { pub(crate) fn of_pod(pod: &PodSummary) -> Option<Self>; } // None: no containers

pub(crate) struct LogTab {
    connection: ClusterConnection,
    target: LogTarget,
    container: String,
    instance: LogInstance,             // the Previous toggle
    shows_timestamps: bool,            // default true
    wraps_lines: bool,                 // default false
    buffer: LogBuffer,                 // log-buffer.md
    stream: LogStreamState,
    filter_input: Entity<InputState>,
    scroller: Entity<MessageScrollerState>,
    _stream: Option<WatchSubscription>,
    _filter_events: Subscription,
}
pub(crate) enum LogInstance { Current, Previous }
pub(crate) enum LogStreamState { Connecting, Streaming, Ended, Failed { message: String } }
```

- `initial_container` comes from the drawer's `default_container` rule (pod_drawer.rs, made `pub(crate)`): the first Main container that is not ready, else the first Main, else the first container.
- `LogStreamState` methods (tested): `start()` moves Connecting → Streaming; `fail(message)` → Failed; `close()` moves Connecting or Streaming → Ended, and Failed stays Failed.

## Starting a stream

`restart_stream()` does four things:

1. It replaces `_stream`, which aborts the old stream.
2. It clears the buffer and calls `scroller.reset(0)`.
3. It sets `stream = Connecting`.
4. It subscribes to `pod_logs` with the source from `instance`: `Current` gives follow plus a tail of 1,000 lines, and `Previous` reloads the previous instance.

Every start is fresh. There is no resume, because the timestamp de-duplication was cut on purpose (YAGNI).

| Trigger | Notes |
|---|---|
| New tab | `instance = Current` |
| A different container picked | keeps the Previous toggle |
| Previous toggled | flips `instance` |
| Reconnect | the same container and toggle. It recovers from Ended, Failed, and a silent half-open stall that still shows Streaming |

## Applying updates

| Item | Effect |
|---|---|
| `Started` | `stream.start()` |
| `Lines(v)` | `let change = buffer.push(v)`, `scroller.splice(0..change.removed_visible, 0)`, and `scroller.append(change.added_visible)` |
| `Failed(e)` | `stream.fail(error_text(&e))` |
| Stream closed (`on_closed`) | `stream.close()` |

## Toolbar (one row, `flex_wrap`, `px_3 py_1p5`, bottom border)

| Control | Kit | Behavior |
|---|---|---|
| Container | ghost small `Button`: the name plus a muted kind tag (`kind_tag_text`, made `pub(crate)`), with `ChevronDown`, and a `DropdownMenu` using `menu_with_check` per container (`{name} · {KIND}`) | picking another container sets `container` and calls `restart_stream()`. Disabled when there is a single container |
| Filter | `Input` on `InputState` (placeholder "Filter lines", cleanable), `flex_1`, `min_w(160px)`, `max_w(360px)` | on `InputEvent::Change`: `buffer.set_filter(value)`, then `scroller.reset(buffer.visible_len())` |
| Previous | `Toggle` | flips `instance` and calls `restart_stream()` |
| Timestamps | `Toggle`, on by default; tooltip "Kubelet time, shown in your local time zone ({zone})" | `scroller.remeasure()` |
| Wrap | `Toggle`, off by default | `scroller.remeasure()` |
| Copy | ghost icon `Button` (`Copy`); tooltip "Copy visible lines" | `cx.write_to_clipboard(buffer.visible_text(shows_timestamps))` |
| Status | muted `text_xs`, pushed right | table below |
| Reconnect | small ghost `Button` (`RefreshCw`, tooltip "Reconnect"). Shown in every state except Connecting | `restart_stream()` |

Copy, Export, Pop out, and Reconnect (the last two when they apply) are one button each in the zoomed and popped-out tabs. In the docked (Compact) tab they sit in one `⋯` menu so the controls stay on one row; the status text wraps below only when the row has no room left (`toolbar_actions` in `log_tab.rs`).

The level toggles (ERROR, WARN, INFO, DEBUG) take Alt-click: only that level stays on, and a second Alt-click on it shows all four again (`LevelSet::toggled_only`). The Since picker (`tail`, `5m`, `15m`, `1h`, `6h`; `log_since.rs`) sits in the toolbar of the zoomed and popped-out tabs and at the end of the `⋯` menu of the docked tab. Picking a window restarts the streams with `since_seconds` and no tail; the status text then ends with `· last {window}`. The status text is not in the toolbar: the dock shows it in its tab strip beside the zoom buttons (`Dock::render_tab_bar`), and a popped-out tab, which has no strip, keeps it in its toolbar. The zoomed histogram draws the start of its first bucket and the end of its last one as two small time labels under it.

A click on a row selects it (the `selection` theme token as its background), a Shift-click extends the range from the last plain click, and the tab takes the focus (`log_selection.rs`). While rows are selected the tab's key context carries `LogSelection`: Ctrl C (`CopyLogLines`) copies the selected rows as `Copy visible lines` writes them (clock time when Timestamps is on, the pod column of a workload tab), and Esc (`ClearLogSelection`) clears the selection. Without a selection both keys keep their workspace meaning. A filter, level, or window change, or a restart, clears the selection; evicted rows shift it.

| State | Status text (`{count}` = `N lines`, or `V of N lines` when filtering; then `· older lines dropped` if any were dropped) |
|---|---|
| Connecting | `Opening…` |
| Streaming | `Streaming · {count}`, or `Paused · {count}` while `scroller.is_scrolled_up()` |
| Ended | `Stream ended · {count}` (Current) / `Previous instance · {count}` |
| Failed | `Failed · {count}` |

## Body

| Condition (first match) | Shows |
|---|---|
| Failed | an error `Alert` (`message`) above the list. Lines already received stay visible below it |
| Connecting with an empty buffer | spinner + `Opening logs of {pod}/{container}…` |
| No lines | muted `No log lines yet`, or `The container wrote no log lines` when Ended |
| Filter with 0 matches | muted `No lines match "{needle}"` |
| Otherwise | `MessageScroller` (below) |

## Rows (`MessageScroller`)

- `gpui_kit::component::message_scroller::MessageScroller::new("log-lines", scroller, row)`, with the scrollbar on, the jump button labeled "Jump to latest", and `with_bottom_fade(theme.background)`. The row style overrides the kit's default 32 px row gap with `pb_0` and `px_3`. The list style is `py_1`.
- `row` captures `cx.entity().downgrade()` and reads `buffer.visible_line(index)`. When the line is missing it renders an empty `div`.
- A row is an `h_flex().gap_2()` in the mono font at `text_xs`:
  - the timestamp: `format_log_time`, muted, `flex_shrink_0`, a fixed width of 13 `ch`. It is blank when `timestamp` is `None`, and absent when Timestamps is off;
  - the text: a `StyledText` whose `find_matches` ranges get a `theme.selection` background. When Wrap is off it is `whitespace_nowrap` and `truncate`; when Wrap is on it wraps. Highlights and times are computed per rendered row, so only visible rows pay for them.

## Opening from menus (`resource_actions.rs`)

- Signature: `pod_menu(menu, pod, live: &LiveCluster, dock: &WeakEntity<LogDock>)`. `PodTableDelegate` holds the `WeakEntity<LogDock>`, and `pod_drawer` gets it from `AppShell`.
- `ViewLogs` availability:
  - `GetPodLogs` allowed: `Enabled`. `LOGS_REASON` is deleted;
  - denied: "Not permitted: get pods/log";
  - Checking or Unknown: unchanged.
- When enabled, the item's click handler runs `LogTarget::of_pod(pod)` and then `dock.update(cx, |dock, cx| dock.open(live.connection().clone(), target, window, cx))`. `LiveCluster::connection()` is a new `pub(crate)` getter.

## Async wiring (amends 0002 app-subscription)

```rust
pub(crate) fn subscribe<V: 'static, U: Send + 'static>(&self,
    updates: impl Stream<Item = U> + Send + 'static, cx: &mut Context<V>,
    apply: impl Fn(&mut V, U, &mut Context<V>) + 'static,
    on_closed: impl FnOnce(&mut V, &mut Context<V>) + 'static) -> WatchSubscription;
```

Only the item type changes, from `WatchUpdate<T>` to `U`. The callers in `cluster_session.rs` need no change.

| Concern | Rule |
|---|---|
| Notify rate | one `notify` per item. `Lines` arrive in 100 ms batches |
| Back-pressure | channel capacity 1. A slow UI stops polling the HTTP body |
| Cancel | closing a tab, `close_all`, a restart, or a session drop aborts the `RuntimeTask`, which drops the stream and closes the HTTP connection |
| Main thread | `push` costs O(batch + evicted). A filter edit rescans the buffer (at most 10,000 lines) |
