# 0044 — Dock polish (W8, W8b)

Status: **built 2026-10-04 on main `b52d77e`, awaiting review** (draft 2026-10-03, amended after the opus review for one cluster at a time, 0046; see [as-built.md](as-built.md)). **Local-only: no new Kubernetes calls** (`crates/cluster` untouched; Pop out moves a running stream, it never opens a second one). Crate: `crates/app`. Prerequisites: merged 0004, 0019, 0024, 0036. Roadmap: [wireframe-gap-audit.md](../../roadmap/wireframe-gap-audit.md) gap 6, order item 7. Wireframes: W8 notes 1, 2 and its `SYS` line; W8b header (`Export`, `Pop out`), the density chart window (`.histo .hwin`), notes 3–5; change note v0.5 (drag up to 60 % of the Workspace, ⤢ zoom, Logs and Shell in separate tabs).

## Goal

- Dock height: a dashed line at 60 % while the handle is dragged; double-click the handle resets 280 px; the height is remembered in `dock.height` (0024 reserved key).
- `SYS` marker lines: container restarts with reason, exit code, and count, in pod and workload tabs (the one marker W8 shows).
- Histogram brush: drag across the histogram to show one time window; the bars keep the whole range.
- Pop out: a log tab moves to its own OS window with its stream, buffer, and filters.

## Non-goals

Pop out for shell tabs (decision 17); docking a popped tab back; restoring pop-out windows at launch; keys inside a pop-out window; per-screen dock heights; persisting zoom or minimize; a brush in the Compact layout (no histogram there); markers for anything but a container restart (no pod join or leave lines); markers in Previous mode; anything multi-cluster (the app shows one cluster at a time, 0046).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Height: `DockSettings`, initial height, save on resize end, double-click reset, dashed 60 % line | 1–5, 11 |
| 2 | `SYS` marker lines | 1–3, 6, 11 |
| 3 | Histogram brush | 1–3, 7, 11 |
| 4 | Pop out (log tabs), `--screen logs-popout`, docs, ui-verifier | 1–3, 8–12 |

## Files

| File | Contents |
|---|---|
| [dock-height.md](dock-height.md) | step 1: setting, restore, save, handle renderer (double-click, dashed line) |
| [log-markers.md](log-markers.md) | step 2: marker lines, buffer kind, restart detection, row look |
| [histogram-brush.md](histogram-brush.md) | step 3: window model, mapping, interaction |
| [pop-out.md](pop-out.md) | step 4: button, dock bookkeeping, window, Shell tabs, lifecycle with `leaving_work` |
| [decisions.md](decisions.md) · [files-to-touch.md](files-to-touch.md) · [test-plan.md](test-plan.md) | decisions; files per step; tests and checks |
| [as-built.md](as-built.md) | deviations, facts for a later change, and the checks that were run |

## Acceptance criteria

- [x] 1. Quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`; no new `#[allow]`; `Cargo.lock` unchanged.
- [x] 2. Every test in [test-plan.md](test-plan.md) exists under its name and passes offline; no test opens a cluster connection.
- [x] 3. Local-only: no diff in `crates/cluster`; no new `ClusterConnection` call site in `crates/app`; a pop out keeps the same `LogTab` entity and stream subscriptions (no second `pod_logs` request; trace shows no new `sending request`) (v0.5 note).
- [x] 4. While the handle is dragged, and only then, a dashed line in `theme.muted_foreground` with the label `max height · 60%` marks the 60 % limit; the drag still stops there (W8 note 2, v0.5).
- [x] 5. A double-click on the handle sets the dock to 280 px and clears `dock.height`; a drag end writes the height in whole pixels through `AppSettings::update`; the next launch opens the dock at the saved height, clamped to 120 px … 60 % of the window from the first frame; a `--screenshot` run writes nothing; zoom and minimize are unchanged (W8 notes 1, 4; 0024 `dock.height`).
- [x] 6. Markers: a rise of a streamed container's `restartCount` adds `── container api terminated: OOMKilled (exit 137) · restart #14 ──` (or `── container api restarted · restart #14 ──` without a last termination) in pod and workload tabs, also when the container's follow stream has already ended. The row shows a `SYS` tag in the warning tone; level chips and the text filter never hide a marker; the histogram does not count it; Export writes it (W8 and W8b `SYS` rows; 0019 follow-up).
- [x] 7. Brush (Full layout): dragging across the histogram shows only lines inside the window; the bars keep every bucket and the window is shaded; a release outside the chart still ends the drag; a chip `HH:MM:SS – HH:MM:SS` with ✕ clears it, as do a click without a drag and a stream restart (W8b density chart, note 4).
- [x] 8. `Pop out` appears on log tabs only, next to Export; it moves the tab to a new OS window whose title (`titlebar.title`) is the tab label, in the Full layout; lines, filter text and mode, level chips, JSON, Wrap, Timestamps, and the brush carry over; the filter input focuses and takes typing in the new window (W8b header, note 4).
- [x] 9. Closing a pop-out ends its stream; View logs on the same target activates the window instead of opening a tab; a cluster switch closes every pop-out with the dock log tabs and adds no `leaving_work` line; closing the main window quits and closes them (W8b note 3; 0036 decision 37).
- [x] 10. Shell tabs offer no Pop out and stay in the dock; `leaving_work` counts and the node-shell close guard are unchanged (W8b notes 3, 5).
- [x] 11. Colors come from theme tokens only (0003 color-literal grep clean).
- [ ] 12. (open: the ui-verifier agent run; the coder shots and the UAT trace are in [as-built.md](as-built.md)) ui-verifier: `--screen logs-zoomed` (histogram, Pop out next to Export) and `--screen logs-popout` (the pop-out window) match W8b with no high-severity defect; the coder records a manual UAT check of the dashed line, the reset, and the brush.

## Open items

1. Settled by the user (2026-10-03): Shell tabs do not pop out (decision 17).
