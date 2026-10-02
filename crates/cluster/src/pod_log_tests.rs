use std::pin::pin;
use std::time::Duration;

use futures::channel::mpsc;
use futures::stream;

use super::*;

type Chunk = io::Result<Vec<u8>>;

fn request(source: LogSource) -> LogRequest {
    LogRequest {
        namespace: "ns".to_owned(),
        pod: "pod".to_owned(),
        container: "app".to_owned(),
        source,
        tail_lines: 1000,
    }
}

fn forbidden() -> ClusterError {
    ClusterError::Forbidden {
        context: "test".to_owned(),
        action: LOG_ACTION,
        message: "scripted".to_owned(),
    }
}

fn text(line: &str) -> Chunk {
    Ok(format!("{line}\n").into_bytes())
}

fn opened(
    chunks: impl Stream<Item = Chunk> + Send + 'static,
) -> impl Stream<Item = LogUpdate> + Send + 'static {
    log_updates(std::future::ready(Ok(chunks)), "test".to_owned())
}

/// The next update, or `None` when the stream stays silent for a long (virtual) time.
async fn next_update<S>(stream: &mut S) -> Option<LogUpdate>
where
    S: Stream<Item = LogUpdate> + Unpin,
{
    tokio::time::timeout(Duration::from_secs(10), stream.next())
        .await
        .ok()
        .flatten()
}

fn texts(update: &LogUpdate) -> Vec<&str> {
    match update {
        LogUpdate::Lines(lines) => lines.iter().map(|line| line.text.as_str()).collect(),
        other => panic!("expected lines, got {other:?}"),
    }
}

fn split(chunks: &[&[u8]]) -> (Vec<LogLine>, LineSplitter) {
    let mut splitter = LineSplitter::default();
    let mut lines = Vec::new();
    for chunk in chunks {
        splitter.push(chunk, &mut lines);
    }
    (lines, splitter)
}

// Request mapping

#[test]
fn log_params_current_follows_with_tail_and_timestamps() {
    let params = log_params(&request(LogSource::Current));
    assert_eq!(params.container.as_deref(), Some("app"));
    assert!(params.follow);
    assert!(!params.previous);
    assert_eq!(params.tail_lines, Some(1000));
    assert!(params.timestamps);
}

#[test]
fn log_params_previous_does_not_follow() {
    let params = log_params(&request(LogSource::Previous));
    assert!(params.previous);
    assert!(!params.follow);
    assert_eq!(params.tail_lines, Some(1000));
    assert!(params.timestamps);
}

#[test]
fn log_params_uses_requested_tail() {
    let params = log_params(&LogRequest {
        tail_lines: 50,
        ..request(LogSource::Current)
    });
    assert_eq!(params.tail_lines, Some(50));
}

// Splitter

#[test]
fn splitter_joins_line_split_across_chunks() {
    let (lines, splitter) = split(&[b"ab", b"c\nd"]);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].text, "abc");
    assert_eq!(splitter.partial, b"d");
}

#[test]
fn splitter_strips_carriage_return() {
    let (lines, _) = split(&[b"x\r\n"]);
    assert_eq!(lines[0].text, "x");
}

#[test]
fn splitter_replaces_invalid_utf8() {
    let (lines, _) = split(&[&[0x66, 0xff, b'\n']]);
    assert_eq!(lines[0].text, "f\u{FFFD}");
}

#[test]
fn splitter_truncates_overlong_line_and_resumes_at_newline() {
    let long = vec![b'a'; 20 * 1024];
    let (lines, _) = split(&[&long, b"\nok\n"]);
    assert_eq!(lines.len(), 2);
    assert_eq!(
        lines[0].text,
        format!("{}{TRUNCATION_MARKER}", "a".repeat(MAX_LINE_BYTES))
    );
    assert_eq!(lines[1].text, "ok");
}

#[test]
fn splitter_keeps_line_of_exactly_the_cap() {
    let exact = vec![b'a'; MAX_LINE_BYTES];
    let (lines, _) = split(&[&exact, b"\n"]);
    assert_eq!(lines[0].text.len(), MAX_LINE_BYTES);
}

#[test]
fn splitter_finish_flushes_partial_last_line() {
    let (_, splitter) = split(&[b"tail"]);
    assert_eq!(
        splitter.finish().map(|line| line.text).as_deref(),
        Some("tail")
    );
}

#[test]
fn splitter_finish_skips_empty_partial() {
    let (_, splitter) = split(&[b"done\n"]);
    assert!(splitter.finish().is_none());
}

// Line parsing

