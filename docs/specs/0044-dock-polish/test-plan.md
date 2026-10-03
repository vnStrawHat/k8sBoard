# 0044 · Test plan

[Back to index](README.md). Offline and deterministic, one behavior per test. Pure helpers carry the logic and get inline unit tests. Tests that need a `LogTab` run in a headless window over the fake API server of the `app_shell_*_tests.rs` files (`go_live_for_test`, recorded requests); extend its answers with a fixed `pods/{name}/log` body if it has none. No test opens a real cluster connection.

## Step 1: height

| Test | File | Checks |
|---|---|---|
| `initial_dock_height_uses_a_valid_saved_height` | `dock.rs` | viewport 1000: `None` → 280; `Some(400.)` → 400; `Some(50.)` → 120; `Some(0.)`, `Some(-1.)`, `Some(f32::INFINITY)` → 280; `Some(900.)` → 600 (first-frame cap); viewport 300 → `Some(400.)` → 180 |
| `saved_dock_height_rounds_and_forgets_the_default` | `dock.rs` | 401.6 px → `Some(402.)`; 280 px → `None` |
| `max_line_offset_is_the_room_left_to_sixty_percent` | `dock.rs` | container 1000, dock 280 → 320; dock 600 → 0; container 0 → `None` |
| `settings_keys_are_the_allow_list` (extended) | `settings_tests.rs` | `dock.height` allowed |
| `dock_height_round_trips_and_is_omitted_when_unset` | `settings_tests.rs` | `Some(402.)` survives JSON; default settings have no `dock` key |
| `resize_end_saves_the_dock_height` | `app_shell_tests.rs` | dock with a tab; `resize_panel(1, 400)` → `AppSettings::get(cx).dock.height == Some(400.)` |
| `double_click_on_the_handle_resets_the_dock` | `app_shell_tests.rs` | dock at 400; a double click (`click_count` 2) on the handle position → `sizes().get(1)` = 280, `dock.height == None` |
| `dock_opens_at_the_saved_height` | `app_shell_tests.rs` | settings `Some(400.)`; open a tab; draw → `sizes().get(1)` = 400 |

## Step 2: markers

| Test | File | Checks |
|---|---|---|
| `markers_ignore_levels_and_the_text_filter` | `log_buffer_tests.rs` | ERROR hidden and filter `timeout`: the marker stays visible |
| `markers_keep_the_continuation_level` | `log_buffer_tests.rs` | ERROR line, marker, indented line → the indented line is ERROR |
| `restart_marker_names_reason_exit_and_count` | `log_workload.rs` | OOMKilled, 137, count 14 → `── container api terminated: OOMKilled (exit 137) · restart #14 ──`; no termination → `── container api restarted · restart #14 ──` |
| `rising_restarts_report_rises_only` | `log_workload.rs` | first sight → none; same → none; 3 → 5 → `("api", 5)`; 5 → 0 → none, baseline 0; an unstreamed container → none |
| `restart_marker_shows_after_the_stream_ended` | `log_tab_tests.rs` (fake API window) | pod tab; its stream ends (state `Ended`); the session pod then reports `restart_count` 14 with `OOMKilled`, exit 137 → one marker `── container api terminated: OOMKilled (exit 137) · restart #14 ──` |
| `volume_lines_skip_markers` | `log_buffer_tests.rs` | a buffer with one marker → `volume_lines` leaves it out |
| `export_writes_markers` | `log_buffer_tests.rs` | `visible_text` holds the marker text |

## Step 3: brush

| Test | File | Checks |
|---|---|---|
| `brush_window_covers_the_dragged_buckets_in_any_order` | `log_volume.rs` | 10 buckets of 5 s, 0.25 → 0.55 and 0.55 → 0.25 → buckets 2..=5 |
| `brush_window_clamps_to_the_chart` | `log_volume.rs` | -0.2 → 1.4 → the whole span |
| `brush_fraction_clamps_outside_the_chart` | `log_volume.rs` | x left and right of the bounds → 0.0 and 1.0; zero width → `None` |
| `release_outside_the_chart_ends_the_brush` | `log_tab_tests.rs` (fake API window) | press on the chart, move and release beyond its right edge → `brush` is `None`, the window reaches the last bucket |
| `window_span_places_the_shade` | `log_volume.rs` | the window of buckets 2..=5 → (0.2, 0.6); outside → `None` |
| `window_hides_lines_outside_and_without_a_timestamp` | `log_buffer_tests.rs` | inside shown; before, after, and untimed hidden |
| `volume_lines_ignore_the_window` | `log_buffer_tests.rs` | window set → `volume_lines` count = visible count without it |
| `restart_clears_the_window` | `log_tab_tests.rs` (fake API window) | window set; `restart_stream` → `buffer.view().window == None` |

## Step 4: pop out

| Test | File | Checks |
|---|---|---|
| `pop_out_moves_the_tab_and_keeps_its_streams` | `dock_tests.rs` (fake API window) | same `LogTab` entity id in the new window; dock tab count −1; the tab's stream count unchanged; `is_popped_out` |
| `pop_out_keeps_the_filter_text` | `log_tab_tests.rs` (fake API window) | filter `timeout` before → the new input holds `timeout`, view unchanged |
| `view_logs_on_a_popped_target_activates_its_window` | `dock_tests.rs` (fake API window) | `open` with the same target → no dock tab, window count unchanged |
| `cluster_switch_closes_every_pop_out` | `dock_tests.rs` (fake API window) | two pop-outs; `close_all` closes both windows and clears `popped` |
| `pop_out_window_is_titled_with_the_tab_label` | `dock_tests.rs` (fake API window) | the window title is the tab label, no cluster suffix |
| `closing_a_pop_out_drops_its_tab` | `dock_tests.rs` (fake API window) | window removed → the weak tab handle is dead |
| `leaving_work_ignores_popped_log_tabs` | `app_shell_tests.rs` | a popped log tab and no shell → `leaving_work` empty |
| `shell_tabs_have_no_pop_out` | `shell_tab_tests.rs` | the shell toolbar renders no `Pop out` button (element id absent) |

## Live checks (UAT, read-only) and ui-verifier

- Manual, release build, existing log reads only: drag the handle (line visible only while dragging), double-click (280 px), restart the app (height kept); zoom a workload tab, brush a range, clear it; pop out, type in its filter, close it.
- Trace (AC 3): `RUST_LOG=cluster=debug`; pop out a running tab → no new `sending request` line.
- ui-verifier: `--screen logs-zoomed` and `--screen logs-popout` against W8b (Pop out next to Export, histogram, no high-severity defect); the `logs-popout` run exits without a leak report (the pop-out closes before `cx.quit()`); 0003 color-literal grep.
