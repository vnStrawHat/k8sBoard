# 0004 · Log buffer (pure)

[Back to index](README.md) · Module: `crates/app/src/log_buffer.rs` (tests in `log_buffer_tests.rs`)

No GPUI types appear here. Everything is synchronous and deterministic.

## Type

```rust
const MAX_LINES: usize = 10_000;
const MAX_BYTES: usize = 8 * 1024 * 1024;     // sum of `text.len()`

pub(crate) struct LogBuffer {
    lines: VecDeque<LogLine>,
    bytes: usize,
    first_seq: u64,                // sequence number of lines[0]; increases on every push
    dropped: u64,                  // lines evicted since the last clear
    filter: Option<LineFilter>,
}
struct LineFilter { needle: String, matches: VecDeque<u64> } // seqs of matching lines, ascending

/// How the visible list changed, applied to the scroller as
/// splice(0..removed_visible, 0) and then append(added_visible).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BufferChange { pub(crate) removed_visible: usize, pub(crate) added_visible: usize }

impl LogBuffer {
    pub(crate) fn new() -> Self;
    pub(crate) fn push(&mut self, lines: Vec<LogLine>) -> BufferChange;
    pub(crate) fn clear(&mut self);                         // keeps the filter
    /// An empty or whitespace-only needle removes the filter.
    pub(crate) fn set_filter(&mut self, needle: &str);
    pub(crate) fn needle(&self) -> Option<&str>;
    pub(crate) fn visible_len(&self) -> usize;
    pub(crate) fn visible_line(&self, index: usize) -> Option<&LogLine>;
    pub(crate) fn total_len(&self) -> usize;
    pub(crate) fn has_dropped(&self) -> bool;
    pub(crate) fn visible_text(&self, shows_timestamps: bool) -> String;
}
```

## `push` algorithm

1. Remember `old_visible = visible_len()` and `first_new_seq`.
2. Append every line, giving each the next seq, and add its bytes. When a filter is set and `find_matches` is not empty, push the seq to `matches`.
3. While `lines.len() > MAX_LINES` or `bytes > MAX_BYTES`, pop the front, subtract its bytes, and increment `first_seq` and `dropped`. Pop `matches` entries whose seq is below the new `first_seq`.
4. Count the evicted visible lines:
   - those with seq below `first_new_seq` → `removed_visible`;
   - those at or above it (a batch larger than the cap) are taken out of `added_visible`.
5. Invariant (tested): `old_visible - removed_visible + added_visible == visible_len()`.

- `visible_line(i)`: with no filter, `lines[i]`. With a filter, `lines[matches[i] - first_seq]`.
- `set_filter` rebuilds `matches` with a full scan. At most 10,000 lines, on each keystroke. There is no debounce until profiling shows a need.
- `visible_text` gives one line per visible line, joined with `\n`. A line is `"{format_log_time(t)} {text}"` when `shows_timestamps` is set and the line has a timestamp, else just `text`.

## Matching

```rust
/// Non-overlapping byte ranges of `needle` in `text`, compared with ASCII case folding.
/// Other characters must match exactly, so every range falls on char boundaries.
pub(crate) fn find_matches(text: &str, needle: &str) -> Vec<Range<usize>>;
```

- Scan the bytes with `eq_ignore_ascii_case` over windows of `needle.len()`. After a hit, skip past it. A range starts only at a char boundary (`text.is_char_boundary`).
- An empty needle returns no ranges. The filter treats "no ranges" as "no match".
- ASCII folding keeps the byte offsets of the original string valid for `StyledText` highlights. Full Unicode case folding can change lengths, so it is not used.

## Time format

```rust
/// `HH:MM:SS.mmm` in UTC, e.g. 10:47:58.902 (W8 column), via `Timestamp::strftime("%H:%M:%S%.3f")`.
pub(crate) fn format_log_time(timestamp: jiff::Timestamp) -> String;
```

- The time is UTC because the workspace jiff has no time-zone features (README open item 3). The Timestamps toggle tooltip says so.
- `jiff` is already an app dependency, so nothing new is added.

## Memory bound per tab

- At most 10,000 lines and 8 MiB of text, plus the `matches` seqs (8 bytes each).
- Each line is at most 16 KiB, because the crate truncates longer ones, so a single line can never exceed the byte cap.
