//! The line diff of Edit YAML (spec 0031): the object as it is now against the server's dry-run
//! answer, both masked and without the edit header, as rows for the Diff tab. Pure.

use gpui_kit::SharedString;
use similar::{ChangeTag, TextDiff};

/// Unchanged lines kept around a change; longer runs fold.
const CONTEXT_LINES: usize = 3;

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
}

impl DiffRow {
    fn folded(lines: usize) -> Self {
        Self {
            kind: DiffRowKind::Folded { lines },
            old_line: None,
            new_line: None,
            text: SharedString::default(),
        }
    }
}

/// The rows of `before` against `after`: every changed line with three lines of context, and one
/// `Folded` row for each longer run of unchanged lines, also at the start and the end.
pub(crate) fn diff_rows(before: &str, after: &str) -> Vec<DiffRow> {
    let diff = TextDiff::from_lines(before, after);
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
                });
            }
        }
        covered = group.last().map_or(covered, |last| last.old_range().end);
    }
    if total > covered {
        rows.push(DiffRow::folded(total - covered));
    }
    rows
}

#[cfg(test)]
#[path = "yaml_diff_tests.rs"]
mod yaml_diff_tests;
