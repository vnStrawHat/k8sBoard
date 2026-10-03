# 0044 · Step 2: `SYS` marker lines

[Back to index](README.md) · Modules: `log_buffer.rs` (+ `log_buffer_tests.rs`), `log_tab.rs`, `log_workload.rs`, `log_rows.rs`. Decisions 7–11. Wireframe: W8 and W8b row `SYS ── container api terminated: OOMKilled (exit 137) · restart #14 ──`; 0019 follow-up.

## The one marker (W8, W8b)

| When | Tabs | Text | Time | Source (prefix, color) |
|---|---|---|---|---|
| A streamed container's `restart_count` rises | pod, workload | `── container {c} terminated: {reason} (exit {code}) · restart #{n} ──`; without `last_termination`: `── container {c} restarted · restart #{n} ──` | `last_termination.finished_at`, else now | the latest stream of that pod and container |

- `{reason}`: `StatusReason`'s `Display` (`OOMKilled`, `Error`, `Completed`); a missing reason gives `terminated (exit {code})`. `{code}`: `Termination::exit_code`. Nothing else of the pod is read (no message; C1).
- No join or leave markers: the wireframe shows only the restart row (decision 9).
- Only the Current instance marks; Previous is a fixed read of the old container (decision 11).
- Local-only: the facts come from the session's pod list (`LiveCluster::pods`), which the watch already keeps; no event list, no pod GET (decision 8).

## Buffer (`log_buffer.rs`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineKind { Log, Marker }
pub(crate) struct SourcedLine { pub(crate) source: SourceId, pub(crate) kind: LineKind, pub(crate) line: LogLine }
pub(crate) struct BufferedLine { /* … */ pub(crate) kind: LineKind }
```

- Stream lines are `LineKind::Log` (the one constructor in `apply_update`).
- A marker skips level detection (`level: None`) and leaves `last_levels` alone, so a continuation line after it keeps its level.
- `LineView::shows`: a marker ignores the level chips and the text filter (decision 10); the brush window of step 3 applies to it.
- Markers count toward `MAX_LINES` and `MAX_BYTES` like any line. `visible_text` (Export) writes them like other lines: time, prefix, text.

## Tab (`log_tab.rs`)

- `restart_seen: HashMap<(String, String), u32>`: pod and container → last seen count. A first sight records without a marker. `restart_stream` clears it with the buffer (a restart re-baselines).
- Pod tabs follow the session too: `LogSubject::Pod` gains `session: WeakEntity<ClusterSession>` and `_pods_observer: Subscription` (`cx.observe(session, |tab, _, cx| tab.note_restarts(cx))`), as workload tabs have. Workload tabs call `note_restarts` at the end of `sync_members`.
- `note_restarts` walks **every streamed (pod, container) pair of the Current instance, whatever its stream state**. A follow stream ends when its container exits, before the kubelet raises `restartCount`; walking only live streams would never show the OOMKilled row. Pairs are taken from `streams` (deduplicated; the source is the pair's latest stream, so a reopened pod keeps its newest prefix and color).
- For each pair: its pod in `live.pods` and the container's `ContainerSummary`; a higher count than seen pushes one marker with the new count (several restarts between two notifications give one line with the latest `#n`); a lower count (pod recreated under the same name) only re-baselines; a pod no longer listed is skipped.
- Markers take the stream lines' path: into `staging` while it is armed (so the merge sort places them), else `push_lines`.

Pure helpers (private, with unit tests in `log_workload.rs`):

```rust
fn restart_marker(container: &ContainerSummary, now: jiff::Timestamp) -> LogLine;
/// Containers of `pod` among `streamed` whose count rose since `seen`; updates `seen`.
fn rising_restarts(seen: &mut HashMap<(String, String), u32>, pod: &PodSummary,
    streamed: &[&str]) -> Vec<(String, u32)>;
```

## Row (`log_rows.rs`) and histogram

- A marker row: the level column always shows `SYS` (also outside JSON mode, where the column is otherwise absent) in `tone_color(StatusTone::Warn)`; the text in `theme.muted_foreground`; no error tint, no JSON block, no filter highlight.
- `LogBuffer::volume_lines` (step 3) leaves markers out: they are not log volume. Until step 3 lands, `current_volume` filters them itself.
