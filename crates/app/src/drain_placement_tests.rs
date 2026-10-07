use cluster::{
    ControllerRef, NodeReadiness, NodeStatus, NodeSystemInfo, NodeTaint, PodPlacement,
    PodToleration,
};

use super::*;
use crate::drain_plan::{DrainOptions, node_plan, pinned_note};

fn node(name: &str, scheduling: NodeScheduling, taints: &[(&str, &str)]) -> NodeSummary {
    NodeSummary {
        name: name.to_owned(),
        status: NodeStatus {
            readiness: NodeReadiness::Ready,
            scheduling,
        },
        roles: Vec::new(),
        taints: taints
            .iter()
            .map(|(key, value)| NodeTaint {
                key: (*key).to_owned(),
                value: Some((*value).to_owned()),
                effect: "NoSchedule".to_owned(),
                time_added: None,
            })
            .collect(),
        kubelet_version: "v1.29.5".to_owned(),
        internal_ip: None,
        created_at: None,
        conditions: Vec::new(),
        addresses: Vec::new(),
        system: NodeSystemInfo::default(),
        resources: Vec::new(),
        labels: Vec::new(),
    }
}

fn pod(name: &str) -> DrainPod {
    DrainPod {
        namespace: "shop".to_owned(),
        name: name.to_owned(),
        uid: format!("uid-{name}"),
        labels: Vec::new(),
        controller: Some(ControllerRef {
            kind: "StatefulSet".to_owned(),
            name: "catalog-db".to_owned(),
        }),
        is_mirror: false,
        has_empty_dir: false,
        is_finished: false,
        is_pending: false,
        is_terminating: false,
        claims: Vec::new(),
        pinned_volume: None,
        placement: PodPlacement::default(),
    }
}

fn plans(pods: &[DrainPod]) -> Vec<NodePlan> {
    vec![node_plan("worker", pods, &[], &DrainOptions::default())]
}

fn cluster() -> Vec<NodeSummary> {
    vec![
        node("worker", NodeScheduling::Enabled, &[]),
        node("worker2", NodeScheduling::Enabled, &[("workload", "data")]),
    ]
}

#[test]
fn a_pod_no_other_node_takes_says_which_taint_stops_it() {
    assert_eq!(
        placement_note(&plans(&[pod("catalog-db-0")]), &cluster()).as_deref(),
        Some(
            "No other node fits catalog-db-0 (taints on worker2: workload=data): its replacement will stay Pending."
        )
    );
}

#[test]
fn a_toleration_gives_the_pod_a_node_to_go_to() {
    let tolerating = DrainPod {
        placement: PodPlacement {
            tolerations: vec![PodToleration {
                key: "workload".to_owned(),
                is_exists: true,
                value: String::new(),
                effect: String::new(),
            }],
            ..PodPlacement::default()
        },
        ..pod("catalog-db-0")
    };
    assert_eq!(placement_note(&plans(&[tolerating]), &cluster()), None);
}

#[test]
fn several_stranded_pods_share_one_line_and_a_cordoned_node_is_no_candidate() {
    let nodes = vec![
        node("worker", NodeScheduling::Enabled, &[]),
        node("worker2", NodeScheduling::Disabled, &[]),
    ];
    assert_eq!(
        placement_note(&plans(&[pod("a-0"), pod("a-1")]), &nodes).as_deref(),
        Some(
            "No other node fits a-0 and 1 more (no other node is schedulable): their replacements will stay Pending."
        )
    );
}

#[test]
fn pods_without_a_replacement_and_unread_nodes_say_nothing() {
    let bare = DrainPod {
        controller: None,
        ..pod("lonely")
    };
    let finished = DrainPod {
        is_finished: true,
        ..pod("done")
    };
    let options = DrainOptions {
        force_unmanaged: true,
        ..DrainOptions::default()
    };
    let plan = vec![node_plan("worker", &[bare, finished], &[], &options)];
    assert_eq!(placement_note(&plan, &cluster()), None);
    assert_eq!(placement_note(&plans(&[pod("catalog-db-0")]), &[]), None);
}

#[test]
fn a_pinned_pod_that_does_not_tolerate_its_own_nodes_taint_cannot_come_back_either() {
    let pinned = DrainPod {
        pinned_volume: Some("data-catalog-db-0".to_owned()),
        claims: vec!["data-catalog-db-0".to_owned()],
        ..pod("catalog-db-0")
    };
    let plan = vec![node_plan(
        "worker2",
        &[pinned],
        &[],
        &DrainOptions::default(),
    )];
    assert_eq!(
        pinned_note(&plan, &cluster()).as_deref(),
        Some(
            "catalog-db-0 cannot move: its volume data-catalog-db-0 lives on this node, so its replacement stays Pending, and it cannot come back here either until taint workload=data is tolerated."
        )
    );
    // A node that carries no taint of the pod's keeps the plain promise.
    assert_eq!(
        pinned_note(&plan, &[node("worker2", NodeScheduling::Enabled, &[])]).as_deref(),
        Some(
            "catalog-db-0 cannot move: its volume data-catalog-db-0 lives on this node, so its replacement stays Pending until the node is back."
        )
    );
}

#[test]
fn two_tainted_nodes_are_named_in_one_list_of_taints() {
    let nodes = vec![
        node("worker", NodeScheduling::Enabled, &[]),
        node(
            "control-plane",
            NodeScheduling::Enabled,
            &[("node-role.kubernetes.io/control-plane", "")],
        ),
        node("worker2", NodeScheduling::Enabled, &[("workload", "data")]),
    ];
    assert_eq!(
        placement_note(&plans(&[pod("web-1")]), &nodes).as_deref(),
        Some(
            "No other node fits web-1 (taints on control-plane: node-role.kubernetes.io/control-plane, worker2: workload=data): its replacement will stay Pending."
        )
    );
}
