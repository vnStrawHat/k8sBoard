use cluster::{
    ContainerKind, ContainerMetrics, ContainerSummary, ControllerRef, PodStatus, ReadyCount,
    ResourceUsage,
};

use super::*;
use crate::history_rings::{COARSE_POINTS, FINE_TICKS};

fn at(seconds: i64) -> jiff::Timestamp {
    jiff::Timestamp::from_second(seconds).expect("valid timestamp")
}

fn usage(millicores: u64, mebibytes: u64) -> ResourceUsage {
    ResourceUsage {
        cpu: CpuAmount::from_nanocores(millicores * 1_000_000),
        memory: ByteAmount::from_bytes(mebibytes << 20),
    }
}

fn pod(namespace: &str, name: &str, containers: &[(&str, ResourceUsage)]) -> PodMetrics {
    PodMetrics {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        sampled_at: None,
        containers: containers
            .iter()
            .map(|(name, usage)| ContainerMetrics {
                name: (*name).to_owned(),
                usage: *usage,
            })
            .collect(),
    }
}

fn node(name: &str, usage: ResourceUsage) -> NodeMetrics {
    NodeMetrics {
        name: name.to_owned(),
        sampled_at: None,
        usage,
    }
}

fn rings_of<'a>(
    history: &'a PodUsageHistory,
    namespace: &str,
    pod: &str,
    container: &str,
) -> &'a Rings<UsagePoint> {
    &history.pods[namespace][pod].containers[container]
}

/// A pod summary for the controller and the OOM kills; the rest is blank.
fn summary(namespace: &str, name: &str, controller: Option<(&str, &str)>) -> PodSummary {
    PodSummary {
        annotations: cluster::AnnotationTerms::default(),
        is_finished: false,
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
        controller: controller.map(|(kind, name)| ControllerRef {
            kind: kind.to_owned(),
            name: name.to_owned(),
        }),
        conditions: Vec::new(),
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        image_pull_secrets: Vec::new(),
        node_selector: Vec::new(),
        node_affinity: Vec::new(),
        containers: Vec::new(),
    }
}

fn oom_container(name: &str, finished_at: jiff::Timestamp) -> ContainerSummary {
    ContainerSummary {
        terminal: cluster::ContainerTerminal::None,
        name: name.to_owned(),
        image: "img".to_owned(),
        kind: ContainerKind::Main,
        state: ContainerState::Running { started_at: None },
        is_ready: true,
        restart_count: 1,
        last_termination: Some(Termination {
            reason: Some(StatusReason::OomKilled),
            exit_code: 137,
            signal: None,
            started_at: None,
            finished_at: Some(finished_at),
        }),
        image_digest: None,
        pull_policy: None,
        is_started: None,
        ports: Vec::new(),
        resources: Vec::new(),
        probes: cluster::ContainerProbes::default(),
        env: Vec::new(),
        env_from: Vec::new(),
        mounts: Vec::new(),
    }
}

#[test]
fn rings_never_exceed_fine_capacity() {
    let mut history = PodUsageHistory::default();
    for tick in 0..300 {
        let sample = [
            pod(
                "a",
                "web",
                &[("app", usage(10, 100)), ("side", usage(1, 10))],
            ),
            pod("b", "db", &[("main", usage(50, 500))]),
        ];
        history.record(at(tick * 15), &sample, &[]);
    }
    assert_eq!(history.tick_count(), 300);
    let lengths: Vec<usize> = history
        .pods
        .values()
        .flat_map(BTreeMap::values)
        .flat_map(|pod| pod.containers.values())
        .map(Rings::len)
        .collect();
    assert_eq!(lengths, [FINE_TICKS; 3]);
}

#[test]
fn rings_never_exceed_coarse_capacity() {
    let mut history = PodUsageHistory::default();
    for tick in 0..6_000 {
        history.record(
            at(tick * 15),
            &[pod("a", "web", &[("app", usage(10, 100))])],
            &[],
        );
    }
    let rings = rings_of(&history, "a", "web", "app");
    assert_eq!(rings.coarse_len(), COARSE_POINTS);
    assert_eq!(history.timelines.coarse_len(), COARSE_POINTS);
    assert_eq!(rings.len(), FINE_TICKS);
}

