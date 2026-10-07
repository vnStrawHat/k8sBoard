use cluster::{
    ContainerDiskIo, ContainerMetrics, ContainerProbes, ContainerResource, ContainerState,
    ControllerRef, DiskIoCounters, DiskIoSample, KubeletSummary, NamespaceScope, NetworkCounters,
    NodeKubeletStats, NodeMetrics, NodeReadiness, NodeResource, NodeScheduling, NodeStatus,
    NodeSystemInfo, PodKubeletStats, PodMetrics, PodStatus, ReadyCount, StatusReason, Termination,
};

use super::*;
use crate::cluster_metrics::FeedStatus;
use crate::kubelet_metrics::{KubeletDemand, KubeletSubject, NodeErrors};
use crate::usage_chart::ReferenceKind;

fn at(seconds: i64) -> jiff::Timestamp {
    jiff::Timestamp::from_second(seconds).expect("valid timestamp")
}

fn usage(millicores: u64, mebibytes: u64) -> ResourceUsage {
    ResourceUsage {
        cpu: CpuAmount::from_nanocores(millicores * 1_000_000),
        memory: ByteAmount::from_bytes(mebibytes << 20),
    }
}

fn resources(
    cpu: (Option<&str>, Option<&str>),
    memory: (Option<&str>, Option<&str>),
) -> Vec<ContainerResource> {
    let make = |name: &str, (request, limit): (Option<&str>, Option<&str>)| ContainerResource {
        name: name.to_owned(),
        request: request.map(str::to_owned),
        limit: limit.map(str::to_owned),
    };
    vec![make("cpu", cpu), make("memory", memory)]
}

fn container(
    name: &str,
    kind: ContainerKind,
    resources: Vec<ContainerResource>,
) -> ContainerSummary {
    ContainerSummary {
        terminal: cluster::ContainerTerminal::None,
        name: name.to_owned(),
        image: "img".to_owned(),
        kind,
        state: ContainerState::Running { started_at: None },
        is_ready: true,
        restart_count: 0,
        last_termination: None,
        image_digest: None,
        pull_policy: None,
        is_started: None,
        ports: Vec::new(),
        resources,
        probes: ContainerProbes::default(),
        env: Vec::new(),
        env_from: Vec::new(),
        mounts: Vec::new(),
    }
}

fn pod(name: &str, containers: Vec<ContainerSummary>) -> PodSummary {
    PodSummary {
        annotations: cluster::AnnotationTerms::default(),
        is_finished: false,
        namespace: "ns".to_owned(),
        name: name.to_owned(),
        status: PodStatus::Reason(StatusReason::Running),
        ready: ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: Some("wk-1".to_owned()),
        created_at: None,
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: Some(ControllerRef {
            kind: "ReplicaSet".to_owned(),
            name: "api-7d9f8c".to_owned(),
        }),
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

fn metrics(
    pod: &PodSummary,
    sampled_at: Option<i64>,
    values: &[(&str, ResourceUsage)],
) -> PodMetrics {
    PodMetrics {
        namespace: pod.namespace.clone(),
        name: pod.name.clone(),
        sampled_at: sampled_at.map(at),
        containers: values
            .iter()
            .map(|(name, usage)| ContainerMetrics {
                name: (*name).to_owned(),
                usage: *usage,
            })
            .collect(),
    }
}

/// `ticks` ticks, 15 s apart from `at(15)`, with the pod reporting `values` each time.
fn history(pods: &[PodSummary], ticks: i64, values: &[(&str, ResourceUsage)]) -> PodUsageHistory {
    let mut history = PodUsageHistory::default();
    for tick in 1..=ticks {
        let sample: Vec<PodMetrics> = pods
            .iter()
            .map(|pod| metrics(pod, Some(tick * 15), values))
            .collect();
        history.record(at(tick * 15), &sample, pods);
    }
    history
}

fn input<'a>(
    subject: MonitorSubject<'a>,
    scope: &'a MonitorScope,
    pods: &'a [PodSummary],
    pod_history: &'a PodUsageHistory,
    node_history: &'a NodeUsageHistory,
    kubelet: &'a KubeletFeed,
) -> MonitorInput<'a> {
    MonitorInput {
        subject,
        scope,
        range: MonitorRange::Minutes15,
        pods,
        pod_history,
        node_history,
        kubelet,
        nodes: &[],
        is_all_namespaces: true,
    }
}

fn reference<'a>(chart: &'a UsageChartModel, label: &str) -> Option<&'a ReferenceLine> {
    chart.references.iter().find(|line| line.label == label)
}

fn api_pod() -> PodSummary {
    pod(
        "api-7d9f8c-aaaaa",
        vec![
            container(
                "app",
                ContainerKind::Main,
                resources((Some("100m"), Some("200m")), (Some("128Mi"), Some("256Mi"))),
            ),
            container(
                "side",
                ContainerKind::Sidecar,
                resources((Some("50m"), Some("100m")), (Some("64Mi"), Some("128Mi"))),
            ),
            container(
                "init",
                ContainerKind::Init,
                resources((Some("1"), Some("2")), (Some("1Gi"), Some("2Gi"))),
            ),
        ],
    )
}

#[test]
fn pod_total_lines_sum_main_and_sidecar() {
    let pod = api_pod();
    let pods = [pod.clone()];
    let pod_history = history(&pods, 3, &[("app", usage(10, 100))]);
    let nodes = NodeUsageHistory::default();
    let scope = MonitorScope::Total;
    let data = monitor_data(&input(
        MonitorSubject::Pod(&pod),
        &scope,
        &pods,
        &pod_history,
        &nodes,
        &KubeletFeed::new(),
    ));
    let [cpu, memory] = &data.charts[..] else {
        panic!("a CPU and a Memory chart");
    };
    assert_eq!(cpu.id, "monitor-cpu");
    assert_eq!(memory.id, "monitor-memory");
    // Init containers do not run alongside, so their 1 core is not counted.
    let request = reference(cpu, "request").expect("a request line");
    assert!((request.value - 0.15).abs() < 1e-9);
    assert_eq!(request.kind, ReferenceKind::Request);
    let limit = reference(cpu, "limit").expect("a limit line");
    assert!((limit.value - 0.3).abs() < 1e-9);
    assert_eq!(limit.kind, ReferenceKind::Limit);
    assert_eq!(
        reference(memory, "request").map(|line| line.value),
        Some(192. * 1_048_576.)
    );
    assert_eq!(cpu.series[0].name, "pod total");
}

