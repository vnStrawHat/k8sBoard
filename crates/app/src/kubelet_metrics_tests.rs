use cluster::{
    ControllerRef, NodeScheduling, NodeStatus, NodeSystemInfo, PodStatus, ReadyCount, StatusReason,
};

use super::*;

fn node(name: &str, readiness: NodeReadiness) -> NodeSummary {
    NodeSummary {
        name: name.to_owned(),
        status: NodeStatus {
            readiness,
            scheduling: NodeScheduling::Enabled,
        },
        roles: Vec::new(),
        taints: Vec::new(),
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

fn ready_nodes(count: usize) -> Vec<NodeSummary> {
    (0..count)
        .map(|index| node(&format!("node-{index:02}"), NodeReadiness::Ready))
        .collect()
}

fn pod(name: &str, node: Option<&str>, reason: StatusReason) -> PodSummary {
    PodSummary {
        namespace: "shop".to_owned(),
        name: name.to_owned(),
        status: PodStatus::Reason(reason),
        ready: ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: node.map(str::to_owned),
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: Some(ControllerRef {
            kind: "StatefulSet".to_owned(),
            name: "db".to_owned(),
        }),
        conditions: Vec::new(),
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        containers: Vec::new(),
    }
}

fn running(name: &str, node: &str) -> PodSummary {
    pod(name, Some(node), StatusReason::Running)
}

fn workload() -> KubeletSubject {
    KubeletSubject::Workload(PodOwner::Controller {
        namespace: "shop".to_owned(),
        kind: "StatefulSet",
        name: "db".to_owned(),
    })
}

fn demand(subject: KubeletSubject, wants_disk_io: bool) -> KubeletDemand {
    KubeletDemand {
        subject: Some(subject),
        wants_disk_io,
    }
}

fn names(nodes: &[String]) -> Vec<&str> {
    nodes.iter().map(String::as_str).collect()
}

#[test]
fn small_clusters_poll_every_ready_node() {
    let mut nodes = ready_nodes(3);
    nodes.push(node("node-down", NodeReadiness::NotReady));
    nodes.push(node("node-unknown", NodeReadiness::Unknown));

    let targets = kubelet_targets(&KubeletDemand::default(), &nodes, &[]);

    assert_eq!(
        names(&targets.summary_nodes),
        ["node-00", "node-01", "node-02"]
    );
    assert!(targets.disk_io_nodes.is_empty());
}

#[test]
fn large_clusters_poll_only_demanded_nodes() {
    let nodes = ready_nodes(12);
    let pods = [running("db-0", "node-03"), running("db-1", "node-07")];

    let targets = kubelet_targets(&demand(workload(), false), &nodes, &pods);
    assert_eq!(names(&targets.summary_nodes), ["node-03", "node-07"]);

    let targets = kubelet_targets(&KubeletDemand::default(), &nodes, &pods);
    assert!(targets.summary_nodes.is_empty());
}

#[test]
fn summary_demand_caps_at_ten_by_pod_count() {
    let nodes = ready_nodes(14);
    // node-00 runs three pods and node-13 two; every other node one.
    let mut pods: Vec<PodSummary> = (0..14)
        .map(|index| running(&format!("db-{index}"), &format!("node-{index:02}")))
        .collect();
    pods.push(running("db-x", "node-00"));
    pods.push(running("db-y", "node-00"));
    pods.push(running("db-z", "node-13"));

    let targets = kubelet_targets(&demand(workload(), false), &nodes, &pods);

    assert_eq!(targets.summary_nodes.len(), SUMMARY_NODE_LIMIT);
    assert!(targets.summary_nodes.contains(&"node-00".to_owned()));
    assert!(targets.summary_nodes.contains(&"node-13".to_owned()));
    assert!(!targets.summary_nodes.contains(&"node-12".to_owned()));
}

#[test]
fn disk_targets_need_disk_demand_and_cap_at_three() {
    let nodes = ready_nodes(5);
    let pods: Vec<PodSummary> = (0..5)
        .map(|index| running(&format!("db-{index}"), &format!("node-{index:02}")))
        .collect();

    let without = kubelet_targets(&demand(workload(), false), &nodes, &pods);
    assert!(without.disk_io_nodes.is_empty());

    let with = kubelet_targets(&demand(workload(), true), &nodes, &pods);
    assert_eq!(
        names(&with.disk_io_nodes),
        ["node-00", "node-01", "node-02"]
    );
    assert_eq!(with.summary_nodes.len(), 5);

    let no_subject = KubeletDemand {
        subject: None,
        wants_disk_io: true,
    };
    assert!(
        kubelet_targets(&no_subject, &nodes, &pods)
            .disk_io_nodes
            .is_empty()
    );
}

#[test]
fn disk_targets_skip_not_ready_nodes() {
    let nodes = [
        node("node-a", NodeReadiness::NotReady),
        node("node-b", NodeReadiness::Ready),
    ];
    let pods = [running("db-0", "node-a"), running("db-1", "node-b")];
    let targets = kubelet_targets(&demand(workload(), true), &nodes, &pods);
    assert_eq!(names(&targets.disk_io_nodes), ["node-b"]);
}

#[test]
fn targets_are_sorted_for_equality() {
    let forward = ready_nodes(12);
    let mut backward = forward.clone();
    backward.reverse();
    let pods = [running("db-0", "node-09"), running("db-1", "node-02")];
    let ask = demand(workload(), true);

    let first = kubelet_targets(&ask, &forward, &pods);
    let second = kubelet_targets(&ask, &backward, &pods);

    assert_eq!(first, second);
    assert_eq!(names(&first.summary_nodes), ["node-02", "node-09"]);
}

#[test]
fn subject_nodes_skip_done_and_unscheduled_pods() {
    let pods = [
        running("db-0", "node-a"),
        running("db-1", "node-a"),
        running("db-2", "node-b"),
        pod("db-3", Some("node-c"), StatusReason::Completed),
        pod("db-4", Some("node-c"), StatusReason::Failed),
        pod("db-5", None, StatusReason::Pending),
    ];

    let shares = subject_nodes(&workload(), &pods);

    assert_eq!(
        shares,
        [
            NodeShare {
                node: "node-a".to_owned(),
                pods: 2,
            },
            NodeShare {
                node: "node-b".to_owned(),
                pods: 1,
            },
        ]
    );
}

#[test]
fn subject_nodes_of_a_pod_is_its_node() {
    let pods = [running("db-0", "node-a"), running("db-1", "node-b")];
    let pod_subject = KubeletSubject::Pod {
        namespace: "shop".to_owned(),
        name: "db-1".to_owned(),
    };
    let shares = subject_nodes(&pod_subject, &pods);
    assert_eq!(shares.len(), 1);
    assert_eq!(shares[0].node, "node-b");
}

#[test]
fn subject_nodes_of_a_node_is_itself() {
    let pods = [running("db-0", "node-a"), running("db-1", "node-b")];
    let shares = subject_nodes(&KubeletSubject::Node("node-a".to_owned()), &pods);
    assert_eq!(
        shares,
        [NodeShare {
            node: "node-a".to_owned(),
            pods: 1,
        }]
    );
}

fn api_error(code: u16) -> ClusterError {
    ClusterError::Api {
        context: "ctx".to_owned(),
        action: "reading kubelet stats",
        code,
        message: "boom".to_owned(),
    }
}

#[test]
fn kubelet_errors_are_worded() {
    assert_eq!(
        kubelet_error_text(&api_error(503)),
        "the kubelet does not answer through the API server proxy"
    );
    assert_eq!(
        kubelet_error_text(&api_error(404)),
        "the kubelet does not serve this endpoint"
    );
    assert!(kubelet_error_text(&api_error(500)).contains("HTTP 500"));
}

#[test]
fn equal_targets_are_not_sent_again() {
    let feed = KubeletFeed::new();
    let mut receiver = feed.targets.subscribe();
    let nodes = ready_nodes(2);

    feed.refresh_targets(&nodes, &[]);
    assert!(receiver.has_changed().expect("sender alive"));
    receiver.mark_unchanged();

    feed.refresh_targets(&nodes, &[]);
    assert!(!receiver.has_changed().expect("sender alive"));
}

#[test]
fn demand_moves_the_targets_once() {
    let mut feed = KubeletFeed::new();
    let nodes = ready_nodes(12);
    let pods = [running("db-0", "node-03")];
    let mut receiver = feed.targets.subscribe();

    feed.set_demand(demand(workload(), false), &nodes, &pods);
    assert_eq!(
        names(&receiver.borrow_and_update().summary_nodes),
        ["node-03"]
    );

    feed.set_demand(demand(workload(), false), &nodes, &pods);
    assert!(!receiver.has_changed().expect("sender alive"));
    assert_eq!(feed.demand(), &demand(workload(), false));
}

fn failed_round() -> Vec<NodeKubeletStats> {
    vec![
        NodeKubeletStats {
            node: "node-a".to_owned(),
            summary: Err(api_error(503)),
            disk_io: None,
        },
        NodeKubeletStats {
            node: "node-b".to_owned(),
            summary: Ok(cluster::KubeletSummary {
                network: None,
                pods: Vec::new(),
            }),
            disk_io: Some(Err(api_error(404))),
        },
    ]
}

#[test]
fn a_round_goes_live() {
    let mut feed = KubeletFeed::new();
    feed.receive(
        WatchUpdate::Snapshot(failed_round()),
        &[],
        &NamespaceScope::All,
    );

    assert_eq!(feed.status, FeedStatus::Live);
    assert_eq!(feed.history.tick_count(), 1);
}

#[test]
fn node_errors_are_kept_and_then_cleared() {
    let mut feed = KubeletFeed::new();
    feed.receive(
        WatchUpdate::Snapshot(failed_round()),
        &[],
        &NamespaceScope::All,
    );

    assert_eq!(
        feed.node_errors["node-a"],
        NodeErrors {
            summary: Some("the kubelet does not answer through the API server proxy".to_owned()),
            disk_io: None,
        }
    );
    assert_eq!(
        feed.node_errors["node-b"].disk_io.as_deref(),
        Some("the kubelet does not serve this endpoint")
    );

    feed.receive(WatchUpdate::Snapshot(Vec::new()), &[], &NamespaceScope::All);
    assert!(feed.node_errors.is_empty());
}

#[test]
fn a_failed_poll_is_failed_before_a_tick_and_interrupted_after() {
    let mut feed = KubeletFeed::new();
    feed.receive(
        WatchUpdate::Failed(api_error(503)),
        &[],
        &NamespaceScope::All,
    );
    assert!(matches!(feed.status, FeedStatus::Failed(_)));

    feed.receive(WatchUpdate::Snapshot(Vec::new()), &[], &NamespaceScope::All);
    feed.receive(
        WatchUpdate::Failed(api_error(404)),
        &[],
        &NamespaceScope::All,
    );
    assert_eq!(
        feed.status,
        FeedStatus::Interrupted("the kubelet does not serve this endpoint".to_owned())
    );
}

#[test]
fn denied_feed_is_unavailable_and_stopped_feed_failed() {
    let mut feed = KubeletFeed::new();
    feed.wait();
    assert_eq!(feed.status, FeedStatus::Checking);

    feed.turn_off("not allowed to get nodes/proxy".to_owned());
    assert_eq!(
        feed.status,
        FeedStatus::Unavailable("not allowed to get nodes/proxy".to_owned())
    );

    feed.mark_stopped();
    assert_eq!(
        feed.status,
        FeedStatus::Failed("kubelet polling stopped unexpectedly".to_owned())
    );
}

#[test]
fn claim_subject_nodes_are_mounting_pods_nodes() {
    let mount = |claim: &str| cluster::MountEntry {
        path: "/data".to_owned(),
        volume: "data".to_owned(),
        source: cluster::VolumeSource::PersistentVolumeClaim {
            claim: claim.to_owned(),
        },
        is_read_only: false,
        sub_path: None,
    };
    let mounting = |name: &str, node: &str, claim: &str, reason: StatusReason| {
        let mut pod = pod(name, Some(node), reason);
        pod.containers.push(cluster::ContainerSummary {
            name: "main".to_owned(),
            image: "img".to_owned(),
            kind: cluster::ContainerKind::Main,
            state: cluster::ContainerState::Running { started_at: None },
            is_ready: true,
            restart_count: 0,
            last_termination: None,
            image_digest: None,
            pull_policy: None,
            is_started: None,
            ports: Vec::new(),
            resources: Vec::new(),
            probes: cluster::ContainerProbes::default(),
            env: Vec::new(),
            env_from: Vec::new(),
            mounts: vec![mount(claim)],
        });
        pod
    };
    let pods = [
        mounting("a", "node-b", "data", StatusReason::Running),
        mounting("b", "node-b", "data", StatusReason::Running),
        mounting("c", "node-a", "data", StatusReason::Running),
        // A finished pod no longer holds the volume, and another claim is not the subject.
        mounting("d", "node-c", "data", StatusReason::Completed),
        mounting("e", "node-d", "other", StatusReason::Running),
    ];
    let subject = KubeletSubject::Claim {
        namespace: "shop".to_owned(),
        claim: "data".to_owned(),
    };
    let shares = subject_nodes(&subject, &pods);
    let listed: Vec<(&str, usize)> = shares
        .iter()
        .map(|share| (share.node.as_str(), share.pods))
        .collect();
    assert_eq!(listed, [("node-b", 2), ("node-a", 1)]);
}
