use cluster::{
    ByteAmount, ClusterError, ContainerDiskIo, ControllerRef, DiskIoCounters, DiskIoSample,
    KubeletSummary, NetworkCounters, PodStatus, ReadyCount, StatusReason,
};

use super::*;

fn at(seconds: i64) -> jiff::Timestamp {
    jiff::Timestamp::from_second(seconds).expect("valid timestamp")
}

fn counters(seconds: i64, rx_bytes: u64, tx_bytes: u64) -> NetworkCounters {
    NetworkCounters {
        sampled_at: Some(at(seconds)),
        rx_bytes,
        tx_bytes,
    }
}

fn pod_stats(namespace: &str, name: &str, uid: &str, network: NetworkCounters) -> PodKubeletStats {
    PodKubeletStats {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
        uid: uid.to_owned(),
        network: Some(network),
        volumes: Vec::new(),
    }
}

fn node_with(name: &str, pods: Vec<PodKubeletStats>) -> NodeKubeletStats {
    NodeKubeletStats {
        node: name.to_owned(),
        summary: Ok(KubeletSummary {
            network: None,
            pods,
        }),
        disk_io: None,
    }
}

fn failed_node(name: &str) -> NodeKubeletStats {
    NodeKubeletStats {
        node: name.to_owned(),
        summary: Err(ClusterError::Rendered {
            message: "down".to_owned(),
        }),
        disk_io: None,
    }
}

fn pod_summary(namespace: &str, name: &str, host_network: bool) -> PodSummary {
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
        controller: None,
        conditions: Vec::new(),
        status_message: None,
        labels: Vec::new(),
        host_network,
        image_pull_secrets: Vec::new(),
        node_selector: Vec::new(),
        node_affinity: Vec::new(),
        containers: Vec::new(),
    }
}

fn pvc(namespace: &str, claim: &str, seconds: i64, used: u64) -> PvcUsage {
    PvcUsage {
        namespace: namespace.to_owned(),
        claim: claim.to_owned(),
        sampled_at: Some(at(seconds)),
        used: Some(ByteAmount::from_bytes(used)),
        capacity: Some(ByteAmount::from_bytes(100)),
        available: None,
        inodes_used: None,
        inodes: None,
    }
}

fn key(namespace: &str, name: &str) -> PodKey {
    PodKey {
        namespace: namespace.to_owned(),
        name: name.to_owned(),
    }
}

fn record_all(history: &mut KubeletHistory, seconds: i64, round: &[NodeKubeletStats]) {
    history.record(at(seconds), round, &[], &NamespaceScope::All);
}

/// The newest network rate of a stored pod.
fn newest_network(history: &KubeletHistory, namespace: &str, name: &str) -> Option<RatePair<u32>> {
    history.pods[&key(namespace, name)]
        .network
        .as_ref()?
        .rings
        .as_ref()?
        .newest()
}

/// How many ticks the network ring of a stored pod holds.
fn network_ring_len(history: &KubeletHistory, namespace: &str, name: &str) -> usize {
    history.pods[&key(namespace, name)]
        .network
        .as_ref()
        .and_then(|network| network.rings.as_ref())
        .map_or(0, Rings::len)
}

/// A pod whose counters grow by 1,500 receive and 750 transmit bytes per second.
fn steady_round(namespace: &str, name: &str, uid: &str, step: i64) -> Vec<NodeKubeletStats> {
    let counters = counters(step * 15, step as u64 * 22_500, step as u64 * 11_250);
    vec![node_with(
        "node-a",
        vec![pod_stats(namespace, name, uid, counters)],
    )]
}

fn sample_state(seconds: i64, first: u64, second: u64) -> CounterState {
    next_rate(None, at(seconds), first, second).1
}

#[test]
fn first_sample_has_no_rate() {
    let (rate, state) = next_rate(None, at(100), 5, 6);
    assert_eq!(rate, None);
    assert_eq!(
        state,
        CounterState {
            at: at(100),
            first: 5,
            second: 6,
            last_rate: None,
        }
    );
}

