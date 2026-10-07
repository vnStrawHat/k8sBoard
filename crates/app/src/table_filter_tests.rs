use std::borrow::Cow;

use super::*;

/// A row with one text column, one qualified column, and one status column.
struct Row {
    namespace: Option<&'static str>,
    name: &'static str,
    labels: Vec<&'static str>,
    images: Vec<&'static str>,
    tone: StatusTone,
    node: &'static str,
    status: &'static str,
    /// What the preset sees: Hide inactive drops a row that is not active.
    is_active: bool,
}

impl Default for Row {
    fn default() -> Self {
        Self {
            namespace: Some("payments"),
            name: "api-7",
            labels: vec!["app=api", "tier=web"],
            images: vec![
                "registry.example.com/team/nginx:1.27",
                "envoyproxy/envoy:v1.30",
            ],
            tone: StatusTone::Ok,
            node: "wk-03",
            status: "Running",
            is_active: true,
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

    fn images(&self) -> impl Iterator<Item = &str> {
        self.images.iter().copied()
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

    fn in_preset(&self, _: &FilterPreset) -> bool {
        self.is_active
    }
}

fn text_filter(text: &str) -> TableFilter {
    TableFilter {
        text: text.to_owned(),
        chips: Vec::new(),
        ..Default::default()
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
        ..Default::default()
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
        ..Default::default()
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
fn the_quick_filter_searches_every_image_reference() {
    let row = Row::default();
    assert!(matches(&row, &text_filter("envoy"), COLUMNS));
    assert!(matches(&row, &text_filter("team/nginx:1.27"), COLUMNS));
    assert!(!matches(&row, &text_filter("redis"), COLUMNS));
}

#[test]
fn an_image_query_searches_images_only() {
    let row = Row::default();
    assert!(matches(&row, &text_filter("image:NGINX"), COLUMNS));
    assert!(matches(&row, &text_filter("image: envoy:v1"), COLUMNS));
    assert!(!matches(&row, &text_filter("image:redis"), COLUMNS));
    // `api` is the name and a label, not an image.
    assert!(!matches(&row, &text_filter("image:api"), COLUMNS));
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

fn equals(column: usize, value: &str) -> TableFilter {
    TableFilter {
        chips: vec![FilterChip::Equals {
            column,
            title: "Column",
            value: value.to_owned().into(),
        }],
        ..Default::default()
    }
}

#[test]
fn equals_compares_a_column_value() {
    let row = Row::default();
    assert!(passes(&row, &equals(0, "wk-03")));
    assert!(!passes(&row, &equals(0, "wk-0")));
    assert!(!passes(&row, &equals(0, "WK-03")));
    // A qualified column compares its text, not the prefix.
    assert!(passes(&row, &equals(2, "owner-name")));
    assert!(!passes(&row, &equals(2, "owner-ns")));
    // A status column compares its text, whatever the tone.
    assert!(passes(&row, &equals(1, "Running")));
    assert!(!passes(&row, &equals(1, "Completed")));
    let completed = Row {
        status: "Completed",
        tone: StatusTone::Done,
        ..Row::default()
    };
    assert!(passes(&completed, &equals(1, "Completed")));
    // Numbers and absent columns never equal.
    assert!(!passes(&row, &equals(3, "42")));
    assert!(!passes(&row, &equals(9, "")));
}

#[test]
fn view_pods_on_node_replaces_the_pod_filters() {
    let filter = TableFilter::on_node("wk-03");
    assert_eq!(
        filter.chips,
        [FilterChip::Equals {
            column: POD_NODE_COLUMN,
            title: "Node",
            value: "wk-03".into(),
        }]
    );
    assert!(filter.text.is_empty());
    assert_eq!(filter.preset, None);
}

#[test]
fn filter_similar_replaces_an_existing_reason_chip() {
    let mut filter = label_filter("label:app=api");
    filter.set_equals(1, "Reason", "BackOff");
    filter.set_equals(0, "Type", "Warning");
    filter.set_equals(1, "Reason", "Failed");
    let reasons: Vec<_> = filter
        .chips
        .iter()
        .filter_map(|chip| match chip {
            FilterChip::Equals {
                column: 1, value, ..
            } => Some(value.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(reasons, ["Failed"]);
    // The label chip and the other column's chip stay.
    assert_eq!(filter.chips.len(), 3);
}

#[test]
fn preset_keeps_only_the_rows_it_accepts() {
    let filter = TableFilter {
        preset: Some(FilterPreset::HideInactive),
        ..Default::default()
    };
    assert!(filter.is_active());
    assert!(passes(&Row::default(), &filter));
    let inactive = Row {
        is_active: false,
        ..Default::default()
    };
    assert!(!passes(&inactive, &filter));
    // The preset combines with the chips by AND.
    let mut with_chip = filter.clone();
    with_chip.chips.push(FilterChip::Unhealthy);
    assert!(!passes(&Row::default(), &with_chip));
}

#[test]
fn mounted_none_waits_for_enter_like_a_label_query() {
    assert_eq!(quick_filter_text("mounted:none"), "");
    assert_eq!(quick_filter_text("mounted:"), "");
    // A word that merely starts alike is still text.
    assert_eq!(quick_filter_text("mount"), "mount");
    assert_eq!(quick_filter_text("web"), "web");
}
