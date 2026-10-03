//! The `<hidden>` placeholder of an edited object (spec 0031 decisions 6-7): it means "keep the
//! server's value at this location". `check` proves a placeholder has a counterpart before
//! anything is sent; `restore` puts the server's raw value back.
//!
//! Nothing here logs: the trees can hold secrets.

use std::collections::HashSet;

use serde_json::Value;

use crate::edit_preview::{FieldPath, PathSegment};

/// What a masked value reads as in the editor text and the previews.
pub(crate) const HIDDEN: &str = "<hidden>";
/// The previews read a masked value whose raw sides differ, or that was matched by position, as
/// these. They are diff markers, never placeholders: text that starts like one is refused.
pub(crate) const HIDDEN_CHANGED: &str = "<hidden, changed>";
pub(crate) const HIDDEN_MOVED: &str = "<hidden, moved>";
pub(crate) const MARKER_PREFIX: &str = "<hidden";

/// Why a placeholder cannot be restored. Both carry the path of the first offender in key order.
#[derive(Debug)]
pub(crate) enum Unrestorable {
    /// A placeholder with no counterpart on the server.
    Unmatched(FieldPath),
    /// Text that starts with `<hidden` but is not exactly the placeholder (a diff marker).
    MarkerText(FieldPath),
}

impl Unrestorable {
    pub(crate) fn path(&self) -> &FieldPath {
        match self {
            Self::Unmatched(path) | Self::MarkerText(path) => path,
        }
    }
}

/// Placeholders that were restored through an index-matched list item whose other fields differ
/// from the server's item: the value may belong to a different item now.
#[derive(Debug)]
pub(crate) struct Restored {
    pub(crate) moved: Vec<FieldPath>,
}

/// Whether every item is an object with a string `name`, and no two names are equal. Only such
/// lists are matched by name.
pub(crate) fn has_unique_names(list: &[Value]) -> bool {
    let mut seen = HashSet::new();
    list.iter()
        .all(|item| name_of(item).is_some_and(|name| seen.insert(name)))
}

pub(crate) fn name_of(item: &Value) -> Option<&str> {
    item.get("name").and_then(Value::as_str)
}

/// The counterpart in `list` of `item`, which sits at `index` of `other`: by `name` when every
/// item of both lists is an object with a unique string `name`, otherwise by index.
pub(crate) fn counterpart<'a>(
    list: &'a [Value],
    item: &Value,
    index: usize,
    other: &[Value],
) -> Option<&'a Value> {
    if has_unique_names(list) && has_unique_names(other) {
        let name = name_of(item)?;
        return list
            .iter()
            .find(|candidate| name_of(candidate) == Some(name));
    }
    list.get(index)
}

/// Proves every placeholder of `edited` has a counterpart in `base`; the error is the first
/// unmatched one in key order. The invariant (decision 7): a base and a fresh object of the same
/// `resourceVersion` have the same shape, so `restore` cannot fail after this passes.
pub(crate) fn check(edited: &Value, base: &Value) -> Result<(), Unrestorable> {
    restore(&mut edited.clone(), base).map(|_restored| ())
}

/// Replaces each placeholder of `edited` with the raw value of its counterpart in `fresh`. The
/// error is the first placeholder without a counterpart, or the first diff marker text, which the
/// server must never receive as a literal.
pub(crate) fn restore(edited: &mut Value, fresh: &Value) -> Result<Restored, Unrestorable> {
    let mut restored = Restored { moved: Vec::new() };
    let mut at = Vec::new();
    restore_at(edited, Some(fresh), &mut at, false, &mut restored)?;
    Ok(restored)
}

fn restore_at(
    edited: &mut Value,
    fresh: Option<&Value>,
    at: &mut Vec<PathSegment>,
    is_moved: bool,
    restored: &mut Restored,
) -> Result<(), Unrestorable> {
    match edited {
        Value::String(text) if text.starts_with(MARKER_PREFIX) && text != HIDDEN => {
            return Err(Unrestorable::MarkerText(FieldPath::new(at.clone())));
        }
        Value::String(text) if text == HIDDEN => {
            let Some(fresh) = fresh else {
                return Err(Unrestorable::Unmatched(FieldPath::new(at.clone())));
            };
            *edited = fresh.clone();
            if is_moved {
                restored.moved.push(FieldPath::new(at.clone()));
            }
        }
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                at.push(PathSegment::Key(key.clone()));
                restore_at(
                    child,
                    fresh.and_then(|fresh| fresh.get(key)),
                    at,
                    is_moved,
                    restored,
                )?;
                at.pop();
            }
        }
        Value::Array(items) => {
            let fresh_list = fresh.and_then(Value::as_array);
            let is_named =
                has_unique_names(items) && fresh_list.is_some_and(|list| has_unique_names(list));
            for index in 0..items.len() {
                let counter =
                    fresh_list.and_then(|list| counterpart(list, &items[index], index, items));
                let segment = match name_of(&items[index]) {
                    Some(name) if is_named => PathSegment::Name(name.to_owned()),
                    _ => PathSegment::Index(index),
                };
                // Only a positional match can attach a value to the wrong item.
                let is_item_moved = is_moved
                    || (!is_named
                        && counter.is_some_and(|counter| {
                            !equal_but_placeholders(&items[index], counter)
                        }));
                at.push(segment);
                restore_at(&mut items[index], counter, at, is_item_moved, restored)?;
                at.pop();
            }
        }
        _ => {}
    }
    Ok(())
}

/// Whether `edited` equals `fresh` everywhere except where `edited` holds a placeholder.
fn equal_but_placeholders(edited: &Value, fresh: &Value) -> bool {
    match (edited, fresh) {
        (Value::String(text), _) if text == HIDDEN => true,
        (Value::Object(edited), Value::Object(fresh)) => {
            edited.len() == fresh.len()
                && edited.iter().all(|(key, value)| {
                    fresh
                        .get(key)
                        .is_some_and(|other| equal_but_placeholders(value, other))
                })
        }
        (Value::Array(edited), Value::Array(fresh)) => {
            edited.len() == fresh.len()
                && edited
                    .iter()
                    .zip(fresh)
                    .all(|(value, other)| equal_but_placeholders(value, other))
        }
        _ => edited == fresh,
    }
}

#[cfg(test)]
#[path = "edit_placeholders_tests.rs"]
mod edit_placeholders_tests;
