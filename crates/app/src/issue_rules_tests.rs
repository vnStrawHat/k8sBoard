use cluster::{
    ContainerMetrics, ContainerProbes, ContainerResource, ControllerRef, EventType, InvolvedObject,
    KubeletSummary, NamespaceScope, NodeKubeletStats, NodeMetrics, NodeResource, NodeScheduling,
    NodeStatus, NodeSystemInfo, PodCondition, PodKubeletStats, PodMetrics, ProbeAction,
    ProbeSummary, PvcUsage, ReadyCount, ResourceUsage, WatchUpdate,
};

use super::*;
use crate::issue_feeds::WarningEvents;
use crate::kubelet_history::KubeletHistory;
use crate::metrics_history::{NodeUsageHistory, PodUsageHistory};
use crate::usage_format::usage_tone;

const NOW: i64 = 1_000_000;

fn at(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).expect("valid timestamp")
}

fn now() -> Timestamp {
    at(NOW)
}

fn ago(seconds: i64) -> Timestamp {
    at(NOW - seconds)
}

fn inputs() -> IssueInputs<'static> {
    IssueInputs {
        pods: None,
        nodes: None,
        scope: &NamespaceScope::All,
        namespaces: None,
        events: None,
        objects: &[],
        pod_usage: None,
        node_usage: None,
        kubelet: None,
        is_job_feed_live: false,
        now: now(),
    }
}

fn container(name: &str, state: ContainerState) -> ContainerSummary {
    ContainerSummary {
        name: name.to_owned(),
        image: "registry/app:1".to_owned(),
        kind: ContainerKind::Main,
        state,
        is_ready: false,
        restart_count: 0,
        last_termination: None,
        image_digest: None,
        pull_policy: None,
        is_started: None,
        ports: Vec::new(),
        resources: Vec::new(),
        probes: ContainerProbes::default(),
        env: Vec::new(),
        env_from: Vec::new(),
        mounts: Vec::new(),
    }
}

fn running() -> ContainerState {
    ContainerState::Running { started_at: None }
}

fn serving(name: &str) -> ContainerSummary {
    ContainerSummary {
        is_ready: true,
        ..container(name, running())
    }
}

fn waiting(reason: StatusReason) -> ContainerState {
    ContainerState::Waiting {
        reason: Some(reason),
        message: None,
    }
}

fn pod_with(name: &str, status: PodStatus, containers: Vec<ContainerSummary>) -> PodSummary {
    PodSummary {
        namespace: "shop".to_owned(),
        name: name.to_owned(),
        status,
        ready: ReadyCount { ready: 0, total: 1 },
        restarts: 0,
        node_name: None,
        created_at: Some(ago(3_600)),
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: Some(ControllerRef {
            kind: "ReplicaSet".to_owned(),
            name: "api-7d9f8c".to_owned(),
        }),
        conditions: Vec::new(),
        containers,
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        image_pull_secrets: Vec::new(),
    }
}

fn running_pod(containers: Vec<ContainerSummary>) -> PodSummary {
    pod_with(
        "api-0",
        PodStatus::Reason(StatusReason::Running),
        containers,
    )
}

fn not_ready_since(seconds_ago: i64) -> PodCondition {
    PodCondition {
        name: "Ready".to_owned(),
        is_true: false,
        reason: None,
        message: None,
        changed_at: Some(ago(seconds_ago)),
    }
}

fn termination(reason: Option<StatusReason>, exit_code: i32, finished_ago: i64) -> Termination {
    Termination {
        reason,
        exit_code,
        signal: None,
        started_at: Some(ago(finished_ago + 30)),
        finished_at: Some(ago(finished_ago)),
    }
}

fn crash_looping() -> ContainerSummary {
    ContainerSummary {
        restart_count: 7,
        last_termination: Some(termination(Some(StatusReason::Error), 1, 120)),
        ..container("api", waiting(StatusReason::CrashLoopBackOff))
    }
}

fn limit(name: &str, quantity: &str) -> ContainerResource {
    ContainerResource {
        name: name.to_owned(),
        request: None,
        limit: Some(quantity.to_owned()),
    }
}

