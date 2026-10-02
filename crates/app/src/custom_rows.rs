//! The row builder of custom resource objects. A row holds the summary (metadata, masked and
//! typed column values, conditions) and never a raw value: the cluster crate already hid what
//! looks like a credential. Nothing here logs or traces.

use cluster::{
    ColumnType, ColumnValue, ConditionStatus, CustomObjectFields, CustomObjectSummary, FieldEntry,
    FieldList, FieldValue, ObjectCondition, PrinterColumn,
};
use jiff::Timestamp;

use crate::age::format_age;
use crate::certificate_expiry::expiry_label;
use crate::cluster_session::LiveList;
use crate::custom_kind::{ColumnRule, CustomKind};
use crate::kind_row::{
    DateRule, DetailRow, DetailSection, KindCell, KindObject, KindRow, LiveContent, chips,
};
use crate::status_tone::{StatusLabel, StatusTone};
use crate::table_selection::ResourceKey;

/// What a hidden column value reads as.
const HIDDEN_TEXT: &str = "<hidden>";
const AGE_COLUMN_NAME: &str = "Age";

pub(crate) fn custom_object_row(kind: CustomKind, summary: &CustomObjectSummary) -> KindRow {
    KindRow {
        namespace: summary.namespace.clone(),
        name: summary.name.clone(),
        created_at: summary.created_at,
        status: custom_status(&summary.conditions, summary.phase.as_deref()),
        cells: custom_cells(kind, summary),
        // Read at paint time: the conditions from the row, the fields from the related watch.
        sections: vec![
            live_section("Conditions", LiveContent::CustomConditions),
            live_section("Status", LiveContent::CustomStatus),
            live_section("Spec", LiveContent::CustomSpec),
        ],
        event: None,
        related_pods: None,
        labels: chips(&summary.labels),
        object: KindObject::Custom(summary.clone()),
    }
}

fn live_section(title: &'static str, content: LiveContent) -> DetailSection {
    DetailSection {
        title,
        rows: vec![DetailRow::Live(content)],
    }
}

/// One cell per printer column; a kind without printer columns shows Age only.
fn custom_cells(kind: CustomKind, summary: &CustomObjectSummary) -> Vec<KindCell> {
    let columns = kind.printer_columns();
    if columns.is_empty() {
        return vec![KindCell::age(summary.created_at)];
    }
    columns
        .iter()
        .zip(kind.column_rules())
        .enumerate()
        .map(|(index, (column, rule))| {
            let value = summary.columns.get(index).unwrap_or(&ColumnValue::Absent);
            custom_cell(column, *rule, value)
        })
        .collect()
}

fn custom_cell(column: &PrinterColumn, rule: ColumnRule, value: &ColumnValue) -> KindCell {
    match value {
        ColumnValue::Absent => KindCell::Absent,
        ColumnValue::Hidden => KindCell::Text(HIDDEN_TEXT.into()),
        ColumnValue::Text(text) => match text_tone(column, rule, text) {
            Some(tone) => KindCell::Toned(StatusLabel {
                text: text.clone().into(),
                tone,
            }),
            None => KindCell::Text(text.clone().into()),
        },
        ColumnValue::Integer(number) => match u64::try_from(*number) {
            Ok(value) => KindCell::Quantity {
                text: number.to_string().into(),
                value,
                tone: None,
            },
            // A negative number cannot sort as a quantity, so it stays text.
            Err(_) => KindCell::Text(number.to_string().into()),
        },
        ColumnValue::Number(text) => KindCell::Text(text.clone().into()),
        ColumnValue::Boolean(flag) => KindCell::Text(flag.to_string().into()),
        ColumnValue::Date(at) if column.name == AGE_COLUMN_NAME => KindCell::age(Some(*at)),
        ColumnValue::Date(at) => KindCell::Date {
            at: *at,
            rule: if rule == ColumnRule::Expiry {
                DateRule::Expiry
            } else {
                DateRule::Plain
            },
        },
    }
}

/// The tone of a string cell: a condition-status column keeps the condition colors, and a string
/// column named like a status (`status_tone`) reads its value through the table.
fn text_tone(column: &PrinterColumn, rule: ColumnRule, text: &str) -> Option<StatusTone> {
    if rule == ColumnRule::ConditionStatus {
        return condition_label(text).map(|label| label.tone);
    }
    if column.column_type != ColumnType::String || !is_status_column(&column.name) {
        return None;
    }
    value_tone(text)
}

/// Whether a column name says it holds a state: it contains Status, Ready, Health, Sync, or Phase.
fn is_status_column(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    ["status", "ready", "health", "sync", "phase"]
        .iter()
        .any(|word| name.contains(word))
}