#[test]
fn request_line_is_partial_when_a_request_is_missing() {
    let mut pod = api_pod();
    pod.containers[1].resources = resources((None, Some("100m")), (None, Some("128Mi")));
    let pods = [pod.clone()];
    let pod_history = history(&pods, 2, &[("app", usage(10, 100))]);
    let nodes = NodeUsageHistory::default();
    let scope = MonitorScope::Total;
    let data = monitor_data(&input(
        MonitorSubject::Pod(&pod),
        &scope,
        &pods,
        &pod_history,
        &nodes,
        &KubeletFeed::new(),
    ));
    let cpu = &data.charts[0];
    let partial = reference(cpu, "request (partial)").expect("a partial request line");
    assert!((partial.value - 0.1).abs() < 1e-9);
    assert!(reference(cpu, "request").is_none());
    // With no request anywhere there is no line at all.
    pod.containers[0].resources = Vec::new();
    pod.containers[1].resources = Vec::new();
    let data = monitor_data(&input(
        MonitorSubject::Pod(&pod),
        &scope,
        &pods,
        &pod_history,
        &nodes,
        &KubeletFeed::new(),
    ));
    assert!(data.charts[0].references.is_empty());
}

#[test]
fn limit_line_needs_every_limit() {
    let mut pod = api_pod();
    pod.containers[1].resources = resources((Some("50m"), None), (Some("64Mi"), None));
    let pods = [pod.clone()];
    let pod_history = history(&pods, 2, &[("app", usage(10, 100))]);
    let nodes = NodeUsageHistory::default();
    let scope = MonitorScope::Total;
    let data = monitor_data(&input(
        MonitorSubject::Pod(&pod),
        &scope,
        &pods,
        &pod_history,
        &nodes,
        &KubeletFeed::new(),
    ));
    // The sidecar has no limit, so the pod's effective limit is unbounded.
    assert!(reference(&data.charts[0], "limit").is_none());
    assert!(reference(&data.charts[0], "request").is_some());
}

#[test]
fn container_scope_uses_its_own_lines_and_marks() {
    let mut pod = api_pod();
    let killed = Termination {
        reason: Some(StatusReason::OomKilled),
        exit_code: 137,
        signal: None,
        started_at: None,
        finished_at: Some(at(40)),
    };
    pod.containers[1].last_termination = Some(killed);
    let pods = [pod.clone()];
    let pod_history = history(&pods, 4, &[("app", usage(10, 100)), ("side", usage(5, 20))]);
    let nodes = NodeUsageHistory::default();
    let own = MonitorScope::Total;
    let data = monitor_data(&input(
        MonitorSubject::Container {
            pod: &pod,
            container: "side",
        },
        &own,
        &pods,
        &pod_history,
        &nodes,
        &KubeletFeed::new(),
    ));
    let cpu = &data.charts[0];
    assert!((reference(cpu, "request").expect("request").value - 0.05).abs() < 1e-9);
    assert!((reference(cpu, "limit").expect("limit").value - 0.1).abs() < 1e-9);
    assert_eq!(cpu.markers, [at(40)]);
    assert_eq!(cpu.series[0].name, "side");
    // The other container has no mark.
    let app = monitor_data(&input(
        MonitorSubject::Container {
            pod: &pod,
            container: "app",
        },
        &own,
        &pods,
        &pod_history,
        &nodes,
        &KubeletFeed::new(),
    ));
    assert!(app.charts[0].markers.is_empty());
    // The pod's total scope selects the same container through a part.
    let part = MonitorScope::Part("side".to_owned());
    let by_part = monitor_data(&input(
        MonitorSubject::Pod(&pod),
        &part,
        &pods,
        &pod_history,
        &nodes,
        &KubeletFeed::new(),
    ));
    assert_eq!(by_part.charts[0].markers, [at(40)]);
    assert_eq!(by_part.scope, part);
}

fn owner() -> PodOwner {
    PodOwner::Deployment {
        namespace: "ns".to_owned(),
        name: "api".to_owned(),
    }
}

fn three_pods() -> Vec<PodSummary> {
    ["api-7d9f8c-aaaaa", "api-7d9f8c-bbbbb", "api-7d9f8c-ccccc"]
        .into_iter()
        .map(|name| {
            pod(
                name,
                vec![container(
                    "app",
                    ContainerKind::Main,
                    resources((Some("100m"), Some("200m")), (Some("128Mi"), Some("256Mi"))),
                )],
            )
        })
        .collect()
}

#[test]
fn workload_total_is_named_by_pod_count() {
    let pods = three_pods();
    let pod_history = history(&pods, 2, &[("app", usage(10, 100))]);
    let nodes = NodeUsageHistory::default();
    let scope = MonitorScope::Total;
    let owner = owner();
    let data = monitor_data(&input(
        MonitorSubject::Workload(&owner),
        &scope,
        &pods,
        &pod_history,
        &nodes,
        &KubeletFeed::new(),
    ));
    assert_eq!(data.charts[0].series[0].name, "3 pods");
    // The lines sum over the owned pods now in the list.
    let request = reference(&data.charts[0], "request").expect("a request line");
    assert!((request.value - 0.3).abs() < 1e-9);
    let last = data.charts[0].series[0].points.last().expect("a point").1;
    assert_eq!(last, Some(0.03));
    let part = MonitorScope::Part("api-7d9f8c-bbbbb".to_owned());
    let one = monitor_data(&input(
        MonitorSubject::Workload(&owner),
        &part,
        &pods,
        &pod_history,
        &nodes,
        &KubeletFeed::new(),
    ));
    assert_eq!(one.charts[0].series[0].name, "api-7d9f8c-bbbbb");
    assert!((reference(&one.charts[0], "request").expect("request").value - 0.1).abs() < 1e-9);
}

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

fn node_history(ticks: i64) -> NodeUsageHistory {
    let mut history = NodeUsageHistory::default();
    for tick in 1..=ticks {
        history.record(
            at(tick * 15),
            &[NodeMetrics {
                name: "wk-1".to_owned(),
                sampled_at: Some(at(tick * 15)),
                usage: usage(500, 2_048),
            }],
        );
    }
    history
}