#[test]
fn rate_divides_by_sample_time() {
    let previous = sample_state(100, 0, 0);
    let (rate, state) = next_rate(Some(&previous), at(115), 15_000, 7_500);
    let expected = RatePair {
        first: 1_000,
        second: 500,
    };
    assert_eq!(rate, Some(expected));
    assert_eq!(state.last_rate, Some(expected));
    assert_eq!((state.first, state.second), (15_000, 7_500));
}

#[test]
fn rate_rounds_to_the_nearest_byte_per_second() {
    let previous = sample_state(0, 0, 0);
    // 10 bytes over 15 s is 0.67 B/s.
    let (rate, _) = next_rate(Some(&previous), at(15), 10, 7);
    assert_eq!(
        rate,
        Some(RatePair {
            first: 1,
            second: 0
        })
    );
}

#[test]
fn same_sample_time_repeats_the_last_rate() {
    let first = sample_state(100, 0, 0);
    let (_, second) = next_rate(Some(&first), at(115), 15_000, 15_000);
    let (rate, state) = next_rate(Some(&second), at(115), 99_999, 99_999);
    assert_eq!(rate, second.last_rate);
    assert_eq!(state, second);
}

#[test]
fn sub_second_step_repeats_the_last_rate() {
    let first = sample_state(100, 0, 0);
    let (_, second) = next_rate(Some(&first), at(115), 15_000, 15_000);
    let almost = jiff::Timestamp::from_millisecond(115_900).expect("timestamp");
    let (rate, state) = next_rate(Some(&second), almost, 20_000, 20_000);
    assert_eq!(rate, second.last_rate);
    assert_eq!(state, second);
}

#[test]
fn rate_ignores_backwards_sample_time() {
    let first = sample_state(100, 0, 0);
    let (_, second) = next_rate(Some(&first), at(115), 15_000, 15_000);
    let (rate, state) = next_rate(Some(&second), at(90), 1, 1);
    assert_eq!(rate, second.last_rate);
    assert_eq!(state, second);
}

#[test]
fn rate_math_does_not_overflow() {
    let previous = sample_state(100, 0, 0);
    let (rate, _) = next_rate(Some(&previous), at(101), u64::MAX, u64::MAX);
    assert_eq!(
        rate,
        Some(RatePair {
            first: u64::MAX,
            second: u64::MAX,
        })
    );
    let (rate, _) = next_rate(Some(&previous), at(1_000_000_000), u64::MAX, 0);
    assert_eq!(rate.map(|rate| rate.second), Some(0));
}

#[test]
fn counter_reset_gives_a_gap() {
    let first = sample_state(100, 50_000, 50_000);
    let (rate, reset) = next_rate(Some(&first), at(115), 10, 60_000);
    assert_eq!(rate, None);
    assert_eq!(reset.last_rate, None);
    let (rate, _) = next_rate(Some(&reset), at(130), 15_010, 75_000);
    assert_eq!(
        rate,
        Some(RatePair {
            first: 1_000,
            second: 1_000,
        })
    );
}

#[test]
fn rate_pair_mean_is_field_wise() {
    let wide = [
        RatePair {
            first: u64::MAX,
            second: 10,
        },
        RatePair {
            first: u64::MAX,
            second: 30,
        },
    ];
    assert_eq!(
        RatePair::<u64>::mean(&wide),
        RatePair {
            first: u64::MAX,
            second: 20,
        }
    );
    let narrow = [
        RatePair {
            first: u32::MAX,
            second: 1,
        },
        RatePair {
            first: u32::MAX,
            second: 4,
        },
    ];
    assert_eq!(
        RatePair::<u32>::mean(&narrow),
        RatePair {
            first: u32::MAX,
            second: 2,
        }
    );
}

#[test]
fn pod_rates_record_and_fold() {
    let mut history = KubeletHistory::default();
    for step in 1..=20 {
        record_all(
            &mut history,
            step * 15,
            &steady_round("shop", "web-1", "u1", step),
        );
    }
    let expected = RatePair {
        first: 1_500,
        second: 750,
    };
    assert_eq!(newest_network(&history, "shop", "web-1"), Some(expected));
    let rings = history.pods[&key("shop", "web-1")]
        .network
        .as_ref()
        .and_then(|network| network.rings.as_ref())
        .expect("rings");
    // The first tick has no rate, but the coarse point after 20 ticks folds the other 19.
    assert_eq!(rings.coarse_len(), 1);
}