#[test]
fn coarse_ring_averages_twenty_ticks() {
    let mut history = PodUsageHistory::default();
    for tick in 1..=20_i64 {
        // The pod reports on even ticks only: the mean covers the values it has.
        let sample = if tick % 2 == 0 {
            vec![pod("a", "web", &[("app", usage(tick as u64 * 10, 100))])]
        } else {
            Vec::new()
        };
        history.record(at(tick * 15), &sample, &[]);
    }
    let series = history.pod_series("a", "web", None, Resolution::Coarse);
    // One coarse point: the mean of 20, 40, ..., 200 millicores. The fine tail is empty.
    assert_eq!(series.points.len(), 1);
    assert_eq!(series.points[0].1, Some(usage(110, 100)));
    // A window with no value folds to None.
    for tick in 21..=40_i64 {
        history.record(at(tick * 15), &[], &[]);
    }
    let series = history.pod_series("a", "web", None, Resolution::Coarse);
    assert_eq!(series.points.len(), 2);
    assert_eq!(series.points[1].1, None);
}

#[test]
fn new_pod_aligns_to_the_newest_tick() {
    let mut history = PodUsageHistory::default();
    for tick in 1..=4 {
        history.record(
            at(tick * 15),
            &[pod("a", "old", &[("c", usage(1, 1))])],
            &[],
        );
    }
    let sample = [
        pod("a", "old", &[("c", usage(1, 1))]),
        pod("a", "new", &[("c", usage(2, 2))]),
    ];
    history.record(at(75), &sample, &[]);
    // The ring holds only the tick it was first seen at, so it aligns to the timeline's end.
    assert_eq!(rings_of(&history, "a", "new", "c").len(), 1);
    assert_eq!(rings_of(&history, "a", "old", "c").len(), 5);
    assert_eq!(history.latest("a", "new"), Some(usage(2, 2)));
    let series = history.pod_series("a", "new", None, Resolution::Fine);
    let values: Vec<_> = series.points.iter().map(|(_, value)| *value).collect();
    assert_eq!(values, [None, None, None, None, Some(usage(2, 2))]);
}

#[test]
fn absent_pod_reads_none_at_the_newest_tick() {
    let mut history = PodUsageHistory::default();
    history.record(at(15), &[pod("a", "web", &[("c", usage(5, 5))])], &[]);
    assert_eq!(history.latest("a", "web"), Some(usage(5, 5)));
    history.record(at(30), &[], &[]);
    assert_eq!(history.latest("a", "web"), None);
    assert_eq!(history.latest_container("a", "web", "c"), None);
    // The older point is kept.
    assert_eq!(rings_of(&history, "a", "web", "c").len(), 2);
}

#[test]
fn latest_sums_containers_and_reads_one() {
    let mut history = PodUsageHistory::default();
    let sample = [pod(
        "a",
        "web",
        &[("app", usage(10, 100)), ("side", usage(5, 20))],
    )];
    history.record(at(15), &sample, &[]);
    assert_eq!(history.latest("a", "web"), Some(usage(15, 120)));
    assert_eq!(
        history.latest_container("a", "web", "side"),
        Some(usage(5, 20))
    );
    assert_eq!(history.latest("a", "other"), None);
    assert_eq!(history.latest_container("a", "web", "none"), None);
}

#[test]
fn latest_container_pair_needs_both_ticks() {
    let mut history = PodUsageHistory::default();
    let sample = |millicores| [pod("a", "web", &[("c", usage(millicores, 10))])];
    history.record(at(15), &sample(1), &[]);
    assert_eq!(history.latest_container_pair("a", "web", "c"), None);
    history.record(at(30), &sample(2), &[]);
    assert_eq!(
        history.latest_container_pair("a", "web", "c"),
        Some([usage(1, 10), usage(2, 10)])
    );
    // A tick without the container breaks the pair.
    history.record(at(45), &[], &[]);
    assert_eq!(history.latest_container_pair("a", "web", "c"), None);
    assert_eq!(history.latest_container_pair("a", "web", "none"), None);
}

