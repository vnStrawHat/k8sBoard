use super::*;
use crate::line_matcher::FilterMode;

fn sourced(source: u16, text: &str) -> SourcedLine {
    SourcedLine {
        source: SourceId(source),
        kind: LineKind::Log,
        line: LogLine {
            timestamp: None,
            text: text.to_owned(),
        },
    }
}

fn line(text: &str) -> SourcedLine {
    sourced(0, text)
}

fn lines(texts: &[&str]) -> Vec<SourcedLine> {
    texts.iter().map(|text| line(text)).collect()
}

fn visible(buffer: &LogBuffer) -> Vec<String> {
    buffer
        .visible_lines()
        .map(|buffered| buffered.line.text.clone())
        .collect()
}

fn first_visible_text(buffer: &LogBuffer) -> Option<&str> {
    buffer
        .visible_line(0)
        .map(|buffered| buffered.line.text.as_str())
}

fn plain_view(text: &str) -> LineView {
    LineView {
        matcher: LineMatcher::parse(text, FilterMode::Plain).expect("plain never fails"),
        ..LineView::default()
    }
}

/// Pushes `batch` and checks the invariant from the spec.
fn push_checked(buffer: &mut LogBuffer, batch: Vec<SourcedLine>) -> BufferChange {
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
    assert_eq!(first_visible_text(&buffer), Some("2"));
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
    buffer.set_view(plain_view("err"));
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
fn change_counts_match_visible_len_with_levels_hidden() {
    let mut buffer = LogBuffer::new();
    buffer.set_view(LineView {
        hidden_levels: LevelSet::default().toggled(LogLevel::Debug),
        ..LineView::default()
    });
    let batch: Vec<_> = (0..MAX_LINES + 10)
        .map(|index| line(if index % 2 == 0 { "ERROR x" } else { "DEBUG x" }))
        .collect();
    push_checked(&mut buffer, batch);
    let change = push_checked(&mut buffer, lines(&["DEBUG y", "ERROR y", "DEBUG z"]));
    assert_eq!(change.added_visible, 1);
    assert_eq!(change.removed_visible, 2);
    assert_eq!(buffer.visible_len(), MAX_LINES / 2 - 1);
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
    assert_eq!(first_visible_text(&buffer), Some("5"));
}

#[test]
fn filter_keeps_only_matching_lines_ignoring_ascii_case() {
    let mut buffer = LogBuffer::new();
    push_checked(&mut buffer, lines(&["an error", "fine", "ERROR again"]));
    buffer.set_view(plain_view("error"));
    assert_eq!(visible(&buffer), ["an error", "ERROR again"]);
    assert_eq!(
        buffer.view().matcher.as_ref().map(LineMatcher::pattern),
        Some("error")
    );
    assert_eq!(buffer.total_len(), 3);
}

#[test]
fn empty_view_shows_every_line() {
    let mut buffer = LogBuffer::new();
    push_checked(&mut buffer, lines(&["a", "b"]));
    buffer.set_view(plain_view("a"));
    assert_eq!(buffer.visible_len(), 1);
    buffer.set_view(plain_view(""));
    assert_eq!(buffer.visible_len(), 2);
    buffer.set_view(plain_view("a"));
    buffer.set_view(plain_view("   "));
    assert_eq!(buffer.visible_len(), 2);
    assert!(buffer.view().matcher.is_none());
}

#[test]
fn filter_applies_to_new_lines() {
    let mut buffer = LogBuffer::new();
    buffer.set_view(plain_view("x"));
    push_checked(&mut buffer, lines(&["ax", "b", "xc"]));
    assert_eq!(visible(&buffer), ["ax", "xc"]);
}

#[test]
fn eviction_removes_evicted_matches() {
    let mut buffer = LogBuffer::new();
    buffer.set_view(plain_view("hit"));
    let batch: Vec<_> = (0..MAX_LINES)
        .map(|index| line(&format!("hit {index}")))
        .collect();
    push_checked(&mut buffer, batch);
    push_checked(&mut buffer, lines(&["miss", "hit last"]));
    assert_eq!(buffer.visible_len(), MAX_LINES - 1);
    for index in 0..buffer.visible_len() {
        assert!(buffer.visible_line(index).is_some());
    }
    assert_eq!(first_visible_text(&buffer), Some("hit 2"));
    assert!(buffer.visible_line(buffer.visible_len()).is_none());
}

#[test]
fn clear_keeps_view_and_resets_lines_and_dropped() {
    let mut buffer = LogBuffer::new();
    buffer.set_view(plain_view("a"));
    let batch: Vec<_> = (0..MAX_LINES + 1).map(|_| line("a")).collect();
    push_checked(&mut buffer, batch);
    buffer.clear();
    assert_eq!(buffer.total_len(), 0);
    assert_eq!(buffer.visible_len(), 0);
    assert!(!buffer.has_dropped());
    assert!(buffer.view().matcher.is_some());
    push_checked(&mut buffer, lines(&["a", "b"]));
    assert_eq!(visible(&buffer), ["a"]);
}

#[test]
fn push_detects_and_stores_levels() {
    let mut buffer = LogBuffer::new();
    push_checked(
        &mut buffer,
        lines(&["2024 ERROR boom", "plain", "level=warn x"]),
    );
    let levels: Vec<_> = buffer.visible_lines().map(|line| line.level).collect();
    assert_eq!(levels, [Some(LogLevel::Error), None, Some(LogLevel::Warn)]);
}

#[test]
fn indented_line_inherits_level_of_same_source() {
    let mut buffer = LogBuffer::new();
    push_checked(
        &mut buffer,
        vec![
            sourced(0, "ERROR boom"),
            sourced(1, "INFO fine"),
            sourced(0, "\tat x.y(Z.java:1)"),
            sourced(1, "  continued"),
            sourced(0, "not indented"),
            sourced(0, "  after reset"),
        ],
    );
    let levels: Vec<_> = buffer.visible_lines().map(|line| line.level).collect();
    assert_eq!(
        levels,
        [
            Some(LogLevel::Error),
            Some(LogLevel::Info),
            Some(LogLevel::Error),
            Some(LogLevel::Info),
            None,
            None
        ]
    );
}

#[test]
fn hidden_level_hides_lines() {
    let mut buffer = LogBuffer::new();
    push_checked(&mut buffer, lines(&["ERROR a", "INFO b", "unknown c"]));
    buffer.set_view(LineView {
        hidden_levels: LevelSet::default().toggled(LogLevel::Info),
        ..LineView::default()
    });
    assert_eq!(visible(&buffer), ["ERROR a"]);
}

#[test]
fn view_combines_levels_and_matcher() {
    let mut buffer = LogBuffer::new();
    push_checked(
        &mut buffer,
        lines(&["ERROR disk", "WARN disk", "ERROR net", "DEBUG disk"]),
    );
    buffer.set_view(LineView {
        matcher: LineMatcher::parse("disk", FilterMode::Plain).expect("plain"),
        hidden_levels: LevelSet::default().toggled(LogLevel::Warn),
        ..LineView::default()
    });
    assert_eq!(visible(&buffer), ["ERROR disk", "DEBUG disk"]);
}

#[test]
fn visible_text_writes_prefixes_and_clock_time() {
    let stamp = |source: u16, text: &str| SourcedLine {
        source: SourceId(source),
        kind: LineKind::Log,
        line: LogLine {
            timestamp: "2024-05-01T10:47:58.902345678Z".parse().ok(),
            text: text.to_owned(),
        },
    };
    let mut buffer = LogBuffer::new();
    push_checked(
        &mut buffer,
        vec![stamp(0, "hello"), stamp(1, "world"), line("plain")],
    );
    let prefixes = [SharedString::from("api-1/app")];
    assert_eq!(
        buffer.visible_text(LineTime::Clock, &TimeZone::UTC, &prefixes),
        "10:47:58.902 api-1/app hello\n10:47:58.902 world\napi-1/app plain"
    );
    assert_eq!(
        buffer.visible_text(LineTime::Hidden, &TimeZone::UTC, &[]),
        "hello\nworld\nplain"
    );
}

#[test]
fn format_log_time_reads_the_clock_of_the_zone_with_millis() {
    let timestamp: jiff::Timestamp = "2024-05-01T10:47:58.902345678Z".parse().expect("valid");
    assert_eq!(format_log_time(timestamp, &TimeZone::UTC), "10:47:58.902");
    let plus_seven = TimeZone::fixed(jiff::tz::offset(7));
    assert_eq!(format_log_time(timestamp, &plus_seven), "17:47:58.902");
}

#[test]
fn visible_text_writes_prefixes_and_rfc3339_time() {
    let stamped = |source: u16, text: &str| SourcedLine {
        source: SourceId(source),
        kind: LineKind::Log,
        line: LogLine {
            timestamp: "2024-05-01T10:47:58.902345678Z".parse().ok(),
            text: text.to_owned(),
        },
    };
    let mut buffer = LogBuffer::new();
    push_checked(
        &mut buffer,
        vec![stamped(0, "hello"), stamped(1, "world"), line("plain")],
    );
    let prefixes = [
        SharedString::from("api-7d9f8c-x2k4q/app"),
        SharedString::from("api-7d9f8c-z9z9z/app"),
    ];
    assert_eq!(
        buffer.visible_text(LineTime::Rfc3339, &TimeZone::UTC, &prefixes),
        "2024-05-01T10:47:58.902345678Z api-7d9f8c-x2k4q/app hello\n\
         2024-05-01T10:47:58.902345678Z api-7d9f8c-z9z9z/app world\n\
         api-7d9f8c-x2k4q/app plain"
    );
}

#[test]
fn revision_bumps_on_push_clear_and_view() {
    let mut buffer = LogBuffer::new();
    let mut last = buffer.revision();
    let mut has_bumped = |buffer: &LogBuffer| {
        let changed = buffer.revision() != last;
        last = buffer.revision();
        changed
    };
    buffer.push(lines(&["a"]));
    assert!(has_bumped(&buffer));
    buffer.set_view(plain_view("a"));
    assert!(has_bumped(&buffer));
    buffer.clear();
    assert!(has_bumped(&buffer));
    assert!(!has_bumped(&buffer));
}

fn marker(text: &str) -> SourcedLine {
    SourcedLine {
        kind: LineKind::Marker,
        ..sourced(0, text)
    }
}

#[test]
fn markers_ignore_levels_and_the_text_filter() {
    let mut buffer = LogBuffer::new();
    push_checked(
        &mut buffer,
        vec![
            line("ERROR timeout"),
            line("INFO fine"),
            marker("── container api restarted · restart #1 ──"),
        ],
    );
    buffer.set_view(LineView {
        matcher: LineMatcher::parse("timeout", FilterMode::Plain).expect("plain"),
        hidden_levels: LevelSet::default().toggled(LogLevel::Error),
        ..LineView::default()
    });
    assert_eq!(
        visible(&buffer),
        ["── container api restarted · restart #1 ──"]
    );
}

#[test]
fn markers_keep_the_continuation_level() {
    let mut buffer = LogBuffer::new();
    push_checked(
        &mut buffer,
        vec![
            line("ERROR boom"),
            marker("── container api restarted · restart #1 ──"),
            line("  at frame"),
        ],
    );
    let levels: Vec<_> = buffer.visible_lines().map(|line| line.level).collect();
    assert_eq!(levels, [Some(LogLevel::Error), None, Some(LogLevel::Error)]);
}

#[test]
fn volume_lines_skip_markers() {
    let mut buffer = LogBuffer::new();
    push_checked(&mut buffer, vec![line("a"), marker("m"), line("b")]);
    assert_eq!(buffer.visible_len(), 3);
    assert_eq!(buffer.volume_lines().count(), 2);
}

#[test]
fn export_writes_markers() {
    let mut buffer = LogBuffer::new();
    push_checked(&mut buffer, vec![line("a"), marker("── restart ──")]);
    let prefixes = [SharedString::from("api-1/app")];
    assert_eq!(
        buffer.visible_text(LineTime::Hidden, &TimeZone::UTC, &prefixes),
        "api-1/app a\napi-1/app ── restart ──"
    );
}

fn stamped(text: &str, time: Option<&str>) -> SourcedLine {
    SourcedLine {
        line: LogLine {
            timestamp: time.map(|time| time.parse().expect("valid time")),
            text: text.to_owned(),
        },
        ..sourced(0, text)
    }
}

fn window(start: &str, end: &str) -> TimeWindow {
    TimeWindow {
        start: start.parse().expect("valid time"),
        end: end.parse().expect("valid time"),
    }
}

#[test]
fn window_hides_lines_outside_and_without_a_timestamp() {
    let mut buffer = LogBuffer::new();
    push_checked(
        &mut buffer,
        vec![
            stamped("before", Some("2024-05-01T10:00:00Z")),
            stamped("start", Some("2024-05-01T10:00:05Z")),
            stamped("inside", Some("2024-05-01T10:00:07Z")),
            stamped("end", Some("2024-05-01T10:00:10Z")),
            stamped("untimed", None),
        ],
    );
    buffer.set_view(LineView {
        window: Some(window("2024-05-01T10:00:05Z", "2024-05-01T10:00:10Z")),
        ..LineView::default()
    });
    assert_eq!(visible(&buffer), ["start", "inside"]);
    assert!(buffer.view().is_filtering());
}

#[test]
fn volume_lines_ignore_the_window() {
    let mut buffer = LogBuffer::new();
    push_checked(
        &mut buffer,
        vec![
            stamped("a", Some("2024-05-01T10:00:00Z")),
            stamped("b", Some("2024-05-01T10:00:07Z")),
            stamped("c", Some("2024-05-01T10:00:20Z")),
        ],
    );
    buffer.set_view(LineView {
        window: Some(window("2024-05-01T10:00:05Z", "2024-05-01T10:00:10Z")),
        ..LineView::default()
    });
    assert_eq!(buffer.visible_len(), 1);
    assert_eq!(buffer.volume_lines().count(), 3);
}

#[test]
fn volume_lines_keep_the_text_filter_while_a_window_is_set() {
    let mut buffer = LogBuffer::new();
    push_checked(
        &mut buffer,
        vec![
            stamped("keep a", Some("2024-05-01T10:00:00Z")),
            stamped("drop b", Some("2024-05-01T10:00:07Z")),
            stamped("keep c", Some("2024-05-01T10:00:20Z")),
        ],
    );
    buffer.set_view(LineView {
        matcher: LineMatcher::parse("keep", FilterMode::Plain).expect("plain"),
        window: Some(window("2024-05-01T10:00:05Z", "2024-05-01T10:00:10Z")),
        ..LineView::default()
    });
    assert_eq!(buffer.volume_lines().count(), 2);
}

#[test]
fn a_window_applies_to_a_marker() {
    let mut buffer = LogBuffer::new();
    let timed_marker = SourcedLine {
        line: LogLine {
            timestamp: "2024-05-01T10:00:20Z".parse().ok(),
            text: "── restart ──".to_owned(),
        },
        ..marker("")
    };
    push_checked(&mut buffer, vec![timed_marker]);
    buffer.set_view(LineView {
        window: Some(window("2024-05-01T10:00:05Z", "2024-05-01T10:00:10Z")),
        ..LineView::default()
    });
    assert_eq!(buffer.visible_len(), 0);
}

#[test]
fn zone_label_names_the_zone_or_its_offset() {
    assert_eq!(zone_label(&TimeZone::UTC), "UTC");
    assert_eq!(
        zone_label(&TimeZone::fixed(jiff::tz::offset(7))),
        "UTC+07:00"
    );
    assert_eq!(
        zone_label(&TimeZone::fixed(jiff::tz::offset(-5))),
        "UTC-05:00"
    );
}
