use cluster::{ControllerRef, NodeSummary, PodSummary};
use gpui_kit::{App, WeakEntity};

use crate::app_shell::{AppShell, Screen};
use crate::cluster_registry::ClusterRef;
use crate::cluster_rows::RowAddress;
use crate::cluster_session::LiveList;
use crate::kind_row::KindRow;
use crate::resource_kind::ResourceKind;
use crate::table_view::TableView;

/// The identity of a selected row. Rows move when a snapshot reorders them, so the
/// selection is a key and the row index is looked up again after every update.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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

    /// The key of a controller owner of an object in `namespace`. Owners live in the object's
    /// namespace.
    pub(crate) fn of_owner(namespace: &str, owner: &ControllerRef) -> Option<Self> {
        Self::of_object(&owner.kind, Some(namespace), &owner.name)
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

/// An object and the cluster it lives in. The same namespace and name exist in several clusters,
/// so the selection, the drawer, and the pending subjects carry both.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ClusterObject {
    pub(crate) cluster: ClusterRef,
    pub(crate) key: ResourceKey,
}

impl ClusterObject {
    pub(crate) fn new(cluster: ClusterRef, key: ResourceKey) -> Self {
        Self { cluster, key }
    }
}

/// The shell and the cluster a tool dialog (Who can, Check permissions, Test traffic) works on. The
/// objects its links name live in that cluster, which is not always the drawer's or the primary.
pub(crate) struct DialogOrigin {
    pub(crate) shell: WeakEntity<AppShell>,
    pub(crate) cluster: ClusterRef,
}

impl DialogOrigin {
    /// Opens the object `key` of the dialog's cluster on its screen, with its drawer.
    pub(crate) fn reveal(&self, key: ResourceKey, cx: &mut App) {
        let object = ClusterObject::new(self.cluster.clone(), key);
        let _ = self
            .shell
            .update(cx, |shell, cx| shell.reveal_object(object, cx));
    }
}

/// The index of the item `is_target` picks in `list`, whatever the filter shows. `None` while
/// the list is loading; otherwise `Some(found)`, with `Some(None)` for a failed list or a missing
/// item. A reveal uses it to make a filtered-out target visible.
pub(crate) fn list_item_index<T>(
    list: &LiveList<T>,
    is_target: impl Fn(&T) -> bool,
) -> Option<Option<usize>> {
    if list.is_loading() {
        return None;
    }
    Some(list.items().iter().position(is_target))
}

/// The table row of the selected item in `list`, the list of slot `slot`, searched in the rows
/// `view` shows. `addresses` maps the merged items of the view to slots and list items. `None`
/// while the list is loading, so the selection waits for the first snapshot; otherwise
/// `Some(found)`. A failed list has no rows, and a filter can hide the item, so the selection
/// is dropped like a missing row (a denied kind reached through a reveal).
pub(crate) fn list_row_index<T>(
    list: &LiveList<T>,
    view: &TableView,
    addresses: &[RowAddress],
    slot: usize,
    is_selected: impl Fn(&T) -> bool,
) -> Option<Option<usize>> {
    if list.is_loading() {
        return None;
    }
    let items = list.items();
    Some(view.rows().iter().position(|&merged| {
        addresses.get(merged).is_some_and(|address| {
            usize::from(address.slot) == slot
                && items.get(address.item as usize).is_some_and(&is_selected)
        })
    }))
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

/// Whether a `SelectRow` for `row` is the echo of the shell's own move (`echo` holds the row the
/// shell just selected). It moves the cursor but never opens the drawer. Consumes the mark, so a
/// click on any row after it counts as a click.
pub(crate) fn take_row_echo(echo: &mut Option<usize>, row: usize) -> bool {
    echo.take() == Some(row)
}

#[cfg(test)]
#[path = "table_selection_tests.rs"]
mod table_selection_tests;