/// The tone of a well-known state word, ASCII case-insensitive. Anything else has none, so an
/// unfamiliar value is never painted as good or bad.
pub(crate) fn value_tone(text: &str) -> Option<StatusTone> {
    const OK: [&str; 9] = [
        "healthy",
        "synced",
        "true",
        "ready",
        "running",
        "succeeded",
        "completed",
        "bound",
        "available",
    ];
    const BAD: [&str; 5] = ["degraded", "failed", "error", "false", "missing"];
    const WARN: [&str; 6] = [
        "outofsync",
        "progressing",
        "pending",
        "unknown",
        "suspended",
        "terminating",
    ];
    let word = text.to_ascii_lowercase();
    let is_in = |words: &[&str]| words.contains(&word.as_str());
    if is_in(&OK) {
        Some(StatusTone::Ok)
    } else if is_in(&BAD) {
        Some(StatusTone::Bad)
    } else if is_in(&WARN) {
        Some(StatusTone::Warn)
    } else {
        None
    }
}

/// `True` is Ok, `False` Bad, `Unknown` Warn; any other text is not a condition status.
pub(crate) fn condition_label(status_text: &str) -> Option<StatusLabel> {
    let tone = match status_text {
        "True" => StatusTone::Ok,
        "False" => StatusTone::Bad,
        "Unknown" => StatusTone::Warn,
        _ => return None,
    };
    Some(StatusLabel {
        text: status_text.to_owned().into(),
        tone,
    })
}

/// The row status, by the operator convention: `Ready`, else `Available`, and a failing
/// condition raises a Warn.
pub(crate) fn custom_status(conditions: &[ObjectCondition], phase: Option<&str>) -> StatusLabel {
    let failing = conditions.iter().find(|condition| is_failing(condition));
    let failing_label = failing.map(|condition| StatusLabel {
        text: format!(
            "{}: {}",
            condition.name,
            condition.reason.as_deref().unwrap_or_default()
        )
        .into(),
        tone: StatusTone::Warn,
    });
    let main = ["Ready", "Available"]
        .into_iter()
        .find_map(|name| conditions.iter().find(|condition| condition.name == name));
    if let Some(main) = main {
        return main_status(main, failing_label);
    }
    if let Some(label) = failing_label {
        return label;
    }
    let (text, tone) = match phase {
        Some(phase) => (
            phase.to_owned(),
            value_tone(phase).unwrap_or(StatusTone::Info),
        ),
        None => ("No status".to_owned(), StatusTone::Info),
    };
    StatusLabel {
        text: text.into(),
        tone,
    }
}

/// A condition other than Ready and Available whose reason names a failure. The substring rule
/// is a ceiling: a reason such as `NoErrors` reads as failing.
pub(crate) fn is_failing(condition: &ObjectCondition) -> bool {
    if matches!(condition.name.as_str(), "Ready" | "Available") {
        return false;
    }
    let reason = condition.reason.as_deref().unwrap_or_default();
    let reason = reason.to_ascii_lowercase();
    reason.contains("fail") || reason.contains("error")
}

fn main_status(main: &ObjectCondition, failing: Option<StatusLabel>) -> StatusLabel {
    let is_ready = main.name == "Ready";
    let reason = main.reason.clone();
    let (text, tone) = match main.status {
        ConditionStatus::True => {
            if let Some(failing) = failing {
                return failing;
            }
            (main.name.clone(), StatusTone::Ok)
        }
        ConditionStatus::False => (
            reason.unwrap_or_else(|| if is_ready { "Not ready" } else { "Unavailable" }.to_owned()),
            StatusTone::Bad,
        ),
        ConditionStatus::Unknown => (
            reason.unwrap_or_else(|| "Unknown".to_owned()),
            StatusTone::Warn,
        ),
    };
    StatusLabel {
        text: text.into(),
        tone,
    }
}

// ---- Drawer sections ----

/// A spec or status value longer than this is stacked under its label.
const MAX_INLINE_FIELD_CHARS: usize = 60;
/// A label longer than this does not fit the label column, so it goes above its value.
const MAX_INLINE_LABEL_CHARS: usize = 20;
/// The key whose value names a Secret of the object's namespace.
const SECRET_NAME_FIELD: &str = "secretName";

/// Which side of the object a fields section shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FieldsSide {
    Status,
    Spec,
}

impl FieldsSide {
    fn empty_note(self) -> &'static str {
        match self {
            Self::Status => "No status fields.",
            Self::Spec => "No spec fields.",
        }
    }
}

/// The Conditions section: type, status (with the reason), and the message under it.
pub(crate) fn conditions_rows(conditions: &[ObjectCondition]) -> Vec<DetailRow> {
    if conditions.is_empty() {
        return vec![DetailRow::Note("No conditions reported.".into())];
    }
    let mut rows = Vec::new();
    for condition in conditions {
        let status = match condition.status {
            ConditionStatus::True => "True",
            ConditionStatus::False => "False",
            ConditionStatus::Unknown => "Unknown",
        };
        let text = match condition.reason.as_deref() {
            Some(reason) => format!("{status} · {reason}"),
            None => status.to_owned(),
        };
        rows.push(DetailRow::field(
            condition.name.clone(),
            KindCell::Toned(StatusLabel {
                text: text.into(),
                tone: condition_tone(condition),
            }),
        ));
        if let Some(message) = &condition.message {
            rows.push(DetailRow::Note(message.clone().into()));
        }
    }
    rows
}