#[test]
fn new_uid_resets_the_counters() {
    let mut history = KubeletHistory::default();
    for step in 1..=3 {
        record_all(
            &mut history,
            step * 15,
            &steady_round("shop", "web-1", "u1", step),
        );
    }
    assert!(newest_network(&history, "shop", "web-1").is_some());
    let rate_ticks = network_ring_len(&history, "shop", "web-1");
    // The pod was replaced under the same name; its counters are far higher than a rate allows.
    record_all(&mut history, 60, &steady_round("shop", "web-1", "u2", 400));
    assert_eq!(newest_network(&history, "shop", "web-1"), None);
    // Only the counter restarted: the points of the old pod are still in the ring.
    assert_eq!(network_ring_len(&history, "shop", "web-1"), rate_ticks + 1);
    record_all(&mut history, 75, &steady_round("shop", "web-1", "u2", 401));
    assert!(newest_network(&history, "shop", "web-1").is_some());
}

#[test]
fn new_uid_keeps_the_controller() {
    let mut history = KubeletHistory::default();
    let mut listed = pod_summary("shop", "web-1", false);
    listed.controller = Some(ControllerRef {
        kind: "StatefulSet".to_owned(),
        name: "web".to_owned(),
    });
    let pods = [listed.clone()];
    for (step, uid) in [(1, "u1"), (2, "u2")] {
        let round = steady_round("shop", "web-1", uid, step);
        history.record(at(step * 15), &round, &pods, &NamespaceScope::All);
    }
    assert_eq!(
        history.pods[&key("shop", "web-1")].controller,
        listed.controller
    );
}

#[test]
fn pod_rates_saturate_at_u32() {
    let mut history = KubeletHistory::default();
    let burst = |seconds, bytes| {
        vec![node_with(
            "node-a",
            vec![pod_stats(
                "shop",
                "web-1",
                "u1",
                counters(seconds, bytes, 0),
            )],
        )]
    };
    record_all(&mut history, 15, &burst(15, 0));
    record_all(&mut history, 30, &burst(30, u64::MAX / 2));
    assert_eq!(
        newest_network(&history, "shop", "web-1"),
        Some(RatePair {
            first: u32::MAX,
            second: 0,
        })
    );
}

#[test]
fn failed_node_records_none_this_tick() {
    let mut history = KubeletHistory::default();
    let round = |step: i64, is_b_failed: bool| {
        let node_b = if is_b_failed {
            failed_node("node-b")
        } else {
            node_with(
                "node-b",
                vec![pod_stats(
                    "shop",
                    "db-1",
                    "u2",
                    counters(step * 15, step as u64 * 15_000, 0),
                )],
            )
        };
        let mut nodes = steady_round("shop", "web-1", "u1", step);
        nodes.push(node_b);
        nodes
    };
    for step in 1..=3 {
        record_all(&mut history, step * 15, &round(step, false));
    }
    assert!(newest_network(&history, "shop", "db-1").is_some());
    record_all(&mut history, 60, &round(4, true));
    assert_eq!(newest_network(&history, "shop", "db-1"), None);
    assert!(newest_network(&history, "shop", "web-1").is_some());
    // The counter stays, so the next good round rates over the two ticks.
    record_all(&mut history, 75, &round(5, false));
    let rate = newest_network(&history, "shop", "db-1").expect("rate");
    assert_eq!(rate.first, 1_000);
}

#[test]
fn rings_never_exceed_fine_ticks() {
    let mut history = KubeletHistory::default();
    for step in 1..=300 {
        record_all(
            &mut history,
            step * 15,
            &steady_round("shop", "web-1", "u1", step),
        );
    }
    let rings = history.pods[&key("shop", "web-1")]
        .network
        .as_ref()
        .and_then(|network| network.rings.as_ref())
        .expect("rings");
    assert_eq!(rings.len(), FINE_TICKS);
    assert_eq!(history.tick_count(), 300);
}

