# 0019 · Files to touch

[Back to index](README.md)

## Cargo (step 1)

```toml
# Cargo.toml [workspace.dependencies]
regex = "1"            # 1.13.1 already locked through tracing-subscriber env-filter
# crates/app/Cargo.toml [dependencies]
regex.workspace = true
serde_json.workspace = true   # already a workspace dependency (cluster crate)
```

- AC 3: `Cargo.lock` gains no `[[package]]` entry; only the `k8sboard` dependency list changes. Check with `grep -c '^\[\[package\]\]' Cargo.lock` before and after.

## `crates/cluster` (step 2a only; decision 33)

| File | Change |
|---|---|
| `src/pod_log.rs` | `LogRequest.tail_lines: u32`; `log_params` uses it; private `LOG_TAIL_LINES` removed |
| `src/pod_log_tests.rs` | the two `log_params_*` tests set `tail_lines: 1000`; new `log_params_uses_requested_tail` |
| `examples/probe.rs` | `--logs-seconds` request passes `tail_lines: 1000` |

## `crates/app`

`log_filter.rs` (the existing tracing `EnvFilter` pin) is **not** touched.

| File | Step | Change |
|---|---|---|
| `src/log_level.rs` (new) | 1 | `LogLevel`, `LevelSet`, `detect_level`, `level_of_word`, `level_of_number`; inline `mod tests` |
| `src/line_matcher.rs` (new) | 1 | `FilterMode`, `LineMatcher`, `InvalidRegex`, `find_matches` (moved from `log_buffer.rs`) |
| `src/line_matcher_tests.rs` (new) | 1 | matcher tests; the 0004 `find_matches_*` tests move here |
| `src/log_json.rs` (new) | 1 | `JsonLine`, `json_line`; inline `mod tests` |
| `src/log_rows.rs` (new) | 1 | `RowStyle`, `log_row` (moved from `LogTab::render_row`, plus prefix, level tag, JSON block, error tint) |
| `src/log_buffer.rs` | 1 | `SourceId`, `SourcedLine`, `BufferedLine`, `LineView`, `LineTime`; `push`/`set_view`/`visible_text`/`revision`; `LineFilter`, `set_filter`, `needle` removed |
| `src/log_buffer_tests.rs` | 1 | adapt the 0004 tests to `SourcedLine` and `set_view`; new tests |
| `src/log_target.rs` (new) | 2a, 2b | `LogTarget`, `PodTarget`, `ContainerChoice`, `WorkloadTarget`, `workload_label`, `is_same` (2a); `NoLogTarget` (2b); inline `mod tests` |
| `src/log_workload.rs` (new) | 2a, 2b | constants, `ranked_pods`, `MemberChange`, `member_change`, `join_slots`, `pod_limit`, `pod_short_name`, `scope_covers` (2a); `container_names` (2b); inline `mod tests` |
| `src/log_tab.rs` | 1–3 | 1: toolbar controls and view state. 2a: `LogSubject`, `TabStream`, `Staging`, `sync_members`, grace and rejoin, merge, `sort_staged`, `workload_tone`, status, prefix column. 2b: container multi-picker, legend. 3: `LogLayout`, histogram memo, Export button wiring |
| `src/log_volume.rs` (new) | 3 | `VolumeBucket`, `Volume`, `volume`, `bucket_width`, `bucket_label`, `volume_chart`; inline `mod tests` |
| `src/log_export.rs` (new) | 3 | `ExportState`, `export_file_name`, `start_export`; inline `mod tests` (pure part) |
| `src/log_dock.rs` | 2a, 3 | 2a: `session` field, `set_session`, `open` with the `LogTarget` enum and the `Explicit` container switch. 3: `new(shell)`, layout push, "+ ▾", `DraggedTab`, `move_tab` |
| `src/kind_row.rs` | 2a | `PodOwner::namespace` |
| `src/resource_actions.rs` (+ tests) | 2a | `kind_menu`: View logs item for workload rows |
| `src/app_shell.rs` | 2a–3 | 2a: `open_workload_logs`, `log_dock.set_session` in `start_session`. 2b: `selected_log_target`. 3: `open_logs_of_selection`, `open_container_logs`, `LogDock::new(shell)` |
| `src/drawer.rs` | 3 | `ContainerTab::Logs` |
| `src/container_detail.rs` (+ tests) | 3 | `CONTAINER_TABS` (5), title, `logs_body`, sub-tab click opens the dock |
| `src/launch_options.rs` (+ tests) | 2b | `LaunchScreen::LogsWorkload` (`logs-workload`); `has_log_dock` true; `screen()` → Pods; `USAGE` |
| `src/screenshot.rs` | 2b | `controller_owner_of(pod) -> Option<PodOwner>` (private; ReplicaSet, StatefulSet, DaemonSet, Job by the kind constants); `is_log_pending` also true while staging |
| `src/main.rs` | 1–3 | `mod log_level; mod line_matcher; mod log_json; mod log_rows; mod log_target; mod log_workload; mod log_volume; mod log_export;` |

All new items are private or `pub(crate)`. One concept per file: targets in `log_target.rs`, membership in `log_workload.rs`, rows in `log_rows.rs`, Export state and flow in `log_export.rs`.

## Screenshot screen (step 2b)

| Screen | Setup, once pods are Ready |
|---|---|
| `logs-workload` | Pods screen, no selection. `pick_logs_pod` → `controller_owner_of` → `open_workload_logs` → `DockMode::Zoomed`. No controller (bare pod) → `logs-zoomed` behavior |

- Settled when no stream is `Connecting` and `staging` is `None` (the 2 s window plus the 0003 post-settle wait).
- PNGs contain log text: keep them in `.tmp/ui-shots/` (git-ignored); never attach them anywhere (0004).

## Docs

- 0003 `screenshot-hook.md`: add `logs-workload` to the `--screen` list.
- 0004 `log-stream.md`: note `LogRequest.tail_lines` (0019 decision 33).
- Roadmap `inventory-shell.md` dock rows and `gap-plan-read-only.md` 0019: flip to Done/Partial after merge; note the kubelet-logs and Pop out deferrals.