fn finding_of(pod: &PodSummary) -> Option<Finding> {
    pod_finding(pod, &inputs())
}

fn node_with(name: &str, readiness: NodeReadiness, conditions: Vec<NodeCondition>) -> NodeSummary {
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
        conditions,
        addresses: Vec::new(),
        system: NodeSystemInfo::default(),
        resources: vec![
            NodeResource {
                name: "cpu".to_owned(),
                capacity: Some("4".to_owned()),
                allocatable: Some("4".to_owned()),
            },
            NodeResource {
                name: "memory".to_owned(),
                capacity: Some("8Gi".to_owned()),
                allocatable: Some("8Gi".to_owned()),
            },
        ],
        labels: Vec::new(),
    }
}

fn condition(
    name: &str,
    status: ConditionStatus,
    message: Option<&str>,
    changed_ago: Option<i64>,
) -> NodeCondition {
    NodeCondition {
        name: name.to_owned(),
        status,
        reason: None,
        message: message.map(str::to_owned),
        changed_at: changed_ago.map(ago),
    }
}

fn warning(
    kind: &str,
    name: &str,
    reason: &str,
    message: &str,
    count: u32,
    first_ago: i64,
    last_ago: i64,
) -> EventSummary {
    EventSummary {
        namespace: "shop".to_owned(),
        name: format!("{name}.{reason}"),
        event_type: EventType::Warning,
        reason: reason.to_owned(),
        object: InvolvedObject {
            kind: kind.to_owned(),
            namespace: Some("shop".to_owned()),
            name: name.to_owned(),
        },
        message: message.to_owned(),
        count,
        first_seen: Some(ago(first_ago)),
        last_seen: Some(ago(last_ago)),
        source: None,
        container: None,
    }
}

fn feed(events: Vec<EventSummary>) -> WarningEvents {
    let mut feed = WarningEvents::default();
    feed.apply(WatchUpdate::Snapshot(events));
    feed
}

fn events_findings(events: Vec<EventSummary>) -> Vec<Finding> {
    let feed = feed(events);
    event_findings(&IssueInputs {
        events: Some(&feed),
        ..inputs()
    })
}

fn usage(millicores: u64, mebibytes: u64) -> ResourceUsage {
    ResourceUsage {
        cpu: CpuAmount::from_nanocores(millicores * 1_000_000),
        memory: ByteAmount::from_bytes(mebibytes << 20),
    }
}

/// A history with one container of `api-0` sampled at each of `samples`.
fn pod_history(samples: &[ResourceUsage]) -> PodUsageHistory {
    let mut history = PodUsageHistory::default();
    for (tick, sample) in samples.iter().enumerate() {
        let metrics = PodMetrics {
            namespace: "shop".to_owned(),
            name: "api-0".to_owned(),
            sampled_at: None,
            containers: vec![ContainerMetrics {
                name: "api".to_owned(),
                usage: *sample,
            }],
        };
        history.record(at(15 * (tick as i64 + 1)), &[metrics], &[]);
    }
    history
}

fn usage_finding(
    pod: &PodSummary,
    history: &PodUsageHistory,
    finder: fn(&PodSummary, &IssueInputs) -> Option<Finding>,
) -> Option<Finding> {
    finder(
        pod,
        &IssueInputs {
            pod_usage: Some(history),
            ..inputs()
        },
    )
}

// ---- pods ----

#[test]
fn crash_loop_is_critical_with_view_logs() {
    let pod = running_pod(vec![crash_looping()]);
    let finding = finding_of(&pod).expect("a finding");
    assert_eq!(finding.rule, IssueRule::PodCrash);
    assert_eq!(finding.severity, IssueSeverity::Critical);
    assert_eq!(finding.reason, "CrashLoopBackOff");
    assert_eq!(finding.container.as_deref(), Some("api"));
    assert_eq!(
        finding.action,
        IssueAction::ViewLogs {
            container: Some("api".to_owned())
        }
    );
    assert_eq!(
        finding.workload,
        Some(IssueObject::new("Deployment", Some("shop"), "api"))
    );
    assert_eq!(finding.object, IssueObject::pod("shop", "api-0"));
    assert_eq!(finding.grace, None);
}

