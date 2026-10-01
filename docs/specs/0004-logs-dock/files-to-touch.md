# 0004 · Files to touch

[Back to index](README.md)

## Cargo

There are no Cargo changes. `log_stream` needs only the kube `client` feature. `futures` (`AsyncBufReadExt`, `stream::unfold`), `tokio` (`time`, `sync`), and `jiff` are already dependencies of both crates. `ws` stays off.

## `crates/cluster`

| File | Change |
|---|---|
| `src/pod_log.rs` (new) | `LogRequest`, `LogSource`, `LogLine`, `LogUpdate`, `ClusterConnection::pod_logs`; private `log_params`, `log_updates`, `LineSplitter`, `parse_log_line`, `LOG_TAIL_LINES`, `MAX_LINE_BYTES` |
| `src/pod_log_tests.rs` (new) | tests ([test-plan.md](test-plan.md)), `#[path]` sibling as in 0001 |
| `src/resource_watch.rs` | `BATCH_WINDOW` becomes `pub(crate)`. Nothing else changes |
| `src/lib.rs` | `mod pod_log;` and `pub use pod_log::{LogLine, LogRequest, LogSource, LogUpdate};` |
| `examples/probe.rs` | new `--logs-seconds <n>` flag (below) |

### Probe flag `--logs-seconds <n>`

- The value must be a positive integer. Anything else prints usage and exits 2. The section runs after all other sections, and only when the flag is given.
- **Pod choice:** use the result of `list_pods(scope)` and take the first pod that has a `Main` container in `ContainerState::Running`. Stream that container with `LogSource::Current`, stopping after `n` seconds (`tokio::time::timeout` around the loop).
- **Output:** one line, counts only. Log text is never printed:
  ```text
  logs payments/api-7d9f8c-x2k4q/api: started, 1000 lines in 3 batches, ended: no, 0 failures
  logs …: not started, 0 lines in 0 batches, ended: no, 1 failures; last error: <ClusterError Display>
  logs: no running pod in scope
  ```
- Exit 1 only when no pod was found, or when neither `Started` nor `Failed` arrived.
- Update the probe's doc comment and `USAGE`. Add one table row to [0001 probe-example.md](../0001-cluster-read-only/probe-example.md).

## `crates/app`

| File | Change |
|---|---|
| `src/log_buffer.rs` (new) | `LogBuffer`, `BufferChange`, `find_matches`, `format_log_time` |
| `src/log_buffer_tests.rs` (new) | buffer tests |
| `src/log_tab.rs` (new) | `LogTarget`, `LogTab` (Render), `LogInstance`, `LogStreamState`, toolbar, rows. Inline `mod tests` |
| `src/log_dock.rs` (new) | `LogDock` (Render), `DockMode`, `active_after_close`, `dock_max_height` and the height constants. Inline `mod tests` |
| `src/cluster_runtime.rs` | `subscribe` generic over the item type `U` ([log-tab.md](log-tab.md)) |
| `src/cluster_session.rs` | `LiveCluster::connection(&self) -> &ClusterConnection` (`pub(crate)`) |
| `src/app_shell.rs` | `log_dock` and `dock_split` fields, plus an observer on `log_dock`; `close_all` in `start_session`; `unzoom` in `show_screen`; `log_dock` passed to the pod delegate and the drawer; pending launch screen extended to logs screens (below) |
| `src/workspace.rs` | the layout per dock mode with the kit `v_resizable` split; the drawer moves into `upper` ([dock-layout.md](dock-layout.md)) |
| `src/resource_actions.rs` | `ViewLogs` enabled when allowed; `LOGS_REASON` removed; `pod_menu` takes `live` and the dock and wires the click |
| `src/resource_actions_tests.rs` | update the logs tests ([test-plan.md](test-plan.md)) |
| `src/pod_table.rs` | the delegate holds a `WeakEntity<LogDock>` and passes it to `pod_menu` |
| `src/pod_drawer.rs` | `default_container` and `kind_tag_text` become `pub(crate)` (reused by the log tab); `pod_menu_button` takes the dock |
| `src/launch_options.rs` | `LaunchScreen::LogsDock` (`logs-dock`) and `LogsZoomed` (`logs-zoomed`); `USAGE` updated |
| `src/screenshot.rs` | `pick_logs_pod` and the settle rule for logs screens (below) |
| `src/main.rs` | `mod log_buffer; mod log_dock; mod log_tab;` |

All new items are private or `pub(crate)`. The app still has no `kube` or `k8s-openapi` dependency.

## Screenshot screens (extends [0003 screenshot-hook](../0003-app-shell-pods-nodes/screenshot-hook.md))

| Screen | Setup, once pods are `Ready` |
|---|---|
| `logs-dock` | Pods screen, no selection. Open the dock on `pick_logs_pod`: the first pod whose default container is Running (a running container usually has log history), else `pick_drawer_pod`. Mode Normal, default height |
| `logs-zoomed` | as `logs-dock`, then `DockMode::Zoomed` |

- `LaunchScreen::screen()` maps both screens to `Screen::Pods`. `has_drawer()` is false for both. A new `has_log_dock()` is true for both.
- `apply_pending_drawer_screen` becomes `apply_pending_launch_screen` and also handles the logs screens. It opens the dock directly, without the RBAC gate: on a denied cluster the 403 error state is what gets captured.
- `SettleInput` gains `is_log_pending: bool`, which is true only while the active tab is `Connecting`. Streaming settles even with 0 lines, so a quiet container still produces a capture with exit 0; the ui-verifier then sees the "No log lines yet" state. The 0003 post-settle wait (300 ms plus one 100 ms tick) covers the first 100 ms batch of a busy container. `is_screen_settled` requires `!is_log_pending` for logs screens.
- Captured PNGs contain log text, so they stay in `.tmp/ui-shots/`, which is git-ignored. Never attach them to commits or issues.

## Docs

- 0001 `probe-example.md`: one row for `--logs-seconds`.
- 0003 `screenshot-hook.md`: add `logs-dock|logs-zoomed` to the `--screen` list in "Flags".
