//! What a node's usage means against its allocatable resources, and what its pods request.

use cluster::{
    ByteAmount, ContainerKind, CpuAmount, NodeResource, NodeSummary, PodStatus, PodSummary,
    ResourceUsage, StatusReason,
};

/// Usage as a share of allocatable (1.0 is all of it). `None` without a sample, without the
/// allocatable entry, or with zero allocatable.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct NodeUsage {
    pub(crate) cpu: Option<f64>,
    pub(crate) memory: Option<f64>,
}

pub(crate) fn node_usage(node: &NodeSummary, latest: Option<ResourceUsage>) -> NodeUsage {
    let Some(latest) = latest else {
        return NodeUsage::default();
    };
    let (cpu, memory) = node_allocatable(node);
    NodeUsage {
        cpu: ratio(
            latest.cpu.nanocores() as f64,
            cpu.map(|cpu| cpu.nanocores() as f64),
        ),
        memory: ratio(
            latest.memory.bytes() as f64,
            memory.map(|memory| memory.bytes() as f64),
        ),
    }
}

fn ratio(used: f64, total: Option<f64>) -> Option<f64> {
    let total = total.filter(|total| *total > 0.)?;
    Some(used / total)
}

/// The `cpu` and `memory` allocatable of `node`.
pub(crate) fn node_allocatable(node: &NodeSummary) -> (Option<CpuAmount>, Option<ByteAmount>) {
    let allocatable = |name: &str| resource(node, name)?.allocatable.as_deref();
    (
        allocatable("cpu").and_then(CpuAmount::parse),
        allocatable("memory").and_then(ByteAmount::parse),
    )
}

/// How many pods the node can run, from the `pods` allocatable.
pub(crate) fn node_pod_limit(node: &NodeSummary) -> Option<u64> {
    resource(node, "pods")?.allocatable.as_deref()?.parse().ok()
}

fn resource<'a>(node: &'a NodeSummary, name: &str) -> Option<&'a NodeResource> {
    node.resources.iter().find(|resource| resource.name == name)
}

/// Whether a pod still takes room: a finished pod (Completed, Succeeded, Failed, Evicted, or Error)
/// holds no requests, so its containers no longer count.
pub(crate) fn takes_room(pod: &PodSummary) -> bool {
    !matches!(
        pod.status,
        PodStatus::Reason(
            StatusReason::Completed
                | StatusReason::Succeeded
                | StatusReason::Failed
                | StatusReason::Evicted
                | StatusReason::Error
        )
    )
}

/// The pods that take room on `node`: scheduled there and not finished.
fn pods_on_node<'a>(node: &'a str, pods: &'a [PodSummary]) -> impl Iterator<Item = &'a PodSummary> {
    pods.iter()
        .filter(move |pod| pod.node_name.as_deref() == Some(node) && takes_room(pod))
}

/// The `cpu` and `memory` requests of the pods on `node`.
pub(crate) fn node_requests(node: &str, pods: &[PodSummary]) -> (CpuAmount, ByteAmount) {
    requests_of(pods_on_node(node, pods))
}

/// What the pods on `node` request as a share of its allocatable (1.0 is all of it); `None` for a
/// resource without an allocatable entry.
pub(crate) fn node_request_share(node: &NodeSummary, pods: &[PodSummary]) -> NodeUsage {
    let (cpu, memory) = node_requests(&node.name, pods);
    let (cpu_total, memory_total) = node_allocatable(node);
    NodeUsage {
        cpu: ratio(
            cpu.nanocores() as f64,
            cpu_total.map(|total| total.nanocores() as f64),
        ),
        memory: ratio(
            memory.bytes() as f64,
            memory_total.map(|total| total.bytes() as f64),
        ),
    }
}

/// The `cpu` and `memory` requests of `pods`, which the caller has limited to those that take room:
/// main and sidecar containers, since init containers do not run alongside them.
pub(crate) fn requests_of<'a>(
    pods: impl Iterator<Item = &'a PodSummary>,
) -> (CpuAmount, ByteAmount) {
    let mut cpu: u64 = 0;
    let mut memory: u64 = 0;
    for container in pods
        .flat_map(|pod| &pod.containers)
        .filter(|container| container.kind != ContainerKind::Init)
    {
        for resource in &container.resources {
            let Some(request) = resource.request.as_deref() else {
                continue;
            };
            match resource.name.as_str() {
                "cpu" => {
                    cpu =
                        cpu.saturating_add(CpuAmount::parse(request).map_or(0, |a| a.nanocores()));
                }
                "memory" => {
                    memory =
                        memory.saturating_add(ByteAmount::parse(request).map_or(0, |a| a.bytes()));
                }
                _ => {}
            }
        }
    }
    (
        CpuAmount::from_nanocores(cpu),
        ByteAmount::from_bytes(memory),
    )
}

pub(crate) fn node_pod_count(node: &str, pods: &[PodSummary]) -> usize {
    pods_on_node(node, pods).count()
}

#[cfg(test)]
mod tests {
    use cluster::{
        ContainerResource, ContainerState, ContainerSummary, NodeReadiness, NodeScheduling,
        NodeStatus, NodeSystemInfo, PodStatus, ReadyCount, StatusReason,
    };

    use super::*;

    fn node(resources: &[(&str, &str)]) -> NodeSummary {
        NodeSummary {
            name: "wk-1".to_owned(),
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
            resources: resources
                .iter()
                .map(|(name, allocatable)| NodeResource {
                    name: (*name).to_owned(),
                    capacity: None,
                    allocatable: Some((*allocatable).to_owned()),
                })
                .collect(),
            labels: Vec::new(),
        }
    }