#[test]
fn fine_ring_is_freed_after_an_hour_and_pod_removed_after_a_day() {
    let mut history = PodUsageHistory::default();
    history.record(at(15), &[pod("a", "gone", &[("c", usage(1, 1))])], &[]);
    for tick in 2..=FINE_TICKS as i64 {
        history.record(at(tick * 15), &[], &[]);
    }
    assert_eq!(rings_of(&history, "a", "gone", "c").len(), FINE_TICKS);
    history.record(at(15 * (FINE_TICKS as i64 + 1)), &[], &[]);
    assert_eq!(rings_of(&history, "a", "gone", "c").len(), 0);
    for tick in (FINE_TICKS as i64 + 2)..=(FINE_TICKS as i64 * 24) {
        history.record(at(tick * 15), &[], &[]);
    }
    assert!(history.pods.contains_key("a"));
    history.record(at(15 * (FINE_TICKS as i64 * 24 + 1)), &[], &[]);
    assert!(history.pods.is_empty());
}

#[test]
fn retain_scope_drops_other_namespaces() {
    let mut history = PodUsageHistory::default();
    let sample = [
        pod("a", "one", &[("c", usage(1, 1))]),
        pod("b", "two", &[("c", usage(1, 1))]),
        pod("c", "three", &[("c", usage(1, 1))]),
    ];
    history.record(at(15), &sample, &[]);
    history.retain_scope(&NamespaceScope::All);
    assert_eq!(history.pods.len(), 3);
    history.retain_scope(&NamespaceScope::of_namespaces([
        "a".to_owned(),
        "c".to_owned(),
    ]));
    assert_eq!(history.pods.keys().collect::<Vec<_>>(), ["a", "c"]);
    history.retain_scope(&NamespaceScope::Named("c".to_owned()));
    assert_eq!(history.pods.keys().collect::<Vec<_>>(), ["c"]);
    assert_eq!(history.tick_count(), 1);
}

#[test]
fn usage_point_saturates() {
    let huge = ResourceUsage {
        cpu: CpuAmount::from_nanocores(5_000 * 1_000_000_000),
        memory: ByteAmount::from_bytes(6 << 40),
    };
    let point = UsagePoint::of(huge);
    assert_eq!(point.cpu_microcores, u32::MAX);
    assert_eq!(point.memory_kib, u32::MAX);
    assert_eq!(UsagePoint::of(usage(310, 498)).usage(), usage(310, 498));
}

#[test]
fn node_rings_keep_u64_usage() {
    let big = ResourceUsage {
        cpu: CpuAmount::from_nanocores(64_000_000_000),
        memory: ByteAmount::from_bytes((6 << 40) + 1),
    };
    let mut history = NodeUsageHistory::default();
    history.record(at(15), &[node("wk-1", big)]);
    assert_eq!(history.latest("wk-1"), Some(big));
    history.record(at(30), &[]);
    assert_eq!(history.latest("wk-1"), None);
    assert_eq!(history.tick_count(), 2);
}

#[test]
fn node_rings_never_exceed_fine_capacity() {
    let mut history = NodeUsageHistory::default();
    for tick in 0..300 {
        history.record(at(tick * 15), &[node("wk-1", usage(1, 1))]);
    }
    assert_eq!(history.nodes["wk-1"].rings.len(), FINE_TICKS);
}

#[test]
fn node_series_reads_the_node_rings_and_the_server_timestamp() {
    let mut history = NodeUsageHistory::default();
    let mut sample = node("wk-1", usage(100, 200));
    sample.sampled_at = Some(at(14));
    history.record(at(15), &[sample]);
    history.record(at(30), &[]);
    let series = history.node_series("wk-1", Resolution::Fine);
    let values: Vec<_> = series.points.iter().map(|(_, value)| *value).collect();
    assert_eq!(values, [Some(usage(100, 200)), None]);
    assert_eq!(series.sampled_at, Some(at(14)));
    assert_eq!(series.step, Duration::from_secs(15));
    assert!(series.oom.is_empty());
    let unknown = history.node_series("nope", Resolution::Fine);
    assert!(unknown.points.iter().all(|(_, value)| value.is_none()));
    assert_eq!(unknown.points.len(), 2);
}