#[test]
fn node_requested_line_only_for_all_namespaces() {
    let node = node(&[("cpu", "4"), ("memory", "8Gi")]);
    let pods = three_pods();
    let pod_history = PodUsageHistory::default();
    let nodes = node_history(3);
    let scope = MonitorScope::Total;
    let feed = KubeletFeed::new();
    let mut all = input(
        MonitorSubject::Node(&node),
        &scope,
        &pods,
        &pod_history,
        &nodes,
        &feed,
    );
    let data = monitor_data(&all);
    let cpu = &data.charts[0];
    // Three pods on the node request 100m each.
    assert!((reference(cpu, "requested").expect("requested").value - 0.3).abs() < 1e-9);
    let allocatable = reference(cpu, "allocatable").expect("allocatable");
    assert_eq!(allocatable.kind, ReferenceKind::Allocatable);
    assert_eq!(allocatable.value, 4.);
    assert_eq!(cpu.series[0].name, "used");
    all.is_all_namespaces = false;
    let scoped = monitor_data(&all);
    assert!(reference(&scoped.charts[0], "requested").is_none());
    assert!(reference(&scoped.charts[0], "allocatable").is_some());
    assert_eq!(scoped.charts[1].references.len(), 1);
}

#[test]
fn scope_choices_follow_the_subject() {
    let pod = api_pod();
    let pods = [pod.clone()];
    let pod_history = history(&pods, 1, &[("app", usage(10, 100))]);
    let nodes = NodeUsageHistory::default();
    let scope = MonitorScope::Total;
    let labels = |data: &MonitorData| -> Vec<String> {
        data.choices
            .iter()
            .map(|choice| choice.label.clone())
            .collect()
    };
    let for_pod = monitor_data(&input(
        MonitorSubject::Pod(&pod),
        &scope,
        &pods,
        &pod_history,
        &nodes,
        &KubeletFeed::new(),
    ));
    // Main and sidecar containers only.
    assert_eq!(
        labels(&for_pod),
        ["Pod total", "Container: app", "Container: side"]
    );
    let three = three_pods();
    let owner = owner();
    let for_workload = monitor_data(&input(
        MonitorSubject::Workload(&owner),
        &scope,
        &three,
        &pod_history,
        &nodes,
        &KubeletFeed::new(),
    ));
    assert_eq!(
        labels(&for_workload),
        [
            "All pods",
            "Pod: api-7d9f8c-aaaaa",
            "Pod: api-7d9f8c-bbbbb",
            "Pod: api-7d9f8c-ccccc"
        ]
    );
    let node = node(&[]);
    let for_node = monitor_data(&input(
        MonitorSubject::Node(&node),
        &scope,
        &pods,
        &pod_history,
        &nodes,
        &KubeletFeed::new(),
    ));
    assert_eq!(labels(&for_node), ["Node total"]);
}

#[test]
fn stale_part_scope_falls_back_to_total() {
    let pod = api_pod();
    let pods = [pod.clone()];
    let pod_history = history(&pods, 2, &[("app", usage(10, 100))]);
    let nodes = NodeUsageHistory::default();
    let gone = MonitorScope::Part("gone".to_owned());
    let data = monitor_data(&input(
        MonitorSubject::Pod(&pod),
        &gone,
        &pods,
        &pod_history,
        &nodes,
        &KubeletFeed::new(),
    ));
    assert_eq!(data.scope, MonitorScope::Total);
    assert_eq!(data.charts[0].series[0].name, "pod total");
    // An init container is not offered either.
    let init = MonitorScope::Part("init".to_owned());
    let data = monitor_data(&input(
        MonitorSubject::Pod(&pod),
        &init,
        &pods,
        &pod_history,
        &nodes,
        &KubeletFeed::new(),
    ));
    assert_eq!(data.scope, MonitorScope::Total);
}

#[test]
fn stale_when_the_server_timestamp_has_not_advanced_for_four_ticks() {
    let pod = api_pod();
    let pods = [pod.clone()];
    let stalled = |ticks: i64| {
        let mut pod_history = PodUsageHistory::default();
        // The poll ticks every 15 s, but the server timestamp stays at 15 s.
        for tick in 1..=ticks {
            pod_history.record(
                at(tick * 15),
                &[metrics(&pod, Some(15), &[("app", usage(10, 100))])],
                &pods,
            );
        }
        pod_history
    };
    let pod_history = stalled(6);
    let nodes = NodeUsageHistory::default();
    let scope = MonitorScope::Total;
    let data = monitor_data(&input(
        MonitorSubject::Pod(&pod),
        &scope,
        &pods,
        &pod_history,
        &nodes,
        &KubeletFeed::new(),
    ));
    // The first poll set the timestamp; the next four brought the same one.
    assert_eq!(data.stale_since, Some(at(15)));
    // Three repeats are not enough, and the rule does not depend on the range.
    let early = stalled(4);
    let data = monitor_data(&input(
        MonitorSubject::Pod(&pod),
        &scope,
        &pods,
        &early,
        &nodes,
        &KubeletFeed::new(),
    ));
    assert_eq!(data.stale_since, None);
    let feed = KubeletFeed::new();
    let mut hours = input(
        MonitorSubject::Pod(&pod),
        &scope,
        &pods,
        &pod_history,
        &nodes,
        &feed,
    );
    hours.range = MonitorRange::Hours6;
    assert_eq!(monitor_data(&hours).stale_since, Some(at(15)));
    let fresh = history(&pods, 4, &[("app", usage(10, 100))]);
    let data = monitor_data(&input(
        MonitorSubject::Pod(&pod),
        &scope,
        &pods,
        &fresh,
        &nodes,
        &KubeletFeed::new(),
    ));
    assert_eq!(data.stale_since, None);
}

#[test]
fn short_history_marks_the_range() {
    let pod = api_pod();
    let pods = [pod.clone()];
    let nodes = NodeUsageHistory::default();
    let scope = MonitorScope::Total;
    // 3 ticks span 30 s, far less than 15 minutes.
    let short = history(&pods, 3, &[("app", usage(10, 100))]);
    let data = monitor_data(&input(
        MonitorSubject::Pod(&pod),
        &scope,
        &pods,
        &short,
        &nodes,
        &KubeletFeed::new(),
    ));
    assert!(data.is_short_for(MonitorRange::Minutes15));
    assert_eq!(data.span, Some(Duration::from_secs(30)));
    // 61 ticks span 15 minutes.
    let long = history(&pods, 61, &[("app", usage(10, 100))]);
    let data = monitor_data(&input(
        MonitorSubject::Pod(&pod),
        &scope,
        &pods,
        &long,
        &nodes,
        &KubeletFeed::new(),
    ));
    assert!(!data.is_short_for(MonitorRange::Minutes15));
    assert!(data.is_short_for(MonitorRange::Hour1));
    // Without a tick there is nothing to show yet.
    let empty = PodUsageHistory::default();
    let data = monitor_data(&input(
        MonitorSubject::Pod(&pod),
        &scope,
        &pods,
        &empty,
        &nodes,
        &KubeletFeed::new(),
    ));
    assert!(data.charts.is_empty());
    assert!(data.is_short_for(MonitorRange::Minutes15));
}

