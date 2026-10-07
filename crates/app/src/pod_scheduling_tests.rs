use cluster::{
    ContainerKind, ContainerProbes, ContainerResource, ContainerState, ContainerSummary,
    NodeReadiness, NodeResource, NodeScheduling, NodeStatus, NodeSystemInfo, PodStatus, ReadyCount,
    StatusReason,
};

use super::*;

const SELECTOR_MESSAGE: &str = "0/2 nodes are available: 2 node(s) didn't match Pod's node affinity/selector. preemption: 0/2 nodes are available: 2 Preemption is not helpful for scheduling.";
const CPU_MESSAGE: &str = "0/2 nodes are available: 2 Insufficient cpu.";

fn node(labels: &[&str], cpu: &str, memory: &str) -> NodeSummary {
    let resource = |name: &str, allocatable: &str| NodeResource {
        name: name.to_owned(),
        capacity: None,
        allocatable: Some(allocatable.to_owned()),
    };
    NodeSummary {
        name: "wk".to_owned(),
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
        system: NodeSystemInfo::default(),
        resources: vec![resource("cpu", cpu), resource("memory", memory)],
        labels: labels.iter().map(|label| (*label).to_owned()).collect(),
    }
}

fn pod(selector: &[&str], requests: &[(&str, &str)]) -> PodSummary {
    PodSummary {
        namespace: "shop".to_owned(),
        name: "api-0".to_owned(),
        status: PodStatus::Reason(StatusReason::Pending),
        ready: ReadyCount { ready: 0, total: 1 },
        restarts: 0,
        node_name: None,
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: None,
        conditions: Vec::new(),
        containers: vec![ContainerSummary {
            name: "app".to_owned(),
            image: "app:1".to_owned(),
            kind: ContainerKind::Main,
            state: ContainerState::NotReported,
            is_ready: false,
            restart_count: 0,
            last_termination: None,
            image_digest: None,
            pull_policy: None,
            is_started: None,
            ports: Vec::new(),
            resources: requests
                .iter()
                .map(|(name, request)| ContainerResource {
                    name: (*name).to_owned(),
                    request: Some((*request).to_owned()),
                    limit: None,
                })
                .collect(),
            probes: ContainerProbes::default(),
            env: Vec::new(),
            env_from: Vec::new(),
            mounts: Vec::new(),
            terminal: cluster::ContainerTerminal::None,
        }],
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        image_pull_secrets: Vec::new(),
        node_selector: selector.iter().map(|term| (*term).to_owned()).collect(),
        node_affinity: Vec::new(),
        is_finished: false,
    }
}

#[test]
fn a_selector_value_no_node_has_lists_the_values_the_nodes_carry() {
    let nodes = [
        node(&["k8sboard.io/pool=web"], "4", "4Gi"),
        node(&["k8sboard.io/pool=data"], "4", "4Gi"),
    ];
    let hints = scheduling_hints(
        &pod(&["k8sboard.io/pool=gpu"], &[]),
        &nodes,
        SELECTOR_MESSAGE,
    );
    assert_eq!(
        hints,
        [
            "nodeSelector k8sboard.io/pool=gpu — no node has it (nodes have k8sboard.io/pool: data, web)"
        ]
    );
}

#[test]
fn a_selector_key_no_node_carries_says_so() {
    let nodes = [node(&["zone=a"], "4", "4Gi")];
    let hints = scheduling_hints(&pod(&["disk=ssd"], &[]), &nodes, SELECTOR_MESSAGE);
    assert_eq!(
        hints,
        ["nodeSelector disk=ssd — no node carries the key disk"]
    );
}

#[test]
fn terms_that_exist_on_different_nodes_are_not_a_match() {
    let nodes = [node(&["a=1"], "4", "4Gi"), node(&["b=2"], "4", "4Gi")];
    let hints = scheduling_hints(&pod(&["a=1", "b=2"], &[]), &nodes, SELECTOR_MESSAGE);
    assert_eq!(
        hints,
        ["nodeSelector a=1, b=2 — no single node has all of them"]
    );
}

#[test]
fn a_selector_some_node_satisfies_is_not_blamed() {
    let nodes = [node(&["pool=web"], "4", "4Gi")];
    assert!(scheduling_hints(&pod(&["pool=web"], &[]), &nodes, SELECTOR_MESSAGE).is_empty());
}

#[test]
fn the_selector_is_only_named_when_the_scheduler_blames_it() {
    let nodes = [node(&["pool=web"], "4", "4Gi")];
    assert!(scheduling_hints(&pod(&["pool=gpu"], &[]), &nodes, CPU_MESSAGE).is_empty());
}

#[test]
fn required_affinity_terms_are_quoted() {
    let mut pending = pod(&[], &[]);
    pending.node_affinity = vec!["disk In (ssd, nvme), gpu Exists".to_owned()];
    let hints = scheduling_hints(&pending, &[node(&[], "4", "4Gi")], SELECTOR_MESSAGE);
    assert_eq!(
        hints,
        ["required node affinity disk In (ssd, nvme), gpu Exists"]
    );
}

#[test]
fn a_request_above_every_node_names_both_sizes() {
    let nodes = [node(&[], "4", "4Gi"), node(&[], "2", "4Gi")];
    let hints = scheduling_hints(&pod(&[], &[("cpu", "64")]), &nodes, CPU_MESSAGE);
    assert_eq!(
        hints,
        ["requests cpu 64 cores — the largest node allocates 4 cores"]
    );
}

#[test]
fn a_memory_request_is_compared_in_bytes() {
    let message = "0/1 nodes are available: 1 Insufficient memory.";
    let hints = scheduling_hints(
        &pod(&[], &[("memory", "256Gi")]),
        &[node(&[], "4", "3968780Ki")],
        message,
    );
    assert_eq!(
        hints,
        ["requests memory 256Gi — the largest node allocates 3.8Gi"]
    );
}

#[test]
fn a_request_that_fits_an_empty_node_is_not_called_too_big() {
    let hints = scheduling_hints(
        &pod(&[], &[("cpu", "2")]),
        &[node(&[], "4", "4Gi")],
        CPU_MESSAGE,
    );
    assert!(hints.is_empty());
}