#[test]
fn pods_outside_scope_are_not_stored() {
    let mut history = KubeletHistory::default();
    let scope = NamespaceScope::Named("shop".to_owned());
    let mut round = steady_round("shop", "web-1", "u1", 1);
    round.extend(steady_round("other", "web-2", "u3", 1));
    history.record(at(15), &round, &[], &scope);
    assert!(history.pods.contains_key(&key("shop", "web-1")));
    assert!(!history.pods.contains_key(&key("other", "web-2")));
}

fn pvc_pod(name: &str, usage: PvcUsage) -> PodKubeletStats {
    PodKubeletStats {
        namespace: "shop".to_owned(),
        name: name.to_owned(),
        uid: name.to_owned(),
        network: None,
        volumes: vec![usage],
    }
}

#[test]
fn summary_pod_with_host_network_is_not_stored() {
    let mut history = KubeletHistory::default();
    let disk = DiskIoSample {
        node: None,
        containers: vec![ContainerDiskIo {
            namespace: "shop".to_owned(),
            pod: "web-1".to_owned(),
            container: "app".to_owned(),
            counters: DiskIoCounters {
                sampled_at: Some(at(15)),
                read_bytes: 1,
                write_bytes: 1,
            },
        }],
    };
    let host_pods = [pod_summary("shop", "web-1", true)];
    for step in 1..=3 {
        let mut stats = pod_stats(
            "shop",
            "web-1",
            "u1",
            counters(step * 15, step as u64 * 1_000, 0),
        );
        stats.volumes = vec![pvc("shop", "data", step * 15, 10)];
        let mut node = node_with("node-a", vec![stats]);
        node.disk_io = Some(Ok(disk.clone()));
        history.record(at(step * 15), &[node], &host_pods, &NamespaceScope::All);
    }
    let entry = &history.pods[&key("shop", "web-1")];
    assert!(entry.network.is_none());
    assert!(entry.containers.contains_key("app"));
    assert!(history.pvc_usage("shop", "data").is_some());
}

#[test]
fn host_network_flag_learned_later_frees_the_rings() {
    let mut history = KubeletHistory::default();
    for step in 1..=3 {
        record_all(
            &mut history,
            step * 15,
            &steady_round("shop", "web-1", "u1", step),
        );
    }
    assert!(history.pods[&key("shop", "web-1")].network.is_some());
    let host_pods = [pod_summary("shop", "web-1", true)];
    history.record(
        at(60),
        &steady_round("shop", "web-1", "u1", 4),
        &host_pods,
        &NamespaceScope::All,
    );
    assert!(history.pods[&key("shop", "web-1")].network.is_none());
}

#[test]
fn node_network_and_disk_rates_are_recorded() {
    let mut history = KubeletHistory::default();
    for step in 1..=3_i64 {
        let mut node = node_with("node-a", Vec::new());
        if let Ok(summary) = &mut node.summary {
            summary.network = Some(counters(step * 15, step as u64 * 30_000, 0));
        }
        let disk = DiskIoSample {
            node: Some(DiskIoCounters {
                sampled_at: Some(at(step * 15)),
                read_bytes: step as u64 * 15_000,
                write_bytes: step as u64 * 45_000,
            }),
            containers: Vec::new(),
        };
        node.disk_io = Some(Ok(disk));
        record_all(&mut history, step * 15, &[node]);
    }
    let node = &history.nodes["node-a"];
    let network = node.network.rings.as_ref().and_then(Rings::newest);
    assert_eq!(
        network,
        Some(RatePair {
            first: 2_000,
            second: 0
        })
    );
    let disk = node.disk.rings.as_ref().and_then(Rings::newest);
    assert_eq!(
        disk,
        Some(RatePair {
            first: 1_000,
            second: 3_000,
        })
    );
}

#[test]
fn controllers_are_filled_from_the_pods_list() {
    let mut history = KubeletHistory::default();
    let mut listed = pod_summary("shop", "web-1", false);
    listed.controller = Some(ControllerRef {
        kind: "ReplicaSet".to_owned(),
        name: "web-7d9f8c".to_owned(),
    });
    history.record(
        at(15),
        &steady_round("shop", "web-1", "u1", 1),
        std::slice::from_ref(&listed),
        &NamespaceScope::All,
    );
    assert_eq!(
        history.pods[&key("shop", "web-1")].controller,
        listed.controller
    );
}

