use cluster::{
    ContainerMetrics, ContainerProbes, ContainerResource, ContainerState, ControllerRef,
    NodeMetrics, NodeReadiness, NodeResource, NodeScheduling, NodeStatus, NodeSystemInfo,
    PodMetrics, PodStatus, ReadyCount, StatusReason, Termination,
};

use super::*;
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
) -> MonitorInput<'a> {
    MonitorInput {
        subject,
        scope,
        range: MonitorRange::Minutes15,
        pods,
        pod_history,
        node_history,
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
    let mut all = input(
        MonitorSubject::Node(&node),
        &scope,
        &pods,
        &pod_history,
        &nodes,
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
    ));
    assert_eq!(data.stale_since, None);
    let mut hours = input(
        MonitorSubject::Pod(&pod),
        &scope,
        &pods,
        &pod_history,
        &nodes,
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
    ));
    assert_eq!(data.charts[0].references.len(), 1);
    let apart = guaranteed("500m", "1");
    let data = monitor_data(&input(
        MonitorSubject::Pod(&apart),
        &scope,
        std::slice::from_ref(&apart),
        &pod_history,
        &nodes,
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
    ));
    // Only the first pod still takes room, so its 100m request is the whole line.
    let request = reference(&data.charts[0], "request").expect("a request line");
    assert!((request.value - 0.1).abs() < 1e-9);
}