#[test]
fn pod_total_sums_containers() {
    let mut history = PodUsageHistory::default();
    history.record(
        at(15),
        &[pod(
            "a",
            "web",
            &[("app", usage(10, 100)), ("side", usage(5, 20))],
        )],
        &[],
    );
    // The sidecar misses the second tick; the pod total still has the app's value.
    history.record(at(30), &[pod("a", "web", &[("app", usage(12, 100))])], &[]);
    history.record(at(45), &[], &[]);
    let total = history.pod_series("a", "web", None, Resolution::Fine);
    let values: Vec<_> = total.points.iter().map(|(_, value)| *value).collect();
    // `None` only when every container is `None`.
    assert_eq!(values, [Some(usage(15, 120)), Some(usage(12, 100)), None]);
    assert_eq!(total.pod_count, 1);
    let side = history.pod_series("a", "web", Some("side"), Resolution::Fine);
    let values: Vec<_> = side.points.iter().map(|(_, value)| *value).collect();
    assert_eq!(values, [Some(usage(5, 20)), None, None]);
    let missing = history.pod_series("a", "web", Some("none"), Resolution::Fine);
    assert_eq!(missing.pod_count, 0);
    let unknown = history.pod_series("a", "nope", None, Resolution::Fine);
    assert_eq!(unknown.points.len(), 3);
    assert!(unknown.points.iter().all(|(_, value)| value.is_none()));
}

fn deployment_owner() -> PodOwner {
    PodOwner::Deployment {
        namespace: "ns".to_owned(),
        name: "api".to_owned(),
    }
}

#[test]
fn owner_series_keeps_pods_replaced_by_a_rollout() {
    let mut history = PodUsageHistory::default();
    let old = summary("ns", "api-7d9f8c-aaaaa", Some(("ReplicaSet", "api-7d9f8c")));
    history.record(
        at(15),
        &[pod("ns", "api-7d9f8c-aaaaa", &[("c", usage(10, 100))])],
        std::slice::from_ref(&old),
    );
    // The rollout replaces the pod; the pods list no longer holds the old one.
    let new = summary("ns", "api-8c7d6b-bbbbb", Some(("ReplicaSet", "api-8c7d6b")));
    history.record(
        at(30),
        &[pod("ns", "api-8c7d6b-bbbbb", &[("c", usage(20, 200))])],
        std::slice::from_ref(&new),
    );
    let series = history.owner_series(&deployment_owner(), None, Resolution::Fine);
    let values: Vec<_> = series.points.iter().map(|(_, value)| *value).collect();
    assert_eq!(values, [Some(usage(10, 100)), Some(usage(20, 200))]);
    assert_eq!(series.pod_count, 2);
    let one = history.owner_series(
        &deployment_owner(),
        Some("api-8c7d6b-bbbbb"),
        Resolution::Fine,
    );
    assert_eq!(one.pod_count, 1);
    // Another workload's pods do not count.
    let other = PodOwner::Deployment {
        namespace: "ns".to_owned(),
        name: "db".to_owned(),
    };
    assert_eq!(
        history
            .owner_series(&other, None, Resolution::Fine)
            .pod_count,
        0
    );
}

#[test]
fn owner_series_counts_a_pod_whose_snapshot_arrived_late() {
    let mut history = PodUsageHistory::default();
    let sample = [pod("ns", "api-7d9f8c-aaaaa", &[("c", usage(10, 100))])];
    // The metrics tick arrives before the pods snapshot: no controller is known yet.
    history.record(at(15), &sample, &[]);
    let early = history.owner_series(&deployment_owner(), None, Resolution::Fine);
    assert_eq!(early.pod_count, 0);
    // Then the pods arrive, and the controller fills in.
    let listed = summary("ns", "api-7d9f8c-aaaaa", Some(("ReplicaSet", "api-7d9f8c")));
    history.record(at(30), &sample, std::slice::from_ref(&listed));
    let late = history.owner_series(&deployment_owner(), None, Resolution::Fine);
    assert_eq!(late.pod_count, 1);
    // The first tick's value counts too: history is read by pod, not by tick.
    assert_eq!(late.points[0].1, Some(usage(10, 100)));
}

