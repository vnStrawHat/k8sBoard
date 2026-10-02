use std::future::Future;
use std::io;
use std::pin::Pin;

use futures::future::Either;
use futures::io::{AsyncBufRead, AsyncBufReadExt};
use futures::{Stream, StreamExt};
use k8s_openapi::api::core::v1::Pod;
use kube::Api;
use kube::api::LogParams;
use tokio::time::Instant;

use crate::connection::{ClusterConnection, ClusterError};
use crate::resource_watch::{BATCH_WINDOW, wait_until};

const LOG_ACTION: &str = "streaming pod logs";
/// Longer lines are cut, so one line can never exceed the app's per-tab byte cap.
const MAX_LINE_BYTES: usize = 16 * 1024;
const TRUNCATION_MARKER: &str = " … [truncated]";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogRequest {
    pub namespace: String,
    pub pod: String,
    pub container: String,
    pub source: LogSource,
    /// Lines of history requested at open.
    pub tail_lines: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogSource {
    /// The running instance: the requested tail, then follow.
    Current,
    /// The last terminated instance (`previous=true`): the requested tail, then end.
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
    ///
    /// Items arrive as `Started` or `Failed`, then any number of `Lines`, then optionally
    /// `Failed`. A stream that ends without `Failed` was closed by the server.
    pub fn pod_logs(&self, request: LogRequest) -> impl Stream<Item = LogUpdate> + Send + 'static {
        let params = log_params(&request);
        let api: Api<Pod> = Api::namespaced(self.client().clone(), &request.namespace);
        let connection = self.clone();
        let open = async move {
            let reader = connection
                .run(LOG_ACTION, api.log_stream(&request.pod, &params))
                .await?;
            Ok(read_chunks(Box::pin(reader)))
        };
        log_updates(open, self.context().to_owned())
    }
}

fn log_params(request: &LogRequest) -> LogParams {
    LogParams {
        container: Some(request.container.clone()),
        follow: request.source == LogSource::Current,
        previous: request.source == LogSource::Previous,
        tail_lines: Some(i64::from(request.tail_lines)),
        timestamps: true,
        ..LogParams::default()
    }
}

/// Turns a buffered reader into raw chunks. An empty buffer means the end.
fn read_chunks<R>(reader: Pin<Box<R>>) -> impl Stream<Item = io::Result<Vec<u8>>> + Send + 'static
where
    R: AsyncBufRead + Send + 'static,
{
    futures::stream::unfold(reader, |mut reader| async move {
        let chunk = match reader.fill_buf().await {
            Ok([]) => return None,
            Ok(buffer) => buffer.to_vec(),
            Err(error) => return Some((Err(error), reader)),
        };
        reader.as_mut().consume(chunk.len());
        Some((Ok(chunk), reader))
    })
}

/// The testable core: any open step and any chunk source work, including fakes.
fn log_updates<O, S>(open: O, context: String) -> impl Stream<Item = LogUpdate> + Send + 'static
where
    O: Future<Output = Result<S, ClusterError>> + Send + 'static,
    S: Stream<Item = io::Result<Vec<u8>>> + Send + 'static,
{
    futures::stream::once(async move {
        match open.await {
            Err(error) => Either::Left(futures::stream::iter([LogUpdate::Failed(error)])),
            Ok(chunks) => Either::Right(
                futures::stream::iter([LogUpdate::Started]).chain(batch_lines(chunks, context)),
            ),
        }
    })
    .flatten()
}

fn batch_lines<S>(chunks: S, context: String) -> impl Stream<Item = LogUpdate> + Send + 'static
where
    S: Stream<Item = io::Result<Vec<u8>>> + Send + 'static,
{
    let batcher = LineBatcher {
        chunks: Box::pin(chunks),
        context,
        splitter: LineSplitter::default(),
        pending: Vec::new(),
        deadline: None,
        failure: None,
        is_source_ended: false,
    };
    futures::stream::unfold(batcher, |mut batcher| async move {
        let update = batcher.next_update().await?;
        Some((update, batcher))
    })
}

