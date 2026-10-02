use cluster::{ContainerMetrics, ResourceUsage};

use super::*;
use crate::history_rings::FINE_TICKS;

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
        history.record(at(tick * 15), &sample);
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
fn new_pod_aligns_to_the_newest_tick() {
    let mut history = PodUsageHistory::default();
    for tick in 1..=4 {
        history.record(at(tick * 15), &[pod("a", "old", &[("c", usage(1, 1))])]);
    }
    let sample = [
        pod("a", "old", &[("c", usage(1, 1))]),
        pod("a", "new", &[("c", usage(2, 2))]),
    ];
    history.record(at(75), &sample);
    // The ring holds only the tick it was first seen at, so it aligns to the timeline's end.
    assert_eq!(rings_of(&history, "a", "new", "c").len(), 1);
    assert_eq!(rings_of(&history, "a", "old", "c").len(), 5);
    assert_eq!(history.latest("a", "new"), Some(usage(2, 2)));
}

#[test]
fn absent_pod_reads_none_at_the_newest_tick() {
    let mut history = PodUsageHistory::default();
    history.record(at(15), &[pod("a", "web", &[("c", usage(5, 5))])]);
    assert_eq!(history.latest("a", "web"), Some(usage(5, 5)));
    history.record(at(30), &[]);
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
    history.record(at(15), &sample);
    assert_eq!(history.latest("a", "web"), Some(usage(15, 120)));
    assert_eq!(
        history.latest_container("a", "web", "side"),
        Some(usage(5, 20))
    );
    assert_eq!(history.latest("a", "other"), None);
    assert_eq!(history.latest_container("a", "web", "none"), None);
}

#[test]
fn fine_ring_is_freed_after_an_hour_and_pod_removed_after_a_day() {
    let mut history = PodUsageHistory::default();
    history.record(at(15), &[pod("a", "gone", &[("c", usage(1, 1))])]);
    for tick in 2..=FINE_TICKS as i64 {
        history.record(at(tick * 15), &[]);
    }
    assert_eq!(rings_of(&history, "a", "gone", "c").len(), FINE_TICKS);
    history.record(at(15 * (FINE_TICKS as i64 + 1)), &[]);
    assert_eq!(rings_of(&history, "a", "gone", "c").len(), 0);
    for tick in (FINE_TICKS as i64 + 2)..=(FINE_TICKS as i64 * 24) {
        history.record(at(tick * 15), &[]);
    }
    assert!(history.pods.contains_key("a"));
    history.record(at(15 * (FINE_TICKS as i64 * 24 + 1)), &[]);
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
    history.record(at(15), &sample);
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
    assert_eq!(history.nodes["wk-1"].len(), FINE_TICKS);
}
