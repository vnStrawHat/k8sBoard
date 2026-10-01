use cluster::{NodeReadiness, NodeScheduling, NodeStatus, PodStatus, ReadyCount, StatusReason};

use super::*;
use crate::status_tone::{StatusLabel, StatusTone};
use crate::table_sort::{SortDirection, TableSort};
use crate::table_view::{CellValue, TableRow};

fn pod(namespace: &str, name: &str) -> PodSummary {
    PodSummary {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        status: PodStatus::Reason(StatusReason::Running),
        ready: ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: None,
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: None,
        conditions: Vec::new(),
        status_message: None,
        labels: Vec::new(),
        containers: Vec::new(),
    }
}

fn node(name: &str) -> NodeSummary {
    NodeSummary {
        name: name.to_owned(),
        status: NodeStatus {
            readiness: NodeReadiness::Ready,
            scheduling: NodeScheduling::Enabled,
        },
        roles: Vec::new(),
        taints: Vec::new(),
        kubelet_version: "v1.29.5".to_owned(),
        internal_ip: None,
        created_at: None,
        conditions: Vec::new(),
        addresses: Vec::new(),
        system: cluster::NodeSystemInfo::default(),
        resources: Vec::new(),
        labels: Vec::new(),
    }
}

/// The selection only needs the view to list item indices, so the cells do not matter.
impl TableRow for KindRow {
    fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn labels(&self) -> impl Iterator<Item = &str> {
        std::iter::empty()
    }

    fn tone(&self) -> StatusTone {
        self.status.tone
    }

    fn value(&self, _: usize) -> CellValue<'_> {
        CellValue::Absent
    }
}

/// A ready list and the view of it as a table shows it.
fn ready_with_view<T: TableRow>(
    items: Vec<T>,
    sort: Option<TableSort>,
) -> (LiveList<T>, TableView) {
    let mut view = TableView::default();
    view.sort = sort;
    view.rebuild(&items, 6, jiff::Timestamp::UNIX_EPOCH);
    let list = LiveList::Ready {
        items,
        interruption: None,
    };
    (list, view)
}

#[test]
fn list_row_index_finds_key_after_reorder() {
    let key = ResourceKey::of_pod(&pod("b", "web"));
    let (list, view) = ready_with_view(
        vec![pod("a", "web"), pod("b", "web"), pod("c", "web")],
        None,
    );
    assert_eq!(
        list_row_index(&list, &view, |item| key.is_pod(item)),
        Some(Some(1))
    );
    let by_name_descending = Some(TableSort {
        column: 0,
        direction: SortDirection::Descending,
    });
    let (list, view) = ready_with_view(
        vec![pod("a", "web"), pod("b", "web"), pod("c", "web")],
        by_name_descending,
    );
    assert_eq!(
        list_row_index(&list, &view, |item| key.is_pod(item)),
        Some(Some(1))
    );
    let key = ResourceKey::of_pod(&pod("a", "web"));
    assert_eq!(
        list_row_index(&list, &view, |item| key.is_pod(item)),
        Some(Some(2))
    );

    let node_key = ResourceKey::of_node(&node("n2"));
    let (nodes, node_view) = ready_with_view(vec![node("n1"), node("n2")], None);
    assert_eq!(
        list_row_index(&nodes, &node_view, |item| node_key.is_node(item)),
        Some(Some(1))
    );
}

#[test]
fn list_row_index_none_when_key_vanished() {
    let key = ResourceKey::of_pod(&pod("a", "gone"));
    let (list, view) = ready_with_view(vec![pod("a", "web"), pod("b", "gone")], None);
    assert_eq!(
        list_row_index(&list, &view, |item| key.is_pod(item)),
        Some(None)
    );
    assert!(!key.is_node(&node("gone")));
}

#[test]
fn list_row_index_searches_the_view() {
    let key = ResourceKey::of_pod(&pod("a", "api"));
    let mut view = TableView::default();
    let items = vec![pod("a", "web"), pod("a", "api")];
    view.filter.text = "web".to_owned();
    view.rebuild(&items, 6, jiff::Timestamp::UNIX_EPOCH);
    let list = LiveList::Ready {
        items,
        interruption: None,
    };
    // The filter hides the subject, so its drawer must close.
    assert_eq!(
        list_row_index(&list, &view, |item| key.is_pod(item)),
        Some(None)
    );
}

#[test]
fn selection_sync_keeps_unchanged_index() {
    assert_eq!(selection_sync(Some(3), Some(3)), SelectionSync::Keep);
}

#[test]
fn selection_sync_moves_on_reorder() {
    assert_eq!(selection_sync(Some(3), Some(1)), SelectionSync::Move(1));
    assert_eq!(selection_sync(None, Some(0)), SelectionSync::Move(0));
}

#[test]
fn selection_sync_clears_when_subject_deleted() {
    assert_eq!(selection_sync(Some(3), None), SelectionSync::Clear);
    assert_eq!(selection_sync(None, None), SelectionSync::Clear);
}