#[test]
fn crash_and_exit_onset_is_ready_transition() {
    let mut crashing = running_pod(vec![crash_looping()]);
    crashing.conditions = vec![not_ready_since(900)];
    assert_eq!(finding_of(&crashing).expect("crash").onset, Some(ago(900)));
    let exited = container(
        "api",
        ContainerState::Terminated(termination(Some(StatusReason::Error), 2, 60)),
    );
    let mut failed = running_pod(vec![exited]);
    failed.conditions = vec![not_ready_since(300)];
    let finding = finding_of(&failed).expect("exit");
    assert_eq!(finding.rule, IssueRule::PodExited);
    assert_eq!(finding.severity, IssueSeverity::Warning);
    assert_eq!(finding.reason, "Error");
    assert_eq!(finding.onset, Some(ago(300)));
    // Without the transition time the board falls back to first seen.
    failed.conditions.clear();
    assert_eq!(finding_of(&failed).expect("exit").onset, None);
}

#[test]
fn image_pull_is_critical() {
    let pod = running_pod(vec![container(
        "api",
        waiting(StatusReason::ImagePullBackOff),
    )]);
    let finding = finding_of(&pod).expect("a finding");
    assert_eq!(finding.rule, IssueRule::PodImage);
    assert_eq!(finding.severity, IssueSeverity::Critical);
    assert_eq!(finding.reason, "ImagePullBackOff");
    assert_eq!(finding.action, IssueAction::Open);
    assert!(
        finding
            .cause
            .starts_with("Cannot pull image registry/app:1")
    );
}

#[test]
fn other_waiting_reasons_are_critical() {
    let pod = running_pod(vec![container(
        "api",
        waiting(StatusReason::CreateContainerConfigError),
    )]);
    let finding = finding_of(&pod).expect("a finding");
    assert_eq!(finding.rule, IssueRule::PodWaiting);
    assert_eq!(finding.reason, "CreateContainerConfigError");
}

#[test]
fn healthy_gated_and_finished_pods_have_no_finding() {
    assert_eq!(finding_of(&running_pod(vec![serving("api")])), None);
    let gated = pod_with(
        "api-0",
        PodStatus::Reason(StatusReason::SchedulingGated),
        vec![],
    );
    assert_eq!(finding_of(&gated), None);
    for status in [
        PodStatus::Terminating,
        PodStatus::Reason(StatusReason::Completed),
    ] {
        let finished = pod_with(
            "api-0",
            status,
            vec![container("api", waiting(StatusReason::CrashLoopBackOff))],
        );
        assert_eq!(finding_of(&finished), None);
    }
}

fn unschedulable_pod() -> PodSummary {
    let mut pending = pod_with("api-0", PodStatus::Reason(StatusReason::Pending), vec![]);
    pending.conditions = vec![PodCondition {
        name: "PodScheduled".to_owned(),
        is_true: false,
        reason: Some("Unschedulable".to_owned()),
        message: Some("0/4 nodes are available.".to_owned()),
        changed_at: Some(ago(30)),
    }];
    pending
}

fn startup_pod(
    period_seconds: u32,
    failure_threshold: u32,
    initial_delay_seconds: u32,
) -> PodSummary {
    let mut starting = container("api", running());
    starting.probes.startup = Some(ProbeSummary {
        action: ProbeAction::HttpGet {
            scheme: "HTTP".to_owned(),
            port: "8080".to_owned(),
            path: "/".to_owned(),
        },
        period_seconds,
        failure_threshold,
        initial_delay_seconds,
    });
    running_pod(vec![starting])
}