#[test]
fn rows_are_newest_first_with_oom_flags() {
    let mut pod = api_pod();
    pod.containers[0].last_termination = Some(Termination {
        reason: Some(StatusReason::OomKilled),
        exit_code: 137,
        signal: None,
        started_at: None,
        finished_at: Some(at(46)),
    });
    let pods = [pod.clone()];
    let mut pod_history = PodUsageHistory::default();
    for tick in 1..=4 {
        // The pod misses tick 3.
        let sample = if tick == 3 {
            Vec::new()
        } else {
            vec![metrics(
                &pod,
                Some(tick * 15),
                &[("app", usage(tick as u64 * 10, 100))],
            )]
        };
        pod_history.record(at(tick * 15), &sample, &pods);
    }
    let nodes = NodeUsageHistory::default();
    let scope = MonitorScope::Total;
    let data = monitor_data(&input(
        MonitorSubject::Pod(&pod),
        &scope,
        &pods,
        &pod_history,
        &nodes,
        &KubeletFeed::new(),
    ));
    let offsets: Vec<u64> = data.rows.iter().map(|row| row.offset).collect();
    assert_eq!(offsets, [0, 15, 30, 45]);
    assert_eq!(data.rows[0].cpu, Some(0.04));
    assert_eq!(data.rows[1].cpu, None);
    assert_eq!(data.rows[1].memory, None);
    // The kill at 46 s is within half a step of the tick at 45 s (the row 15 s back) only.
    let flags: Vec<bool> = data.rows.iter().map(|row| row.is_oom).collect();
    assert_eq!(flags, [false, true, false, false]);
}

#[test]
fn a_range_is_short_only_by_more_than_one_step() {
    let pod = api_pod();
    let pods = [pod.clone()];
    let nodes = NodeUsageHistory::default();
    let scope = MonitorScope::Total;
    // The history keeps at most 24 h minus a coarse step, so 24 h must not stay dimmed forever.
    let day = history(&pods, 5_780, &[("app", usage(10, 100))]);
    let data = monitor_data(&input(
        MonitorSubject::Pod(&pod),
        &scope,
        &pods,
        &day,
        &nodes,
        &KubeletFeed::new(),
    ));
    assert!(!data.is_short_for(MonitorRange::Hours24));
    assert!(!data.is_short_for(MonitorRange::Hours6));
    // One coarse step less than that is short again.
    let almost = history(&pods, 5_700, &[("app", usage(10, 100))]);
    let data = monitor_data(&input(
        MonitorSubject::Pod(&pod),
        &scope,
        &pods,
        &almost,
        &nodes,
        &KubeletFeed::new(),
    ));
    assert!(data.is_short_for(MonitorRange::Hours24));
}

#[test]
fn equal_request_and_limit_merge_into_one_line() {
    let guaranteed = |request: &str, limit: &str| {
        let mut pod = api_pod();
        pod.containers = vec![container(
            "app",
            ContainerKind::Main,
            resources((Some(request), Some(limit)), (Some("128Mi"), Some("128Mi"))),
        )];
        pod
    };
    let nodes = NodeUsageHistory::default();
    let scope = MonitorScope::Total;
    let pod = guaranteed("500m", "500m");
    let pods = [pod.clone()];
    let pod_history = history(&pods, 2, &[("app", usage(10, 100))]);
    let data = monitor_data(&input(
        MonitorSubject::Pod(&pod),
        &scope,
        &pods,
        &pod_history,
        &nodes,
        &KubeletFeed::new(),
    ));
    let cpu = &data.charts[0];
    assert_eq!(cpu.references.len(), 1);
    assert_eq!(cpu.references[0].label, "request = limit");
    assert_eq!(cpu.references[0].kind, ReferenceKind::Limit);
    assert_eq!(cpu.references[0].value, 0.5);
    assert_eq!(data.charts[1].references.len(), 1);
    // Within half a percent still counts; further apart keeps both lines.
    let near = guaranteed("1000m", "1004m");
    let data = monitor_data(&input(
        MonitorSubject::Pod(&near),
        &scope,
        std::slice::from_ref(&near),
        &pod_history,
        &nodes,
        &KubeletFeed::new(),
    ));
    assert_eq!(data.charts[0].references.len(), 1);
    let apart = guaranteed("500m", "1");
    let data = monitor_data(&input(
        MonitorSubject::Pod(&apart),
        &scope,
        std::slice::from_ref(&apart),
        &pod_history,
        &nodes,
        &KubeletFeed::new(),
    ));
    assert_eq!(data.charts[0].references.len(), 2);
}

#[test]
fn workload_lines_sum_requests_of_running_pods_only() {
    let mut pods = three_pods();
    pods[1].status = PodStatus::Reason(StatusReason::Completed);
    pods[2].status = PodStatus::Reason(StatusReason::Failed);
    let pod_history = history(&pods, 2, &[("app", usage(10, 100))]);
    let nodes = NodeUsageHistory::default();
    let scope = MonitorScope::Total;
    let owner = owner();
    let data = monitor_data(&input(
        MonitorSubject::Workload(&owner),
        &scope,
        &pods,
        &pod_history,
        &nodes,
        &KubeletFeed::new(),
    ));
    // Only the first pod still takes room, so its 100m request is the whole line.
    let request = reference(&data.charts[0], "request").expect("a request line");
    assert!((request.value - 0.1).abs() < 1e-9);
}

// ---- kubelet cards ----

fn node_named(name: &str, readiness: NodeReadiness) -> NodeSummary {
    let mut summary = node(&[]);
    summary.name = name.to_owned();
    summary.status.readiness = readiness;
    summary
}

fn pod_on(name: &str, node_name: &str) -> PodSummary {
    let mut summary = pod(
        name,
        vec![container("app", ContainerKind::Main, Vec::new())],
    );
    summary.node_name = Some(node_name.to_owned());
    summary
}

