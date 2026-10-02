//! Rules folded into a verbs-by-resources table, and the Can do chips read from it. Pure.

use std::collections::{BTreeMap, BTreeSet};

use cluster::RbacRule;
use gpui_kit::SharedString;

use crate::status_tone::StatusTone;

pub(crate) const TABLE_VERBS: [&str; 8] = [
    "get",
    "list",
    "watch",
    "create",
    "update",
    "patch",
    "delete",
    "deletecollection",
];
const WILDCARD: &str = "*";
const MAX_ROWS: usize = 300;
const MAX_CHIPS: usize = 12;

/// What one verb column of a row says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum VerbCell {
    Empty,
    All,
    /// Only these objects (`resourceNames`).
    Names(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PermissionRow {
    pub(crate) group: String,
    /// Subresources read `pods/log`.
    pub(crate) resource: String,
    pub(crate) cells: [VerbCell; 8],
    /// Verbs outside the eight columns, plus `*`, sorted.
    pub(crate) other_verbs: Vec<String>,
    /// The `other_verbs` that only reach named objects (`impersonate` on `users` for `alice`).
    pub(crate) named_other_verbs: Vec<String>,
    /// Group, resource, and a verb are all `*`.
    pub(crate) is_everything: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UrlRow {
    pub(crate) url: String,
    pub(crate) verbs: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PermissionTable {
    pub(crate) rows: Vec<PermissionRow>,
    pub(crate) url_rows: Vec<UrlRow>,
    /// Rows past the cap.
    pub(crate) hidden_rows: usize,
}

struct RowParts {
    cells: [VerbCell; 8],
    /// Never `Empty`: a verb is recorded when a rule gives it.
    other_verbs: BTreeMap<String, VerbCell>,
}

impl PermissionRow {
    /// The Other column: the verbs outside the eight columns, `(named)` on a limited one.
    pub(crate) fn other_text(&self) -> String {
        self.other_verbs
            .iter()
            .map(|verb| {
                if self.named_other_verbs.contains(verb) {
                    format!("{verb} (named)")
                } else {
                    verb.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Whether every verb the row grants is limited to named objects.
    fn is_named_only(&self) -> bool {
        let has_all_cell = self.cells.contains(&VerbCell::All);
        let has_all_other = self
            .other_verbs
            .iter()
            .any(|verb| !self.named_other_verbs.contains(verb));
        !has_all_cell && !has_all_other
    }
}

/// One row per (group, resource): groups times resources of every rule, verbs merged. Everything
/// rows come first, then core group, then by group and resource.
pub(crate) fn permission_table<'a>(
    rules: impl IntoIterator<Item = &'a RbacRule>,
) -> PermissionTable {
    let mut rows: BTreeMap<(String, String), RowParts> = BTreeMap::new();
    let mut urls: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for rule in rules {
        for group in &rule.api_groups {
            for resource in &rule.resources {
                let parts = rows
                    .entry((group.clone(), resource.clone()))
                    .or_insert_with(|| RowParts {
                        cells: std::array::from_fn(|_| VerbCell::Empty),
                        other_verbs: BTreeMap::new(),
                    });
                add_verbs(parts, rule);
            }
        }
        for url in &rule.non_resource_urls {
            urls.entry(url.clone())
                .or_default()
                .extend(rule.verbs.iter().cloned());
        }
    }
    let mut rows: Vec<PermissionRow> = rows
        .into_iter()
        .map(|((group, resource), parts)| PermissionRow {
            is_everything: group == WILDCARD
                && resource == WILDCARD
                && parts.other_verbs.get(WILDCARD) == Some(&VerbCell::All),
            group,
            resource,
            cells: parts.cells,
            named_other_verbs: parts
                .other_verbs
                .iter()
                .filter(|(_, cell)| matches!(cell, VerbCell::Names(_)))
                .map(|(verb, _)| verb.clone())
                .collect(),
            other_verbs: parts.other_verbs.into_keys().collect(),
        })
        .collect();
    // Stable, so the group and resource order of the map survives within each half.
    rows.sort_by_key(|row| !row.is_everything);
    let hidden_rows = rows.len().saturating_sub(MAX_ROWS);
    rows.truncate(MAX_ROWS);
    PermissionTable {
        rows,
        url_rows: urls
            .into_iter()
            .map(|(url, verbs)| UrlRow {
                url,
                verbs: verbs.into_iter().collect(),
            })
            .collect(),
        hidden_rows,
    }
}

fn add_verbs(parts: &mut RowParts, rule: &RbacRule) {
    for verb in &rule.verbs {
        if verb == WILDCARD {
            for cell in &mut parts.cells {
                merge_cell(cell, &rule.resource_names);
            }
        }
        if let Some(column) = TABLE_VERBS.iter().position(|known| known == verb) {
            merge_cell(&mut parts.cells[column], &rule.resource_names);
        } else {
            let cell = parts
                .other_verbs
                .entry(verb.clone())
                .or_insert(VerbCell::Empty);
            merge_cell(cell, &rule.resource_names);
        }
    }
}

/// No names means every object, which wins over any named list.
fn merge_cell(cell: &mut VerbCell, names: &[String]) {
    if names.is_empty() {
        *cell = VerbCell::All;
        return;
    }
    match cell {
        VerbCell::All => {}
        VerbCell::Empty => *cell = VerbCell::Names(sorted_unique(names.iter().cloned())),
        VerbCell::Names(current) => {
            *cell = VerbCell::Names(sorted_unique(current.iter().chain(names).cloned()));
        }
    }
}

fn sorted_unique(names: impl Iterator<Item = String>) -> Vec<String> {
    names.collect::<BTreeSet<_>>().into_iter().collect()
}

/// The chips of the Can do section.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CanDoChips {
    pub(crate) chips: Vec<(SharedString, Option<StatusTone>)>,
    /// Rows past the chip cap.
    pub(crate) more: usize,
}

/// One chip per resource row, `get, list pods`; a wildcard row is the Warn chip `everything`.
pub(crate) fn can_do_chips(table: &PermissionTable) -> CanDoChips {
    let chips: Vec<(SharedString, Option<StatusTone>)> = table
        .rows
        .iter()
        .take(MAX_CHIPS)
        .map(|row| {
            if row.is_everything {
                ("everything".into(), Some(StatusTone::Warn))
            } else {
                (chip_text(row).into(), None)
            }
        })
        .collect();
    CanDoChips {
        more: table.rows.len() - chips.len() + table.hidden_rows,
        chips,
    }
}

fn chip_text(row: &PermissionRow) -> String {
    let filled: Vec<&str> = TABLE_VERBS
        .iter()
        .zip(&row.cells)
        .filter(|(_, cell)| **cell != VerbCell::Empty)
        .map(|(verb, _)| *verb)
        .collect();
    let others = row.other_verbs.iter().filter(|verb| *verb != WILDCARD);
    let mut verbs = if filled.len() == TABLE_VERBS.len() {
        vec!["all verbs"]
    } else {
        filled
    };
    verbs.extend(others.map(String::as_str));
    let resource = if row.resource == WILDCARD {
        "all resources".to_owned()
    } else if row.group.is_empty() {
        row.resource.clone()
    } else {
        format!("{}.{}", row.resource, row.group)
    };
    let named = if row.is_named_only() { " (named)" } else { "" };
    format!("{} {resource}{named}", verbs.join(", "))
}

#[cfg(test)]
#[path = "permission_table_tests.rs"]
mod permission_table_tests;