/// Ready and Available are good when True; any other type is Info unless its reason names a
/// failure.
fn condition_tone(condition: &ObjectCondition) -> StatusTone {
    if matches!(condition.name.as_str(), "Ready" | "Available") {
        return match condition.status {
            ConditionStatus::True => StatusTone::Ok,
            ConditionStatus::False => StatusTone::Bad,
            ConditionStatus::Unknown => StatusTone::Warn,
        };
    }
    if is_failing(condition) {
        StatusTone::Bad
    } else {
        StatusTone::Info
    }
}

/// The rows of one side of the object, from the related fields watch: a note while it loads, why
/// it failed, or that the object is gone (an empty snapshot).
pub(crate) fn field_list_rows(
    state: Option<&LiveList<CustomObjectFields>>,
    side: FieldsSide,
    namespace: Option<&str>,
) -> Vec<DetailRow> {
    let note = |text: String| vec![DetailRow::Note(text.into())];
    match state {
        None | Some(LiveList::Loading) => note("Loading…".to_owned()),
        Some(LiveList::Failed { message }) => note(message.clone()),
        Some(LiveList::Ready { items, .. }) => match items.first() {
            None => note("The object no longer exists.".to_owned()),
            Some(fields) => {
                let list = match side {
                    FieldsSide::Status => &fields.status,
                    FieldsSide::Spec => &fields.spec,
                };
                fields_rows(list, side, namespace)
            }
        },
    }
}

fn fields_rows(list: &FieldList, side: FieldsSide, namespace: Option<&str>) -> Vec<DetailRow> {
    if list.entries.is_empty() && list.omitted == 0 {
        return vec![DetailRow::Note(side.empty_note().into())];
    }
    let mut rows: Vec<DetailRow> = list
        .entries
        .iter()
        .map(|entry| field_row(entry, namespace))
        .collect();
    if list.omitted > 0 {
        rows.push(DetailRow::Note(
            format!("{} more fields in the YAML tab.", list.omitted).into(),
        ));
    }
    rows
}

fn field_row(entry: &FieldEntry, namespace: Option<&str>) -> DetailRow {
    let label = entry.path.clone();
    let has_long_label = label.chars().count() > MAX_INLINE_LABEL_CHARS;
    let text = match &entry.value {
        FieldValue::Hidden => {
            return labelled(label, KindCell::Text(HIDDEN_TEXT.into()), false);
        }
        FieldValue::Items(count) => {
            return labelled(
                label,
                KindCell::Text(format!("{count} items").into()),
                false,
            );
        }
        FieldValue::Fields(count) => {
            return labelled(
                label,
                KindCell::Text(format!("{count} fields").into()),
                false,
            );
        }
        FieldValue::Text(text) => text,
    };
    let is_secret_name = entry.path.rsplit('.').next() == Some(SECRET_NAME_FIELD);
    // A cluster-scoped object has no namespace to look a Secret up in.
    let target = namespace
        .filter(|_| is_secret_name)
        .and_then(|namespace| ResourceKey::of_object("Secret", Some(namespace), text));
    if let Some(target) = target {
        let (label, text) = (label.into(), text.clone().into());
        return if has_long_label {
            DetailRow::StackedLink {
                label,
                text,
                target,
            }
        } else {
            DetailRow::Link {
                label,
                text,
                target,
            }
        };
    }
    let is_long_value = text.chars().count() > MAX_INLINE_FIELD_CHARS;
    labelled(label, KindCell::Mono(text.clone().into()), is_long_value)
}

/// A field row, with the label above the value when either is too long for one line.
fn labelled(label: String, value: KindCell, is_long_value: bool) -> DetailRow {
    if is_long_value || label.chars().count() > MAX_INLINE_LABEL_CHARS {
        DetailRow::stacked(label, value)
    } else {
        DetailRow::field(label, value)
    }
}

/// `5d` for a past date, `in 6d` for a future one (kubectl prints `<invalid>` there).
pub(crate) fn date_text(at: Timestamp, now: Timestamp) -> String {
    if at > now {
        format!("in {}", format_age(Some(now), at))
    } else {
        format_age(Some(at), now)
    }
}

/// The tone of a date cell: only an expiry within 14 days or past is toned, like the TLS expiry.
pub(crate) fn date_tone(rule: DateRule, at: Timestamp, now: Timestamp) -> Option<StatusTone> {
    match rule {
        DateRule::Plain => None,
        DateRule::Expiry => Some(expiry_label(at, now).tone).filter(|tone| *tone != StatusTone::Ok),
    }
}

#[cfg(test)]
#[path = "custom_rows_tests.rs"]
mod custom_rows_tests;