    fn usage(millicores: u64, mebibytes: u64) -> ResourceUsage {
        ResourceUsage {
            cpu: CpuAmount::from_nanocores(millicores * 1_000_000),
            memory: ByteAmount::from_bytes(mebibytes << 20),
        }
    }

    #[test]
    fn node_usage_divides_by_allocatable() {
        let node = node(&[("cpu", "4"), ("memory", "8Gi")]);
        let used = node_usage(&node, Some(usage(1_256, 4_096)));
        assert_eq!(used.cpu, Some(0.314));
        assert_eq!(used.memory, Some(0.5));
    }

    #[test]
    fn node_usage_is_none_without_sample_or_allocatable() {
        let full = node(&[("cpu", "4"), ("memory", "8Gi")]);
        assert_eq!(node_usage(&full, None), NodeUsage::default());
        let no_memory = node(&[("cpu", "4")]);
        assert_eq!(node_usage(&no_memory, Some(usage(1_000, 1))).memory, None);
        let zero = node(&[("cpu", "0"), ("memory", "0")]);
        assert_eq!(
            node_usage(&zero, Some(usage(1_000, 1))),
            NodeUsage::default()
        );
        let junk = node(&[("cpu", "many")]);
        assert_eq!(node_usage(&junk, Some(usage(1, 1))).cpu, None);
    }

    fn container(kind: ContainerKind, requests: &[(&str, &str)]) -> ContainerSummary {
        ContainerSummary {
            terminal: cluster::ContainerTerminal::None,
            name: "c".to_owned(),
            image: "img".to_owned(),
            kind,
            state: ContainerState::NotReported,
            is_ready: true,
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
            probes: cluster::ContainerProbes::default(),
            env: Vec::new(),
            env_from: Vec::new(),
            mounts: Vec::new(),
        }
    }

    fn pod(
        node: Option<&str>,
        reason: StatusReason,
        containers: Vec<ContainerSummary>,
    ) -> PodSummary {
        PodSummary {
            annotations: cluster::AnnotationTerms::default(),
            is_finished: false,
            namespace: "ns".to_owned(),
            name: "p".to_owned(),
            status: PodStatus::Reason(reason),
            ready: ReadyCount { ready: 1, total: 1 },
            restarts: 0,
            node_name: node.map(str::to_owned),
            created_at: None,
            pod_ip: None,
            qos_class: None,
            service_account: None,
            controller: None,
            conditions: Vec::new(),
            status_message: None,
            labels: Vec::new(),
            host_network: false,
            image_pull_secrets: Vec::new(),
            node_selector: Vec::new(),
            node_affinity: Vec::new(),
            containers,
        }
    }

    #[test]
    fn node_requests_sum_main_and_sidecar_on_the_node() {
        let busy = vec![
            container(ContainerKind::Main, &[("cpu", "250m"), ("memory", "128Mi")]),
            container(ContainerKind::Sidecar, &[("cpu", "100m")]),
            container(ContainerKind::Init, &[("cpu", "2"), ("memory", "1Gi")]),
        ];
        let pods = [
            pod(Some("wk-1"), StatusReason::Running, busy.clone()),
            pod(Some("wk-1"), StatusReason::Completed, busy.clone()),
            pod(Some("wk-2"), StatusReason::Running, busy),
            pod(None, StatusReason::Running, Vec::new()),
        ];
        let (cpu, memory) = node_requests("wk-1", &pods);
        assert_eq!(cpu.nanocores(), 350_000_000);
        assert_eq!(memory.bytes(), 128 << 20);
        assert_eq!(node_pod_count("wk-1", &pods), 1);
        assert_eq!(node_pod_count("wk-9", &pods), 0);
    }

    #[test]
    fn finished_pods_take_no_room() {
        let with = |reason: StatusReason| pod(Some("wk-1"), reason, Vec::new());
        assert!(takes_room(&with(StatusReason::Running)));
        assert!(takes_room(&with(StatusReason::CrashLoopBackOff)));
        assert!(takes_room(&with(StatusReason::Pending)));
        for finished in [
            StatusReason::Completed,
            StatusReason::Succeeded,
            StatusReason::Failed,
            StatusReason::Evicted,
            StatusReason::Error,
        ] {
            assert!(!takes_room(&with(finished.clone())), "{finished:?}");
        }
    }

    #[test]
    fn node_pod_limit_reads_the_pods_allocatable() {
        assert_eq!(node_pod_limit(&node(&[("pods", "110")])), Some(110));
        assert_eq!(node_pod_limit(&node(&[("pods", "lots")])), None);
        assert_eq!(node_pod_limit(&node(&[])), None);
    }

    #[test]
    fn node_request_share_divides_the_requests_by_allocatable() {
        let mut node = node(&[("cpu", "4"), ("memory", "8Gi")]);
        node.name = "wk-1".to_owned();
        let pods = [
            pod(
                Some("wk-1"),
                StatusReason::Running,
                vec![container(
                    ContainerKind::Main,
                    &[("cpu", "1"), ("memory", "2Gi")],
                )],
            ),
            // Another node's pod and a finished one add nothing.
            pod(
                Some("wk-2"),
                StatusReason::Running,
                vec![container(ContainerKind::Main, &[("cpu", "3")])],
            ),
            pod(
                Some("wk-1"),
                StatusReason::Completed,
                vec![container(ContainerKind::Main, &[("cpu", "3")])],
            ),
        ];
        let share = node_request_share(&node, &pods);
        assert_eq!(share.cpu, Some(0.25));
        assert_eq!(share.memory, Some(0.25));
        // No allocatable entry, no share.
        let mut no_memory = self::node(&[("cpu", "4")]);
        no_memory.name = "wk-1".to_owned();
        assert_eq!(node_request_share(&no_memory, &pods).memory, None);
    }
}
