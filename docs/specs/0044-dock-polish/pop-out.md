# 0044 · Step 4: Pop out

[Back to index](README.md) · Modules: `log_window.rs` (new), `dock.rs`, `log_tab.rs`, `main.rs` (`mod`), `launch_options.rs`, `screenshot.rs`. Decisions 16–22. Wireframe: W8b header (`Export`, `Pop out`), notes 3–5.

## Scope

- **Log tabs** pop out. W8b puts `Pop out` beside `Export` in the log pane header.
- **Shell tabs stay in the dock** (decision 17). The W8b shell pane (note 5) has no Pop out, and a `ShellTab` holds window-bound state: a focus handle with `needs_focus`, a Find input subscribed with `subscribe_in(.., window, ..)`, and terminal metrics. Its session also counts in `leaving_work` and in the node-shell close guard (`main_window_may_close`); a second window would split that lifecycle.

## Button (`log_tab.rs`)

- In the toolbar after Export, both layouts: ghost small, `IconName::ExternalLink`, tooltip `Open in a new window`; the label `Pop out` in Full. Hidden once the tab is popped out (`is_popped_out`).
- Click → `cx.emit(LogTabEvent::PopOut)`; `impl EventEmitter<LogTabEvent> for LogTab`. The dock subscribes when it creates the tab in `Dock::open` (`cx.subscribe(&tab, Self::on_log_tab_event).detach()`). The event is delivered after the tab's update, so the dock may update the tab (a direct call from the click would borrow the tab twice).

## Dock (`dock.rs`)

```rust
struct PoppedTab { cluster: ClusterRef, tab: WeakEntity<LogTab>, window: WindowHandle<LogWindow> }
// `Dock` gains `popped: Vec<PoppedTab>`.
fn pop_out(&mut self, tab: &Entity<LogTab>, cx: &mut Context<Self>);
fn take_tab(&mut self, index: usize, cx: &mut Context<Self>) -> Option<DockTab>; // shared with `close_tab`
```

- `pop_out`: find the tab's index; `take_tab` (active index and the empty-dock mode rule of `close_tab`); `open_log_window(tab, title, cx)` with `tab_title(label, cluster_label, is_multi)`; push a `PoppedTab`. If the window cannot open, the tab goes back to its index, active, and a warning notice says `Could not open a new window`.
- `open`: before adding a tab, a live popped tab that `is_for(&origin.cluster, &target)` gets its window activated (`window.update(cx, |_, w, _| w.activate_window())`); no dock tab, no stream. A container pick follows the same rule as a dock tab (`pick_container` on an explicit choice).
- `close_all` and `close_tabs_of(cluster)` also close the matching pop-out windows (`window.update(cx, |_, w, _| w.remove_window())`) and drop their entries.
- Dead entries (windows the user closed) are dropped by `popped.retain(|p| p.tab.upgrade().is_some())` before each use.
- `has_tabs`, `shell_count_of`, `node_shell_count_of` read dock tabs only: a popped tab is not in the dock, so the dock hides when only pop-outs remain.

## Window (`log_window.rs`)

```rust
pub(crate) struct LogWindow { tab: Entity<LogTab>, title: SharedString }
/// Opens a window for `tab`; the tab moves its window-bound parts there and uses the Full layout.
pub(crate) fn open_log_window(tab: Entity<LogTab>, title: String, cx: &mut App)
    -> Option<WindowHandle<LogWindow>>;
```

- Like `open_settings_window`: `gpui_kit::open_window(WindowOptions { window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1000.), px(640.)), cx))), ..TitleBar::window_options() }, cx, |window, cx| ..)`. In the builder: `tab.update(cx, |tab, cx| { tab.move_to_window(window, cx); tab.set_layout(LogLayout::Full, cx) })`, then `cx.new(|_| LogWindow { .. })`. An open error is traced (`tracing::error!`) and returns `None`.
- Render: `TitleBar` with the title (mono, truncated), then the tab view `flex_1`, on `theme.background`.
- `LogTab::move_to_window(window, cx)`: the kit `InputState` ties focus, blur, and window activation to the window that created it (`gpui-base` `InputState::new_in_mode`), so the filter input is created again in the new window with the same text and placeholder, `_filter_events` is subscribed again, and the input is focused. `is_popped_out = true`. The buffer, streams, scroller, staging, export task, and brush are window-free and move as they are (decision 18).
- Closing (title bar ✕ or the OS): the window's root view drops; the dock holds only a weak handle, so the `LogTab` drops and its stream subscriptions abort (0004: dropping a tab ends its stream). No confirm: a log tab is a read and L opens it again.

## Lifecycle and `leaving_work`

| Event | Pop-out window |
|---|---|
| The user closes it | closes; its stream ends |
| L, View logs, or a container pick for the same target and cluster | activated; no new tab, no new stream |
| Cluster switch, view change, Remove from view (`release_all` → `close_all`; `close_tabs_of`) | closes with that cluster's dock log tabs; `LeavingWork` gains no field: log tabs never ask (0036 decision 37) |
| Main window closes | the app quits (`quit_when_main_window_closes`) and every window closes; the node-shell guard runs first, as today |
| Settings window, theme change, read-only lock | no effect beyond the shared theme |

`quit_when_main_window_closes` stays as is: closing a pop-out is not the main window and leaves windows open, so it never quits.

## Screenshot

- `LaunchScreen::LogsPopout` (`--screen logs-popout`, screenshot feature): as `logs-zoomed`, then the active tab pops out; `screenshot.rs` captures the pop-out window and waits until its tab is not connecting. Listed in `USAGE`.
