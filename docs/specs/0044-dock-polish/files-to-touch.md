# 0044 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own. All in `crates/app/src`; `crates/cluster` untouched; `Cargo.lock` unchanged (`jiff`, `serde`, gpui-kit are present).

## Source

| S | File | Change |
|---|---|---|
| 1 | `settings.rs` (+ `settings_tests.rs`) | `DockSettings { height }`, `Settings.dock`, allow-list entry `dock.height` |
| 1 | `dock.rs` | `initial_dock_height`, `saved_dock_height`, `max_line_offset`; inline tests |
| 1 | `workspace.rs` | `dock_handle_appearance` (kit line, double-click hit area, dashed max line); panel initial size from settings |
| 1 | `app_shell.rs` (+ `app_shell_tests.rs`) | `_dock_split_events` subscription, `save_dock_height`; window tests |
| 2 | `log_buffer.rs` (+ `log_buffer_tests.rs`) | `LineKind`; marker rules in `push`, `level_of`, `LineView::shows` |
| 2 | `log_workload.rs` | `restart_marker`, `membership_markers`, `rising_restarts`; tests |
| 2 | `log_tab.rs` | `restart_seen`, `has_synced`, pod-tab session observer, `note_restarts`, markers in `sync_members`, markers through staging; volume input skips markers |
| 2 | `log_rows.rs` | `SYS` tag and muted text for markers |
| 3 | `log_buffer.rs` | `TimeWindow`, `LineView.window`, `volume_lines` |
| 3 | `log_volume.rs` | `brush_window`, `window_span`, shade, chip, mouse handlers, bounds `canvas`; tests |
| 3 | `log_tab.rs` (+ `log_tab_tests.rs`, new) | brush drag state, bounds cell, `current_volume` from `volume_lines`, clear on `restart_stream`; fake-API window tests |
| 4 | `log_window.rs` (new), `main.rs` | `LogWindow`, `open_log_window`; `mod log_window;` |
| 4 | `dock.rs` (+ `dock_tests.rs`, new) | `PoppedTab`, `popped`, `take_tab`, `pop_out`, `on_log_tab_event`, `open` activation, `close_all` / `close_tabs_of` closing windows; fake-API window tests (the inline `mod tests` keeps the pure ones) |
| 4 | `log_tab.rs` | `LogTabEvent`, Pop out button, `move_to_window`, `is_popped_out` |
| 4 | `shell_tab_tests.rs`, `app_shell_tests.rs` | no Pop out on shells; `leaving_work` unchanged |
| 4 | `launch_options.rs` (+ tests), `screenshot.rs` | `LaunchScreen::LogsPopout`, `USAGE`, capture of the pop-out window |

## Docs (step 4)

| File | Change |
|---|---|
| `docs/specs/0024-settings-store/persisted-prefs.md` | reserved `dock.height`: owner 0044 |
| `docs/specs/0019-workload-logs/decisions.md` | decisions 21 and 25: superseded by 0044 (brush, Pop out for log tabs) |
| `docs/specs/0004-logs-dock/dock-layout.md` | the "double-click reset is dropped" bullet: done by 0044 through a handle renderer |
| `docs/roadmap/wireframe-gap-audit.md` | W8 and W8b rows → built (0044) |
| `docs/roadmap/README.md`, `docs/roadmap/inventory-shell.md` | dock row and K3: Pop out, SYS lines, brush done; Shell pop out not planned |
