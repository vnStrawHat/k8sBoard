# 0044 · Step 2: `SYS` marker lines

[Back to index](README.md) · Modules: `log_buffer.rs` (+ `log_buffer_tests.rs`), `log_tab.rs`, `log_workload.rs`, `log_rows.rs`, `log_tab` volume input. Decisions 7–11. Wireframe: W8 and W8b row `SYS ── container api terminated: OOMKilled (exit 137) · restart #14 ──`; 0019 follow-up.

## Lines

| When | Tabs | Text | Time | Source (prefix, color) |
|---|---|---|---|---|
| A streamed container's `restart_count` rises | pod, workload | `── container {c} terminated: {reason} (exit {code}) · restart #{n} ──`; without `last_termination`: `── container {c} restarted · restart #{n} ──` | `last_termination.finished_at`, else now | the stream of that pod and container |
| A pod joins the members, after the first sync | workload | `── pod {short} joined ──` | now | the pod's first new stream |
| A pod leaves the members | workload | `── pod {short} left ──` | now | a stream of that pod |

- `{reason}`: `StatusReason`'s `Display` (`OOMKilled`, `Error`, `Completed`); missing reason → `terminated (exit {code})` without the colon part. `{code}`: `Termination::exit_code`. Nothing else of the pod is read (no message; C1).
- `{short}`: `pod_short_name(&owner, pod)`, the prefix the tab already shows.
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
- `note_restarts`: for each live stream of the Current instance, read its pod in `live.pods` and the streamed container's `ContainerSummary`; a higher count than seen pushes one marker with the new count (several restarts between two notifications give one line with the latest `#n`); a lower count (pod recreated under the same name) only re-baselines.
- `sync_members`: `membership_markers(&plan.change, &owner, !has_synced, now)`; `has_synced: bool` is set after the first sync and reset by `restart_stream`. A joiner's marker uses its first new stream as source, a leaver's its existing stream. Frozen membership (`is_frozen`) marks nothing. A pod admitted later because a slot freed (`member_change` with the pod limit) also reads `joined`: it joined the tab.
- Markers take the stream lines' path: into `staging` while it is armed (so the merge sort places them), else `push_lines`.

Pure helpers (private, with unit tests in `log_workload.rs`, next to `plan_sync`):

```rust
fn restart_marker(container: &ContainerSummary, now: jiff::Timestamp) -> LogLine;
/// `(pod, line)` per joiner (none on the first sync) and per leaver of `change`.
fn membership_markers(change: &MemberChange, owner: &PodOwner, is_first_sync: bool,
    now: jiff::Timestamp) -> Vec<(String, LogLine)>;
/// Containers of `pod` among `streamed` whose count rose since `seen`; updates `seen`.
fn rising_restarts(seen: &mut HashMap<(String, String), u32>, pod: &PodSummary,
    streamed: &[&str]) -> Vec<(String, u32)>;
```

## Row (`log_rows.rs`) and histogram

- A marker row: the level column always shows `SYS` (also outside JSON mode, where the column is otherwise absent) in `tone_color(StatusTone::Warn)`; the text in `theme.muted_foreground`; no error tint, no JSON block, no filter highlight.
- `current_volume` (the `volume` input in `log_tab.rs`) skips markers: they are not log volume.
