use std::borrow::Cow;

use super::*;

/// A row with one text column, one qualified column, and one status column.
struct Row {
    namespace: Option<&'static str>,
    name: &'static str,
    labels: Vec<&'static str>,
    tone: StatusTone,
    node: &'static str,
    status: &'static str,
}

impl Default for Row {
    fn default() -> Self {
        Self {
            namespace: Some("payments"),
            name: "api-7",
            labels: vec!["app=api", "tier=web"],
            tone: StatusTone::Ok,
            node: "wk-03",
            status: "Running",
        }
    }
}

impl TableRow for Row {
    fn namespace(&self) -> Option<&str> {
        self.namespace
    }

    fn name(&self) -> &str {
        self.name
    }

    fn labels(&self) -> impl Iterator<Item = &str> {
        self.labels.iter().copied()
    }

    fn tone(&self) -> StatusTone {
        self.tone
    }

    fn value(&self, column: usize) -> CellValue<'_> {
        match column {
            0 => CellValue::Text(Cow::Borrowed(self.node)),
            1 => CellValue::Status {
                tone: self.tone,
                text: self.status.into(),
            },
            2 => CellValue::Qualified {
                prefix: Some("owner-ns"),
                text: "owner-name",
            },
            3 => CellValue::Number(42),
            _ => CellValue::Absent,
        }
    }
}

fn text_filter(text: &str) -> TableFilter {
    TableFilter {
        text: text.to_owned(),
        chips: Vec::new(),
    }
}

fn label_filter(query: &str) -> TableFilter {
    TableFilter {
        text: String::new(),
        chips: parse_label_queries(query)
            .expect("a valid label query")
            .into_iter()
            .map(FilterChip::Label)
            .collect(),
    }
}

const COLUMNS: usize = 4;

fn passes(row: &Row, filter: &TableFilter) -> bool {
    matches(row, filter, COLUMNS)
}

#[test]
fn text_matches_name_namespace_columns_and_labels() {
    let row = Row::default();
    for needle in [
        "api-7",        // name
        "payments",     // namespace
        "payments/api", // namespace and name
        "wk-03",        // text column
        "Running",      // status column
        "owner-ns",     // qualified prefix
        "owner-name",   // qualified text
        "app=api",      // label term
    ] {
        assert!(passes(&row, &text_filter(needle)), "{needle}");
    }
    assert!(!passes(&row, &text_filter("absent-text")));
    // Numbers are not searched: a count must not match by accident.
    assert!(!passes(&row, &text_filter("42")));
}

#[test]
fn text_ignores_ascii_case_and_surrounding_spaces() {
    let row = Row::default();
    assert!(passes(&row, &text_filter("  RUNNING ")));
    assert!(passes(&row, &text_filter("Payments/API")));
    assert!(passes(&row, &text_filter("   ")));
    assert!(contains_ignore_ascii_case("CrashLoopBackOff", "loopback"));
    assert!(contains_ignore_ascii_case("anything", ""));
    assert!(!contains_ignore_ascii_case("ab", "abc"));
}

#[test]
fn unhealthy_keeps_warn_bad_and_info() {
    let filter = TableFilter {
        text: String::new(),
        chips: vec![FilterChip::Unhealthy],
    };
    for (tone, kept) in [
        (StatusTone::Ok, false),
        (StatusTone::Done, false),
        (StatusTone::Info, true),
        (StatusTone::Warn, true),
        (StatusTone::Bad, true),
    ] {
        let row = Row {
            tone,
            ..Default::default()
        };
        assert_eq!(passes(&row, &filter), kept, "{tone:?}");
    }
}

#[test]
fn label_tests_follow_kubectl_semantics() {
    let row = Row::default();
    assert!(passes(&row, &label_filter("label:app")));
    assert!(!passes(&row, &label_filter("label:missing")));
    assert!(passes(&row, &label_filter("label:app=api")));
    assert!(!passes(&row, &label_filter("label:app=web")));
    assert!(passes(&row, &label_filter("label:app!=web")));
    assert!(!passes(&row, &label_filter("label:app!=api")));
    // An absent key passes `!=`, like kubectl.
    assert!(passes(&row, &label_filter("label:missing!=x")));
    // A key is not a prefix match.
    assert!(!passes(&row, &label_filter("label:ap")));
}

#[test]
fn parse_label_queries_reads_comma_lists() {
    let queries = parse_label_queries("label:a=b, c!=d,e").expect("valid");
    assert_eq!(
        queries,
        [
            LabelQuery {
                key: "a".to_owned(),
                test: LabelTest::Equals("b".to_owned())
            },
            LabelQuery {
                key: "c".to_owned(),
                test: LabelTest::NotEquals("d".to_owned())
            },
            LabelQuery {
                key: "e".to_owned(),
                test: LabelTest::Exists
            },
        ]
    );
    let texts: Vec<_> = queries.iter().map(LabelQuery::text).collect();
    assert_eq!(texts, ["label:a=b", "label:c!=d", "label:e"]);
}

#[test]
fn parse_label_queries_rejects_bad_input() {
    assert_eq!(parse_label_queries("app=api"), None);
    assert_eq!(parse_label_queries("label:"), None);
    assert_eq!(parse_label_queries("label:=api"), None);
    assert_eq!(parse_label_queries("label:a=b,"), None);
    assert_eq!(parse_label_queries("label:a,,b"), None);
    assert_eq!(parse_label_queries("label: !=x"), None);
}

#[test]
fn filter_parts_combine_with_and() {
    let row = Row::default();
    let mut filter = text_filter("payments");
    filter.chips.push(FilterChip::Label(LabelQuery {
        key: "app".to_owned(),
        test: LabelTest::Equals("api".to_owned()),
    }));
    assert!(passes(&row, &filter));
    filter.chips.push(FilterChip::Unhealthy);
    assert!(!passes(&row, &filter));
    assert!(filter.is_active());
    assert!(!TableFilter::default().is_active());
    assert!(!text_filter("  ").is_active());
}

#[test]
fn quick_filter_text_ignores_a_label_query_in_progress() {
    assert_eq!(quick_filter_text("argo"), "argo");
    assert_eq!(quick_filter_text("label:app=api"), "");
    assert_eq!(quick_filter_text("  label:"), "");
    // Only the prefix makes it a query.
    assert_eq!(quick_filter_text("my-label:x"), "my-label:x");
}