/// A disk read of one node: the root series and one container per `(pod, container)`.
fn disk_sample(step: i64, has_root: bool, containers: &[(&str, &str)]) -> DiskIoSample {
    let counters = |seed: u64| DiskIoCounters {
        sampled_at: Some(at(step * 15)),
        read_bytes: step as u64 * 15 * seed,
        write_bytes: step as u64 * 15 * seed * 2,
    };
    DiskIoSample {
        node: has_root.then(|| counters(1_000)),
        containers: containers
            .iter()
            .map(|(pod, container)| ContainerDiskIo {
                namespace: "ns".to_owned(),
                pod: (*pod).to_owned(),
                container: (*container).to_owned(),
                counters: counters(500),
            })
            .collect(),
    }
}

/// One node of one round: each pod of `pod_names` receives 1,000 B/s and sends 500 B/s, sampled
/// `offset` seconds after the metrics tick.
fn kubelet_round(
    node: &str,
    pod_names: &[&str],
    step: i64,
    offset: i64,
    disk: Option<DiskIoSample>,
) -> NodeKubeletStats {
    let sampled_at = Some(at(step * 15 + offset));
    let pods = pod_names
        .iter()
        .map(|name| PodKubeletStats {
            namespace: "ns".to_owned(),
            name: (*name).to_owned(),
            uid: format!("uid-{name}"),
            network: Some(NetworkCounters {
                sampled_at,
                rx_bytes: step as u64 * 15_000,
                tx_bytes: step as u64 * 7_500,
            }),
            volumes: Vec::new(),
        })
        .collect();
    NodeKubeletStats {
        node: node.to_owned(),
        summary: Ok(KubeletSummary {
            network: None,
            pods,
        }),
        disk_io: disk.map(Ok),
    }
}

/// A live feed with `ticks` rounds on `wk-1`, 15 s apart from `at(15) + offset`.
fn kubelet_feed(
    pods: &[PodSummary],
    ticks: i64,
    offset: i64,
    disk: impl Fn(i64) -> Option<DiskIoSample>,
) -> KubeletFeed {
    let mut feed = KubeletFeed::new();
    let names: Vec<&str> = pods.iter().map(|pod| pod.name.as_str()).collect();
    for step in 1..=ticks {
        let round = [kubelet_round("wk-1", &names, step, offset, disk(step))];
        feed.history
            .record(at(step * 15 + offset), &round, pods, &NamespaceScope::All);
    }
    feed.status = FeedStatus::Live;
    feed
}

fn no_disk(_: i64) -> Option<DiskIoSample> {
    None
}

fn with_nodes<'a>(base: MonitorInput<'a>, nodes: &'a [NodeSummary]) -> MonitorInput<'a> {
    MonitorInput { nodes, ..base }
}

fn notice(chart: &UsageChartModel) -> Option<&str> {
    chart.notice.as_deref()
}

/// Pod `api_pod()` on `wk-1`, with the metrics history behind it.
struct Fixture {
    pods: Vec<PodSummary>,
    pod_history: PodUsageHistory,
    node_history: NodeUsageHistory,
    nodes: Vec<NodeSummary>,
}

impl Fixture {
    fn new() -> Self {
        let pods = vec![api_pod()];
        let pod_history = history(&pods, 4, &[("app", usage(10, 100))]);
        Self {
            pods,
            pod_history,
            node_history: node_history(4),
            nodes: vec![node_named("wk-1", NodeReadiness::Ready)],
        }
    }

    fn data(
        &self,
        subject: MonitorSubject,
        scope: &MonitorScope,
        feed: &KubeletFeed,
    ) -> MonitorData {
        let base = input(
            subject,
            scope,
            &self.pods,
            &self.pod_history,
            &self.node_history,
            feed,
        );
        monitor_data(&with_nodes(base, &self.nodes))
    }
}

#[test]
fn charts_are_cpu_memory_network_disk() {
    let fixture = Fixture::new();
    let feed = kubelet_feed(&fixture.pods, 4, 0, |step| {
        Some(disk_sample(step, true, &[("api-7d9f8c-aaaaa", "app")]))
    });
    let pod = &fixture.pods[0];
    let data = fixture.data(MonitorSubject::Pod(pod), &MonitorScope::Total, &feed);

    let titles: Vec<&str> = data
        .charts
        .iter()
        .chain(&data.kubelet_charts)
        .map(|chart| chart.title.as_ref())
        .collect();
    assert_eq!(titles, ["CPU", "Memory", "Network", "Disk I/O"]);
    let [network, disk] = &data.kubelet_charts[..] else {
        panic!("a Network and a Disk I/O chart");
    };
    assert_eq!(
        (network.id.as_ref(), disk.id.as_ref()),
        ("monitor-network", "monitor-disk")
    );
    let names = |chart: &UsageChartModel| -> Vec<String> {
        chart
            .series
            .iter()
            .map(|series| series.name.to_string())
            .collect()
    };
    assert_eq!(names(network), ["receive", "transmit"]);
    assert_eq!(names(disk), ["read", "write"]);
    assert_eq!(network.unit, Measure::Rate);
    assert_eq!(disk.unit, Measure::Rate);
    // 15,000 B over 15 s received, half of it sent.
    let newest = |chart: &UsageChartModel, series: usize| {
        chart.series[series].points.last().and_then(|point| point.1)
    };
    assert_eq!(newest(network, 0), Some(1_000.));
    assert_eq!(newest(network, 1), Some(500.));
    assert_eq!(newest(disk, 0), Some(500.));
    assert_eq!(newest(disk, 1), Some(1_000.));
    assert_eq!(notice(network), None);
    assert_eq!(notice(disk), None);
}

#[test]
fn collecting_until_two_ticks() {
    let fixture = Fixture::new();
    let pod = &fixture.pods[0];
    let one = kubelet_feed(&fixture.pods, 1, 0, no_disk);
    let data = fixture.data(MonitorSubject::Pod(pod), &MonitorScope::Total, &one);
    for chart in &data.kubelet_charts {
        assert_eq!(notice(chart), Some("Collecting… rates need two samples"));
    }
    let two = kubelet_feed(&fixture.pods, 2, 0, no_disk);
    let data = fixture.data(MonitorSubject::Pod(pod), &MonitorScope::Total, &two);
    assert_eq!(notice(&data.kubelet_charts[0]), None);
}