#[test]
fn rule_graces() {
    let node = |readiness| {
        let finding = node_finding(&node_with("node-a", readiness, vec![]), &inputs());
        finding.expect("a finding")
    };
    let cases = [
        (
            "PodUnschedulable",
            finding_of(&unschedulable_pod()),
            Some(UNSCHEDULABLE_GRACE),
        ),
        (
            "PodNotReady",
            finding_of(&running_pod(vec![container("api", running())])),
            Some(NOT_READY_GRACE),
        ),
        (
            "PodStuck",
            finding_of(&pod_with(
                "api-0",
                PodStatus::Reason(StatusReason::ContainerCreating),
                vec![],
            )),
            Some(STUCK_STARTING_AFTER),
        ),
        (
            "PodStartup from the probe",
            finding_of(&startup_pod(5, 3, 0)),
            Some(SignedDuration::from_secs(15)),
        ),
        (
            "PodStartup without a probe period",
            finding_of(&startup_pod(0, 3, 0)),
            Some(STARTUP_FALLBACK_GRACE),
        ),
        (
            "NodeNotReady Unknown",
            Some(node(NodeReadiness::Unknown)),
            Some(NODE_UNKNOWN_GRACE),
        ),
        (
            "NodeNotReady False",
            Some(node(NodeReadiness::NotReady)),
            None,
        ),
    ];
    for (name, finding, grace) in cases {
        assert_eq!(finding.expect(name).grace, grace, "{name}");
    }
}

#[test]
fn unschedulable_onset_is_the_condition_time() {
    let finding = finding_of(&unschedulable_pod()).expect("a finding");
    assert_eq!(finding.rule, IssueRule::PodUnschedulable);
    assert_eq!(finding.severity, IssueSeverity::Warning);
    assert_eq!(finding.reason, "Pending");
    assert_eq!(finding.onset, Some(ago(30)));
}

#[test]
fn stuck_creating_quotes_newest_warning_event() {
    let pod = pod_with(
        "api-0",
        PodStatus::Reason(StatusReason::ContainerCreating),
        vec![],
    );
    let events = feed(vec![
        warning(
            "Pod",
            "api-0",
            "FailedMount",
            "old mount\nproblem",
            2,
            900,
            800,
        ),
        warning(
            "Pod",
            "api-0",
            "FailedAttachVolume",
            "volume busy",
            1,
            60,
            50,
        ),
    ]);
    let finding = pod_finding(
        &pod,
        &IssueInputs {
            events: Some(&events),
            ..inputs()
        },
    )
    .expect("a finding");
    assert_eq!(finding.rule, IssueRule::PodStuck);
    assert_eq!(finding.reason, "ContainerCreating");
    assert_eq!(
        finding.cause,
        "Stuck in ContainerCreating for 1h. FailedAttachVolume: volume busy"
    );
    assert_eq!(finding.onset, pod.created_at);
}

#[test]
fn oom_restart_within_window() {
    let mut oom = serving("api");
    oom.restart_count = 1;
    oom.last_termination = Some(termination(Some(StatusReason::OomKilled), 137, 300));
    oom.resources = vec![limit("memory", "512Mi")];
    let finding = finding_of(&running_pod(vec![oom.clone()])).expect("a finding");
    assert_eq!(finding.rule, IssueRule::PodRestarts);
    assert_eq!(finding.reason, "OOMKilled");
    assert_eq!(finding.cause, "OOMKilled 5m ago; memory limit 512Mi.");
    assert_eq!(finding.onset, Some(ago(300)));
    assert_eq!(finding.container.as_deref(), Some("api"));
    assert!(matches!(finding.action, IssueAction::ViewLogs { .. }));
    oom.resources.clear();
    let finding = finding_of(&running_pod(vec![oom])).expect("a finding");
    assert_eq!(finding.cause, "OOMKilled 5m ago. No memory limit is set.");
}

#[test]
fn old_restart_is_quiet() {
    let mut old = serving("api");
    old.restart_count = 12;
    // A lifetime total of restarts says nothing about the last hour.
    old.last_termination = Some(termination(Some(StatusReason::OomKilled), 137, 7_200));
    assert_eq!(finding_of(&running_pod(vec![old.clone()])), None);
    // Few restarts, and not an OOM kill, stay quiet even when recent.
    old.restart_count = 2;
    old.last_termination = Some(termination(Some(StatusReason::Error), 1, 60));
    assert_eq!(finding_of(&running_pod(vec![old])), None);
}