struct LineBatcher<S> {
    chunks: Pin<Box<S>>,
    context: String,
    splitter: LineSplitter,
    pending: Vec<LogLine>,
    deadline: Option<Instant>,
    /// The `Failed` that follows the final flush.
    failure: Option<LogUpdate>,
    is_source_ended: bool,
}

impl<S: Stream<Item = io::Result<Vec<u8>>>> LineBatcher<S> {
    async fn next_update(&mut self) -> Option<LogUpdate> {
        loop {
            if self.is_source_ended {
                return self.flush().or_else(|| self.failure.take());
            }
            // Cancel-safe: the deadline lives in `self`, and `next()` loses nothing when
            // its future is dropped.
            tokio::select! {
                biased;
                () = wait_until(self.deadline) => {
                    if let Some(update) = self.flush() {
                        return Some(update);
                    }
                }
                item = self.chunks.next() => match item {
                    Some(Ok(chunk)) => self.handle_chunk(&chunk),
                    Some(Err(error)) => self.end_source(Some(error)),
                    None => self.end_source(None),
                },
            }
        }
    }

    fn handle_chunk(&mut self, chunk: &[u8]) {
        self.splitter.push(chunk, &mut self.pending);
        if !self.pending.is_empty() && self.deadline.is_none() {
            self.deadline = Some(Instant::now() + BATCH_WINDOW);
        }
    }

    fn end_source(&mut self, error: Option<io::Error>) {
        self.pending
            .extend(std::mem::take(&mut self.splitter).finish());
        self.failure = error.map(|error| {
            LogUpdate::Failed(ClusterError::Unreachable {
                context: self.context.clone(),
                action: LOG_ACTION,
                source: Box::new(error),
            })
        });
        self.is_source_ended = true;
    }

    fn flush(&mut self) -> Option<LogUpdate> {
        self.deadline = None;
        if self.pending.is_empty() {
            return None;
        }
        Some(LogUpdate::Lines(std::mem::take(&mut self.pending)))
    }
}

#[derive(Default)]
struct LineSplitter {
    partial: Vec<u8>,
    is_truncating: bool,
}

impl LineSplitter {
    fn push(&mut self, chunk: &[u8], lines: &mut Vec<LogLine>) {
        let mut rest = chunk;
        while let Some(end) = rest.iter().position(|byte| *byte == b'\n') {
            self.append(&rest[..end]);
            lines.push(self.take_line());
            rest = &rest[end + 1..];
        }
        self.append(rest);
    }

    /// The last line when it has no trailing `\n`. An empty one is not emitted.
    fn finish(mut self) -> Option<LogLine> {
        if self.partial.is_empty() {
            return None;
        }
        Some(self.take_line())
    }

    fn append(&mut self, bytes: &[u8]) {
        if self.is_truncating {
            return;
        }
        let room = MAX_LINE_BYTES - self.partial.len();
        if bytes.len() > room {
            self.is_truncating = true;
        }
        self.partial
            .extend_from_slice(&bytes[..bytes.len().min(room)]);
    }

    fn take_line(&mut self) -> LogLine {
        let raw = std::mem::take(&mut self.partial);
        parse_log_line(&raw, std::mem::take(&mut self.is_truncating))
    }
}

/// Splits the kubelet's RFC 3339 prefix off one raw line.
fn parse_log_line(raw: &[u8], is_truncated: bool) -> LogLine {
    let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
    let text = String::from_utf8_lossy(raw);
    let parsed = text
        .split_once(' ')
        .and_then(|(prefix, rest)| Some((prefix.parse::<jiff::Timestamp>().ok()?, rest)));
    let (timestamp, body) = match parsed {
        Some((timestamp, rest)) => (Some(timestamp), rest),
        None => (None, text.as_ref()),
    };
    let mut text = body.to_owned();
    if is_truncated {
        text.push_str(TRUNCATION_MARKER);
    }
    LogLine { timestamp, text }
}

#[cfg(test)]
#[path = "pod_log_tests.rs"]
mod pod_log_tests;
