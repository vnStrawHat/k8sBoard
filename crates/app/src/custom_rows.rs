//! The row builder of custom resource objects. A row holds the summary (metadata, masked and
//! typed column values, conditions) and never a raw value: the cluster crate already hid what
//! looks like a credential. Nothing here logs or traces.

use cluster::{ColumnValue, ConditionStatus, CustomObjectSummary, ObjectCondition, PrinterColumn};
use jiff::Timestamp;

use crate::age::format_age;
use crate::certificate_expiry::expiry_label;
use crate::custom_kind::{ColumnRule, CustomKind};
use crate::kind_row::{DateRule, KindCell, KindObject, KindRow, chips};
use crate::status_tone::{StatusLabel, StatusTone};

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
        // The drawer sections arrive with the object drawer.
        sections: Vec::new(),
        event: None,
        related_pods: None,
        labels: chips(&summary.labels),
        object: KindObject::Custom(summary.clone()),
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
        ColumnValue::Text(text) => {
            match condition_label(text).filter(|_| rule == ColumnRule::ConditionStatus) {
                Some(label) => KindCell::Toned(label),
                None => KindCell::Text(text.clone().into()),
            }
        }
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
        Some(phase) => (phase.to_owned(), StatusTone::Info),
        None => ("No status".to_owned(), StatusTone::Info),
    };
    StatusLabel {
        text: text.into(),
        tone,
    }
}

/// A condition other than Ready and Available whose reason names a failure. The substring rule
/// is a ceiling: a reason such as `NoErrors` reads as failing.
fn is_failing(condition: &ObjectCondition) -> bool {
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
