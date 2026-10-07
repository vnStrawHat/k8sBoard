use std::borrow::Cow;

use super::*;
use crate::resource_kind::{Align, column};
use crate::status_tone::StatusTone;
use crate::table_filter::FilterPreset;
use crate::table_view::CellValue;

struct Row {
    namespace: Option<&'static str>,
    name: &'static str,
    status: &'static str,
    note: &'static str,
    restarts: i64,
}

impl TableRow for Row {
    fn namespace(&self) -> Option<&str> {
        self.namespace
    }

    fn name(&self) -> &str {
        self.name
    }

    fn labels(&self) -> impl Iterator<Item = &str> {
        std::iter::empty()
    }

    fn tone(&self) -> StatusTone {
        StatusTone::Ok
    }

    fn value(&self, column: usize) -> CellValue<'_> {
        match column {
            0 => CellValue::Qualified {
                prefix: self.namespace,
                text: self.name,
            },
            1 => CellValue::Status {
                tone: StatusTone::Ok,
                text: self.status.into(),
            },
            2 => CellValue::Text(Cow::Borrowed(self.note)),
            3 => CellValue::Number(self.restarts),
            4 => CellValue::Age(Some(
                jiff::Timestamp::from_second(0).expect("a valid timestamp"),
            )),
            _ => CellValue::Absent,
        }
    }

    fn in_preset(&self, _: &FilterPreset) -> bool {
        true
    }
}

fn plan() -> ColumnPlan {
    ColumnPlan {
        specs: vec![
            column("Name", 100., Align::Left),
            column("Status", 80., Align::Left),
            column("Note", 80., Align::Left),
            column("Restarts", 60., Align::Right),
            column("Age", 60., Align::Right),
        ],
        flexible: 0,
    }
}

fn items() -> Vec<Row> {
    vec![
        Row {
            namespace: Some("shop"),
            name: "api-0",
            status: "Running",
            note: "plain",
            restarts: 0,
        },
        Row {
            namespace: Some("shop"),
            name: "api-1",
            status: "CrashLoopBackOff",
            note: "says \"hi\", then leaves",
            restarts: 12,
        },
        Row {
            namespace: None,
            name: "cluster-thing",
            status: "Running",
            note: "=HYPERLINK(\"http://x\")",
            restarts: 3,
        },
    ]
}

fn now() -> jiff::Timestamp {
    jiff::Timestamp::from_second(90).expect("a valid timestamp")
}

#[test]
fn the_csv_has_a_header_and_one_line_per_shown_row() {
    let mut view = TableView::default();
    view.rebuild(&items(), 5, now());
    let csv = table_csv(&view, &plan(), &items(), now());
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines[0], "Name,Status,Note,Restarts,Age");
    assert_eq!(lines[1], "shop/api-0,Running,plain,0,1m");
    assert_eq!(lines.len(), 4);
}

#[test]
fn the_csv_skips_hidden_columns_and_filtered_rows_and_keeps_the_sort() {
    let mut view = TableView::default();
    view.hidden.insert(2);
    view.hidden.insert(4);
    view.filter.text = "Running".to_owned();
    // Descending by Name: a namespaced name sorts above a cluster-wide one.
    view.sort = Some(crate::table_sort::TableSort {
        column: 0,
        direction: crate::table_sort::SortDirection::Descending,
    });
    view.rebuild(&items(), 5, now());
    let csv = table_csv(&view, &plan(), &items(), now());
    assert_eq!(
        csv,
        "Name,Status,Restarts\nshop/api-0,Running,0\ncluster-thing,Running,3\n"
    );
}

#[test]
fn a_field_with_a_comma_a_quote_or_a_line_break_is_quoted() {
    assert_eq!(csv_field("plain"), "plain");
    assert_eq!(csv_field("a,b"), "\"a,b\"");
    assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
    assert_eq!(csv_field("two\nlines"), "\"two\nlines\"");
}

#[test]
fn a_text_a_spreadsheet_would_run_as_a_formula_gets_a_leading_quote() {
    assert_eq!(csv_field("=1+1"), "'=1+1");
    assert_eq!(csv_field("@SUM(A1)"), "'@SUM(A1)");
    assert_eq!(csv_field("-rf"), "'-rf");
    assert_eq!(csv_field("+cmd"), "'+cmd");
    // A number is no formula.
    assert_eq!(csv_field("-5"), "-5");
    assert_eq!(csv_field("+3.5"), "+3.5");
    // The guard and the quoting go together.
    assert_eq!(csv_field("=HYPERLINK(\"x\")"), "\"'=HYPERLINK(\"\"x\"\")\"");
}

#[test]
fn the_default_export_text_reads_every_kind_of_cell() {
    let at = |seconds: i64| jiff::Timestamp::from_second(seconds).expect("a valid timestamp");
    let text = |value| crate::table_view::cell_text(value, at(100));
    assert_eq!(text(CellValue::Text(Cow::Borrowed("x"))), "x");
    assert_eq!(
        text(CellValue::Qualified {
            prefix: None,
            text: "node-1"
        }),
        "node-1"
    );
    assert_eq!(text(CellValue::Number(-4)), "-4");
    assert_eq!(text(CellValue::Age(Some(at(40)))), "1m");
    assert_eq!(text(CellValue::Age(None)), "");
    assert_eq!(
        text(CellValue::Span {
            started: Some(at(10)),
            finished: Some(at(22))
        }),
        "12s"
    );
    assert_eq!(
        text(CellValue::Span {
            started: Some(at(10)),
            finished: None
        }),
        "1m"
    );
    assert_eq!(text(CellValue::Absent), "");
}
