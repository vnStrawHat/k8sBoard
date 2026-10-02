//! The line filter: plain substring or regular expression. No GPUI types.

use std::fmt;
use std::ops::Range;

use regex::{Regex, RegexBuilder};

/// Bounds the compiled program, so a hostile pattern cannot exhaust memory.
const REGEX_SIZE_LIMIT: usize = 1 << 20;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FilterMode {
    #[default]
    Plain,
    Regex,
}

#[derive(Clone, Debug)]
pub(crate) enum LineMatcher {
    Plain(String),
    Regex(Regex),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct InvalidRegex;

impl fmt::Display for InvalidRegex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Invalid regex")
    }
}

impl LineMatcher {
    /// `Ok(None)` for empty or whitespace-only text.
    pub(crate) fn parse(text: &str, mode: FilterMode) -> Result<Option<Self>, InvalidRegex> {
        if text.trim().is_empty() {
            return Ok(None);
        }
        match mode {
            FilterMode::Plain => Ok(Some(Self::Plain(text.to_owned()))),
            // The engine runs in linear time, so no pattern can hang the main thread.
            FilterMode::Regex => RegexBuilder::new(text)
                .case_insensitive(true)
                .size_limit(REGEX_SIZE_LIMIT)
                .build()
                .map(|regex| Some(Self::Regex(regex)))
                .map_err(|_| InvalidRegex),
        }
    }

    pub(crate) fn is_match(&self, text: &str) -> bool {
        match self {
            Self::Plain(needle) => !find_matches(text, needle).is_empty(),
            Self::Regex(regex) => regex.is_match(text),
        }
    }

    /// Non-empty, non-overlapping byte ranges on char boundaries, for highlights.
    pub(crate) fn ranges(&self, text: &str) -> Vec<Range<usize>> {
        match self {
            Self::Plain(needle) => find_matches(text, needle),
            Self::Regex(regex) => regex
                .find_iter(text)
                .map(|found| found.range())
                .filter(|range| !range.is_empty())
                .collect(),
        }
    }

    pub(crate) fn pattern(&self) -> &str {
        match self {
            Self::Plain(needle) => needle,
            Self::Regex(regex) => regex.as_str(),
        }
    }
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

#[cfg(test)]
#[path = "line_matcher_tests.rs"]
mod line_matcher_tests;
