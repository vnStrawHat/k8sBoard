//! Sorting rules for table columns. Pure, so it is tested without a window.

use std::cmp::Ordering;

use crate::status_tone::StatusTone;
use crate::table_view::CellValue;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TableSort {
    /// A logical column.
    pub(crate) column: usize,
    pub(crate) direction: SortDirection,
}

/// A header click: another column sorts ascending, then descending, then back to source order.
pub(crate) fn next_sort(current: Option<TableSort>, column: usize) -> Option<TableSort> {
    let direction = match current {
        Some(sort) if sort.column == column => match sort.direction {
            SortDirection::Ascending => SortDirection::Descending,
            SortDirection::Descending => return None,
        },
        _ => SortDirection::Ascending,
    };
    Some(TableSort { column, direction })
}

/// Orders two cells. Absent values come after everything, in both directions. `now` ends the
/// span of a running item.
pub(crate) fn compare_values(
    a: &CellValue,
    b: &CellValue,
    direction: SortDirection,
    now: jiff::Timestamp,
) -> Ordering {
    match (is_absent(a), is_absent(b)) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        (false, false) => {}
    }
    let ordering = compare_present(a, b, now);
    match direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    }
}

fn is_absent(value: &CellValue) -> bool {
    matches!(
        value,
        CellValue::Absent | CellValue::Age(None) | CellValue::Span { started: None, .. }
    )
}

fn compare_present(a: &CellValue, b: &CellValue, now: jiff::Timestamp) -> Ordering {
    match (a, b) {
        (CellValue::Text(a), CellValue::Text(b)) => natural_cmp(a, b),
        (
            CellValue::Qualified {
                prefix: a_prefix,
                text: a_text,
            },
            CellValue::Qualified {
                prefix: b_prefix,
                text: b_text,
            },
        ) => compare_prefix(*a_prefix, *b_prefix).then_with(|| natural_cmp(a_text, b_text)),
        (CellValue::Number(a), CellValue::Number(b)) => a.cmp(b),
        (
            CellValue::Status {
                tone: a_tone,
                text: a_text,
            },
            CellValue::Status {
                tone: b_tone,
                text: b_text,
            },
        ) => severity(*a_tone)
            .cmp(&severity(*b_tone))
            .then_with(|| natural_cmp(a_text, b_text)),
        // A later timestamp is a younger item, and ascending shows the youngest first.
        (CellValue::Age(a), CellValue::Age(b)) => b.cmp(a),
        (
            CellValue::Span {
                started: a_started,
                finished: a_finished,
            },
            CellValue::Span {
                started: b_started,
                finished: b_finished,
            },
        ) => elapsed_seconds(*a_started, *a_finished, now).cmp(&elapsed_seconds(
            *b_started,
            *b_finished,
            now,
        )),
        _ => variant_rank(a).cmp(&variant_rank(b)),
    }
}

/// `None` sorts before any prefix.
fn compare_prefix(a: Option<&str>, b: Option<&str>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(a), Some(b)) => natural_cmp(a, b),
    }
}

fn elapsed_seconds(
    started: Option<jiff::Timestamp>,
    finished: Option<jiff::Timestamp>,
    now: jiff::Timestamp,
) -> i64 {
    let Some(started) = started else {
        return 0;
    };
    finished.unwrap_or(now).as_second() - started.as_second()
}

/// Problems sort last when ascending.
fn severity(tone: StatusTone) -> u8 {
    match tone {
        StatusTone::Ok => 0,
        StatusTone::Done => 1,
        StatusTone::Info => 2,
        StatusTone::Warn => 3,
        StatusTone::Bad => 4,
    }
}

/// Orders cells of different variants, which only happens when a column mixes them.
fn variant_rank(value: &CellValue) -> u8 {
    match value {
        CellValue::Text(_) => 0,
        CellValue::Qualified { .. } => 1,
        CellValue::Number(_) => 2,
        CellValue::Status { .. } => 3,
        CellValue::Age(_) => 4,
        CellValue::Span { .. } => 5,
        CellValue::Absent => 6,
    }
}

/// Digit runs compare by value (`pod-2` before `pod-10`), other runs ignore ASCII case, and
/// plain byte order breaks the remaining ties so the order is total.
pub(crate) fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a_runs, mut b_runs) = (runs(a), runs(b));
    loop {
        match (a_runs.next(), b_runs.next()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(a_run), Some(b_run)) => match compare_runs(a_run, b_run) {
                Ordering::Equal => {}
                other => return other,
            },
        }
    }
}

fn compare_runs(a: &str, b: &str) -> Ordering {
    if is_digit_run(a) && is_digit_run(b) {
        let (a, b) = (a.trim_start_matches('0'), b.trim_start_matches('0'));
        return a.len().cmp(&b.len()).then_with(|| a.cmp(b));
    }
    a.bytes()
        .map(|byte| byte.to_ascii_lowercase())
        .cmp(b.bytes().map(|byte| byte.to_ascii_lowercase()))
}

fn is_digit_run(run: &str) -> bool {
    run.starts_with(|c: char| c.is_ascii_digit())
}