#[test]
fn coarse_series_appends_the_fine_tail() {
    let mut history = PodUsageHistory::default();
    for tick in 1..=45_i64 {
        history.record(
            at(tick * 15),
            &[pod("a", "web", &[("c", usage(tick as u64, 1))])],
            &[],
        );
    }
    let coarse = history.pod_series("a", "web", None, Resolution::Coarse);
    // Coarse points at ticks 20 and 40, then the fine ticks 41..=45.
    assert_eq!(coarse.points.len(), 2 + 5);
    assert_eq!(coarse.step, Duration::from_secs(300));
    assert_eq!(coarse.points[0].0, at(300));
    assert_eq!(coarse.points[2].0, at(41 * 15));
    assert_eq!(coarse.points[6].1, Some(usage(45, 1)));
    let fine = history.pod_series("a", "web", None, Resolution::Fine);
    assert_eq!(fine.points.len(), 45);
    assert_eq!(fine.step, Duration::from_secs(15));
    assert_eq!(history.span(), Some(Duration::from_secs(675 - 15)));
    assert_eq!(history.newest_tick(), Some(at(675)));
}

#[test]
fn oom_marks_are_recorded_once() {
    let mut history = PodUsageHistory::default();
    let killed_at = at(100);
    let mut listed = summary("a", "web", None);
    listed.containers = vec![oom_container("app", killed_at)];
    let sample = [pod("a", "web", &[("app", usage(1, 1))])];
    for tick in 1..=3 {
        history.record(at(100 + tick * 15), &sample, std::slice::from_ref(&listed));
    }
    let series = history.pod_series("a", "web", None, Resolution::Fine);
    assert_eq!(series.oom, [killed_at]);
    let container = history.pod_series("a", "web", Some("app"), Resolution::Fine);
    assert_eq!(container.oom, [killed_at]);
    let other = history.pod_series("a", "web", Some("side"), Resolution::Fine);
    assert!(other.oom.is_empty());
    // A current terminated state with the OOM reason counts like the last termination.
    let mut dead = summary("a", "web", None);
    let mut container = oom_container("app", at(120));
    container.state = ContainerState::Terminated(Termination {
        reason: Some(StatusReason::OomKilled),
        exit_code: 137,
        signal: None,
        started_at: None,
        finished_at: Some(at(130)),
    });
    container.last_termination = None;
    dead.containers = vec![container];
    history.record(at(200), &sample, std::slice::from_ref(&dead));
    let series = history.pod_series("a", "web", None, Resolution::Fine);
    assert_eq!(series.oom, [killed_at, at(130)]);
}

#[test]
fn oom_marks_age_out_after_a_day_and_cap_at_32() {
    let mut history = PodUsageHistory::default();
    let sample = [pod("a", "web", &[("app", usage(1, 1))])];
    let now = at(10 * 86_400);
    // A kill older than a day is not recorded.
    let mut stale = summary("a", "web", None);
    stale.containers = vec![oom_container("app", at(10 * 86_400 - 86_401))];
    history.record(now, &sample, std::slice::from_ref(&stale));
    assert!(
        history
            .pod_series("a", "web", None, Resolution::Fine)
            .oom
            .is_empty()
    );
    // Forty distinct kills keep the newest 32.
    for kill in 0..40 {
        let mut listed = summary("a", "web", None);
        listed.containers = vec![oom_container("app", at(10 * 86_400 - 3_600 + kill))];
        history.record(now, &sample, std::slice::from_ref(&listed));
    }
    let marks = history.pod_series("a", "web", None, Resolution::Fine).oom;
    assert_eq!(marks.len(), 32);
    assert_eq!(marks[0], at(10 * 86_400 - 3_600 + 8));
    // A day later every mark has aged out.
    history.record(at(11 * 86_400 + 1), &sample, &[]);
    assert!(
        history
            .pod_series("a", "web", None, Resolution::Fine)
            .oom
            .is_empty()
    );
}
