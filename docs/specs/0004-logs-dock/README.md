# 0004 — Logs dock (read-only)

Status: amended after advisor review (6 should-fix items plus orchestrator decisions 7–9). Crates: `crates/cluster` (log stream) and `crates/app` (dock). Requires [0003](../0003-app-shell-pods-nodes/README.md). Wireframes: W8, W8b, W4 note 2.

## Goal

Stream one pod container's logs into a bottom dock. Users open it from the pod row menu or the drawer ⋯ menu, and get one tab per pod with a container picker, a text filter, and Previous, Timestamps, Wrap, and Copy controls. The dock can be resized up to 60% of the workspace, zoomed over the whole workspace, or minimized to its tab bar.

## Non-goals

- **Shell tab.** UAT denies `create pods/exec`, and the app is read-only. How Shell fits in later is covered in [dock-layout.md](dock-layout.md).
- Workload logs (`deploy/…` tabs), merged multi-container views, level toggles, JSON, regex, the density histogram, Export, and Pop out (W8b note 4).
- The "+ ▾" new-tab button, tab drag-and-drop, keyboard shortcuts (`L`, `` Ctrl+` ``, `Ctrl+Shift+M`, `Ctrl+W`, `Ctrl+Tab`, `Esc`), the dashed max-height line, double-click height reset (the kit handle has no click hook), and a persisted dock height.
- Local-time display, auto-reconnect, resume from the last timestamp, and writing logs to disk.

## Decisions (defaults chosen by the architect)

1. **Logs only.** There is no disabled Shell placeholder tab: the menu already explains why Shell is unavailable.
2. **One tab per pod.** Opening the same pod again activates its tab. The container is picked inside the tab with the drawer's default-container rule, so the menu needs no container submenu.
3. **Plain chunked HTTP** through `Api::<Pod>::log_stream`. In kube-client 4.2 this is not behind the `ws` feature (checked in source), so `ws` stays off.
4. **`timestamps=true` is always requested.** The Timestamps toggle only hides the column, so toggling never restarts the stream.
5. **Bounds.** Open with the last 1,000 lines. Each tab keeps at most 10,000 lines and 8 MiB, evicting the oldest first. Any line over 16 KiB is truncated.
6. **The crate batches lines every 100 ms**, the same window as 0002. The app calls `notify` once per batch, through a generalized `ClusterRuntime::subscribe`.
7. **The list is the kit `MessageScroller`** (a GPUI `list` with variable row heights, tail-follow, a scrollbar, and "Jump to latest"). Variable heights make Wrap possible, which `uniform_list` cannot do.
8. **Follow means tail-follow.** Scrolling up pauses it, and "Jump to latest" resumes it. Evicting old lines does not move the rows being read, because GPUI's `splice` shifts the scroll anchor.
9. **The filter is a plain substring match**, case-insensitive for ASCII letters. It hides non-matching lines and highlights the matches.
10. **Reconnect is manual and always fresh.** It is available in every state except Connecting. It clears the buffer and restarts with the same container and toggle (`Current` = follow + tail 1,000). There is no resume and no automatic retry loop, which would keep hitting the CrashLoopBackOff 400s.
11. **The dock sits below the table region**, so no table rows are hidden behind it. The drawer still overlays only the table region. Zoom covers the whole workspace.
12. **Height:** the kit `v_resizable` split. 280 px by default; `size_range` runs from 120 px to 60% of the measured workspace height. Zoom and minimize leave the split and keep its state for the return.
13. **The status bar does not change.** Stream state is shown by a dot on each tab and by the toolbar.
14. **What happens on navigation:**
    - a context switch closes every log tab, because each stream belongs to the old connection;
    - namespace and screen switches keep the tabs;
    - clicking a navigation item un-zooms the dock (W8b note 1).
15. **Log text is never traced, stored, or exported.** The only way out is the explicit Copy button.

## Files

| File | Contents |
|---|---|
| [log-stream.md](log-stream.md) | cluster crate: `pod_logs`, request mapping, line splitting, batching, errors |
| [dock-layout.md](dock-layout.md) | workspace split, `LogDock` state, tab bar, resize, zoom, minimize, Shell slot |
| [log-tab.md](log-tab.md) | `LogTab`: toolbar, states, rows, opening from menus, async wiring |
| [log-buffer.md](log-buffer.md) | pure buffer: caps, filter, match ranges, time format |
| [files-to-touch.md](files-to-touch.md) | modules, `lib.rs`, probe flag, screenshot screens |
| [test-plan.md](test-plan.md) | unit tests, live checks, ui-verifier checklist |

## Acceptance criteria

- [ ] 1. The quality gate passes, and so does `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`.
- [ ] 2. Every test in [test-plan.md](test-plan.md) exists under that name and passes. None of them touches a network.
- [ ] 3. The read-only guard from 0001 AC4 still holds and also covers the new files. `kube` features still exclude `ws`. No `kube` or `k8s_openapi` type appears in a public signature. The crate never spawns tasks.
- [ ] 4. No `tracing` call in `pod_log.rs` or `crates/app/src/log_*.rs` receives line text or chunk bytes (reviewed, plus a scoped grep).
- [ ] 5. Probe: `--logs-seconds 5` against UAT prints `started`, more than 0 lines, and 0 failures. The 0001 AC7 credential script reports 0 for every count.
- [ ] 6. App on UAT:
  - "View logs" is enabled and streams lines;
  - Previous and the container picker restart the stream;
  - Shell and Port-forward are still disabled.
- [ ] 7. The `logs-dock` and `logs-zoomed` screenshots exist in light and dark, and the ui-verifier reports no high-severity defect against W8/W8b.
- [ ] 8. The 0003 AC4 color-literal grep is still clean.

## Open items

1. A follow stream on a half-open connection can go silent, because there is no read timeout (0002 open item 1). The user can recover with Reconnect, which is shown while Streaming; there is no automatic detection.
2. (Deferred) The number of tabs is unbounded. Each tab holds at most 8 MiB and one HTTP stream. Add a cap if users open many tabs.
3. (Deferred) Local time needs jiff time-zone features, so timestamps are shown in UTC for now.
4. (Deferred) When a container restarts, its follow stream ends. Auto-reconnect could follow the new instance.
