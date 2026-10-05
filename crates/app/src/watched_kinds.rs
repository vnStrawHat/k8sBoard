//! The kinds that have a running watch, for the status bar tooltip (spec 0054).

/// A kind with its running watches: one per namespace of the scope for a namespaced kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WatchedKind {
    pub(crate) name: &'static str,
    pub(crate) count: usize,
}

/// Merges rows of the same name (the Pods screen and the Pods feed are one kind), drops rows
/// without a watch, and sorts by name so the list does not jump as feeds start.
pub(crate) fn merge_watched(
    rows: impl IntoIterator<Item = (&'static str, usize)>,
) -> Vec<WatchedKind> {
    let mut kinds: Vec<WatchedKind> = Vec::new();
    for (name, count) in rows.into_iter().filter(|(_, count)| *count > 0) {
        match kinds.iter_mut().find(|kind| kind.name == name) {
            Some(kind) => kind.count += count,
            None => kinds.push(WatchedKind { name, count }),
        }
    }
    kinds.sort_by_key(|kind| kind.name.to_lowercase());
    kinds
}

/// `Pods` for one watch, `Pods ×3` for several.
pub(crate) fn watched_row(kind: &WatchedKind) -> (&'static str, String) {
    let count = if kind.count == 1 {
        String::new()
    } else {
        format!("×{}", kind.count)
    };
    (kind.name, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_of_one_name_merge_and_zero_rows_drop() {
        let kinds = merge_watched([("Pods", 2), ("Nodes", 1), ("Pods", 1), ("Jobs", 0)]);
        assert_eq!(
            kinds,
            vec![
                WatchedKind {
                    name: "Nodes",
                    count: 1
                },
                WatchedKind {
                    name: "Pods",
                    count: 3
                },
            ]
        );
    }

    #[test]
    fn kinds_sort_by_name_ignoring_case() {
        let names: Vec<_> = merge_watched([("PDBs", 1), ("Nodes", 1), ("jobs", 1), ("Events", 1)])
            .into_iter()
            .map(|kind| kind.name)
            .collect();
        assert_eq!(names, ["Events", "jobs", "Nodes", "PDBs"]);
    }

    #[test]
    fn a_single_watch_shows_no_multiplier() {
        let row = |count| {
            watched_row(&WatchedKind {
                name: "Pods",
                count,
            })
        };
        assert_eq!(row(1), ("Pods", String::new()));
        assert_eq!(row(3), ("Pods", "×3".to_owned()));
    }
}
