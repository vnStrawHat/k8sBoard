//! The JSON view of one log line: a headline message and the remaining fields. No GPUI types.

use std::collections::BTreeMap;

use serde_json::Value;

const MESSAGE_KEYS: [&str; 2] = ["msg", "message"];
/// Shown as the level tag or the row time already.
const HIDDEN_KEYS: [&str; 8] = [
    "level",
    "severity",
    "lvl",
    "log.level",
    "time",
    "ts",
    "timestamp",
    "@timestamp",
];

pub(crate) struct JsonLine {
    pub(crate) headline: Option<String>,
    /// The other fields as pretty-printed lines; empty when nothing remains.
    pub(crate) details: Vec<String>,
}

/// `None` unless `text` (trimmed) is a JSON object.
pub(crate) fn json_line(text: &str) -> Option<JsonLine> {
    let trimmed = text.trim();
    if !trimmed.starts_with('{') {
        return None;
    }
    // A `BTreeMap` sorts the keys even when another crate turns on serde_json's `preserve_order`.
    let mut object: BTreeMap<String, Value> = serde_json::from_str(trimmed).ok()?;
    let headline_key = MESSAGE_KEYS
        .iter()
        .find(|key| matches!(object.get(**key), Some(Value::String(_))));
    let headline = headline_key.and_then(|key| match object.remove(*key) {
        Some(Value::String(message)) => Some(message),
        _ => None,
    });
    for key in HIDDEN_KEYS {
        object.remove(key);
    }
    let details = if object.is_empty() {
        Vec::new()
    } else {
        serde_json::to_string_pretty(&object)
            .map(|pretty| pretty.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    };
    Some(JsonLine { headline, details })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_line_splits_message_and_details() {
        let line = json_line(r#"{"time":"t","msg":"hello","level":"info","b":2,"a":"x"}"#)
            .expect("an object");
        assert_eq!(line.headline.as_deref(), Some("hello"));
        assert_eq!(line.details, ["{", r#"  "a": "x","#, r#"  "b": 2"#, "}"]);
    }

    #[test]
    fn json_line_without_message_has_no_headline() {
        let line = json_line(r#"{"level":"warn","code":7}"#).expect("an object");
        assert_eq!(line.headline, None);
        assert_eq!(line.details, ["{", r#"  "code": 7"#, "}"]);
    }

    #[test]
    fn json_line_with_only_a_message_has_no_details() {
        let bare = json_line(r#"{"msg":"only"}"#).expect("an object");
        assert_eq!(bare.headline.as_deref(), Some("only"));
        assert!(bare.details.is_empty());
    }

    #[test]
    fn non_object_text_is_not_json() {
        assert!(json_line("[1,2]").is_none());
        assert!(json_line("{broken").is_none());
        assert!(json_line("plain text").is_none());
    }
}
