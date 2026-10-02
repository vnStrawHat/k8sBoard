# 0021 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own, and every new item has a production user in its own step. No `Cargo.toml`/`Cargo.lock` change.

## Prerequisites

| Step | Merged first | Why |
|---|---|---|
| 1 | 0010, 0011 (done); 0012 if it is in flight (`OpenWatches`, `node_usage` edits) | node metrics and the kubelet PVC store; avoids conflicts in `cluster_session.rs` |
| 2 | step 1 | the screen and the panel layout |
| 3 | step 2, 0020 step 1b | `IssueBoard`, `IssueSummary`, `Coverage`, `Screen::Issues`, the logs path of the Issues menu |
| 4 | step 3, 0019 step 3 | `log_export.rs` (`ExportState`, `export_file_name`, the save flow) |

- **Navigation tests.** 0020 asserts that Overview is disabled; 0021 step 1 enables it. Whichever of 0020 and 0021 step 1 lands **second** amends the other's navigation test (`enabled_items_are_pods_nodes_and_explorer_kinds`, and the 0020 sidebar checks), so both specs hold on `main`.
- **Watch count.** If 0020 lands first, 0021 adds `change_events` to its `OpenWatches`; otherwise 0020 keeps 0021's field. The stated bound is the sum of both.

## `crates/cluster`

| S | File | Change |
|---|---|---|
| 2 | `src/event.rs` (+ `event_tests.rs`) | `ChangeEventKind`, `watch_change_events`, `change_event_selector`, a shared private watch helper; `lib.rs` exports `ChangeEventKind` |

## `crates/app`

| S | File | Change |
|---|---|---|
| 1 | `src/cluster_capacity.rs` (new) + `cluster_capacity_tests.rs` | `Layers`, `CapacityRow`, `VolumeTotals`, `CapacityInputs`, `cluster_capacity`, `volume_totals` |
| 1 | `src/node_heatmap.rs` (new, tests in module) | `HeatCell`, `heat_cells`, `node_heatmap` |
| 1 | `src/usage_bar.rs` | `CapacityBar`, `capacity_bar` (+ tests in module) |
| 1 | `src/usage_format.rs` | `Measure::format_shared`; `format_pair` built on it (existing tests unchanged) |
| 1 | `src/overview.rs` (new, tests in module) | `headline_text`, `cluster_region`, `Counted`, `ClusterStats`, `cluster_stats`, `panel`, body layout, the Capacity and Nodes panels |
| 1 | `src/kubelet_history.rs` | `pvc_usages()`, only if 0020 has not added it |
| 1 | `src/app_shell.rs` | `Screen::Overview`; a `show_screen` arm; no-table arms; body dispatch to `overview.rs` |
| 1 | `src/workspace.rs` | Overview header (title, headline, stats line); no filter bar, selection bar, or drawer on Overview; `group_digits` becomes `pub(super)` |
| 1 | `src/navigation.rs` (+ tests) | `screen_of("Overview")`; the enabled-items test gains `Overview` |
| 1 | `src/launch_options.rs` (+ tests) | `--screen overview`, `LaunchScreen::Overview`, `USAGE` |
| 1 | `src/main.rs` | `mod cluster_capacity; mod node_heatmap; mod overview;` |
| 1 | `src/screenshot.rs` (+ tests) | Overview settle rule (step 1 part) |
| 2 | `src/recent_changes.rs` (new) + `recent_changes_tests.rs` | `ChangeWindow`, `ChangeKind`, `ChangeEntry`, `ChangeInputs`, `recent_changes`, `CHANGE_ROWS` |
| 2 | `src/cluster_session.rs` (+ tests) | `is_overview_visible`, `set_overview_visible`, `ChangeEvents`, `is_change_feed_denied`, start/stop/scope/review wiring; `OpenWatches.change_events` via `scope_multiplicity`; the `open_watch_count` doc |
| 2 | `src/overview.rs`, `src/app_shell.rs` | the Recent changes panel, footnote, range dropdown; `OverviewState { window }`, `set_change_window`; the `set_overview_visible` call in `show_screen`; `View all →` resets the event filter |
| 2 | `src/screenshot.rs`, `src/main.rs` | settle waits for the change feed; `mod recent_changes;` |
| 3 | `src/overview.rs` | the Needs attention panel (`ATTENTION_ROWS`, `object_line`, `AttentionAction`, `attention_action`) |
| 3 | `src/resource_actions.rs` (+ tests) | the `logs_launch` split, if the 0020 menu path is menu-only |
| 3 | `src/launch_options.rs` (+ tests), `src/main.rs` | default screen `overview`; `--window-width`; window width from options |
| 3 | `src/screenshot.rs` | settle waits for the issues summary |
| 4 | `src/file_export.rs` (new, tests in module) | `ExportState` and `export_file_name(label, extension, now)` moved from `log_export.rs`, with their tests |
| 4 | `src/log_export.rs`, `src/log_tab.rs` | use `file_export`; `exported_lines` stays on the tab |
| 4 | `src/overview_report.rs` (new, tests in module) | `ReportInputs`, `overview_report`, `cell`, the save flow |
| 4 | `src/overview.rs`, `src/workspace.rs`, `src/app_shell.rs`, `src/main.rs` | the Export button, status text, error alert, `OverviewState.export`; `mod file_export; mod overview_report;` |

## Docs (updated by the coder, in the step that completes the row)

- `docs/roadmap/inventory-screens.md`: O3 and O4 → Done (step 1). O5 → Partial (step 2; managedFields and diff → 0031). O2 → Done (step 3). O1 → Partial (step 3), then Done (step 4).
- `docs/roadmap/inventory-shell.md`: N5 → Overview Done (step 1).
- `docs/roadmap/README.md`: Overview row status; the default landing note (step 3).
- `docs/roadmap/gap-plan-read-only.md` 0021: link this folder.
- Step 3: any ui-verifier script or doc that relies on Pods as the default screen passes `--screen pods`.
