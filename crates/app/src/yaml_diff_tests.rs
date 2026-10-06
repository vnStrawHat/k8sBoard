use super::*;

fn numbered(count: usize) -> String {
    (1..=count).map(|n| format!("line {n}\n")).collect()
}

fn kinds(rows: &[DiffRow]) -> Vec<DiffRowKind> {
    rows.iter().map(|row| row.kind).collect()
}

#[test]
fn identical_texts_fold_into_one_row() {
    let text = numbered(10);
    let rows = diff_rows(&text, &text);
    assert_eq!(kinds(&rows), [DiffRowKind::Folded { lines: 10 }]);
}

#[test]
fn diff_rows_fold_unchanged_runs() {
    let before = numbered(20);
    let after = before.replace("line 10\n", "line ten\n");
    let rows = diff_rows(&before, &after);
    assert_eq!(
        kinds(&rows),
        [
            DiffRowKind::Folded { lines: 6 },
            DiffRowKind::Same,
            DiffRowKind::Same,
            DiffRowKind::Same,
            DiffRowKind::Removed,
            DiffRowKind::Added,
            DiffRowKind::Same,
            DiffRowKind::Same,
            DiffRowKind::Same,
            DiffRowKind::Folded { lines: 7 },
        ]
    );
}

#[test]
fn two_changes_far_apart_fold_the_gap_between_them() {
    let before = numbered(30);
    let after = before
        .replace("line 3\n", "line three\n")
        .replace("line 28\n", "line twenty-eight\n");
    let rows = diff_rows(&before, &after);
    let gaps: Vec<usize> = rows
        .iter()
        .filter_map(|row| match row.kind {
            DiffRowKind::Folded { lines } => Some(lines),
            _ => None,
        })
        .collect();
    assert_eq!(gaps, [18]);
}

#[test]
fn diff_rows_number_both_sides() {
    let before = "a\nb\nc\n";
    let after = "a\nx\ny\nc\n";
    let rows = diff_rows(before, after);
    let numbers: Vec<(Option<usize>, Option<usize>, &str)> = rows
        .iter()
        .map(|row| (row.old_line, row.new_line, row.text.as_ref()))
        .collect();
    assert_eq!(
        numbers,
        [
            (Some(1), Some(1), "a"),
            (Some(2), None, "b"),
            (None, Some(2), "x"),
            (None, Some(3), "y"),
            (Some(3), Some(4), "c"),
        ]
    );
}

#[test]
fn a_changed_last_line_without_a_newline_has_clean_text() {
    let rows = diff_rows("a\nb", "a\nc");
    let texts: Vec<&str> = rows.iter().map(|row| row.text.as_ref()).collect();
    assert_eq!(texts, ["a", "b", "c"]);
}

#[test]
fn an_empty_side_adds_every_line() {
    let rows = diff_rows("", "a\nb\n");
    assert_eq!(kinds(&rows), [DiffRowKind::Added, DiffRowKind::Added]);
}

#[test]
fn a_one_to_one_change_marks_only_the_differing_span() {
    let rows = diff_rows("image: app:1.2\n", "image: app:1.3\n");
    assert_eq!(rows[0].changed, Some(13..14));
    assert_eq!(rows[1].changed, Some(13..14));
}

#[test]
fn a_pure_insertion_marks_only_the_added_side() {
    assert_eq!(changed_spans("name: a", "name: ab"), (7..7, 7..8));
}

#[test]
fn the_span_split_never_overlaps_and_respects_characters() {
    // The prefix and the suffix share the repeated `a`: the spans stay inside the shorter line.
    assert_eq!(changed_spans("aa", "aaa"), (2..2, 2..3));
    assert_eq!(changed_spans("héllo", "hállo"), (1..3, 1..3));
}

#[test]
fn several_removed_or_added_lines_get_no_span() {
    let rows = diff_rows("a: 1\nb: 2\n", "a: 9\nb: 8\nc: 7\n");
    assert!(rows.iter().all(|row| row.changed.is_none()));
}