#[test]
fn restart_text_says_in_total() {
    let mut restarting = serving("api");
    restarting.restart_count = 5;
    restarting.last_termination = Some(termination(Some(StatusReason::Error), 1, 600));
    let finding = finding_of(&running_pod(vec![restarting.clone()])).expect("a finding");
    assert_eq!(finding.reason, "Restarting");
    assert_eq!(
        finding.cause,
        "Restarted 5 times in total; last exit Error 10m ago."
    );
    restarting.last_termination = Some(termination(None, 3, 600));
    let finding = finding_of(&running_pod(vec![restarting])).expect("a finding");
    assert_eq!(
        finding.cause,
        "Restarted 5 times in total; last exit code 3 10m ago."
    );
}

fn job_pod_that_exited() -> PodSummary {
    let exited = container(
        "worker",
        ContainerState::Terminated(termination(Some(StatusReason::Error), 1, 60)),
    );
    let mut pod = running_pod(vec![exited]);
    pod.controller = Some(ControllerRef {
        kind: "Job".to_owned(),
        name: "import".to_owned(),
    });
    pod
}

#[test]
fn job_pod_exit_left_to_live_job_feed() {
    let finding = pod_finding(
        &job_pod_that_exited(),
        &IssueInputs {
            is_job_feed_live: true,
            ..inputs()
        },
    );
    assert_eq!(finding, None);
}

#[test]
fn job_pod_exit_reported_without_job_feed() {
    let finding = finding_of(&job_pod_that_exited()).expect("a finding");
    assert_eq!(finding.rule, IssueRule::PodExited);
    assert_eq!(
        finding.workload,
        Some(IssueObject::new("Job", Some("shop"), "import"))
    );
}

#[test]
fn old_eviction_is_quiet() {
    let evicted = |seconds_ago: i64| {
        let mut pod = pod_with("api-0", PodStatus::Reason(StatusReason::Evicted), vec![]);
        pod.status_message = Some("The node was low on resource: memory.".to_owned());
        pod.conditions = vec![not_ready_since(seconds_ago)];
        pod
    };
    let recent = finding_of(&evicted(3_600)).expect("a recent eviction");
    assert_eq!(recent.rule, IssueRule::PodFailed);
    assert_eq!(recent.reason, "Evicted");
    assert_eq!(recent.severity, IssueSeverity::Warning);
    assert_eq!(finding_of(&evicted(3 * 86_400)), None);
}

#[test]
fn memory_near_limit_needs_two_samples() {
    let mut api = serving("api");
    api.resources = vec![limit("memory", "512Mi")];
    let pod = running_pod(vec![api]);
    let near = usage(100, 480);
    let finding =
        |samples: &[ResourceUsage]| usage_finding(&pod, &pod_history(samples), memory_finding);
    assert_eq!(finding(&[near]), None, "one sample could be a spike");
    assert_eq!(finding(&[usage(100, 100), near]), None);
    let found = finding(&[near, near]).expect("two samples in a row");
    assert_eq!(found.rule, IssueRule::PodMemory);
    assert_eq!(found.reason, "Near memory limit");
    assert_eq!(
        found.cause,
        "Container api uses 480Mi of its 512Mi memory limit."
    );
    assert_eq!(found.container.as_deref(), Some("api"));
}

#[test]
fn cpu_cause_does_not_claim_throttling() {
    let mut api = serving("api");
    api.resources = vec![limit("cpu", "500m")];
    let pod = running_pod(vec![api]);
    let history = pod_history(&[usage(480, 100)]);
    let found = usage_finding(&pod, &history, cpu_finding).expect("a finding");
    assert_eq!(found.rule, IssueRule::PodCpu);
    assert_eq!(found.reason, "At CPU limit");
    assert_eq!(
        found.cause,
        "Container api uses 480m of its 500m CPU limit."
    );
    assert!(!found.cause.to_lowercase().contains("throttl"));
    let calm = pod_history(&[usage(100, 100)]);
    assert_eq!(usage_finding(&pod, &calm, cpu_finding), None);
}

