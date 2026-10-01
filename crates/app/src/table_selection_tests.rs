use cluster::{NodeReadiness, NodeScheduling, NodeStatus, PodStatus, ReadyCount, StatusReason};

use super::*;
use crate::status_tone::{StatusLabel, StatusTone};

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
    }
}

#[test]
fn row_index_finds_key_after_reorder() {
    let key = ResourceKey::of_pod(&pod("b", "web"));
    let before = [pod("a", "web"), pod("b", "web"), pod("c", "web")];
    let after = [pod("b", "web"), pod("a", "web"), pod("c", "web")];
    assert_eq!(row_index(&before, |item| key.is_pod(item)), Some(1));
    assert_eq!(row_index(&after, |item| key.is_pod(item)), Some(0));

    let node_key = ResourceKey::of_node(&node("n2"));
    let nodes = [node("n1"), node("n2")];
    assert_eq!(row_index(&nodes, |item| node_key.is_node(item)), Some(1));
}

#[test]
fn row_index_none_when_key_vanished() {
    let key = ResourceKey::of_pod(&pod("a", "gone"));
    let pods = [pod("a", "web"), pod("b", "gone")];
    assert_eq!(row_index(&pods, |item| key.is_pod(item)), None);
    assert!(!key.is_node(&node("gone")));
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
