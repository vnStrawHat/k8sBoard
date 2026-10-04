# 0044 · As built

[Back to index](README.md). Built on main `b52d77e` (one cluster at a time, 0046) in four steps, each with the full gate. Local-only: `crates/cluster` and `Cargo.lock` are untouched, and no `ClusterConnection` call site was added.

## Deviations from the spec

| # | Spec | As built | Why |
|---|---|---|---|
| 1 | Max line is `border_t_1().border_dashed()` | `dashed_rule` in `workspace.rs` paints 6 px dashes with 4 px gaps through a `canvas` (`paint_quad`), in `theme.muted_foreground` | a dashed border on a one-pixel-high box paints nothing in the gpui Windows renderer (checked in a real window: the label showed, the line did not) |
| 2 | `pop_out(&mut self, tab, cx)` | also takes `window: &mut Window` | the "Could not open a new window" notice needs the main window; `on_log_tab_event` is a `subscribe_in` handler, which has it |
| 3 | `volume_lines` returns `impl Iterator` | returns `Box<dyn Iterator>` | two paths (the visible index without a window, a scan with one) |
| 4 | `TabStream` as before | gains `namespace` and `container` | the restart walk needs each stream's (namespace, pod, container); a node workload spans namespaces |
| 5 | `volume_chart` takes the window, drag, and handlers | takes one `BrushView { window, drag, bounds, handlers: BrushHandlers }`; handlers get a fraction of the chart width and are `FractionHandler`s | keeps `log_volume.rs` free of `LogTab` (no module cycle) |
| 6 | the filter input is created again with the same text | `InputState::default_value(text)` | `set_value` after creation would be silent too, but the builder is the plain way; the view carries over without a recompute |
| 7 | Test fixture inside the test files | `log_fixtures.rs` (`#[cfg(test)]` in `main.rs`): a headless window, a live session over a closed port, and a `FakeApi` that answers the log reads | `log_tab_tests.rs` and `dock_tests.rs` share it, and the `app_shell_*` fixtures are private to `app_shell` |

## Facts a later change needs

- `LogWindow` owns the tab; the dock holds `PoppedTab { WeakEntity<LogTab>, AnyWindowHandle }`. A tab must leave the dock's element tree before its window draws: the same entity rendered in two windows hangs the test executor.
- A release closer than 3 px to the press is a click and clears the window (`MIN_BRUSH_DRAG_PX`). The pointer maps onto the chart through the bounds a `canvas` stores at prepaint; the chart cell has the debug selector `log-volume-brush`.
- `note_restarts` runs on every session notification (pod tabs observe the session like workload tabs) and at the end of `sync_members`; the first sight of a (pod, container) only records.
- At 1320 px the zoomed toolbar wraps: `Pop out` and Reconnect land on a second row. It does not overlap anything.

## Checks

- Gate after each step: `cargo fmt --all -- --check`, `clippy --workspace --all-targets -D warnings`, `clippy -p k8sboard --all-targets --features screenshot -D warnings`, `cargo test --workspace` (final: 4,578 tests pass, 2 ignored).
- Real-window shots (UAT, existing log reads only), light and dark, `.tmp/ui-shots/v95-*`: `logs-zoomed`, `logs-popout`, the dashed line mid-drag, a `SYS` row (a sample line injected by a throwaway local patch, since no UAT pod restarts on demand), the brush after a drag and during one, and a dock restored from a seeded `dock.height` of 400. Trace of `--screen logs-popout`: one `sending request … streaming pod logs`, none after the pop out.
- Not run: the ui-verifier agent (AC 12 stays open); a release-build manual pass of the double-click reset in a real window (covered by `double_click_on_the_handle_resets_the_dock`); a test of "closing the main window closes the pop-outs" (it rests on the existing `quit_when_main_window_closes` hook, which quits the app).
