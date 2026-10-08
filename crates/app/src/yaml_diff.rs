//! The line diff of Edit YAML (spec 0031, 0059): rows for the Diff tab and the revision diff dialog.
//! Pure.

use std::ops::Range;
use std::time::Duration;

use gpui_kit::SharedString;
use similar::{ChangeTag, TextDiff};

/// Unchanged lines kept around a change; longer runs fold.
const CONTEXT_LINES: usize = 3;
/// The most time the line diff may take on the main thread of a huge text.
const DIFF_DEADLINE: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiffRowKind {
    Same,
    Removed,
    Added,
    /// A run of unchanged lines, drawn as one muted row.
    Folded {
        lines: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiffRow {
    pub(crate) kind: DiffRowKind,
    /// 1-based line numbers of the two sides; a removed row has no new number, an added row no old.
    pub(crate) old_line: Option<usize>,
    pub(crate) new_line: Option<usize>,
    pub(crate) text: SharedString,
    /// Byte range of `text` that differs from the paired line of a 1:1 `−`/`+` change; `None` for
    /// every other row.
    pub(crate) changed: Option<Range<usize>>,
}

impl DiffRow {
    fn folded(lines: usize) -> Self {
        Self {
            kind: DiffRowKind::Folded { lines },
            old_line: None,
            new_line: None,
            text: SharedString::default(),
            changed: None,
        }
    }
}

/// The rows of `before` against `after`: every changed line with three lines of context, and one
/// `Folded` row for each longer run of unchanged lines, also at the start and the end.
pub(crate) fn diff_rows(before: &str, after: &str) -> Vec<DiffRow> {
    // A diff that would take longer than this is cut short and comes out coarser, never late.
    let diff = TextDiff::configure()
        .timeout(DIFF_DEADLINE)
        .diff_lines(before, after);
    let total = diff.old_len();
    let mut rows = Vec::new();
    // Old lines already shown or folded: unchanged runs have the same length on both sides, so the
    // old side is enough to size every gap.
    let mut covered = 0;
    for group in diff.grouped_ops(CONTEXT_LINES) {
        let Some(first) = group.first() else {
            continue;
        };
        let start = first.old_range().start;
        if start > covered {
            rows.push(DiffRow::folded(start - covered));
        }
        for op in &group {
            for change in diff.iter_changes(op) {
                rows.push(DiffRow {
                    kind: match change.tag() {
                        ChangeTag::Equal => DiffRowKind::Same,
                        ChangeTag::Delete => DiffRowKind::Removed,
                        ChangeTag::Insert => DiffRowKind::Added,
                    },
                    old_line: change.old_index().map(|index| index + 1),
                    new_line: change.new_index().map(|index| index + 1),
                    text: change
                        .value()
                        .trim_end_matches(['\n', '\r'])
                        .to_owned()
                        .into(),
                    changed: None,
                });
            }
        }
        covered = group.last().map_or(covered, |last| last.old_range().end);
    }
    if total > covered {
        rows.push(DiffRow::folded(total - covered));
    }
    mark_changed_spans(&mut rows);
    rows
}

/// Sets `changed` on both rows of every `−` line directly followed by one `+` line. A block of
/// several removed or added lines has no certain pairing, so it gets no span.
fn mark_changed_spans(rows: &mut [DiffRow]) {
    let mut at = 0;
    while at < rows.len() {
        if rows[at].kind != DiffRowKind::Removed {
            at += 1;
            continue;
        }
        let removed = rows[at..]
            .iter()
            .take_while(|row| row.kind == DiffRowKind::Removed)
            .count();
        let added = rows[at + removed..]
            .iter()
            .take_while(|row| row.kind == DiffRowKind::Added)
            .count();
        if removed == 1 && added == 1 {
            let (old, new) = changed_spans(&rows[at].text, &rows[at + 1].text);
            rows[at].changed = Some(old).filter(|span| !span.is_empty());
            rows[at + 1].changed = Some(new).filter(|span| !span.is_empty());
        }
        at += removed + added;
    }
}

/// The byte ranges of `old` and `new` left after cutting their common prefix and common suffix;
/// the two never overlap, so `image: a:1` against `image: a:2` marks only the last character.
fn changed_spans(old: &str, new: &str) -> (Range<usize>, Range<usize>) {
    let prefix: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    let suffix: usize = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    (prefix..old.len() - suffix, prefix..new.len() - suffix)
}

#[cfg(test)]
#[path = "yaml_diff_tests.rs"]
mod yaml_diff_tests;