/// Splits into alternating digit and non-digit runs.
fn runs(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        let is_digit = rest.chars().next()?.is_ascii_digit();
        let end = rest
            .find(|c: char| c.is_ascii_digit() != is_digit)
            .unwrap_or(rest.len());
        let (run, tail) = rest.split_at(end);
        rest = tail;
        Some(run)
    })
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;

    fn text(value: &str) -> CellValue<'_> {
        CellValue::Text(Cow::Borrowed(value))
    }

    fn status(tone: StatusTone, value: &str) -> CellValue<'static> {
        CellValue::Status {
            tone,
            text: value.to_owned().into(),
        }
    }

    fn at(second: i64) -> jiff::Timestamp {
        jiff::Timestamp::from_second(second).expect("valid timestamp")
    }

    fn ascending(a: &CellValue, b: &CellValue) -> Ordering {
        compare_values(a, b, SortDirection::Ascending, at(1_000))
    }

    #[test]
    fn natural_cmp_orders_digit_runs_numerically() {
        assert_eq!(natural_cmp("7", "10"), Ordering::Less);
        assert_eq!(natural_cmp("pod-2", "pod-10"), Ordering::Less);
        assert_eq!(natural_cmp("v1.9", "v1.10"), Ordering::Less);
        assert_eq!(natural_cmp("10.0.0.9", "10.0.0.10"), Ordering::Less);
        assert_eq!(natural_cmp("Api", "apj"), Ordering::Less);
        // Equal by value: the byte order breaks the tie.
        assert_eq!(natural_cmp("007", "7"), Ordering::Less);
        assert_eq!(natural_cmp("7", "7"), Ordering::Equal);
    }

    #[test]
    fn next_sort_cycles_ascending_descending_none() {
        let first = next_sort(None, 2);
        assert_eq!(
            first,
            Some(TableSort {
                column: 2,
                direction: SortDirection::Ascending
            })
        );
        let second = next_sort(first, 2);
        assert_eq!(
            second.map(|sort| sort.direction),
            Some(SortDirection::Descending)
        );
        assert_eq!(next_sort(second, 2), None);
        let other = next_sort(second, 3);
        assert_eq!(
            other,
            Some(TableSort {
                column: 3,
                direction: SortDirection::Ascending
            })
        );
    }

    #[test]
    fn absent_sorts_last_in_both_directions() {
        let now = at(1_000);
        for direction in [SortDirection::Ascending, SortDirection::Descending] {
            assert_eq!(
                compare_values(&CellValue::Absent, &text("a"), direction, now),
                Ordering::Greater
            );
            assert_eq!(
                compare_values(&text("a"), &CellValue::Absent, direction, now),
                Ordering::Less
            );
            let missing_age = CellValue::Age(None);
            let some_age = CellValue::Age(Some(at(1)));
            assert_eq!(
                compare_values(&missing_age, &some_age, direction, now),
                Ordering::Greater
            );
        }
    }

    #[test]
    fn descending_reverses_present_values() {
        let now = at(1_000);
        assert_eq!(
            compare_values(&text("a"), &text("b"), SortDirection::Descending, now),
            Ordering::Greater
        );
    }

    #[test]
    fn status_sorts_by_severity_then_text() {
        let ok = status(StatusTone::Ok, "Running");
        let done = status(StatusTone::Done, "Completed");
        let warn = status(StatusTone::Warn, "Pending");
        let bad = status(StatusTone::Bad, "CrashLoopBackOff");
        assert_eq!(ascending(&ok, &done), Ordering::Less);
        assert_eq!(ascending(&done, &warn), Ordering::Less);
        assert_eq!(ascending(&warn, &bad), Ordering::Less);
        let other_bad = status(StatusTone::Bad, "Error");
        assert_eq!(ascending(&bad, &other_bad), Ordering::Less);
    }

    #[test]
    fn age_ascending_puts_youngest_first() {
        let young = CellValue::Age(Some(at(900)));
        let old = CellValue::Age(Some(at(100)));
        assert_eq!(ascending(&young, &old), Ordering::Less);
    }

    #[test]
    fn span_uses_now_for_running_items() {
        let finished = CellValue::Span {
            started: Some(at(100)),
            finished: Some(at(160)),
        };
        let running = CellValue::Span {
            started: Some(at(900)),
            finished: None,
        };
        // 60 s against 100 s so far.
        assert_eq!(ascending(&finished, &running), Ordering::Less);
        let not_started = CellValue::Span {
            started: None,
            finished: None,
        };
        assert_eq!(ascending(&running, &not_started), Ordering::Less);
    }

    #[test]
    fn qualified_sorts_by_prefix_then_text() {
        let qualified = |prefix, text| CellValue::Qualified { prefix, text };
        assert_eq!(
            ascending(&qualified(None, "z"), &qualified(Some("a"), "a")),
            Ordering::Less
        );
        assert_eq!(
            ascending(&qualified(Some("a"), "z"), &qualified(Some("b"), "a")),
            Ordering::Less
        );
        assert_eq!(
            ascending(
                &qualified(Some("a"), "pod-2"),
                &qualified(Some("a"), "pod-10")
            ),
            Ordering::Less
        );
    }
}