#[test]
fn retain_scope_drops_pods_and_pvcs() {
    let mut history = KubeletHistory::default();
    let round = vec![node_with(
        "node-a",
        vec![
            pvc_pod("web-1", pvc("shop", "data", 15, 10)),
            PodKubeletStats {
                namespace: "other".to_owned(),
                ..pvc_pod("web-2", pvc("other", "data", 15, 10))
            },
        ],
    )];
    record_all(&mut history, 15, &round);
    assert_eq!(history.pods.len(), 2);

    history.retain_scope(&NamespaceScope::All);
    assert_eq!(history.pods.len(), 2);
    history.retain_scope(&NamespaceScope::Named("shop".to_owned()));

    assert_eq!(history.pods.len(), 1);
    assert!(history.pvc_usage("shop", "data").is_some());
    assert!(history.pvc_usage("other", "data").is_none());
    assert_eq!(history.tick_count(), 1);
}

#[test]
fn pvc_usage_keeps_the_newest_sample() {
    let mut history = KubeletHistory::default();
    // A ReadWriteMany claim is reported by two pods; the stale report comes second.
    let round = vec![node_with(
        "node-a",
        vec![
            pvc_pod("web-1", pvc("shop", "shared", 30, 80)),
            pvc_pod("web-2", pvc("shop", "shared", 15, 70)),
        ],
    )];
    record_all(&mut history, 30, &round);
    let used = history
        .pvc_usage("shop", "shared")
        .and_then(|usage| usage.used);
    assert_eq!(used, Some(ByteAmount::from_bytes(80)));
    assert!(history.pvc_usage("shop", "missing").is_none());
}

#[test]
fn pvc_usages_yield_newest_per_claim() {
    let mut history = KubeletHistory::default();
    let round = vec![node_with(
        "node-a",
        vec![
            pvc_pod("web-1", pvc("shop", "shared", 30, 80)),
            pvc_pod("web-2", pvc("shop", "shared", 15, 70)),
            pvc_pod("web-3", pvc("shop", "data", 30, 10)),
        ],
    )];
    record_all(&mut history, 30, &round);
    let mut used: Vec<(String, u64)> = history
        .pvc_usages()
        .filter_map(|usage| Some((usage.claim.clone(), usage.used?.bytes())))
        .collect();
    used.sort();
    assert_eq!(
        used,
        [("data".to_owned(), 10), ("shared".to_owned(), 80)],
        "one entry per claim, the newest sample"
    );
}

#[test]
fn a_later_round_replaces_the_pvc_usage() {
    let mut history = KubeletHistory::default();
    let round = vec![node_with(
        "node-a",
        vec![pvc_pod("web-1", pvc("shop", "shared", 30, 80))],
    )];
    record_all(&mut history, 30, &round);
    let round = vec![node_with(
        "node-a",
        vec![pvc_pod("web-1", pvc("shop", "shared", 45, 90))],
    )];
    record_all(&mut history, 45, &round);
    let used = history
        .pvc_usage("shop", "shared")
        .and_then(|usage| usage.used);
    assert_eq!(used, Some(ByteAmount::from_bytes(90)));
}

#[test]
fn unseen_pvc_is_dropped() {
    let mut history = KubeletHistory::default();
    let round = vec![node_with(
        "node-a",
        vec![pvc_pod("web-1", pvc("shop", "data", 15, 10))],
    )];
    record_all(&mut history, 15, &round);
    for step in 2..=FINE_TICKS as i64 {
        record_all(&mut history, step * 15, &[]);
    }
    assert!(history.pvc_usage("shop", "data").is_some());
    record_all(&mut history, (FINE_TICKS as i64 + 1) * 15, &[]);
    assert!(history.pvc_usage("shop", "data").is_none());
}

