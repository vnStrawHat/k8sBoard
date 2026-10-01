use super::*;

fn line(text: &str) -> LogLine {
    LogLine {
        timestamp: None,
        text: text.to_owned(),
    }
}

fn lines(texts: &[&str]) -> Vec<LogLine> {
    texts.iter().map(|text| line(text)).collect()
}

fn visible(buffer: &LogBuffer) -> Vec<String> {
    (0..buffer.visible_len())
        .filter_map(|index| buffer.visible_line(index))
        .map(|line| line.text.clone())
        .collect()
}

/// Pushes `batch` and checks the invariant from the spec.
fn push_checked(buffer: &mut LogBuffer, batch: Vec<LogLine>) -> BufferChange {
    let old_visible = buffer.visible_len();
    let change = buffer.push(batch);
    assert_eq!(
        old_visible - change.removed_visible + change.added_visible,
        buffer.visible_len()
    );
    change
}

#[test]
fn push_appends_in_order() {
    let mut buffer = LogBuffer::new();
    push_checked(&mut buffer, lines(&["a", "b"]));
    push_checked(&mut buffer, lines(&["c"]));
    assert_eq!(visible(&buffer), ["a", "b", "c"]);
    assert_eq!(buffer.total_len(), 3);
    assert!(!buffer.has_dropped());
}

#[test]
fn push_evicts_oldest_over_line_cap() {
    let mut buffer = LogBuffer::new();
    let batch: Vec<_> = (0..MAX_LINES)
        .map(|index| line(&index.to_string()))
        .collect();
    push_checked(&mut buffer, batch);
    let change = push_checked(&mut buffer, lines(&["x", "y"]));
    assert_eq!(
        change,
        BufferChange {
            removed_visible: 2,
            added_visible: 2
        }
    );
    assert_eq!(buffer.total_len(), MAX_LINES);
    assert!(buffer.has_dropped());
    assert_eq!(
        buffer.visible_line(0).map(|line| line.text.as_str()),
        Some("2")
    );
}

#[test]
fn push_evicts_oldest_over_byte_cap() {
    let mut buffer = LogBuffer::new();
    let megabyte = "a".repeat(1024 * 1024);
    for _ in 0..9 {
        push_checked(&mut buffer, vec![line(&megabyte)]);
    }
    assert_eq!(buffer.total_len(), 8);
    assert!(buffer.has_dropped());
}

#[test]
fn change_counts_match_visible_len_without_filter() {
    let mut buffer = LogBuffer::new();
    let batch: Vec<_> = (0..MAX_LINES + 3)
        .map(|index| line(&index.to_string()))
        .collect();
    push_checked(&mut buffer, batch);
    push_checked(&mut buffer, lines(&["a", "b", "c", "d"]));
    push_checked(&mut buffer, Vec::new());
}

#[test]
fn change_counts_match_visible_len_with_filter() {
    let mut buffer = LogBuffer::new();
    buffer.set_filter("err");
    let batch: Vec<_> = (0..MAX_LINES + 10)
        .map(|index| line(if index % 2 == 0 { "error" } else { "ok" }))
        .collect();
    push_checked(&mut buffer, batch);
    let change = push_checked(&mut buffer, lines(&["ok", "err", "ok", "err"]));
    assert_eq!(change.added_visible, 2);
    assert_eq!(change.removed_visible, 2);
    assert_eq!(buffer.visible_len(), MAX_LINES / 2);
}

#[test]
fn batch_larger_than_cap_keeps_newest() {
    let mut buffer = LogBuffer::new();
    let batch: Vec<_> = (0..MAX_LINES + 5)
        .map(|index| line(&index.to_string()))
        .collect();
    let change = push_checked(&mut buffer, batch);
    assert_eq!(change.added_visible, MAX_LINES);
    assert_eq!(change.removed_visible, 0);
    assert_eq!(
        buffer.visible_line(0).map(|line| line.text.as_str()),
        Some("5")
    );
}

