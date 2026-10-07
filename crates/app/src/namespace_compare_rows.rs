//! The lines of the namespace comparison dialog (spec 0057): a comparison turned into a flat list
//! of headers, names, field changes and diff rows. Pure; the dialog only draws them.

use std::collections::HashSet;

use cluster::{
    FieldChange, KindComparison, KindOutcome, NamespaceComparison, ObjectDifference, ObjectKind,
};

use crate::yaml_diff::{DiffRow, diff_rows};

/// The differing objects whose line diff is open: the kind and the name.
pub(crate) type OpenDiffs = HashSet<(ObjectKind, String)>;

/// What the comparison dialog shows for an absent side of a field change.
const ABSENT: &str = "—";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CompareLine {
    /// A kind with something to show, and how many objects of it differ.
    Kind {
        title: &'static str,
        summary: String,
    },
    /// `Only in lab-shop (1)`, `Differs (2)`.
    Group(String),
    /// An object that exists in one namespace only.
    Name(String),
    /// A differing object, with the toggle of its line diff.
    Object {
        kind: ObjectKind,
        name: String,
        is_open: bool,
    },
    /// `spec.replicas: 1 → 2`.
    Change(String),
    Diff(DiffRow),
    /// A muted line: an unreadable kind, the changes beyond the cap, or an empty result.
    Note(String),
}

/// `lab-shop ↔ lab-shop-stg · 3 differ · 1 only in lab-shop · 0 only in lab-shop-stg · 7 same`.
pub(crate) fn summary_text(comparison: &NamespaceComparison) -> String {
    let counts = comparison.counts();
    format!(
        "{} ↔ {} · {} differ · {} only in {} · {} only in {} · {} same",
        comparison.left,
        comparison.right,
        counts.differ,
        counts.only_left,
        comparison.left,
        counts.only_right,
        comparison.right,
        counts.same,
    )
}

/// The note under the summary when env literals are hidden, `None` when none are.
pub(crate) fn hidden_env_note(comparison: &NamespaceComparison) -> Option<String> {
    let count = comparison.hidden_env_values;
    let words = match count {
        0 => return None,
        1 => "env value is",
        _ => "env values are",
    };
    Some(format!(
        "{count} {words} hidden and not compared; Show env values compares {}",
        if count == 1 { "it" } else { "them" }
    ))
}

/// Every line of the comparison, kinds with nothing to show left out. `open` names the differing
/// objects whose line diff is shown under them.
pub(crate) fn compare_lines(
    comparison: &NamespaceComparison,
    open: &OpenDiffs,
) -> Vec<CompareLine> {
    let mut lines = Vec::new();
    for kind in &comparison.kinds {
        push_kind(&mut lines, kind, comparison, open);
    }
    if lines.is_empty() {
        let counts = comparison.counts();
        lines.push(CompareLine::Note(match counts.same {
            0 => "Neither namespace has objects of the compared kinds.".to_owned(),
            same => format!("The two namespaces are the same ({same} objects)."),
        }));
    }
    lines
}

fn push_kind(
    lines: &mut Vec<CompareLine>,
    kind: &KindComparison,
    comparison: &NamespaceComparison,
    open: &OpenDiffs,
) {
    let title = kind.kind.name();
    let (only_left, only_right, differs) = match &kind.outcome {
        KindOutcome::Unreadable(message) => {
            lines.push(CompareLine::Kind {
                title,
                summary: String::new(),
            });
            lines.push(CompareLine::Note(format!("Could not be read: {message}")));
            return;
        }
        KindOutcome::Compared {
            only_left,
            only_right,
            differs,
            ..
        } => (only_left, only_right, differs),
    };
    if only_left.is_empty() && only_right.is_empty() && differs.is_empty() {
        return;
    }
    lines.push(CompareLine::Kind {
        title,
        summary: format!(
            "{} differ · {} only in {} · {} only in {}",
            differs.len(),
            only_left.len(),
            comparison.left,
            only_right.len(),
            comparison.right,
        ),
    });
    push_only(lines, "Only in", &comparison.left, only_left);
    push_only(lines, "Only in", &comparison.right, only_right);
    if differs.is_empty() {
        return;
    }
    lines.push(CompareLine::Group(format!("Differs ({})", differs.len())));
    for difference in differs {
        push_difference(lines, kind.kind, difference, open);
    }
}

fn push_only(lines: &mut Vec<CompareLine>, label: &str, namespace: &str, names: &[String]) {
    if names.is_empty() {
        return;
    }
    lines.push(CompareLine::Group(format!(
        "{label} {namespace} ({})",
        names.len()
    )));
    lines.extend(names.iter().cloned().map(CompareLine::Name));
}

fn push_difference(
    lines: &mut Vec<CompareLine>,
    kind: ObjectKind,
    difference: &ObjectDifference,
    open: &OpenDiffs,
) {
    let is_open = open.contains(&(kind, difference.name.clone()));
    lines.push(CompareLine::Object {
        kind,
        name: difference.name.clone(),
        is_open,
    });
    lines.extend(difference.changes.iter().map(change_line));
    if difference.more_changes > 0 {
        lines.push(CompareLine::Note(format!(
            "and {} more changes",
            difference.more_changes
        )));
    }
    if is_open {
        lines.extend(
            diff_rows(&difference.left_text, &difference.right_text)
                .into_iter()
                .map(CompareLine::Diff),
        );
    }
}

fn change_line(change: &FieldChange) -> CompareLine {
    let side = |value: &Option<String>| value.clone().unwrap_or_else(|| ABSENT.to_owned());
    CompareLine::Change(format!(
        "{}: {} → {}",
        change.path,
        side(&change.old),
        side(&change.new)
    ))
}

#[cfg(test)]
#[path = "namespace_compare_rows_tests.rs"]
mod namespace_compare_rows_tests;