fn kind_row(namespace: Option<&str>, name: &str) -> KindRow {
    KindRow {
        namespace: namespace.map(str::to_owned),
        name: name.to_owned(),
        created_at: None,
        status: StatusLabel {
            text: "Active".into(),
            tone: StatusTone::Ok,
        },
        cells: Vec::new(),
        sections: Vec::new(),
        event: None,
        related_pods: None,
        labels: Vec::new(),
    }
}

#[test]
fn kind_key_matches_row_by_kind_namespace_and_name() {
    let row = kind_row(Some("team-a"), "api");
    let key = ResourceKey::of_row(ResourceKind::Deployments, &row);
    assert!(key.is_row(ResourceKind::Deployments, &row));
    assert!(!key.is_row(ResourceKind::Namespaces, &row));
    assert!(!key.is_row(ResourceKind::Deployments, &kind_row(Some("team-b"), "api")));
    assert!(!key.is_row(ResourceKind::Deployments, &kind_row(Some("team-a"), "web")));
    assert!(!key.is_row(ResourceKind::Deployments, &kind_row(None, "api")));
    assert!(!key.is_pod(&pod("team-a", "api")));
}

#[test]
fn resource_key_screen_matches_kind() {
    use crate::app_shell::Screen;

    assert_eq!(ResourceKey::of_pod(&pod("ns", "a")).screen(), Screen::Pods);
    let node = ResourceKey::Node {
        name: "n1".to_owned(),
    };
    assert_eq!(node.screen(), Screen::Nodes);
    let row = kind_row(Some("ns"), "api");
    let key = ResourceKey::of_row(ResourceKind::Deployments, &row);
    assert_eq!(key.screen(), Screen::Kind(ResourceKind::Deployments));
}

#[test]
fn pending_reveal_key_resolves_through_selection_sync() {
    let key = ResourceKey::of_row(ResourceKind::Deployments, &kind_row(Some("ns"), "api"));
    let (list, view) = ready_with_view(
        vec![kind_row(Some("ns"), "web"), kind_row(Some("ns"), "api")],
        None,
    );
    let found = list_row_index(&list, &view, |row| {
        key.is_row(ResourceKind::Deployments, row)
    })
    .flatten();
    // The table has nothing selected yet, so the row is moved to.
    assert_eq!(selection_sync(None, found), SelectionSync::Move(1));

    let (list, view) = ready_with_view(vec![kind_row(Some("ns"), "web")], None);
    let missing = list_row_index(&list, &view, |row| {
        key.is_row(ResourceKind::Deployments, row)
    })
    .flatten();
    assert_eq!(selection_sync(None, missing), SelectionSync::Clear);
}

#[test]
fn list_row_index_waits_while_loading_and_drops_a_failed_list() {
    let key = ResourceKey::of_row(ResourceKind::Deployments, &kind_row(Some("ns"), "api"));
    let is_key = |row: &KindRow| key.is_row(ResourceKind::Deployments, row);

    let no_view = TableView::default();
    let loading = LiveList::<KindRow>::Loading;
    assert_eq!(list_row_index(&loading, &no_view, is_key), None);

    let failed = LiveList::<KindRow>::Failed {
        message: "denied".to_owned(),
    };
    assert_eq!(list_row_index(&failed, &no_view, is_key), Some(None));

    let ready = LiveList::Ready {
        items: vec![kind_row(Some("ns"), "web"), kind_row(Some("ns"), "api")],
        interruption: None,
    };
    let mut view = TableView::default();
    view.rebuild(ready.items(), 6, jiff::Timestamp::UNIX_EPOCH);
    assert_eq!(list_row_index(&ready, &view, is_key), Some(Some(1)));
}

#[test]
fn of_object_maps_pods_nodes_and_kinds() {
    assert_eq!(
        ResourceKey::of_object("Pod", Some("shop"), "api-0"),
        Some(ResourceKey::Pod {
            namespace: "shop".to_owned(),
            name: "api-0".to_owned(),
        })
    );
    assert_eq!(ResourceKey::of_object("Pod", None, "api-0"), None);
    assert_eq!(
        ResourceKey::of_object("Node", None, "node-1"),
        Some(ResourceKey::Node {
            name: "node-1".to_owned()
        })
    );
    assert_eq!(
        ResourceKey::of_object("ReplicaSet", Some("shop"), "api-7d"),
        Some(ResourceKey::Kind {
            kind: ResourceKind::ReplicaSets,
            namespace: Some("shop".to_owned()),
            name: "api-7d".to_owned(),
        })
    );
    // Cluster-scoped kinds drop the namespace.
    assert_eq!(
        ResourceKey::of_object("Namespace", Some("shop"), "shop"),
        Some(ResourceKey::Kind {
            kind: ResourceKind::Namespaces,
            namespace: None,
            name: "shop".to_owned(),
        })
    );
    assert_eq!(ResourceKey::of_object("Event", Some("shop"), "e"), None);
    assert_eq!(ResourceKey::of_object("Widget", Some("shop"), "w"), None);
}