#[test]
fn no_limit_no_usage_finding() {
    let pod = running_pod(vec![serving("api")]);
    let history = pod_history(&[usage(5_000, 9_000), usage(5_000, 9_000)]);
    assert_eq!(usage_finding(&pod, &history, memory_finding), None);
    assert_eq!(usage_finding(&pod, &history, cpu_finding), None);
}

// ---- nodes ----

#[test]
fn node_not_ready_reads_the_ready_condition() {
    let ready = condition(
        "Ready",
        ConditionStatus::False,
        Some("kubelet stopped posting status"),
        Some(600),
    );
    let node = node_with("node-a", NodeReadiness::NotReady, vec![ready]);
    let finding = node_finding(&node, &inputs()).expect("a finding");
    assert_eq!(finding.rule, IssueRule::NodeNotReady);
    assert_eq!(finding.severity, IssueSeverity::Critical);
    assert_eq!(finding.reason, "NotReady");
    assert_eq!(
        finding.cause,
        "Ready is False for 10m. kubelet stopped posting status"
    );
    assert_eq!(finding.onset, Some(ago(600)));
    assert_eq!(finding.object, IssueObject::node("node-a"));
    let healthy = node_with("node-b", NodeReadiness::Ready, vec![]);
    assert_eq!(node_finding(&healthy, &inputs()), None);
}

#[test]
fn network_unavailable_is_critical() {
    let node = node_with(
        "node-a",
        NodeReadiness::Ready,
        vec![condition(
            "NetworkUnavailable",
            ConditionStatus::True,
            None,
            None,
        )],
    );
    let finding = node_finding(&node, &inputs()).expect("a finding");
    assert_eq!(finding.rule, IssueRule::NodeNetwork);
    assert_eq!(finding.severity, IssueSeverity::Critical);
    assert_eq!(finding.cause, "The node network is not configured.");
}

#[test]
fn node_pressure_lists_other_pressures() {
    let node = node_with(
        "node-a",
        NodeReadiness::Ready,
        vec![
            condition(
                "MemoryPressure",
                ConditionStatus::True,
                Some("low memory"),
                Some(60),
            ),
            condition("DiskPressure", ConditionStatus::True, None, None),
            condition("PIDPressure", ConditionStatus::False, None, None),
        ],
    );
    let finding = node_finding(&node, &inputs()).expect("a finding");
    assert_eq!(finding.rule, IssueRule::NodePressure);
    assert_eq!(finding.severity, IssueSeverity::Warning);
    assert_eq!(finding.reason, "MemoryPressure");
    assert_eq!(finding.cause, "low memory. Also DiskPressure.");
}

#[test]
fn problem_detector_conditions_are_warnings() {
    let mut broken = condition(
        "KernelDeadlock",
        ConditionStatus::True,
        Some("task blocked"),
        None,
    );
    broken.reason = Some("DockerHung".to_owned());
    let calm = condition("FrequentRestart", ConditionStatus::False, None, None);
    let node = node_with("node-a", NodeReadiness::Ready, vec![calm, broken]);
    let finding = node_finding(&node, &inputs()).expect("a finding");
    assert_eq!(finding.rule, IssueRule::NodeCondition);
    assert_eq!(finding.reason, "KernelDeadlock");
    assert_eq!(finding.cause, "DockerHung: task blocked");
}

#[test]
fn cordoned_node_is_not_an_issue() {
    let mut node = node_with("node-a", NodeReadiness::Ready, vec![]);
    node.status.scheduling = NodeScheduling::Disabled;
    assert_eq!(node_finding(&node, &inputs()), None);
}

#[test]
fn node_usage_at_the_limit_is_a_warning() {
    let node = node_with("node-a", NodeReadiness::Ready, vec![]);
    let mut history = NodeUsageHistory::default();
    let sample = NodeMetrics {
        name: "node-a".to_owned(),
        sampled_at: None,
        usage: ResourceUsage {
            cpu: CpuAmount::from_nanocores(1_000_000_000),
            memory: ByteAmount::from_bytes(15 << 29),
        },
    };
    history.record(at(15), &[sample]);
    let finding = node_finding(
        &node,
        &IssueInputs {
            node_usage: Some(&history),
            ..inputs()
        },
    )
    .expect("a finding");
    assert_eq!(finding.rule, IssueRule::NodeMemory);
    assert_eq!(finding.reason, "High memory");
    assert_eq!(finding.cause, "Uses 94% of allocatable memory.");
}