#[test]
fn parse_line_splits_rfc3339_nano_prefix() {
    let line = parse_log_line(b"2024-05-01T10:47:58.902345678Z hello", false);
    let expected: jiff::Timestamp = "2024-05-01T10:47:58.902345678Z".parse().expect("valid");
    assert_eq!(line.timestamp, Some(expected));
    assert_eq!(line.text, "hello");
}

#[test]
fn parse_line_without_timestamp_keeps_whole_text() {
    let line = parse_log_line(b"hello world", false);
    assert_eq!(line.timestamp, None);
    assert_eq!(line.text, "hello world");
}

// Stream pipeline

#[tokio::test(start_paused = true)]
async fn open_failure_emits_only_failed_then_ends() {
    let open = std::future::ready(Err::<stream::Empty<Chunk>, _>(forbidden()));
    let updates: Vec<_> = log_updates(open, "test".to_owned()).collect().await;
    assert!(matches!(
        updates.as_slice(),
        [LogUpdate::Failed(ClusterError::Forbidden { .. })]
    ));
}

#[tokio::test(start_paused = true)]
async fn open_success_emits_started_first() {
    let updates: Vec<_> = opened(stream::iter([text("a")])).collect().await;
    assert!(matches!(updates[0], LogUpdate::Started));
    assert_eq!(texts(&updates[1]), ["a"]);
}

#[tokio::test(start_paused = true)]
async fn burst_within_window_emits_one_batch() {
    let lines: Vec<_> = (0..50)
        .map(|index| text(&format!("line-{index}")))
        .collect();
    let mut stream = pin!(opened(stream::iter(lines).chain(stream::pending())));
    assert!(matches!(
        next_update(&mut stream).await,
        Some(LogUpdate::Started)
    ));
    let batch = next_update(&mut stream).await.expect("one batch");
    let texts = texts(&batch);
    assert_eq!(texts.len(), 50);
    assert_eq!(texts[0], "line-0");
    assert_eq!(texts[49], "line-49");
    assert!(next_update(&mut stream).await.is_none());
}

#[tokio::test(start_paused = true)]
async fn separate_windows_emit_separate_batches() {
    let (sender, receiver) = mpsc::unbounded::<Chunk>();
    let mut stream = pin!(opened(receiver));
    assert!(matches!(
        next_update(&mut stream).await,
        Some(LogUpdate::Started)
    ));

    sender.unbounded_send(text("a")).expect("receiver is alive");
    let first = next_update(&mut stream).await.expect("first batch");
    assert_eq!(texts(&first), ["a"]);

    tokio::time::advance(Duration::from_millis(150)).await;
    sender.unbounded_send(text("b")).expect("receiver is alive");
    let second = next_update(&mut stream).await.expect("second batch");
    assert_eq!(texts(&second), ["b"]);
}

#[tokio::test(start_paused = true)]
async fn chunk_without_newline_emits_nothing_until_end() {
    let (sender, receiver) = mpsc::unbounded::<Chunk>();
    let mut stream = pin!(opened(receiver));
    assert!(matches!(
        next_update(&mut stream).await,
        Some(LogUpdate::Started)
    ));

    sender
        .unbounded_send(Ok(b"partial".to_vec()))
        .expect("receiver is alive");
    assert!(next_update(&mut stream).await.is_none());

    drop(sender);
    let flushed = next_update(&mut stream).await.expect("the partial line");
    assert_eq!(texts(&flushed), ["partial"]);
}

#[tokio::test(start_paused = true)]
async fn io_error_flushes_lines_then_fails_and_ends() {
    let chunks = stream::iter([text("a"), Err(io::Error::other("reset"))]);
    let updates: Vec<_> = opened(chunks).collect().await;
    assert_eq!(updates.len(), 3);
    assert!(matches!(updates[0], LogUpdate::Started));
    assert_eq!(texts(&updates[1]), ["a"]);
    assert!(matches!(
        updates[2],
        LogUpdate::Failed(ClusterError::Unreachable { .. })
    ));
}

#[tokio::test(start_paused = true)]
async fn source_end_flushes_partial_line_and_ends() {
    let chunks = stream::iter([text("a"), Ok(b"b".to_vec())]);
    let updates: Vec<_> = opened(chunks).collect().await;
    assert_eq!(updates.len(), 2);
    assert_eq!(texts(&updates[1]), ["a", "b"]);
}

#[tokio::test(start_paused = true)]
async fn dropping_stream_drops_source() {
    let (sender, receiver) = mpsc::unbounded::<Chunk>();
    let mut stream = Box::pin(opened(receiver));
    assert!(matches!(
        next_update(&mut stream).await,
        Some(LogUpdate::Started)
    ));
    assert!(!sender.is_closed());
    drop(stream);
    assert!(sender.is_closed());
}
