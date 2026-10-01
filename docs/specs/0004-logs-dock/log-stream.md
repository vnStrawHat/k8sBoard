# 0004 · Cluster crate: pod log stream

[Back to index](README.md) · Module: `src/pod_log.rs` (tests in `src/pod_log_tests.rs`)

## Public API

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogRequest { pub namespace: String, pub pod: String, pub container: String, pub source: LogSource }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogSource {
    /// The running instance: the last 1,000 lines, then follow.
    Current,
    /// The last terminated instance (`previous=true`): the last 1,000 lines, then end.
    Previous,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// The kubelet timestamp, or `None` when the line has no parsable prefix.
    pub timestamp: Option<jiff::Timestamp>,
    pub text: String,
}

#[derive(Debug)]
pub enum LogUpdate {
    /// The API server accepted the request. Sent once, before any line.
    Started,
    /// New lines in arrival order, at most one batch per 100 ms.
    Lines(Vec<LogLine>),
    /// The stream failed. Nothing follows.
    Failed(ClusterError),
}

impl ClusterConnection {
    /// action: "streaming pod logs". Nothing happens until the stream is polled, and it must
    /// be polled on the app's tokio runtime (the batch timer is a tokio sleep).
    pub fn pod_logs(&self, request: LogRequest) -> impl Stream<Item = LogUpdate> + Send + 'static;
}
```

- Item order: `Started` or `Failed` comes first, then any number of `Lines`, then optionally `Failed`, then the end. If the stream ends without `Failed`, the server closed the body: the container stopped or restarted, the previous log is complete, or the pod is gone.
- `lib.rs`: `pub use pod_log::{LogLine, LogRequest, LogSource, LogUpdate};`. Every other item is private.

## Request mapping (private, pure: `fn log_params(request: &LogRequest) -> LogParams`)

| `LogParams` field | `Current` | `Previous` |
|---|---|---|
| `container` | `Some(container)` | same |
| `follow` | true | false |
| `previous` | false | true |
| `tail_lines` | `Some(1000)` (`LOG_TAIL_LINES`) | `Some(1000)` |
| `timestamps` | true | true |

The other fields (`since_seconds`, `since_time`, `limit_bytes`, `pretty`) keep their defaults. There is no resume: Reconnect starts over ([log-tab.md](log-tab.md)).

- The API is `Api::<Pod>::namespaced(client, namespace).log_stream(pod, &params)`. It is a plain HTTP GET with a chunked body, and it is not behind `ws` (kube-client 4.2 `api/subresource.rs`).

## Pipeline

```text
open    connection.run(action, api.log_stream(..))   // REQUEST_TIMEOUT (30 s) covers the response head only
chunks  futures::stream::unfold(reader, fill_buf -> to_vec -> consume)  // Stream<Item = io::Result<Vec<u8>>>
lines   LineSplitter::push -> parse_log_line
batch   100 ms window (resource_watch::BATCH_WINDOW, made pub(crate)), tokio::select! like batch_updates
```

The testable core gets the open step as a future, so tests can use fakes:

```rust
fn log_updates<O, S>(open: O, context: String) -> impl Stream<Item = LogUpdate> + Send + 'static
where O: Future<Output = Result<S, ClusterError>> + Send + 'static,
      S: Stream<Item = std::io::Result<Vec<u8>>> + Send + 'static;
```

- Do not use `AsyncBufReadExt::lines`: it fails the whole stream on invalid UTF-8, and it has no length cap.
- There is no read timeout (0001 keeps it off for watches). A quiet container simply sends nothing.

## Line splitting (private, sync, pure)

```rust
const MAX_LINE_BYTES: usize = 16 * 1024;
struct LineSplitter { partial: Vec<u8>, is_truncating: bool }
impl LineSplitter {
    fn push(&mut self, chunk: &[u8], lines: &mut Vec<LogLine>);
    fn finish(self) -> Option<LogLine>;           // the last line when it has no trailing '\n'
}
fn parse_log_line(raw: &[u8], is_truncated: bool) -> LogLine;
```

- Split on `\n`, and strip one trailing `\r`. A line can span any number of chunks.
- When `partial` reaches `MAX_LINE_BYTES`, set `is_truncating` and skip bytes up to the next `\n`. The emitted text ends with ` … [truncated]`.
- Decode with `String::from_utf8_lossy`. A cut in the middle of a character becomes one U+FFFD.
- `parse_log_line` splits at the first space and parses the prefix as a `jiff::Timestamp` (RFC 3339 with nanoseconds, as the kubelet writes it). If parsing fails, `timestamp` is `None` and `text` is the whole line.
- An empty final partial line is not emitted.

## Batching (`log_updates`)

1. Await `open`. On `Err(e)`, emit `Failed(e)` and end. On `Ok`, emit `Started`.
2. `select!` on the next chunk and on `sleep_until(deadline)`. The sleep is armed only while a deadline is set.
3. `Ok(chunk)`: `push` the chunk into `pending`. If `pending` is not empty and there is no deadline, set `deadline = now + BATCH_WINDOW`.
4. When the deadline fires: emit `Lines(take(pending))` and clear the deadline.
5. `Err(io)`: flush `pending` as `Lines` (if any), then emit `Failed(Unreachable { context, action, source: Box::new(io) })`, then end.
6. Source end: add `splitter.finish()` to `pending`, flush it, then end.

## Errors at open (`classify_error`, unchanged)

| Case | HTTP | `ClusterError` |
|---|---|---|
| RBAC denies `get pods/log` | 403 | `Forbidden` |
| Container still waiting (ContainerCreating, CrashLoopBackOff, image pull) | 400 | `Api { code: 400, message }`, e.g. `container "api" in pod "x" is waiting to start: …` |
| `Previous` with no terminated instance | 400 | `Api { code: 400, message: "previous terminated container … not found" }` |
| Pod or container gone | 404 | `Api { code: 404, .. }` |
| Response head slower than 30 s | — | `TimedOut` |

## Cancellation, memory, security

- Dropping the stream drops the reader and closes the HTTP connection. The crate spawns nothing.
- Memory per stream: one partial line (at most 16 KiB) plus one 100 ms window of lines. A slow consumer stops the polling, so TCP back-pressure holds the rest.
- No `tracing` call receives chunk bytes, `LogLine`, or `LogUpdate`. Only `run`'s existing debug event (context and action) is logged.
