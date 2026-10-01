//! The bounded line store behind one log tab: caps, filter, and match ranges. No GPUI types.

use std::collections::VecDeque;
use std::ops::Range;

use cluster::LogLine;

const MAX_LINES: usize = 10_000;
/// Sum of `text.len()` over the kept lines.
const MAX_BYTES: usize = 8 * 1024 * 1024;

pub(crate) struct LogBuffer {
    lines: VecDeque<LogLine>,
    bytes: usize,
    /// The sequence number of `lines[0]`. Every pushed line gets the next one, so a line's
    /// sequence number never changes while it is kept.
    first_seq: u64,
    /// Lines evicted since the last clear.
    dropped: u64,
    filter: Option<LineFilter>,
}

struct LineFilter {
    needle: String,
    /// Sequence numbers of the matching lines, ascending.
    matches: VecDeque<u64>,
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
            filter: None,
        }
    }

    pub(crate) fn push(&mut self, lines: Vec<LogLine>) -> BufferChange {
        let first_new_seq = self.next_seq();
        let mut added_visible = 0;
        for line in lines {
            let seq = self.next_seq();
            self.bytes += line.text.len();
            let is_visible = match &mut self.filter {
                Some(filter) if is_match(&line.text, &filter.needle) => {
                    filter.matches.push_back(seq);
                    true
                }
                Some(_) => false,
                None => true,
            };
            added_visible += usize::from(is_visible);
            self.lines.push_back(line);
        }

        let mut removed_visible = 0;
        while self.lines.len() > MAX_LINES || self.bytes > MAX_BYTES {
            let Some(evicted) = self.lines.pop_front() else {
                break;
            };
            let seq = self.first_seq;
            self.bytes -= evicted.text.len();
            self.first_seq += 1;
            self.dropped += 1;
            let was_visible = match &mut self.filter {
                Some(filter) if filter.matches.front() == Some(&seq) => {
                    filter.matches.pop_front();
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

    /// Drops every line and keeps the filter.
    pub(crate) fn clear(&mut self) {
        self.lines.clear();
        self.bytes = 0;
        self.first_seq = 0;
        self.dropped = 0;
        if let Some(filter) = &mut self.filter {
            filter.matches.clear();
        }
    }

    /// An empty or whitespace-only needle removes the filter.
    pub(crate) fn set_filter(&mut self, needle: &str) {
        if needle.trim().is_empty() {
            self.filter = None;
            return;
        }
        // At most `MAX_LINES` lines are scanned, on each keystroke; profile before debouncing.
        let matches = self
            .lines
            .iter()
            .zip(self.first_seq..)
            .filter(|(line, _)| is_match(&line.text, needle))
            .map(|(_, seq)| seq)
            .collect();
        self.filter = Some(LineFilter {
            needle: needle.to_owned(),
            matches,
        });
    }

    pub(crate) fn needle(&self) -> Option<&str> {
        self.filter.as_ref().map(|filter| filter.needle.as_str())
    }

    pub(crate) fn visible_len(&self) -> usize {
        match &self.filter {
            Some(filter) => filter.matches.len(),
            None => self.lines.len(),
        }
    }

    pub(crate) fn visible_line(&self, index: usize) -> Option<&LogLine> {
        match &self.filter {
            None => self.lines.get(index),
            Some(filter) => {
                let seq = *filter.matches.get(index)?;
                self.lines.get(usize::try_from(seq - self.first_seq).ok()?)
            }
        }
    }

    pub(crate) fn total_len(&self) -> usize {
        self.lines.len()
    }

    pub(crate) fn has_dropped(&self) -> bool {
        self.dropped > 0
    }

    /// One line per visible line. The time is shown only for lines that have one.
    pub(crate) fn visible_text(&self, shows_timestamps: bool) -> String {
        let mut text = String::new();
        for index in 0..self.visible_len() {
            let Some(line) = self.visible_line(index) else {
                continue;
            };
            if index > 0 {
                text.push('\n');
            }
            if let (true, Some(timestamp)) = (shows_timestamps, line.timestamp) {
                text.push_str(&format_log_time(timestamp));
                text.push(' ');
            }
            text.push_str(&line.text);
        }
        text
    }

    fn next_seq(&self) -> u64 {
        self.first_seq + self.lines.len() as u64
    }
}

fn is_match(text: &str, needle: &str) -> bool {
    !find_matches(text, needle).is_empty()
}

/// Non-overlapping byte ranges of `needle` in `text`, compared with ASCII case folding.
/// Other characters must match exactly, so every range falls on char boundaries and the
/// offsets stay valid for the original string. Full Unicode folding can change lengths.
pub(crate) fn find_matches(text: &str, needle: &str) -> Vec<Range<usize>> {
    let (text_bytes, needle_bytes) = (text.as_bytes(), needle.as_bytes());
    let mut ranges = Vec::new();
    if needle_bytes.is_empty() {
        return ranges;
    }
    let mut start = 0;
    while start + needle_bytes.len() <= text_bytes.len() {
        let end = start + needle_bytes.len();
        if text.is_char_boundary(start) && text_bytes[start..end].eq_ignore_ascii_case(needle_bytes)
        {
            ranges.push(start..end);
            start = end;
        } else {
            start += 1;
        }
    }
    ranges
}

/// `HH:MM:SS.mmm` in UTC, for example `10:47:58.902`. The workspace jiff has no time-zone
/// support, so local time is not available.
pub(crate) fn format_log_time(timestamp: jiff::Timestamp) -> String {
    timestamp.strftime("%H:%M:%S%.3f").to_string()
}

#[cfg(test)]
#[path = "log_buffer_tests.rs"]
mod log_buffer_tests;
