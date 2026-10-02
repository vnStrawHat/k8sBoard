# 0019 · Step 1: level detection and line matcher

[Back to index](README.md) · Modules: `log_level.rs`, `line_matcher.rs` (new, pure, no GPUI types). Buffer, JSON view, toolbar, and rows: [buffer-and-rows.md](buffer-and-rows.md).

**Name note:** `crates/app/src/log_filter.rs` already exists (the tracing `EnvFilter` pin) and stays untouched. The matcher is `line_matcher.rs`.

## Levels (`log_level.rs`, inline tests)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LogLevel { Error, Warn, Info, Debug }
impl LogLevel { pub(crate) const ALL: [Self; 4]; pub(crate) fn label(self) -> &'static str; } // "ERROR" …
/// The levels the chips hide; default hides none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LevelSet { hidden: [bool; 4] }
impl LevelSet { pub(crate) fn is_hidden(self, level: LogLevel) -> bool;
    pub(crate) fn toggled(self, level: LogLevel) -> Self; pub(crate) fn hides_none(self) -> bool; }
pub(crate) fn detect_level(text: &str) -> Option<LogLevel>;
fn level_of_word(word: &str) -> Option<LogLevel>;       // ASCII case-insensitive
fn level_of_number(value: u64) -> Option<LogLevel>;     // pino/bunyan scale
```

| Words (`level_of_word`) | Numbers (`level_of_number`) | Level |
|---|---|---|
| error, err, fatal, panic, crit, critical, alert, emerg | 50, 60 | Error |
| warn, warning | 40 | Warn |
| info, notice | 30 | Info |
| debug, trace | 10, 20 | Debug |

Other numbers map to `None`.

| Order | Form | Example | Rule |
|---|---|---|---|
| 1 | JSON object | `{"level":"error",…}`, `{"level":50,…}` | trimmed text starts with `{` and ends with `}`; parse as `serde_json::Map`; the first key present among `level`, `severity`, `lvl`, `log.level`: a string → `level_of_word`, an unsigned integer → `level_of_number`, anything else → `None` |
| 2 | logfmt | `ts=… level=warn msg=…` | a token `level=`, `lvl=`, or `severity=` at the start or after a space; value up to the next space, `"` trimmed |
| 3 | klog | `E0501 10:47:58.902 1 x.go:12] …` | byte 0 in `IWEF`, bytes 1–4 ASCII digits, byte 5 a space; `F` → Error |
| 4 | keyword | `… ERROR [main] …`, `[error] …`, `\tinfo\t` | within the first 64 bytes (cut at a char boundary), split into words of ASCII alphanumerics **and `_`** (so `ERROR_COUNT`, `no_error` are one word and never match); the first word that maps AND is all-uppercase, or is enclosed in `[`…`]`, or is tab-delimited on both sides |
| — | none | | `None` |

Continuation (applied in the buffer): a line with `None` that starts with a space or tab takes the previous line's level **of the same source**.

## Matcher (`line_matcher.rs`, tests in `line_matcher_tests.rs`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FilterMode { Plain, Regex }
pub(crate) enum LineMatcher { Plain(String), Regex(regex::Regex) }
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct InvalidRegex; // Display: "Invalid regex"
const REGEX_SIZE_LIMIT: usize = 1 << 20;
impl LineMatcher {
    /// `Ok(None)` for empty or whitespace-only text.
    pub(crate) fn parse(text: &str, mode: FilterMode) -> Result<Option<Self>, InvalidRegex>;
    pub(crate) fn is_match(&self, text: &str) -> bool;
    /// Non-empty, non-overlapping byte ranges (char boundaries) for highlights.
    pub(crate) fn ranges(&self, text: &str) -> Vec<Range<usize>>;
    pub(crate) fn pattern(&self) -> &str;              // for `No lines match "…"`
}
pub(crate) fn find_matches(text: &str, needle: &str) -> Vec<Range<usize>>; // moved unchanged from log_buffer.rs
```

- Regex: `RegexBuilder::new(text).case_insensitive(true).size_limit(REGEX_SIZE_LIMIT).build()`. `is_match` uses `Regex::is_match` (a pattern like `x?` matches every line); `ranges` skips empty matches.
- Plain: unchanged 0004 semantics (`find_matches`, ASCII case folding).
- The `regex` engine runs in linear time, so no pattern can hang the main thread; the size limit bounds compile memory.
