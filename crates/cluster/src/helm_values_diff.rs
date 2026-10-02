//! A path diff of two revisions' values, masked like the Values tab.
//!
//! A line diff of masked YAML would hide a changed secret, so values are compared leaf by leaf
//! and a changed secret still lists as `<hidden> -> <hidden>`. Nothing here logs or traces.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::helm_release_detail::HelmText;
use crate::object_yaml::HIDDEN;

// ponytail: fixed caps; add paging or a per-subtree summary if users hit them.
pub(crate) const DIFF_LIMIT: usize = 500;
// ponytail: long values (PEM blocks) are cut; Reveal and the Values tab show them whole.
pub(crate) const DIFF_VALUE_CHARS: usize = 200;

/// Whether string and number values are shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ValueVisibility {
    #[default]
    Masked,
    Revealed,
}

/// The changes between two revisions, for the user values and for the computed values.
/// Derives nothing: revealed texts must not reach a log.
pub struct HelmValuesDiff {
    pub user: Vec<ValueChange>,
    pub computed: Vec<ValueChange>,
    /// Changes beyond `DIFF_LIMIT`, not listed.
    pub omitted_user: usize,
    pub omitted_computed: usize,
}

/// Added: `before` is `None`. Removed: `after` is `None`. Changed: both. `path` holds keys only.
pub struct ValueChange {
    pub path: String,
    pub before: Option<HelmText>,
    pub after: Option<HelmText>,
}

/// Leaves keyed by path: scalars, empty objects, and empty arrays.
fn flatten(value: &Value) -> BTreeMap<String, &Value> {
    let mut leaves = BTreeMap::new();
    collect_leaves(value, String::new(), &mut leaves);
    leaves
}

fn collect_leaves<'a>(value: &'a Value, path: String, leaves: &mut BTreeMap<String, &'a Value>) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                collect_leaves(child, key_path(&path, key), leaves);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for (index, child) in items.iter().enumerate() {
                collect_leaves(child, format!("{path}[{index}]"), leaves);
            }
        }
        // An empty root has no leaves; the root itself is never a path.
        _ if path.is_empty() => {}
        _ => {
            leaves.insert(path, value);
        }
    }
}

/// Plain keys join with `.`; any other key is quoted, like `annotations["a/b"]`.
fn key_path(parent: &str, key: &str) -> String {
    let is_plain = !key.is_empty()
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
    if is_plain {
        return if parent.is_empty() {
            key.to_owned()
        } else {
            format!("{parent}.{key}")
        };
    }
    let quoted = Value::from(key).to_string();
    format!("{parent}[{quoted}]")
}

/// Changes sorted by path, at most `DIFF_LIMIT`; the second field counts the rest.
pub(crate) fn values_diff(
    before: &Value,
    after: &Value,
    visibility: ValueVisibility,
) -> (Vec<ValueChange>, usize) {
    let mut sides: BTreeMap<String, (Option<&Value>, Option<&Value>)> = BTreeMap::new();
    for (path, value) in flatten(before) {
        sides.entry(path).or_default().0 = Some(value);
    }
    for (path, value) in flatten(after) {
        sides.entry(path).or_default().1 = Some(value);
    }
    let mut changes = Vec::new();
    let mut omitted = 0;
    for (path, (old, new)) in sides {
        if old == new {
            continue;
        }
        if changes.len() == DIFF_LIMIT {
            omitted += 1;
            continue;
        }
        changes.push(ValueChange {
            path,
            before: old.map(|value| HelmText::new(leaf_text(value, visibility))),
            after: new.map(|value| HelmText::new(leaf_text(value, visibility))),
        });
    }
    (changes, omitted)
}

/// A leaf as JSON text, cut at `DIFF_VALUE_CHARS`. Masked strings and numbers read `<hidden>`.
fn leaf_text(value: &Value, visibility: ValueVisibility) -> String {
    let is_secret_capable = matches!(value, Value::String(_) | Value::Number(_));
    let text = if visibility == ValueVisibility::Masked && is_secret_capable {
        HIDDEN.to_owned()
    } else {
        value.to_string()
    };
    match text.char_indices().nth(DIFF_VALUE_CHARS) {
        Some((end, _)) => format!("{}\u{2026}", &text[..end]),
        None => text,
    }
}

#[cfg(test)]
#[path = "helm_values_diff_tests.rs"]
mod helm_values_diff_tests;
