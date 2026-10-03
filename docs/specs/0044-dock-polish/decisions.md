# 0044 · Decisions

[Back to index](README.md). Architect defaults; the user confirms. All local-only: no new Kubernetes call.

## Height (step 1)

| # | Decision | Rationale |
|---|---|---|
| 1 | One `dock.height` for the main window, in whole pixels; a reset stores `None` | W8 note 1 says per window: the main window is the only one with a dock (pop-outs have none); `None` lets the default change later |
| 2 | Restore through the panel's initial `size`; clamp at layout (and on the first frame, decision 4) | the kit clamps to 120 px … 60 % every frame; a smaller window must not overwrite the preference |
| 3 | Dashed line and double-click live in a custom `ResizeHandleRenderer` that wraps the kit look | the base handle owns the drag and has no click hook (0004); the renderer is the kit's extension point and sees `Dragging` in the same frame |
| 4 | Clamp the first frame to 60 % of the viewport (`initial_dock_height(saved, viewport)`) | before the first layout the workspace is unmeasured and `size_range` has no upper cap; a height saved in a larger window would flash too tall (the review showed the hit area gets the press, so no double-click fallback is needed) |
| 5 | The max line shows only in `Dragging`, not on hover or press | W8 note 2: "only while dragging" |
| 6 | No key for the reset | W8 names only the double-click |

## Markers (step 2)

| # | Decision | Rationale |
|---|---|---|
| 7 | Markers are buffer lines of kind `Marker` with the stream's source | one store and scroller; time sort, prefix, color, and Export for free |
| 8 | Restart = a rise of `restart_count` in the session's pod list, checked for every streamed (pod, container) pair whatever its stream state; text from `last_termination` | the pod watch already carries it (no event list or pod GET); a follow stream ends when its container exits, before the count rises |
| 9 | The restart row is the only marker: no pod join or leave lines | the wireframe shows only the restart `SYS` row; the legend already shows which pods stream |
| 10 | Markers ignore the level chips and the text filter; the brush window applies | W8 shows the `SYS` row among filtered lines; a marker has a time |
| 11 | No markers in Previous | a fixed read of the old container |

## Brush (step 3)

| # | Decision | Rationale |
|---|---|---|
| 12 | The brush filters the list to its window; the bars keep the whole range with the window shaded. An interpretation: W8b draws the window (`.hwin`) but does not say what it does | filtering is what a window over a density chart usually means; the user sees where the window sits and can redraw it; supersedes 0019 decision 21's ceiling |
| 13 | Lines without a timestamp hide while a window is set | they cannot be placed in time |
| 14 | ✕, a click without a drag, and a stream restart clear the window; a release anywhere (`on_mouse_up_out` too) ends a drag, fractions clamped to 0..=1 | no new key; a restarted buffer has other lines; a drag must not stick when the pointer leaves the chart |
| 15 | Bucket hit test by equal division of the chart width | the kit `BarChart` exposes no hit test; one bucket of error at worst (ponytail) |

## Pop out (step 4)

| # | Decision | Rationale |
|---|---|---|
| 16 | Pop out moves the `LogTab` entity: stream, buffer, filters, and brush are kept | local-only (no second request); nothing the user set is lost |
| 17 | Shell tabs do not pop out (accepted by the user, 2026-10-03) | W8b shows Pop out in the log pane only; a shell is window-bound (focus, Find input, terminal metrics) and counted by `leaving_work` and the node-shell close guard |
| 18 | The filter input is created again in the new window | the kit `InputState` binds focus, blur, and activation to its creating window |
| 19 | The window owns the tab; the dock keeps a weak handle | closing the window ends the stream with no bookkeeping |
| 20 | A cluster switch closes every pop-out; no `leaving_work` line | the app shows one cluster at a time (0046); same as the dock log tabs; logs are reads and reopen with L |
| 21 | Opening logs for a popped target activates its window | no duplicate stream for the same pod or workload |
| 22 | No dock-back, no restore at launch, no keys in pop-outs | YAGNI; close it and press L to dock it again |