#[test]
fn filter_keeps_only_matching_lines_ignoring_ascii_case() {
    let mut buffer = LogBuffer::new();
    push_checked(&mut buffer, lines(&["an error", "fine", "ERROR again"]));
    buffer.set_filter("error");
    assert_eq!(visible(&buffer), ["an error", "ERROR again"]);
    assert_eq!(buffer.needle(), Some("error"));
    assert_eq!(buffer.total_len(), 3);
}

#[test]
fn empty_needle_removes_filter() {
    let mut buffer = LogBuffer::new();
    push_checked(&mut buffer, lines(&["a", "b"]));
    buffer.set_filter("a");
    assert_eq!(buffer.visible_len(), 1);
    buffer.set_filter("");
    assert_eq!(buffer.visible_len(), 2);
    buffer.set_filter("a");
    buffer.set_filter("   ");
    assert_eq!(buffer.visible_len(), 2);
    assert_eq!(buffer.needle(), None);
}

#[test]
fn filter_applies_to_new_lines() {
    let mut buffer = LogBuffer::new();
    buffer.set_filter("x");
    push_checked(&mut buffer, lines(&["ax", "b", "xc"]));
    assert_eq!(visible(&buffer), ["ax", "xc"]);
}

#[test]
fn eviction_removes_evicted_matches() {
    let mut buffer = LogBuffer::new();
    buffer.set_filter("hit");
    let batch: Vec<_> = (0..MAX_LINES)
        .map(|index| line(&format!("hit {index}")))
        .collect();
    push_checked(&mut buffer, batch);
    push_checked(&mut buffer, lines(&["miss", "hit last"]));
    assert_eq!(buffer.visible_len(), MAX_LINES - 1);
    for index in 0..buffer.visible_len() {
        assert!(buffer.visible_line(index).is_some());
    }
    assert_eq!(
        buffer.visible_line(0).map(|line| line.text.as_str()),
        Some("hit 2")
    );
    assert!(buffer.visible_line(buffer.visible_len()).is_none());
}

#[test]
fn clear_keeps_filter_and_resets_lines_and_dropped() {
    let mut buffer = LogBuffer::new();
    buffer.set_filter("a");
    let batch: Vec<_> = (0..MAX_LINES + 1).map(|_| line("a")).collect();
    push_checked(&mut buffer, batch);
    buffer.clear();
    assert_eq!(buffer.total_len(), 0);
    assert_eq!(buffer.visible_len(), 0);
    assert!(!buffer.has_dropped());
    assert_eq!(buffer.needle(), Some("a"));
    push_checked(&mut buffer, lines(&["a", "b"]));
    assert_eq!(visible(&buffer), ["a"]);
}

#[test]
fn visible_text_joins_lines_with_optional_timestamps() {
    let stamped = LogLine {
        timestamp: "2024-05-01T10:47:58.902345678Z".parse().ok(),
        text: "hello".to_owned(),
    };
    let mut buffer = LogBuffer::new();
    push_checked(&mut buffer, vec![stamped, line("plain")]);
    assert_eq!(buffer.visible_text(true), "10:47:58.902 hello\nplain");
    assert_eq!(buffer.visible_text(false), "hello\nplain");
}

#[test]
fn find_matches_returns_non_overlapping_ranges() {
    let ranges = find_matches("aaa", "aa");
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges.first(), Some(&(0..2)));
    assert_eq!(find_matches("a-A-a", "a"), [0..1, 2..3, 4..5]);
    assert!(find_matches("abc", "").is_empty());
}

#[test]
fn find_matches_on_non_ascii_text_keeps_char_boundaries() {
    let text = "Ärger error";
    let ranges = find_matches(text, "error");
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges.first(), Some(&(7..12)));
    assert!(
        ranges
            .iter()
            .all(|range| text.is_char_boundary(range.start))
    );
    assert!(find_matches("Ärger", "ärger").is_empty());
}

#[test]
fn format_log_time_is_utc_with_millis() {
    let timestamp: jiff::Timestamp = "2024-05-01T10:47:58.902345678Z".parse().expect("valid");
    assert_eq!(format_log_time(timestamp), "10:47:58.902");
}