#[test]
fn a_waiting_feed_is_collecting_whatever_its_ticks() {
    let fixture = Fixture::new();
    let mut feed = kubelet_feed(&fixture.pods, 4, 0, no_disk);
    feed.status = FeedStatus::Waiting;
    let data = fixture.data(
        MonitorSubject::Pod(&fixture.pods[0]),
        &MonitorScope::Total,
        &feed,
    );
    assert_eq!(
        notice(&data.kubelet_charts[0]),
        Some("Collecting… rates need two samples")
    );
}

#[test]
fn not_ready_and_failed_nodes_have_notices() {
    let mut fixture = Fixture::new();
    let feed = kubelet_feed(&fixture.pods, 4, 0, no_disk);
    let pod = fixture.pods[0].clone();
    fixture.nodes = vec![node_named("wk-1", NodeReadiness::NotReady)];
    let data = fixture.data(MonitorSubject::Pod(&pod), &MonitorScope::Total, &feed);
    for chart in &data.kubelet_charts {
        assert_eq!(
            notice(chart),
            Some("Node wk-1 is not ready: no kubelet stats")
        );
    }
    // The node itself, too.
    let node = fixture.nodes[0].clone();
    let data = fixture.data(MonitorSubject::Node(&node), &MonitorScope::Total, &feed);
    assert_eq!(
        notice(&data.kubelet_charts[0]),
        Some("Node wk-1 is not ready: no kubelet stats")
    );
}

#[test]
fn failed_node_has_a_notice() {
    let fixture = Fixture::new();
    let mut feed = kubelet_feed(&fixture.pods, 4, 0, no_disk);
    feed.node_errors.insert(
        "wk-1".to_owned(),
        NodeErrors {
            summary: Some("the kubelet does not answer through the API server proxy".to_owned()),
            disk_io: None,
        },
    );
    let data = fixture.data(
        MonitorSubject::Pod(&fixture.pods[0]),
        &MonitorScope::Total,
        &feed,
    );
    assert_eq!(
        notice(&data.kubelet_charts[0]),
        Some("Kubelet on wk-1: the kubelet does not answer through the API server proxy")
    );
}

#[test]
fn node_errors_split_summary_and_disk() {
    let fixture = Fixture::new();
    let mut feed = kubelet_feed(&fixture.pods, 4, 0, |step| {
        Some(disk_sample(step, true, &[]))
    });
    feed.node_errors.insert(
        "wk-1".to_owned(),
        NodeErrors {
            summary: None,
            disk_io: Some("the kubelet does not serve this endpoint".to_owned()),
        },
    );
    let pod = &fixture.pods[0];
    let data = fixture.data(MonitorSubject::Pod(pod), &MonitorScope::Total, &feed);
    let [network, disk] = &data.kubelet_charts[..] else {
        panic!("two kubelet charts");
    };
    assert_eq!(notice(network), None);
    assert_eq!(
        notice(disk),
        Some("Kubelet on wk-1: the kubelet does not serve this endpoint")
    );
    feed.node_errors.insert(
        "wk-1".to_owned(),
        NodeErrors {
            summary: Some("summary failed".to_owned()),
            disk_io: None,
        },
    );
    let data = fixture.data(MonitorSubject::Pod(pod), &MonitorScope::Total, &feed);
    assert_eq!(
        notice(&data.kubelet_charts[0]),
        Some("Kubelet on wk-1: summary failed")
    );
    assert_ne!(
        notice(&data.kubelet_charts[1]),
        Some("Kubelet on wk-1: summary failed")
    );
}

#[test]
fn host_network_pod_has_no_network_series() {
    let mut fixture = Fixture::new();
    fixture.pods[0].host_network = true;
    let feed = kubelet_feed(&fixture.pods, 4, 0, no_disk);
    let pod = fixture.pods[0].clone();
    let data = fixture.data(MonitorSubject::Pod(&pod), &MonitorScope::Total, &feed);
    let network = &data.kubelet_charts[0];
    assert_eq!(
        notice(network),
        Some("Host network: traffic is the node's (see the node's Monitor tab)")
    );
    assert!(
        network
            .series
            .iter()
            .all(|series| series.points.iter().all(|point| point.1.is_none()))
    );
}

fn spread_pods() -> Vec<PodSummary> {
    (1..=7)
        .map(|index| pod_on(&format!("api-7d9f8c-{index}"), &format!("wk-{index}")))
        .collect()
}

fn twelve_ready_nodes() -> Vec<NodeSummary> {
    (1..=12)
        .map(|index| node_named(&format!("wk-{index}"), NodeReadiness::Ready))
        .collect()
}

/// The workload's seven pods, one per node, on twelve Ready nodes: the summary follows the
/// demand, and the disk reads at most three nodes (`wk-1` to `wk-3`).
fn spread_feed(pods: &[PodSummary], nodes: &[NodeSummary]) -> KubeletFeed {
    let mut feed = KubeletFeed::new();
    feed.set_demand(
        KubeletDemand {
            subject: Some(KubeletSubject::Workload(owner())),
            wants_disk_io: true,
        },
        nodes,
        pods,
    );
    for step in 1..=4 {
        let round: Vec<NodeKubeletStats> = (1..=7)
            .map(|index| {
                let name = format!("api-7d9f8c-{index}");
                let disk = (index <= 3).then(|| disk_sample(step, true, &[(name.as_str(), "app")]));
                kubelet_round(&format!("wk-{index}"), &[name.as_str()], step, 0, disk)
            })
            .collect();
        feed.history
            .record(at(step * 15), &round, pods, &NamespaceScope::All);
    }
    feed.status = FeedStatus::Live;
    feed
}

fn workload_data(
    pods: &[PodSummary],
    nodes: &[NodeSummary],
    feed: &KubeletFeed,
    scope: &MonitorScope,
) -> MonitorData {
    let owner = owner();
    let pod_history = history(pods, 4, &[("app", usage(10, 100))]);
    let node_usage = NodeUsageHistory::default();
    let base = input(
        MonitorSubject::Workload(&owner),
        scope,
        pods,
        &pod_history,
        &node_usage,
        feed,
    );
    monitor_data(&with_nodes(base, nodes))
}

#[test]
fn workload_coverage_notice_counts_nodes() {
    let pods = spread_pods();
    let nodes = twelve_ready_nodes();
    let feed = spread_feed(&pods, &nodes);
    let data = workload_data(&pods, &nodes, &feed, &MonitorScope::Total);
    // All seven nodes are polled for the summary, three for the disk.
    assert_eq!(notice(&data.kubelet_charts[0]), None);
    assert_eq!(
        notice(&data.kubelet_charts[1]),
        Some("Covers pods on 3 of 7 nodes")
    );
}

