use cluster::{NodeSummary, PodSummary};

use crate::app_shell::Screen;
use crate::cluster_session::LiveList;
use crate::kind_row::KindRow;
use crate::resource_kind::ResourceKind;
use crate::table_view::TableView;

/// The identity of a selected row. Rows move when a snapshot reorders them, so the
/// selection is a key and the row index is looked up again after every update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResourceKey {
    Pod {
        namespace: String,
        name: String,
    },
    Node {
        name: String,
    },
    Kind {
        kind: ResourceKind,
        namespace: Option<String>,
        name: String,
    },
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

    /// The key of an object named by an event or an owner reference, when k8sBoard has a screen
    /// for its kind. A Pod needs a namespace; Events have no screen of their own.
    pub(crate) fn of_object(kind: &str, namespace: Option<&str>, name: &str) -> Option<Self> {
        let name = name.to_owned();
        match (kind, namespace) {
            ("Pod", Some(namespace)) => Some(Self::Pod {
                namespace: namespace.to_owned(),
                name,
            }),
            ("Node", _) => Some(Self::Node { name }),
            (kind, namespace) => {
                let kind = ResourceKind::from_object_kind(kind)
                    .filter(|kind| *kind != ResourceKind::Events)?;
                Some(Self::Kind {
                    kind,
                    namespace: namespace
                        .filter(|_| kind.is_namespaced())
                        .map(str::to_owned),
                    name,
                })
            }
        }
    }

    pub(crate) fn of_row(kind: ResourceKind, row: &KindRow) -> Self {
        Self::Kind {
            kind,
            namespace: row.namespace.clone(),
            name: row.name.clone(),
        }
    }

    /// The screen that lists this object.
    pub(crate) fn screen(&self) -> Screen {
        match self {
            Self::Pod { .. } => Screen::Pods,
            Self::Node { .. } => Screen::Nodes,
            Self::Kind { kind, .. } => Screen::Kind(*kind),
        }
    }

    pub(crate) fn is_pod(&self, pod: &PodSummary) -> bool {
        matches!(self, Self::Pod { namespace, name }
            if *namespace == pod.namespace && *name == pod.name)
    }

    pub(crate) fn is_node(&self, node: &NodeSummary) -> bool {
        matches!(self, Self::Node { name } if *name == node.name)
    }

    pub(crate) fn is_row(&self, kind: ResourceKind, row: &KindRow) -> bool {
        matches!(self, Self::Kind { kind: key_kind, namespace, name }
            if *key_kind == kind && *namespace == row.namespace && *name == row.name)
    }
}

/// The table row of the selected item in `list`, searched in the rows `view` shows. `None`
/// while the list is loading, so the selection waits for the first snapshot; otherwise
/// `Some(found)`. A failed list has no rows, and a filter can hide the item, so the selection
/// is dropped like a missing row (a denied kind reached through a reveal).
pub(crate) fn list_row_index<T>(
    list: &LiveList<T>,
    view: &TableView,
    is_selected: impl Fn(&T) -> bool,
) -> Option<Option<usize>> {
    if list.is_loading() {
        return None;
    }
    let items = list.items();
    Some(
        view.rows()
            .iter()
            .position(|&item| items.get(item).is_some_and(&is_selected)),
    )
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