#[test]
fn usage_threshold_is_the_red_bar() {
    assert_eq!(usage_tone(USAGE_WARN_RATIO), Some(StatusTone::Bad));
}

// ---- volume usage ----

fn kubelet_with_claim(used: u64, available: Option<u64>) -> KubeletHistory {
    let usage = PvcUsage {
        namespace: "shop".to_owned(),
        claim: "data".to_owned(),
        sampled_at: Some(at(30)),
        used: Some(ByteAmount::from_bytes(used << 20)),
        capacity: Some(ByteAmount::from_bytes(100 << 20)),
        available: available.map(|bytes| ByteAmount::from_bytes(bytes << 20)),
        inodes_used: None,
        inodes: None,
    };
    let round = vec![NodeKubeletStats {
        node: "node-a".to_owned(),
        summary: Ok(KubeletSummary {
            network: None,
            pods: vec![PodKubeletStats {
                namespace: "shop".to_owned(),
                name: "db-0".to_owned(),
                uid: "uid".to_owned(),
                network: None,
                volumes: vec![usage],
            }],
        }),
        disk_io: None,
    }];
    let mut history = KubeletHistory::default();
    history.record(at(30), &round, &[], &NamespaceScope::All);
    history
}

#[test]
fn volume_almost_full_from_kubelet() {
    let full = kubelet_with_claim(95, None);
    let findings = volume_findings(&IssueInputs {
        kubelet: Some(&full),
        ..inputs()
    });
    let [finding] = findings.as_slice() else {
        panic!("one finding, got {findings:?}");
    };
    assert_eq!(finding.rule, IssueRule::VolumeFull);
    assert_eq!(finding.severity, IssueSeverity::Warning);
    assert_eq!(finding.reason, "Volume almost full");
    assert_eq!(finding.cause, "95Mi of 100Mi used (95%).");
    assert_eq!(
        finding.object,
        IssueObject::new("PersistentVolumeClaim", Some("shop"), "data")
    );
    let roomy = kubelet_with_claim(50, None);
    let none = volume_findings(&IssueInputs {
        kubelet: Some(&roomy),
        ..inputs()
    });
    assert!(none.is_empty());
}

// ---- events ----

#[test]
fn failed_create_on_replica_set_names_deployment() {
    let findings = events_findings(vec![warning(
        "ReplicaSet",
        "api-7d9f8c",
        "FailedCreate",
        "pods \"api-7d9f8c-\" is forbidden: exceeded quota",
        3,
        120,
        30,
    )]);
    let [finding] = findings.as_slice() else {
        panic!("one finding, got {findings:?}");
    };
    assert_eq!(finding.rule, IssueRule::EventFailedCreate);
    assert_eq!(finding.severity, IssueSeverity::Critical);
    assert_eq!(finding.reason, "FailedCreate");
    assert_eq!(
        finding.object,
        IssueObject::new("Deployment", Some("shop"), "api")
    );
    assert_eq!(finding.onset, Some(ago(120)));
    assert!(finding.cause.contains("exceeded quota"));
}

#[test]
fn job_failure_events_are_warnings() {
    let findings = events_findings(vec![warning(
        "Job",
        "import",
        "BackoffLimitExceeded",
        "Job has reached the specified backoff limit",
        1,
        60,
        60,
    )]);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].rule, IssueRule::EventJobFailed);
    assert_eq!(findings[0].reason, "BackoffLimitExceeded");
}

#[test]
fn event_burst_needs_count_and_window() {
    let burst = |count, last_ago| {
        events_findings(vec![warning(
            "Pod",
            "api-0",
            "BackOff",
            "Back-off restarting failed container",
            count,
            600,
            last_ago,
        )])
    };
    assert!(burst(WARNING_BURST - 1, 30).is_empty());
    let found = burst(WARNING_BURST, 30);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].rule, IssueRule::EventBurst);
    assert_eq!(found[0].severity, IssueSeverity::Warning);
    assert_eq!(
        found[0].cause,
        "Back-off restarting failed container (×10 in 10m)."
    );
    // The last event is older than the window.
    assert!(burst(WARNING_BURST, 1_800).is_empty());
}