#[test]
fn part_scope_on_an_unread_node_gets_the_coverage_notice() {
    let pods = spread_pods();
    let nodes = twelve_ready_nodes();
    let feed = spread_feed(&pods, &nodes);
    // The seventh pod runs on a node the disk does not read: not "still collecting".
    let part = MonitorScope::Part("api-7d9f8c-7".to_owned());
    let data = workload_data(&pods, &nodes, &feed, &part);
    assert_eq!(
        notice(&data.kubelet_charts[1]),
        Some("Covers pods on 0 of 1 nodes")
    );
    assert_eq!(notice(&data.kubelet_charts[0]), None);
}

#[test]
fn workload_network_counts_host_network_pods() {
    let mut pods = spread_pods();
    pods[0].host_network = true;
    pods[1].host_network = true;
    let feed = {
        let mut feed = KubeletFeed::new();
        for step in 1..=4 {
            let round: Vec<NodeKubeletStats> = pods
                .iter()
                .map(|pod| {
                    let node = pod.node_name.clone().unwrap_or_default();
                    kubelet_round(&node, &[pod.name.as_str()], step, 0, None)
                })
                .collect();
            feed.history
                .record(at(step * 15), &round, &pods, &NamespaceScope::All);
        }
        feed.status = FeedStatus::Live;
        feed
    };
    let nodes: Vec<NodeSummary> = (1..=7)
        .map(|index| node_named(&format!("wk-{index}"), NodeReadiness::Ready))
        .collect();
    let covering = feed;
    covering.refresh_targets(&nodes, &pods);
    let owner = owner();
    let pod_history = history(&pods, 4, &[("app", usage(10, 100))]);
    let node_usage = NodeUsageHistory::default();
    let scope = MonitorScope::Total;
    let base = input(
        MonitorSubject::Workload(&owner),
        &scope,
        &pods,
        &pod_history,
        &node_usage,
        &covering,
    );
    let data = monitor_data(&with_nodes(base, &nodes));
    assert_eq!(
        notice(&data.kubelet_charts[0]),
        Some("2 host-network pods not counted")
    );
}

#[test]
fn node_without_root_disk_io_has_notice() {
    let fixture = Fixture::new();
    let feed = kubelet_feed(&fixture.pods, 4, 0, |step| {
        Some(disk_sample(step, false, &[]))
    });
    let node = fixture.nodes[0].clone();
    let data = fixture.data(MonitorSubject::Node(&node), &MonitorScope::Total, &feed);
    assert_eq!(
        notice(&data.kubelet_charts[1]),
        Some("The kubelet reports no node-level disk I/O")
    );
}

#[test]
fn node_with_root_disk_io_has_data_and_no_notice() {
    let fixture = Fixture::new();
    let feed = kubelet_feed(&fixture.pods, 4, 0, |step| {
        Some(disk_sample(step, true, &[]))
    });
    let node = fixture.nodes[0].clone();
    let data = fixture.data(MonitorSubject::Node(&node), &MonitorScope::Total, &feed);
    let disk = &data.kubelet_charts[1];
    assert_eq!(notice(disk), None);
    assert_eq!(
        disk.series[0].points.last().and_then(|point| point.1),
        Some(1_000.)
    );
}

#[test]
fn pod_without_disk_series_has_notice() {
    let fixture = Fixture::new();
    // The node was read, but cAdvisor listed no container of this pod.
    let feed = kubelet_feed(&fixture.pods, 4, 0, |step| {
        Some(disk_sample(step, true, &[("other-pod", "app")]))
    });
    let pod = &fixture.pods[0];
    let total = fixture.data(MonitorSubject::Pod(pod), &MonitorScope::Total, &feed);
    assert_eq!(
        notice(&total.kubelet_charts[1]),
        Some("The kubelet reports no disk I/O for this pod")
    );
    let container = fixture.data(
        MonitorSubject::Container {
            pod,
            container: "app",
        },
        &MonitorScope::Total,
        &feed,
    );
    assert_eq!(
        notice(&container.kubelet_charts[1]),
        Some("The kubelet reports no disk I/O for this container")
    );
}

#[test]
fn pod_disk_is_collecting_until_its_node_is_read() {
    let fixture = Fixture::new();
    let feed = kubelet_feed(&fixture.pods, 4, 0, no_disk);
    let data = fixture.data(
        MonitorSubject::Pod(&fixture.pods[0]),
        &MonitorScope::Total,
        &feed,
    );
    assert_eq!(
        notice(&data.kubelet_charts[1]),
        Some("Collecting… rates need two samples")
    );
}

#[test]
fn container_scope_network_is_the_pods_with_a_notice() {
    let fixture = Fixture::new();
    let feed = kubelet_feed(&fixture.pods, 4, 0, no_disk);
    let pod = &fixture.pods[0];
    let whole = fixture.data(MonitorSubject::Pod(pod), &MonitorScope::Total, &feed);
    let part = fixture.data(
        MonitorSubject::Pod(pod),
        &MonitorScope::Part("app".to_owned()),
        &feed,
    );
    assert_eq!(
        part.kubelet_charts[0].series[0].points,
        whole.kubelet_charts[0].series[0].points
    );
    assert_eq!(
        notice(&part.kubelet_charts[0]),
        Some("Pod network, shared by all containers")
    );
    assert_eq!(notice(&whole.kubelet_charts[0]), None);
}

#[test]
fn workload_without_disk_series_has_notice() {
    let pods = vec![
        pod_on("api-7d9f8c-1", "wk-1"),
        pod_on("api-7d9f8c-2", "wk-1"),
    ];
    // The node was read; no container of either pod has a series.
    let feed = kubelet_feed(&pods, 4, 0, |step| {
        Some(disk_sample(step, true, &[("elsewhere", "app")]))
    });
    let nodes = [node_named("wk-1", NodeReadiness::Ready)];
    let owner = owner();
    let pod_history = history(&pods, 4, &[("app", usage(10, 100))]);
    let node_usage = NodeUsageHistory::default();
    let scope = MonitorScope::Total;
    let base = input(
        MonitorSubject::Workload(&owner),
        &scope,
        &pods,
        &pod_history,
        &node_usage,
        &feed,
    );
    let data = monitor_data(&with_nodes(base, &nodes));
    assert_eq!(
        notice(&data.kubelet_charts[1]),
        Some("The kubelet reports no disk I/O for these pods")
    );
}