#[test]
fn pods_and_nodes_unseen_for_a_day_are_forgotten() {
    let mut history = KubeletHistory::default();
    record_all(&mut history, 15, &steady_round("shop", "web-1", "u1", 1));
    for step in 2..=DROP_AFTER_TICKS as i64 {
        record_all(&mut history, step * 15, &[]);
    }
    assert!(history.pods.contains_key(&key("shop", "web-1")));
    record_all(&mut history, (DROP_AFTER_TICKS as i64 + 1) * 15, &[]);
    assert!(history.pods.is_empty());
    assert!(history.nodes.is_empty());
}

#[test]
fn host_network_flag_is_found_for_the_second_pod_of_the_list() {
    let mut history = KubeletHistory::default();
    let listed = [
        pod_summary("shop", "web-1", false),
        pod_summary("shop", "web-2", true),
    ];
    for step in 1..=3_i64 {
        let node = node_with(
            "node-a",
            vec![
                pod_stats(
                    "shop",
                    "web-1",
                    "u1",
                    counters(step * 15, step as u64 * 1_000, 0),
                ),
                pod_stats(
                    "shop",
                    "web-2",
                    "u2",
                    counters(step * 15, step as u64 * 1_000, 0),
                ),
            ],
        );
        history.record(at(step * 15), &[node], &listed, &NamespaceScope::All);
    }
    assert!(newest_network(&history, "shop", "web-1").is_some());
    assert!(history.pods[&key("shop", "web-2")].network.is_none());
}

// ---- readers ----

fn disk_round(step: i64, has_root: bool, containers: &[(&str, u64)]) -> NodeKubeletStats {
    let counters = |seed: u64| DiskIoCounters {
        sampled_at: Some(at(step * 15)),
        read_bytes: step as u64 * 15 * seed,
        write_bytes: step as u64 * 15 * seed * 3,
    };
    let sample = DiskIoSample {
        node: has_root.then(|| counters(100)),
        containers: containers
            .iter()
            .map(|(container, seed)| ContainerDiskIo {
                namespace: "shop".to_owned(),
                pod: "web-1".to_owned(),
                container: (*container).to_owned(),
                counters: counters(*seed),
            })
            .collect(),
    };
    let mut node = node_with("node-a", Vec::new());
    node.disk_io = Some(Ok(sample));
    node
}

fn rates(series: &RateSeries) -> Vec<Option<(u64, u64)>> {
    series
        .points
        .iter()
        .map(|(_, rate)| rate.map(|rate| (rate.first, rate.second)))
        .collect()
}

#[test]
fn pod_disk_total_sums_containers() {
    let mut history = KubeletHistory::default();
    for step in 1..=3 {
        record_all(
            &mut history,
            step * 15,
            &[disk_round(step, true, &[("app", 10), ("side", 5)])],
        );
    }
    let total = history.pod_rates(RateKind::DiskIo, "shop", "web-1", None, Resolution::Fine);
    assert_eq!(rates(&total), [None, Some((15, 45)), Some((15, 45))]);
    let app = history.pod_rates(
        RateKind::DiskIo,
        "shop",
        "web-1",
        Some("app"),
        Resolution::Fine,
    );
    assert_eq!(rates(&app), [None, Some((10, 30)), Some((10, 30))]);
}

#[test]
fn has_disk_series_follows_container_entries() {
    let mut history = KubeletHistory::default();
    assert!(!history.has_disk_series("shop", "web-1", None));
    for step in 1..=2 {
        record_all(
            &mut history,
            step * 15,
            &[disk_round(step, true, &[("app", 10), ("side", 5)])],
        );
    }
    assert!(history.has_disk_series("shop", "web-1", None));
    assert!(history.has_disk_series("shop", "web-1", Some("side")));
    assert!(!history.has_disk_series("shop", "web-1", Some("other")));
    assert!(!history.has_disk_series("shop", "web-2", None));
}

#[test]
fn container_scope_network_is_the_pods() {
    let mut history = KubeletHistory::default();
    for step in 1..=3 {
        record_all(
            &mut history,
            step * 15,
            &steady_round("shop", "web-1", "u1", step),
        );
    }
    let pod = history.pod_rates(RateKind::Network, "shop", "web-1", None, Resolution::Fine);
    let container = history.pod_rates(
        RateKind::Network,
        "shop",
        "web-1",
        Some("app"),
        Resolution::Fine,
    );
    assert_eq!(rates(&pod), rates(&container));
    assert_eq!(rates(&pod).last(), Some(&Some((1_500, 750))));
}

