use serde_json::json;

use super::*;

fn paths(changes: &[ValueChange]) -> Vec<&str> {
    changes.iter().map(|change| change.path.as_str()).collect()
}

fn text(side: &Option<HelmText>) -> Option<&str> {
    side.as_ref().map(HelmText::as_str)
}

#[test]
fn diff_lists_added_removed_changed_sorted() {
    let before = json!({"b": true, "a": false, "gone": true});
    let after = json!({"b": false, "a": false, "new": true});
    let (changes, omitted) = values_diff(&before, &after, ValueVisibility::Revealed);
    assert_eq!(omitted, 0);
    assert_eq!(paths(&changes), ["b", "gone", "new"]);
    assert_eq!(
        (text(&changes[0].before), text(&changes[0].after)),
        (Some("true"), Some("false"))
    );
    assert_eq!(
        (text(&changes[1].before), text(&changes[1].after)),
        (Some("true"), None)
    );
    assert_eq!(
        (text(&changes[2].before), text(&changes[2].after)),
        (None, Some("true"))
    );
}

#[test]
fn diff_masks_strings_and_numbers() {
    let before = json!({"pass": "old-secret", "port": 80, "debug": false});
    let after = json!({"pass": "new-secret", "port": 81, "debug": true});
    let (changes, _) = values_diff(&before, &after, ValueVisibility::Masked);
    assert_eq!(paths(&changes), ["debug", "pass", "port"]);
    assert_eq!(text(&changes[0].after), Some("true"));
    for change in &changes[1..] {
        assert_eq!(text(&change.before), Some("<hidden>"));
        assert_eq!(text(&change.after), Some("<hidden>"));
    }
    let (revealed, _) = values_diff(&before, &after, ValueVisibility::Revealed);
    assert_eq!(text(&revealed[1].before), Some("\"old-secret\""));
    assert_eq!(text(&revealed[2].after), Some("81"));
}

#[test]
fn diff_keeps_masked_change_visible() {
    let (changes, _) = values_diff(
        &json!({"token": "one"}),
        &json!({"token": "two"}),
        ValueVisibility::Masked,
    );
    assert_eq!(changes.len(), 1);
    assert_eq!(text(&changes[0].before), Some("<hidden>"));
    assert_eq!(text(&changes[0].after), Some("<hidden>"));
}

#[test]
fn diff_of_equal_values_is_empty() {
    let value = json!({"a": {"b": [1, 2]}, "c": {}, "d": []});
    let (changes, omitted) = values_diff(&value, &value.clone(), ValueVisibility::Revealed);
    assert!(changes.is_empty());
    assert_eq!(omitted, 0);
}

#[test]
fn diff_paths_quote_unusual_keys() {
    let after =
        json!({"ingress": {"annotations": {"kubernetes.io/ingress.class": "nginx"}}, "a b": true});
    let (changes, _) = values_diff(&json!({}), &after, ValueVisibility::Masked);
    assert_eq!(
        paths(&changes),
        [
            "[\"a b\"]",
            "ingress.annotations[\"kubernetes.io/ingress.class\"]"
        ]
    );
}

#[test]
fn diff_paths_index_arrays() {
    let after = json!({"servers": [{"host": "a"}, {"host": "b"}]});
    let (changes, _) = values_diff(&json!({}), &after, ValueVisibility::Masked);
    assert_eq!(paths(&changes), ["servers[0].host", "servers[1].host"]);
}

#[test]
fn diff_cuts_long_values() {
    let long = "x".repeat(DIFF_VALUE_CHARS + 50);
    let (changes, _) = values_diff(&json!({}), &json!({"pem": long}), ValueVisibility::Revealed);
    let shown = text(&changes[0].after).expect("added");
    // Two quotes around the string count toward the cut.
    assert_eq!(shown.chars().count(), DIFF_VALUE_CHARS + 1);
    assert!(shown.ends_with('\u{2026}'));
}

#[test]
fn diff_caps_at_limit() {
    let after: serde_json::Map<String, Value> = (0..DIFF_LIMIT + 7)
        .map(|index| (format!("k{index:04}"), Value::from(true)))
        .collect();
    let (changes, omitted) =
        values_diff(&json!({}), &Value::Object(after), ValueVisibility::Masked);
    assert_eq!(changes.len(), DIFF_LIMIT);
    assert_eq!(omitted, 7);
}