#[test]
fn workload_with_one_disk_series_has_no_notice() {
    let pods = vec![
        pod_on("api-7d9f8c-1", "wk-1"),
        pod_on("api-7d9f8c-2", "wk-1"),
    ];
    let feed = kubelet_feed(&pods, 4, 0, |step| {
        Some(disk_sample(step, true, &[("api-7d9f8c-2", "app")]))
    });
    let nodes = [node_named("wk-1", NodeReadiness::Ready)];
    let owner = owner();
    let pod_history = history(&pods, 4, &[("app", usage(10, 100))]);
    let node_usage = NodeUsageHistory::default();
    let scope = MonitorScope::Total;
    let mut with_targets = feed;
    let demand = KubeletDemand {
        subject: Some(KubeletSubject::Workload(owner.clone())),
        wants_disk_io: true,
    };
    with_targets.set_demand(demand, &nodes, &pods);
    let base = input(
        MonitorSubject::Workload(&owner),
        &scope,
        &pods,
        &pod_history,
        &node_usage,
        &with_targets,
    );
    let data = monitor_data(&with_nodes(base, &nodes));
    assert_eq!(notice(&data.kubelet_charts[1]), None);
}

#[test]
fn kubelet_cards_end_at_the_kubelet_tick_without_metrics() {
    let pods = vec![api_pod()];
    let feed = kubelet_feed(&pods, 4, 5, no_disk);
    let nodes = [node_named("wk-1", NodeReadiness::Ready)];
    let no_pods = PodUsageHistory::default();
    let no_nodes = NodeUsageHistory::default();
    let scope = MonitorScope::Total;
    let pod = &pods[0];
    let base = input(
        MonitorSubject::Pod(pod),
        &scope,
        &pods,
        &no_pods,
        &no_nodes,
        &feed,
    );
    let data = monitor_data(&with_nodes(base, &nodes));
    // No metrics tick: no CPU or Memory card, and the kubelet cards end at their own newest tick.
    assert!(data.charts.is_empty());
    assert_eq!(data.kubelet_charts[0].end, at(4 * 15 + 5));
    assert_eq!(data.kubelet_charts[1].end, at(4 * 15 + 5));

    // With metrics ticks the four cards share the metrics end.
    let with_metrics = history(&pods, 4, &[("app", usage(10, 100))]);
    let base = input(
        MonitorSubject::Pod(pod),
        &scope,
        &pods,
        &with_metrics,
        &no_nodes,
        &feed,
    );
    let data = monitor_data(&with_nodes(base, &nodes));
    assert_eq!(data.charts[0].end, at(4 * 15));
    assert_eq!(data.kubelet_charts[0].end, at(4 * 15));
}

#[test]
fn rows_follow_the_kubelet_timeline_without_metrics() {
    let pods = vec![api_pod()];
    let feed = kubelet_feed(&pods, 4, 0, no_disk);
    let nodes = [node_named("wk-1", NodeReadiness::Ready)];
    let no_pods = PodUsageHistory::default();
    let no_nodes = NodeUsageHistory::default();
    let scope = MonitorScope::Total;
    let base = input(
        MonitorSubject::Pod(&pods[0]),
        &scope,
        &pods,
        &no_pods,
        &no_nodes,
        &feed,
    );
    let data = monitor_data(&with_nodes(base, &nodes));
    let offsets: Vec<u64> = data.rows.iter().map(|row| row.offset).collect();
    assert_eq!(offsets, [0, 15, 30, 45]);
    assert!(
        data.rows
            .iter()
            .all(|row| row.cpu.is_none() && row.memory.is_none())
    );
    assert_eq!(
        data.rows[0].network,
        Some(RatePair {
            first: 1_000.,
            second: 500.,
        })
    );
    // The first kubelet tick has no rate yet.
    assert_eq!(data.rows[3].network, None);
}

#[test]
fn rows_join_the_nearest_kubelet_tick() {
    let pods = vec![api_pod()];
    let nodes = [node_named("wk-1", NodeReadiness::Ready)];
    let no_nodes = NodeUsageHistory::default();
    let scope = MonitorScope::Total;
    let metrics = history(&pods, 4, &[("app", usage(10, 100))]);
    // Two kubelet ticks, at 22 s and 37 s; only the second has a rate.
    let feed = kubelet_feed(&pods, 2, 7, no_disk);
    let base = input(
        MonitorSubject::Pod(&pods[0]),
        &scope,
        &pods,
        &metrics,
        &no_nodes,
        &feed,
    );
    let rows = monitor_data(&with_nodes(base, &nodes)).rows;
    let joined: Vec<(u64, bool)> = rows
        .iter()
        .map(|row| (row.offset, row.network.is_some()))
        .collect();
    // Metrics ticks at 60, 45, 30, 15 s. Only 30 s has a kubelet rate within 7.5 s (37 s).
    assert_eq!(joined, [(0, false), (15, false), (30, true), (45, false)]);
}

#[test]
fn a_card_without_a_rate_yet_is_collecting() {
    let fixture = Fixture::new();
    // The node was read, and the pod has a disk series, but only one disk sample so far.
    let mut feed = kubelet_feed(&fixture.pods, 4, 0, no_disk);
    let round = [kubelet_round(
        "wk-1",
        &["api-7d9f8c-aaaaa"],
        5,
        0,
        Some(disk_sample(5, true, &[("api-7d9f8c-aaaaa", "app")])),
    )];
    feed.history
        .record(at(5 * 15), &round, &fixture.pods, &NamespaceScope::All);
    let data = fixture.data(
        MonitorSubject::Pod(&fixture.pods[0]),
        &MonitorScope::Total,
        &feed,
    );
    assert_eq!(
        notice(&data.kubelet_charts[1]),
        Some("Collecting… rates need two samples")
    );
    assert_eq!(notice(&data.kubelet_charts[0]), None);
}

#[test]
fn part_scope_on_a_host_network_pod_reads_one_host_network_pod() {
    let mut pods = spread_pods();
    pods[0].host_network = true;
    let nodes = twelve_ready_nodes();
    let feed = spread_feed(&pods, &nodes);
    let part = MonitorScope::Part("api-7d9f8c-1".to_owned());
    let data = workload_data(&pods, &nodes, &feed, &part);
    assert_eq!(
        notice(&data.kubelet_charts[0]),
        Some("1 host-network pod not counted")
    );
}