fn owner() -> PodOwner {
    PodOwner::Controller {
        namespace: "shop".to_owned(),
        kind: "StatefulSet",
        name: "web".to_owned(),
    }
}

fn owned_pod(name: &str, host_network: bool) -> PodSummary {
    let mut summary = pod_summary("shop", name, host_network);
    summary.controller = Some(ControllerRef {
        kind: "StatefulSet".to_owned(),
        name: "web".to_owned(),
    });
    summary
}

#[test]
fn owner_network_skips_host_network_pods() {
    let mut history = KubeletHistory::default();
    let listed = [owned_pod("web-1", false), owned_pod("web-2", true)];
    for step in 1..=3_i64 {
        let pods = ["web-1", "web-2"].map(|name| {
            pod_stats(
                "shop",
                name,
                name,
                counters(step * 15, step as u64 * 15_000, 0),
            )
        });
        let round = [node_with("node-a", pods.to_vec())];
        history.record(at(step * 15), &round, &listed, &NamespaceScope::All);
    }
    let owned = history.owner_rates(RateKind::Network, &owner(), None, Resolution::Fine);
    // Only web-1 counts: the host-network pod has no network rings.
    assert_eq!(rates(&owned).last(), Some(&Some((1_000, 0))));
    let one = history.owner_rates(RateKind::Network, &owner(), Some("web-1"), Resolution::Fine);
    assert_eq!(rates(&one), rates(&owned));
    let other = history.owner_rates(RateKind::Network, &owner(), Some("web-2"), Resolution::Fine);
    assert!(rates(&other).iter().all(Option::is_none));
}

#[test]
fn owner_rates_skip_pods_of_other_workloads() {
    let mut history = KubeletHistory::default();
    let mut stranger = pod_summary("shop", "db-1", false);
    stranger.controller = Some(ControllerRef {
        kind: "StatefulSet".to_owned(),
        name: "db".to_owned(),
    });
    let listed = [stranger, owned_pod("web-1", false)];
    for step in 1..=3_i64 {
        let pods = ["db-1", "web-1"].map(|name| {
            pod_stats(
                "shop",
                name,
                name,
                counters(step * 15, step as u64 * 15_000, 0),
            )
        });
        let round = [node_with("node-a", pods.to_vec())];
        history.record(at(step * 15), &round, &listed, &NamespaceScope::All);
    }
    let owned = history.owner_rates(RateKind::Network, &owner(), None, Resolution::Fine);
    assert_eq!(rates(&owned).last(), Some(&Some((1_000, 0))));
}

#[test]
fn node_rates_read_the_nodes_own_rings() {
    let mut history = KubeletHistory::default();
    for step in 1..=3 {
        record_all(&mut history, step * 15, &[disk_round(step, true, &[])]);
    }
    let disk = history.node_rates(RateKind::DiskIo, "node-a", Resolution::Fine);
    assert_eq!(rates(&disk), [None, Some((100, 300)), Some((100, 300))]);
    let missing = history.node_rates(RateKind::DiskIo, "node-z", Resolution::Fine);
    assert_eq!(rates(&missing), [None, None, None]);
    assert!(history.newest_tick().is_some());
    assert!(history.span().is_some());
}

#[test]
fn disk_io_state_follows_the_newest_sample() {
    let mut history = KubeletHistory::default();
    assert_eq!(history.disk_io_state("node-a"), DiskIoState::NotSampled);
    record_all(&mut history, 15, &[disk_round(1, true, &[])]);
    assert_eq!(
        history.disk_io_state("node-a"),
        DiskIoState::Sampled { has_root: true }
    );
    record_all(&mut history, 30, &[disk_round(2, false, &[])]);
    assert_eq!(
        history.disk_io_state("node-a"),
        DiskIoState::Sampled { has_root: false }
    );
    // A round that does not read the disk leaves the node unsampled again.
    record_all(&mut history, 45, &[node_with("node-a", Vec::new())]);
    assert_eq!(history.disk_io_state("node-a"), DiskIoState::NotSampled);
}