#[test]
fn event_burst_needs_recent_first_seen() {
    // Fifty events over two days are a slow series, not a burst.
    let findings = events_findings(vec![warning(
        "Pod",
        "api-0",
        "BackOff",
        "restarting",
        50,
        2 * 86_400,
        30,
    )]);
    assert!(findings.is_empty());
}

#[test]
fn failed_scheduling_is_never_an_event_issue() {
    let findings = events_findings(vec![warning(
        "Pod",
        "api-0",
        "FailedScheduling",
        "0/4 nodes are available",
        99,
        600,
        30,
    )]);
    assert!(findings.is_empty());
}

#[test]
fn one_event_finding_per_object_with_the_highest_count() {
    let findings = events_findings(vec![
        warning("Pod", "api-0", "BackOff", "first", 12, 600, 30),
        warning("Pod", "api-0", "Unhealthy", "second", 40, 600, 40),
    ]);
    let [finding] = findings.as_slice() else {
        panic!("one finding, got {findings:?}");
    };
    assert_eq!(finding.reason, "Unhealthy");
}

#[test]
fn event_cause_is_cut_and_single_line() {
    let message = format!("{}\nsecond line", "x".repeat(300));
    let findings = events_findings(vec![warning(
        "ReplicaSet",
        "api-7d9f8c",
        "FailedCreate",
        &message,
        1,
        60,
        60,
    )]);
    let cause = &findings[0].cause;
    assert_eq!(cause.chars().count(), CAUSE_MESSAGE_CHARS);
    assert!(cause.ends_with('…'));
    assert!(!cause.contains('\n'));
}

#[test]
fn startup_grace_adds_the_initial_delay() {
    let grace = |pod: &PodSummary| finding_of(pod).expect("a finding").grace;
    assert_eq!(
        grace(&startup_pod(5, 3, 30)),
        Some(SignedDuration::from_secs(45))
    );
    // A probe with no period still gets the fallback, whatever its delay.
    assert_eq!(grace(&startup_pod(0, 3, 30)), Some(STARTUP_FALLBACK_GRACE));
}

#[test]
fn volume_fullness_reads_what_is_left() {
    // 10 Mi counted as used, but only 4 Mi of the 100 Mi are left (reserved blocks).
    let history = kubelet_with_claim(10, Some(4));
    let findings = volume_findings(&IssueInputs {
        kubelet: Some(&history),
        ..inputs()
    });
    let [finding] = findings.as_slice() else {
        panic!("one finding, got {findings:?}");
    };
    assert_eq!(finding.cause, "96Mi of 100Mi used (96%).");
}

#[test]
fn critical_failed_create_is_not_hidden_by_a_warning_burst() {
    let mut burst = warning(
        "ReplicaSet",
        "api-7d9f8c",
        "BackOff",
        "restarting",
        50,
        600,
        30,
    );
    burst.reason = "Unhealthy".to_owned();
    let create = warning(
        "ReplicaSet",
        "api-7d9f8c",
        "FailedCreate",
        "exceeded quota",
        2,
        600,
        30,
    );
    for events in [vec![burst.clone(), create.clone()], vec![create, burst]] {
        let findings = events_findings(events);
        let [finding] = findings.as_slice() else {
            panic!("one finding, got {findings:?}");
        };
        assert_eq!(finding.rule, IssueRule::EventFailedCreate);
        assert_eq!(finding.severity, IssueSeverity::Critical);
    }
}

#[test]
fn job_failure_event_is_not_hidden_by_a_burst() {
    let failed = warning("Job", "import", "BackoffLimitExceeded", "limit", 1, 600, 30);
    let burst = warning("Job", "import", "Unhealthy", "probe", 40, 600, 30);
    let findings = events_findings(vec![burst, failed]);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].rule, IssueRule::EventJobFailed);
}
