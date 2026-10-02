# 0019 · Step 1: buffer, JSON view, toolbar, rows

[Back to index](README.md) · Modules: `log_buffer.rs`, `log_json.rs` (new, pure), `log_rows.rs` (new), `log_tab.rs`. Levels and matcher: [filters-and-levels.md](filters-and-levels.md).

## Buffer (`log_buffer.rs`; replaces 0004 `LineFilter`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SourceId(pub(crate) u16);          // index into the tab's source table
pub(crate) struct SourcedLine { pub(crate) source: SourceId, pub(crate) line: LogLine }
pub(crate) struct BufferedLine { pub(crate) source: SourceId, pub(crate) level: Option<LogLevel>, pub(crate) line: LogLine }
#[derive(Default)]
pub(crate) struct LineView { pub(crate) matcher: Option<LineMatcher>, pub(crate) hidden_levels: LevelSet }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineTime { Hidden, Clock, Rfc3339 }

impl LogBuffer {
    pub(crate) fn push(&mut self, lines: Vec<SourcedLine>) -> BufferChange; // detects levels
    pub(crate) fn clear(&mut self);                                     // keeps the view
    pub(crate) fn set_view(&mut self, view: LineView);                  // full rescan
    pub(crate) fn view(&self) -> &LineView;
    pub(crate) fn visible_line(&self, index: usize) -> Option<&BufferedLine>;
    pub(crate) fn visible_lines(&self) -> impl Iterator<Item = &BufferedLine>;
    /// One line per visible line: `{time} {prefix} {text}`; a missing prefix index means none.
    pub(crate) fn visible_text(&self, time: LineTime, prefixes: &[SharedString]) -> String;
    pub(crate) fn revision(&self) -> u64;   // bumps on push, clear, set_view (histogram memo key)
    // visible_len, total_len, has_dropped unchanged; needle() removed (use view().matcher)
}
```

- Fields: `lines: VecDeque<BufferedLine>`, `visible: Option<VecDeque<u64>>` (`None` exactly when the view hides nothing), `last_levels: Vec<Option<LogLevel>>` indexed by `SourceId` (continuation; reset by `clear`).
- Visible ⇔ `!hidden_levels.is_hidden(level.unwrap_or(Info))` and the matcher (if any) matches `line.text`.
- The 0004 eviction algorithm and the `BufferChange` invariant are unchanged; only the predicate is generalized.
- `LineTime::Clock` = `format_log_time`; `Rfc3339` = jiff `Timestamp` Display. A line without a timestamp writes no time.
- `prefixes` is chosen by the caller: Copy passes the on-screen short prefixes (`x2k4q/api`); Export passes the full `{pod}/{container}` prefixes (decision 32). Pod tabs pass `&[]`.
- **Fairness ceiling:** one buffer per tab is shared by all streams, so a chatty pod can evict a quiet pod's lines. Accepted (decision 9); upgrade path: per-source quotas.
- `SharedString` is the only GPUI type here (a plain string type); keep the module otherwise GPUI-free.

## JSON view (`log_json.rs`, inline tests)

```rust
pub(crate) struct JsonLine { pub(crate) headline: Option<String>, pub(crate) details: Vec<String> }
/// `None` unless `text` (trimmed) is a JSON object.
pub(crate) fn json_line(text: &str) -> Option<JsonLine>;
```

- `headline`: the first string value of `msg`, `message`. `details`: `serde_json::to_string_pretty` of the object without the headline key, the level keys (above), and `time`, `ts`, `timestamp`, `@timestamp`, split into lines; empty when nothing remains. Keys are sorted (`serde_json::Map` without `preserve_order`).

## Toolbar additions (`log_tab.rs`)

| Control | Kit | Behavior |
|---|---|---|
| Regex | `Toggle` with `IconName::Regex`, tooltip "Regular expression" | flips `filter_mode`, re-parses the input |
| ERROR WARN INFO DEBUG | four small `Toggle`s, checked = shown | `hidden_levels.toggled(level)` → `set_view` |
| JSON | `Toggle`, off | `remeasure` |
| Filter input | placeholder `Filter lines` / `Regex, e.g. error\|timeout` by mode | parse → on `Ok` `set_view` + `scroller.reset(visible_len)`; on `Err` keep the view, set `has_invalid_filter` |

- Status: `Invalid regex` in `tone_color(Bad)` before the count while `has_invalid_filter`.
- Empty states: `No lines match "{pattern}"` (matcher set), `No lines at the selected levels` (levels only).

## Rows (`log_rows.rs`; row code moves out of `log_tab.rs`)

`fn log_row(line: &BufferedLine, style: &RowStyle, cx: &App) -> AnyElement` where `RowStyle` carries `shows_timestamps`, `wraps_lines`, `shows_json`, the matcher ref, and the prefix (`Option<(SharedString, Hsla)>`).

- Cells: time (0004) · prefix (workloads, step 2a; short form, `rems(9.)`, truncated, colored) · JSON level tag (JSON mode, toned: Error `Bad`, Warn `Warn`, else muted) · text with `ranges` highlights.
- JSON mode and `json_line` is `Some`: text = headline (or the raw line when absent); `details` as muted mono lines indented to the text column, wrapping by the Wrap toggle.
- `level == Some(Error)` → row background `tone_color(Bad).opacity(0.08)`.
