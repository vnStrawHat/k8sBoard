use cluster::{NodeSummary, PodSummary};

/// The identity of a selected row. Rows move when a snapshot reorders them, so the
/// selection is a key and the row index is looked up again after every update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResourceKey {
    Pod { namespace: String, name: String },
    Node { name: String },
}

impl ResourceKey {
    pub(crate) fn of_pod(pod: &PodSummary) -> Self {
        Self::Pod {
            namespace: pod.namespace.clone(),
            name: pod.name.clone(),
        }
    }

    pub(crate) fn of_node(node: &NodeSummary) -> Self {
        Self::Node {
            name: node.name.clone(),
        }
    }

    pub(crate) fn is_pod(&self, pod: &PodSummary) -> bool {
        matches!(self, Self::Pod { namespace, name }
            if *namespace == pod.namespace && *name == pod.name)
    }

    pub(crate) fn is_node(&self, node: &NodeSummary) -> bool {
        matches!(self, Self::Node { name } if *name == node.name)
    }
}

pub(crate) fn row_index<T>(items: &[T], is_selected: impl Fn(&T) -> bool) -> Option<usize> {
    items.iter().position(is_selected)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectionSync {
    /// The table already highlights the right row.
    Keep,
    /// The row moved; the table must select the new index.
    Move(usize),
    /// The subject is gone.
    Clear,
}

/// `Move` re-emits `SelectRow` and scrolls, so it is only returned when the index really
/// changed; `Keep` avoids a scroll jump on every snapshot.
pub(crate) fn selection_sync(table_row: Option<usize>, found: Option<usize>) -> SelectionSync {
    match found {
        None => SelectionSync::Clear,
        Some(index) if table_row == Some(index) => SelectionSync::Keep,
        Some(index) => SelectionSync::Move(index),
    }
}

#[cfg(test)]
#[path = "table_selection_tests.rs"]
mod table_selection_tests;
