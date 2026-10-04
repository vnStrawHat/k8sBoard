//! The bounded line store behind one log tab: caps, view (matcher and levels), and levels per
//! line. The only GPUI type is `SharedString`, a plain string.

use std::collections::VecDeque;

use cluster::LogLine;
use gpui_kit::SharedString;

use crate::line_matcher::LineMatcher;
use crate::log_level::{LevelSet, LogLevel, detect_level};

const MAX_LINES: usize = 10_000;
/// Sum of `text.len()` over the kept lines.
const MAX_BYTES: usize = 8 * 1024 * 1024;

/// An index into the tab's source table (one source per pod container stream).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SourceId(pub(crate) u16);

/// What a line is: written by a container, or added by the tab (a restart marker).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineKind {
    Log,
    Marker,
}

pub(crate) struct SourcedLine {
    pub(crate) source: SourceId,
    pub(crate) kind: LineKind,
    pub(crate) line: LogLine,
}

pub(crate) struct BufferedLine {
    pub(crate) source: SourceId,
    pub(crate) kind: LineKind,
    pub(crate) level: Option<LogLevel>,
    pub(crate) line: LogLine,
}

/// A span of time the brush picked on the histogram. `start` is inclusive, `end` exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TimeWindow {
    pub(crate) start: jiff::Timestamp,
    pub(crate) end: jiff::Timestamp,
}

impl TimeWindow {
    /// A line without a timestamp cannot be placed in time, so it is outside every window.
    fn contains(&self, time: Option<jiff::Timestamp>) -> bool {
        time.is_some_and(|time| self.start <= time && time < self.end)
    }
}

/// What the tab shows: lines that match the filter, are not at a hidden level, and fall inside
/// the brush window.
#[derive(Default)]
pub(crate) struct LineView {
    pub(crate) matcher: Option<LineMatcher>,
    pub(crate) hidden_levels: LevelSet,
    pub(crate) window: Option<TimeWindow>,
}

impl LineView {
    /// Whether the view can hide a line.
    pub(crate) fn is_filtering(&self) -> bool {
        self.matcher.is_some() || !self.hidden_levels.hides_none() || self.window.is_some()
    }

    fn shows(&self, line: &BufferedLine) -> bool {
        self.window
            .is_none_or(|window| window.contains(line.line.timestamp))
            && self.shows_outside_window(line)
    }

    /// A line without a detected level counts as INFO (decision 15). A marker ignores the level
    /// chips and the text filter: it is the tab's own note, not a line to search.
    fn shows_outside_window(&self, line: &BufferedLine) -> bool {
        if line.kind == LineKind::Marker {
            return true;
        }
        let level = line.level.unwrap_or(LogLevel::Info);
        !self.hidden_levels.is_hidden(level)
            && self
                .matcher
                .as_ref()
                .is_none_or(|matcher| matcher.is_match(&line.line.text))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineTime {
    Hidden,
    Clock,
    Rfc3339,
}

pub(crate) struct LogBuffer {
    lines: VecDeque<BufferedLine>,
    bytes: usize,
    /// The sequence number of `lines[0]`. Every pushed line gets the next one, so a line's
    /// sequence number never changes while it is kept.
    first_seq: u64,
    /// Lines evicted since the last clear.
    dropped: u64,
    view: LineView,
    /// Sequence numbers of the visible lines, ascending; `None` exactly when the view hides
    /// nothing.
    visible: Option<VecDeque<u64>>,
    /// The level of the last pushed line per source, for indented continuation lines.
    last_levels: Vec<Option<LogLevel>>,
    revision: u64,
}

/// How the visible list changed, applied to the scroller as `splice(0..removed_visible, 0)`
/// and then `append(added_visible)`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BufferChange {
    pub(crate) removed_visible: usize,
    pub(crate) added_visible: usize,
}

impl LogBuffer {
    pub(crate) fn new() -> Self {
        Self {
            lines: VecDeque::new(),
            bytes: 0,
            first_seq: 0,
            dropped: 0,
            view: LineView::default(),
            visible: None,
            last_levels: Vec::new(),
            revision: 0,
        }
    }

    pub(crate) fn push(&mut self, lines: Vec<SourcedLine>) -> BufferChange {
        self.revision += 1;
        let first_new_seq = self.next_seq();
        let mut added_visible = 0;
        for sourced in lines {
            let seq = self.next_seq();
            let level = self.level_of(&sourced);
            let line = BufferedLine {
                source: sourced.source,
                kind: sourced.kind,
                level,
                line: sourced.line,
            };
            self.bytes += line.line.text.len();
            let is_visible = self.view.shows(&line);
            if let (true, Some(visible)) = (is_visible, &mut self.visible) {
                visible.push_back(seq);
            }
            added_visible += usize::from(is_visible);
            self.lines.push_back(line);
        }

        let mut removed_visible = 0;
        while self.lines.len() > MAX_LINES || self.bytes > MAX_BYTES {
            let Some(evicted) = self.lines.pop_front() else {
                break;
            };
            let seq = self.first_seq;
            self.bytes -= evicted.line.text.len();
            self.first_seq += 1;
            self.dropped += 1;
            let was_visible = match &mut self.visible {
                Some(visible) if visible.front() == Some(&seq) => {
                    visible.pop_front();
                    true
                }
                Some(_) => false,
                None => true,
            };
            if !was_visible {
                continue;
            }
            // A line of this very batch can be evicted when the batch exceeds the cap; the
            // scroller never saw it, so it is taken out of the additions instead.
            if seq < first_new_seq {
                removed_visible += 1;
            } else {
                added_visible -= 1;
            }
        }
        BufferChange {
            removed_visible,
            added_visible,
        }
    }

