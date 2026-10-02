//! What a table filter keeps: quick text and chips, all of which must pass. Pure, so it is
//! tested without a window.

use gpui_kit::SharedString;

use crate::node_summary::NodeGroup;
use crate::pod_table::NODE as POD_NODE_COLUMN;
use crate::status_tone::StatusTone;
use crate::table_view::{CellValue, TableRow};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TableFilter {
    /// The quick filter text, as typed.
    pub(crate) text: String,
    pub(crate) chips: Vec<FilterChip>,
    /// A screen's own switch, such as Hide inactive or a Nodes summary chip.
    pub(crate) preset: Option<FilterPreset>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FilterChip {
    /// Tone Warn, Bad, or Info.
    Unhealthy,
    /// The text of a column equals `value`: `Node: wk-03` on Pods, `Reason: BackOff` on Events.
    Equals {
        column: usize,
        title: &'static str,
        value: SharedString,
    },
    Label(LabelQuery),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FilterPreset {
    /// ReplicaSets scaled to zero are hidden.
    HideInactive,
    /// A Nodes summary chip.
    Nodes(NodeGroup),
}

/// One kubectl-style label test, such as `app=api`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LabelQuery {
    pub(crate) key: String,
    pub(crate) test: LabelTest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LabelTest {
    Exists,
    Equals(String),
    NotEquals(String),
}

impl TableFilter {
    /// Whether the filter can hide a row.
    pub(crate) fn is_active(&self) -> bool {
        !self.text.trim().is_empty() || !self.chips.is_empty() || self.preset.is_some()
    }

    /// The Pods filter of "View pods on node": only the node chip, so every pod on the node
    /// shows.
    pub(crate) fn on_node(name: &str) -> Self {
        Self {
            chips: vec![FilterChip::Equals {
                column: POD_NODE_COLUMN,
                title: "Node",
                value: name.to_owned().into(),
            }],
            ..Self::default()
        }
    }

    /// Replaces the `Equals` chip of `column`, if any, with a new one; other chips stay.
    pub(crate) fn set_equals(&mut self, column: usize, title: &'static str, value: &str) {
        self.chips.retain(
            |chip| !matches!(chip, FilterChip::Equals { column: other, .. } if *other == column),
        );
        self.chips.push(FilterChip::Equals {
            column,
            title,
            value: value.to_owned().into(),
        });
    }
}

impl LabelQuery {
    /// `label:app=api`, `label:app!=api`, or `label:app`.
    pub(crate) fn text(&self) -> String {
        match &self.test {
            LabelTest::Exists => format!("label:{}", self.key),
            LabelTest::Equals(value) => format!("label:{}={value}", self.key),
            LabelTest::NotEquals(value) => format!("label:{}!={value}", self.key),
        }
    }

    /// Terms are `key=value`. `!=` follows kubectl: a row without the key passes.
    fn passes<'a>(&self, mut terms: impl Iterator<Item = &'a str>) -> bool {
        let value_of = |term: &'a str| term.strip_prefix(self.key.as_str())?.strip_prefix('=');
        match &self.test {
            LabelTest::Exists => terms.any(|term| value_of(term).is_some()),
            LabelTest::Equals(wanted) => terms.any(|term| value_of(term) == Some(wanted)),
            LabelTest::NotEquals(unwanted) => !terms.any(|term| value_of(term) == Some(unwanted)),
        }
    }
}

const LABEL_PREFIX: &str = "label:";

/// Reads `label:k=v,k2!=v2,k3`. `None` when the `label:` prefix is missing, or a part is empty
/// or has an empty key.
pub(crate) fn parse_label_queries(text: &str) -> Option<Vec<LabelQuery>> {
    let parts = text.trim().strip_prefix(LABEL_PREFIX)?;
    parts.split(',').map(parse_label_query).collect()
}

fn parse_label_query(part: &str) -> Option<LabelQuery> {
    let part = part.trim();
    let (key, test) = if let Some((key, value)) = part.split_once("!=") {
        (key, LabelTest::NotEquals(value.trim().to_owned()))
    } else if let Some((key, value)) = part.split_once('=') {
        (key, LabelTest::Equals(value.trim().to_owned()))
    } else {
        (part, LabelTest::Exists)
    };
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    Some(LabelQuery {
        key: key.to_owned(),
        test,
    })
}

/// The quick filter text for what the input holds. A `label:` query waits for Enter to become
/// chips, so it filters nothing by its literal text while it is typed.
pub(crate) fn quick_filter_text(input: &str) -> &str {
    if input.trim_start().starts_with(LABEL_PREFIX) {
        return "";
    }
    input
}

/// Whether `row` passes the text and every chip. `column_count` is the number of logical
/// columns whose text the quick filter searches, hidden columns included.
pub(crate) fn matches<T: TableRow>(row: &T, filter: &TableFilter, column_count: usize) -> bool {
    text_matches(row, filter.text.trim(), column_count)
        && filter.chips.iter().all(|chip| chip_matches(row, chip))
        && filter
            .preset
            .as_ref()
            .is_none_or(|preset| row.in_preset(preset))
}

fn chip_matches<T: TableRow>(row: &T, chip: &FilterChip) -> bool {
    match chip {
        FilterChip::Unhealthy => matches!(
            row.tone(),
            StatusTone::Warn | StatusTone::Bad | StatusTone::Info
        ),
        FilterChip::Equals { column, value, .. } => match row.value(*column) {
            CellValue::Text(text) => text == value.as_ref(),
            CellValue::Qualified { text, .. } => text == value.as_ref(),
            _ => false,
        },
        FilterChip::Label(query) => query.passes(row.labels()),
    }
}

fn text_matches<T: TableRow>(row: &T, needle: &str, column_count: usize) -> bool {
    if needle.is_empty() {
        return true;
    }
    let found = |text: &str| contains_ignore_ascii_case(text, needle);
    // `namespace/name` is only built when the needle can span the slash.
    let is_qualified_match = row.namespace().is_some_and(|namespace| {
        needle.contains('/') && found(&format!("{namespace}/{}", row.name()))
    });
    is_qualified_match
        || row.namespace().is_some_and(found)
        || found(row.name())
        || (0..column_count).any(|column| match row.value(column) {
            CellValue::Text(text) => found(&text),
            CellValue::Qualified { prefix, text } => prefix.is_some_and(found) || found(text),
            CellValue::Status { text, .. } => found(&text),
            CellValue::Number(_)
            | CellValue::Age(_)
            | CellValue::Span { .. }
            | CellValue::Absent => false,
        })
        || row.labels().any(found)
}

/// ASCII case-insensitive substring test; an empty needle is found.
fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    let needle = needle.as_bytes();
    if needle.is_empty() {
        return true;
    }
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

#[cfg(test)]
#[path = "table_filter_tests.rs"]
mod table_filter_tests;
