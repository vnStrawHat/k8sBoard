# 0021 · Export report (step 4)

[Back to index](README.md) · Modules: `file_export.rs` (new; shared export state and file name, moved from 0019's `log_export.rs`), `overview_report.rs` (new; pure report builder and save flow, tests in module), `overview.rs`, `workspace.rs` · Needs 0019 step 3 merged. C9 is user-approved (0019 decision 22).

## Shared with 0019 (`file_export.rs`)

0021 is the second exporter, so it moves the shared part out of `log_export.rs` (code style: extract once a second module needs it):

```rust
/// One export, from button click to result. Both the log dock and Overview use it.
pub(crate) enum ExportState { Idle, Choosing, Saving, Saved { file_name: String }, Failed { message: String } }
/// `{label}-{YYYYMMDD-HHMMSS}Z.{extension}`. The one sanitizer for every exporter (0019, 0021, 0022): each char
/// outside `[A-Za-z0-9._-]` (path-unsafe ones such as `/`, `\`, `:`, `@`, spaces) becomes `-`.
pub(crate) fn export_file_name(label: &str, extension: &str, now: jiff::Timestamp) -> String;
```

- `Saved` carries no line count (amended in 0019 [dock-polish.md](../0019-workload-logs/dock-polish.md)). Each exporter keeps its own detail text: the log tab keeps `exported_lines`, and Overview needs none.
- `log_export.rs` is gone: one generic `start_export(name, noun, snapshot, set_state, finish, cx)` in `file_export.rs` serves every exporter (the log tab, Overview, and 0022 as the third). The dialog opens first; `snapshot` runs after the user confirmed a path and returns the text to write or a failure message. `export_file_name` cuts the label at 150 chars. The moved 0019 test expects `-` now: `deploy/api` → `deploy-api-20240501-104758Z.log` (amended for 0022, which exports `topology-readonly-Monitor-{ns}-…png`).

## Content (pure)

```rust
pub(crate) struct ReportInputs<'a> {
    pub(crate) headline: &'a str, pub(crate) stats: &'a str,
    pub(crate) checked: bool, // the issue board has run; false prints `Not checked yet.`
    pub(crate) issues: &'a [Issue], pub(crate) coverage_note: Option<String>,
    pub(crate) capacity: &'a [CapacityRow], pub(crate) cells: &'a [HeatCell],
    pub(crate) changes: &'a [ChangeEntry], pub(crate) changes_note: Option<String>, // why there is no table: loading, failed, denied
    pub(crate) window: ChangeWindow,
    pub(crate) now: jiff::Timestamp,
}
/// Markdown: the title and generated time (RFC 3339), then the four W3 sections as tables.
pub(crate) fn overview_report(inputs: &ReportInputs) -> String;
```

| Section | Rows |
|---|---|
| `# Overview — {headline}` | `Generated {rfc3339}` and the stats line |
| `## Needs attention ({n})` | **every** issue: Severity, Reason, Object (`object_line`), Cause, Since (RFC 3339); then the coverage note when coverage is partial; `No issues found.` when empty |
| `## Capacity` | Name, `label()`, `note()`, `ceiling()` |
| `## Nodes` | Node, Status, CPU %, Memory % (`—` when missing) |
| `## Recent changes ({window label})` | Time (RFC 3339), Change (`{kind label} {object} {text}`), Who; then the footnote line |

- Table cells escape `|` and replace newlines with spaces (`fn cell(text: &str) -> String`).
- The report holds no kubeconfig data: no server URL, user, token, or certificate. Only the context name appears, as the header shows it.

## Save flow

- **Button.** `Export report`: a ghost small button with `IconName::Download`, in the Overview header right, after the range dropdown. Disabled without a live session and while `Choosing`/`Saving`.
- **Click.** `export_file_name("overview-{context}", "md", now)` → `cx.prompt_for_new_path(&home_dir, Some(&name))`. On confirm, build the report **at that moment** from the current snapshots, then write it with `background_spawn(std::fs::write)` and set the result state. These are the same six steps as 0019.
- **State.** The state lives in `AppShell.overview.export: ExportState`, and its task in `_export: Option<Task<()>>`.
  - `Saved` shows muted `Saved to {file_name}` next to the button.
  - `Failed` shows an error `Alert` under the header: `Could not save the report: {error_text}`.
  - Both clear on the next export.
- **Same cluster.** The snapshot closure compares the session captured at the click with the current one; a context switch while the dialog is open fails with `the cluster changed while the dialog was open`.
- **Cancel.** Cancel → `Idle`; nothing is written. Paths and file names are never traced (0019 decision 31).