    /// The detected level; an indented line without one continues the previous line of its
    /// source (a stack trace follows its error).
    fn level_of(&mut self, sourced: &SourcedLine) -> Option<LogLevel> {
        // A marker is no log text: it has no level and must not break a continuation.
        if sourced.kind == LineKind::Marker {
            return None;
        }
        let slot = usize::from(sourced.source.0);
        if self.last_levels.len() <= slot {
            self.last_levels.resize(slot + 1, None);
        }
        let text = &sourced.line.text;
        let is_indented = text.starts_with([' ', '\t']);
        let level = detect_level(text).or_else(|| {
            if is_indented {
                self.last_levels[slot]
            } else {
                None
            }
        });
        self.last_levels[slot] = level;
        level
    }

    /// Drops every line and keeps the view.
    pub(crate) fn clear(&mut self) {
        self.revision += 1;
        self.lines.clear();
        self.bytes = 0;
        self.first_seq = 0;
        self.dropped = 0;
        self.last_levels.clear();
        if let Some(visible) = &mut self.visible {
            visible.clear();
        }
    }

    pub(crate) fn set_view(&mut self, view: LineView) {
        self.revision += 1;
        self.visible = if view.is_filtering() {
            // At most `MAX_LINES` lines are scanned, on each keystroke; profile before
            // debouncing.
            Some(
                self.lines
                    .iter()
                    .zip(self.first_seq..)
                    .filter(|(line, _)| view.shows(line))
                    .map(|(_, seq)| seq)
                    .collect(),
            )
        } else {
            None
        };
        self.view = view;
    }

    pub(crate) fn view(&self) -> &LineView {
        &self.view
    }

    pub(crate) fn visible_len(&self) -> usize {
        match &self.visible {
            Some(visible) => visible.len(),
            None => self.lines.len(),
        }
    }

    pub(crate) fn visible_line(&self, index: usize) -> Option<&BufferedLine> {
        match &self.visible {
            None => self.lines.get(index),
            Some(visible) => {
                let seq = *visible.get(index)?;
                self.lines.get(usize::try_from(seq - self.first_seq).ok()?)
            }
        }
    }

    pub(crate) fn visible_lines(&self) -> impl Iterator<Item = &BufferedLine> {
        (0..self.visible_len()).filter_map(|index| self.visible_line(index))
    }

    /// What the histogram counts: the log lines the view shows without its window, so the bars
    /// keep the whole range while a window is set. A marker is not log volume.
    pub(crate) fn volume_lines(&self) -> Box<dyn Iterator<Item = &BufferedLine> + '_> {
        if self.view.window.is_none() {
            return Box::new(
                self.visible_lines()
                    .filter(|buffered| buffered.kind == LineKind::Log),
            );
        }
        // ponytail: rescans up to 10,000 lines per buffer revision while a window is set (Full
        // layout only); keep a second index if traces show the cost.
        Box::new(
            self.lines
                .iter()
                .filter(|buffered| buffered.kind == LineKind::Log)
                .filter(|buffered| self.view.shows_outside_window(buffered)),
        )
    }

    pub(crate) fn total_len(&self) -> usize {
        self.lines.len()
    }

    pub(crate) fn has_dropped(&self) -> bool {
        self.dropped > 0
    }

    /// Bumps on push, clear, and set_view; the histogram memo key.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// One line per visible line: `{time} {prefix} {text}`. The time is written only for
    /// lines that have one; `prefixes` is indexed by source, and a missing index writes none.
    pub(crate) fn visible_text(&self, time: LineTime, prefixes: &[SharedString]) -> String {
        let mut text = String::new();
        for (index, buffered) in self.visible_lines().enumerate() {
            if index > 0 {
                text.push('\n');
            }
            let stamp = buffered.line.timestamp.and_then(|timestamp| match time {
                LineTime::Hidden => None,
                LineTime::Clock => Some(format_log_time(timestamp)),
                LineTime::Rfc3339 => Some(timestamp.to_string()),
            });
            if let Some(stamp) = stamp {
                text.push_str(&stamp);
                text.push(' ');
            }
            if let Some(prefix) = prefixes.get(usize::from(buffered.source.0)) {
                text.push_str(prefix);
                text.push(' ');
            }
            text.push_str(&buffered.line.text);
        }
        text
    }

    fn next_seq(&self) -> u64 {
        self.first_seq + self.lines.len() as u64
    }
}

/// `HH:MM:SS.mmm` in UTC, for example `10:47:58.902`. The workspace jiff has no time-zone
/// support, so local time is not available.
pub(crate) fn format_log_time(timestamp: jiff::Timestamp) -> String {
    timestamp.strftime("%H:%M:%S%.3f").to_string()
}

#[cfg(test)]
#[path = "log_buffer_tests.rs"]
mod log_buffer_tests;
