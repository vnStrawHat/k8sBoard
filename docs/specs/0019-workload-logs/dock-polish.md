# 0019 · Step 3: layouts, histogram, Export, "+ ▾", reorder, Logs sub-tab

[Back to index](README.md) · Modules: `log_volume.rs`, `log_export.rs` (new), `log_tab.rs`, `log_dock.rs`, `drawer.rs`, `container_detail.rs`, `app_shell.rs`.

## Layouts

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LogLayout { Compact, Full }
impl LogTab { pub(crate) fn set_layout(&mut self, layout: LogLayout, cx: &mut Context<Self>); }
```

- `LogDock` pushes `Full` iff `DockMode::Zoomed`, from `set_mode` (every mode change) and after `open` creates a tab.
- Full, top to bottom: legend row (workload tabs only, [workload-streams.md](workload-streams.md); step 2b drew it in every layout, step 3 limits it to Full), toolbar, histogram (when `volume` is `Some`), lines. Compact: toolbar, lines (0004).

## Histogram (`log_volume.rs`, inline tests)

```rust
pub(crate) struct VolumeBucket { pub(crate) start: jiff::Timestamp, pub(crate) lines: u32, pub(crate) errors: u32 }
pub(crate) struct Volume { pub(crate) width: Duration, pub(crate) buckets: Vec<VolumeBucket> }
const MAX_BUCKETS: usize = 60;
const BUCKET_WIDTHS_SECS: [u64; 13] = [1, 5, 15, 30, 60, 300, 900, 1800, 3600, 10_800, 21_600, 43_200, 86_400];
/// `None` without two distinct timestamps. Input: visible lines with a timestamp.
pub(crate) fn volume(lines: impl Iterator<Item = (jiff::Timestamp, Option<LogLevel>)>) -> Option<Volume>;
/// The smallest width with `span / width < MAX_BUCKETS`; else one day, keeping the newest 60 buckets.
fn bucket_width(span: Duration) -> Duration;
/// `HH:MM:SS` under a minute, `HH:MM` under an hour, `MM-DD HH:MM` under a day, `MM-DD` otherwise (UTC).
fn bucket_label(start: jiff::Timestamp, width: Duration) -> String;
pub(crate) fn volume_chart(volume: Rc<Volume>, cx: &App) -> AnyElement;
```

- Buckets are aligned to `floor(unix_seconds / width) * width` and span the first to the last bucket with zeros in between. Lines arrive nearly sorted, so find min/max in one pass first.
- `errors` counts `Some(Error)` only (decision 15 does not apply here).
- Chart: `gpui_kit::component::chart::BarChart::new(buckets)` with `.id("log-volume")`, `.band(|b| index string)` (unique; the axis is hidden), `.value(|b| b.lines as f64)`, `.fill(…)` → `tone_color(Bad)` when `errors > 0` else `theme.chart_1` (colors captured before the closure), `.label_axis(false).value_axis(false).grid(false)`, `.tooltip_title(|b| bucket_label)`, `.tooltip_value(|b, v| "{v} lines · {errors} errors")`. 48 px high, `px_3`; a muted `text_xs` caption `Lines per {1s|5s|…|1d}` on the left.
- Memo: `volume_memo: Option<(u64, Option<Rc<Volume>>)>` keyed by `buffer.revision()`, recomputed in render only in Full layout.
- Ceiling (decision 21): no brush selection; bars only carry tooltips.

## Export (`log_export.rs`: state, flow, file name; inline tests for the pure part)

```rust
pub(crate) enum ExportState { Idle, Choosing, Saving, Saved { file_name: String }, Failed { message: String } } // shared with 0021 (it moves to `file_export.rs` there)
/// `{label}-{YYYYMMDD-HHMMSS}Z.log`; every char outside `[A-Za-z0-9._-]` becomes `_`.
pub(crate) fn export_file_name(label: &str, now: jiff::Timestamp) -> String;
/// Steps 1–6 below. `LogTab` stores the returned task and exposes `export_snapshot()` and `set_export_state()`.
pub(crate) fn start_export(name: String, cx: &mut Context<LogTab>) -> Task<()>;
```

- Button: ghost small `IconName::Download`, tooltip `Export visible lines…`, in the toolbar after Copy (both layouts). Disabled while `Choosing`/`Saving` or when `visible_len() == 0`.
- Label for the name: pod tab `{pod}-{container}`, workload `{label}` (`deploy/api` → `deploy_api`).

| Step | Code |
|---|---|
| 1 click | `state = Choosing`; `let directory = std::env::home_dir().unwrap_or_default();` `let path = cx.prompt_for_new_path(&directory, Some(&name));` |
| 2 await | `cx.spawn(async move |tab, cx| …)`, task kept in `_export: Option<Task<()>>` |
| 3 cancel | `Ok(Ok(None))` or a dropped sender → `Idle`; `Ok(Err(e))` → `Failed("Could not open the save dialog: {e}")` |
| 4 confirm | snapshot now via `tab.export_snapshot()`: `buffer.visible_text(LineTime::Rfc3339, &full_prefixes)` (`{pod}/{container}`, decision 32) + trailing `\n`, line count; `state = Saving` |
| 5 write | `cx.background_spawn(async move { std::fs::write(&path, text) }).await` |
| 6 result | `exported_lines = lines` on the tab, then `Saved { file_name: path.file_name() lossy }` or `Failed("Could not save the logs: {io error}")` |

- Saved → status prefix `Saved {exported_lines} lines to {file_name} · ` (the count is tab text, not part of `ExportState`); Failed → an error `Alert` above the lines; both clear on the next export or restart.
- Overwrite confirmation is the platform dialog's (Windows `IFileSaveDialog` default `FOS_OVERWRITEPROMPT`). Nothing is written on cancel. No retry loop. No tracing of `path`, `file_name`, or `text` (AC 4).
- No other code path writes files; never automatic (C9).
- **Partial-write trade-off (decision 35):** `std::fs::write` truncates, then writes; a failure mid-write can leave a partial file and is reported as `Could not save the logs: …`. No temp file + rename.

## "+ ▾" new-tab menu (`log_dock.rs`)

- `LogDock::new(shell: WeakEntity<AppShell>)` (AppShell passes its weak handle).
- After the last tab: ghost xsmall `Button` with `IconName::Plus` and `dropdown_caret`, tooltip `New tab`.

| Item | Enabled when | Click |
|---|---|---|
| `Logs of selected` | `shell.read(cx).selected_log_target(cx)` is `Ok`; else `disabled_menu_item` with the `NoLogTarget` `Display` text | `shell.update(…, open_logs_of_selection(window, cx))` |
| `Shell into selected` | never in this version: `disabled_menu_item` with the `action_availability(OpenShell, access)` reason (or `Read-only mode` without a session) | — |

- The builder runs during event dispatch, not inside an `AppShell` update, so reading the shell is safe.

## Tab reorder (`log_dock.rs`)

```rust
#[derive(Clone)] struct DraggedTab { index: usize, label: SharedString } // impl Render: the tab label chip
/// Moves `from` to `to`; returns the active index so the same tab stays active.
fn move_tab<T>(tabs: &mut Vec<T>, from: usize, to: usize, active: Option<usize>) -> Option<usize>;
```

- Each tab: `.on_drag(DraggedTab { … }, |tab, _, _, cx| cx.new(|_| tab.clone()))`, `.drag_over::<DraggedTab>(|style, _, _, cx| style.bg(cx.theme().accent))`, `.on_drop(cx.listener(move |dock, dragged: &DraggedTab, _, cx| dock.move_tab(dragged.index, index, cx)))`.
- `from == to` or out of range → no change.

## Container Logs sub-tab (`drawer.rs`, `container_detail.rs`, `app_shell.rs`)

- `ContainerTab::Logs` between `Mounts` and `Monitor`; `CONTAINER_TABS` has 5 entries; title `Logs`.
- Sub-tab click on Logs: `shell.set_container_tab(Logs, cx)` then `shell.open_container_logs(window, cx)`.
- `AppShell::open_container_logs(&mut self, window, cx)`: the selected pod from the live list, the container = `drawer.selected_container` or `default_container`; `LogTarget::of_container` → `log_dock.open` (Minimized becomes Normal, 0004). Disabled access → no-op (the body shows why).
- `logs_body`: muted `Logs of {container} open in the dock below.` + ghost small button `Show in dock` (`IconName::FileText`) → `open_container_logs`. When `action_availability(ViewLogs)` is disabled: the reason text instead of the button.
- `container_tab` keeps surviving subject changes (0008), but moving to another pod never opens a stream; the body button does.
