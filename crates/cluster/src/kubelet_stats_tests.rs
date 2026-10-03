use std::pin::pin;

use futures::StreamExt;
use futures::io::Cursor;
use serde_json::json;
use tokio::sync::mpsc;

use super::*;
use crate::resource_metrics::METRICS_INTERVAL;

fn timestamp(text: &str) -> jiff::Timestamp {
    text.parse().expect("valid timestamp")
}

fn decode(value: serde_json::Value) -> KubeletSummary {
    kubelet_summary(serde_json::from_value(value).expect("summary document"))
}

fn network(value: serde_json::Value, rule: InterfaceRule) -> Option<NetworkCounters> {
    network_counters(
        &serde_json::from_value(value).expect("network document"),
        rule,
    )
}

fn pod_json(extra: serde_json::Value) -> serde_json::Value {
    let mut pod = json!({
        "podRef": {"name": "web-1", "namespace": "shop", "uid": "uid-1"},
    });
    if let (Some(pod), Some(extra)) = (pod.as_object_mut(), extra.as_object()) {
        pod.extend(extra.clone());
    }
    pod
}

#[test]
fn kubelet_paths_are_fixed() {
    assert_eq!(
        KubeletPath::StatsSummary.subresource(),
        "proxy/stats/summary"
    );
    assert_eq!(KubeletPath::StatsSummary.action(), "reading kubelet stats");
    assert_eq!(
        KubeletPath::MetricsCadvisor.subresource(),
        "proxy/metrics/cadvisor"
    );
    assert_eq!(
        KubeletPath::MetricsCadvisor.action(),
        "reading kubelet cAdvisor metrics"
    );
}

#[test]
fn proxy_request_path_is_the_node_and_the_fixed_subresource() {
    let request = kube::core::Request::new(NODES_URL)
        .get_subresource(KubeletPath::MetricsCadvisor.subresource(), "node-1")
        .expect("request");
    assert_eq!(request.method(), "GET");
    assert_eq!(
        request.uri().to_string(),
        "/api/v1/nodes/node-1/proxy/metrics/cadvisor"
    );
}

#[test]
fn summary_reads_pod_network_and_uid() {
    let summary = decode(json!({
        "pods": [pod_json(json!({
            "network": {"time": "2026-10-02T10:00:00Z", "rxBytes": 1200, "txBytes": 800},
        }))],
    }));
    let [pod] = summary.pods.as_slice() else {
        panic!("one pod expected: {:?}", summary.pods);
    };
    assert_eq!(
        (pod.namespace.as_str(), pod.name.as_str(), pod.uid.as_str()),
        ("shop", "web-1", "uid-1")
    );
    assert_eq!(
        pod.network,
        Some(NetworkCounters {
            sampled_at: Some(timestamp("2026-10-02T10:00:00Z")),
            rx_bytes: 1200,
            tx_bytes: 800,
        })
    );
}

#[test]
fn summary_skips_pods_without_pod_ref() {
    let summary = decode(json!({
        "pods": [
            {"network": {"rxBytes": 1, "txBytes": 2}},
            {"podRef": {"name": "a", "namespace": "shop"}},
            {"podRef": {"name": "a", "uid": "u"}},
            pod_json(json!({})),
        ],
    }));
    assert_eq!(summary.pods.len(), 1);
    assert_eq!(summary.pods[0].name, "web-1");
}

#[test]
fn summary_accepts_null_and_missing_lists() {
    let summary = decode(json!({"node": {"nodeName": "n"}, "pods": null}));
    assert!(summary.pods.is_empty());
    assert_eq!(summary.network, None);
    let summary = decode(json!({"pods": [pod_json(json!({"volume": null}))]}));
    assert!(summary.pods[0].volumes.is_empty());
}

#[test]
fn network_prefers_top_level_counters() {
    let counters = network(
        json!({
            "rxBytes": 10, "txBytes": 20,
            "interfaces": [{"name": "eth0", "rxBytes": 999, "txBytes": 999}],
        }),
        InterfaceRule::Pod,
    )
    .expect("counters");
    assert_eq!((counters.rx_bytes, counters.tx_bytes), (10, 20));
}

#[test]
fn pod_network_sums_interfaces_except_loopback() {
    let counters = network(
        json!({"interfaces": [
            {"name": "eth0", "rxBytes": 100, "txBytes": 10},
            {"name": "net1", "rxBytes": 50, "txBytes": 5},
            {"name": "lo", "rxBytes": 7000, "txBytes": 7000},
        ]}),
        InterfaceRule::Pod,
    )
    .expect("counters");
    assert_eq!((counters.rx_bytes, counters.tx_bytes), (150, 15));
}

#[test]
fn node_network_skips_virtual_interfaces() {
    let counters = network(
        json!({"interfaces": [
            {"name": "ens5", "rxBytes": 100, "txBytes": 10},
            {"name": "veth1", "rxBytes": 1, "txBytes": 1},
            {"name": "cni0", "rxBytes": 1, "txBytes": 1},
            {"name": "cali9", "rxBytes": 1, "txBytes": 1},
            {"name": "kube-ipvs0", "rxBytes": 1, "txBytes": 1},
            {"name": "br-1a2b", "rxBytes": 1, "txBytes": 1},
            {"name": "lo", "rxBytes": 1, "txBytes": 1},
        ]}),
        InterfaceRule::Node,
    )
    .expect("counters");
    assert_eq!((counters.rx_bytes, counters.tx_bytes), (100, 10));
}

#[test]
fn network_without_counters_is_none() {
    assert_eq!(network(json!({}), InterfaceRule::Pod), None);
    assert_eq!(network(json!({"rxBytes": 5}), InterfaceRule::Pod), None);
    let only_loopback = json!({"interfaces": [{"name": "lo", "rxBytes": 1, "txBytes": 1}]});
    assert_eq!(network(only_loopback, InterfaceRule::Pod), None);
    let only_virtual = json!({"interfaces": [{"name": "veth1", "rxBytes": 1, "txBytes": 1}]});
    assert_eq!(network(only_virtual, InterfaceRule::Node), None);
}

#[test]
fn node_network_comes_from_the_node_object() {
    let summary = decode(json!({
        "node": {"network": {"time": "2026-10-02T10:00:00Z", "rxBytes": 3, "txBytes": 4}},
    }));
    let counters = summary.network.expect("node network");
    assert_eq!((counters.rx_bytes, counters.tx_bytes), (3, 4));
}

#[test]
fn summary_reads_pvc_volumes_only() {
    let summary = decode(json!({"pods": [pod_json(json!({"volume": [
        {"name": "config", "usedBytes": 4096},
        {
            "time": "2026-10-02T10:00:00Z",
            "name": "data",
            "pvcRef": {"name": "data-claim", "namespace": "shop"},
            "availableBytes": 20,
            "capacityBytes": 100,
            "usedBytes": 80,
            "inodesFree": 5,
            "inodes": 1000,
            "inodesUsed": 400,
        },
        {"name": "partial", "pvcRef": {"name": "orphan"}},
    ]}))]}));
    let [volume] = summary.pods[0].volumes.as_slice() else {
        panic!("one PVC expected: {:?}", summary.pods[0].volumes);
    };
    assert_eq!(
        *volume,
        PvcUsage {
            namespace: "shop".to_owned(),
            claim: "data-claim".to_owned(),
            sampled_at: Some(timestamp("2026-10-02T10:00:00Z")),
            used: Some(ByteAmount::from_bytes(80)),
            capacity: Some(ByteAmount::from_bytes(100)),
            available: Some(ByteAmount::from_bytes(20)),
            inodes_used: Some(400),
            inodes: Some(1000),
        }
    );
}

#[test]
fn bad_time_keeps_counters() {
    let counters = network(
        json!({"time": "yesterday", "rxBytes": 1, "txBytes": 2}),
        InterfaceRule::Pod,
    )
    .expect("counters");
    assert_eq!(counters.sampled_at, None);
    assert_eq!((counters.rx_bytes, counters.tx_bytes), (1, 2));
}

#[test]
fn summary_ignores_unknown_fields() {
    let text = r#"{
        "node": {
            "nodeName": "n1",
            "cpu": {"usageNanoCores": 5},
            "memory": {"workingSetBytes": 6},
            "rootfs": {"usedBytes": 7},
            "network": {"name": "eth0", "rxBytes": 8, "txBytes": 9, "extra": [1, 2]}
        },
        "pods": [{
            "podRef": {"name": "web-1", "namespace": "shop", "uid": "u"},
            "cpu": {"usageNanoCores": 5},
            "ephemeral-storage": {"usedBytes": 1},
            "containers": [{"name": "app", "logs": {"usedBytes": 2}}]
        }]
    }"#;
    let doc: SummaryDoc = serde_json::from_str(text).expect("document");
    let summary = kubelet_summary(doc);
    assert_eq!(summary.network.map(|counters| counters.rx_bytes), Some(8));
    assert_eq!(summary.pods.len(), 1);
}

fn stats(node: &str, summary: Result<KubeletSummary, ClusterError>) -> NodeKubeletStats {
    NodeKubeletStats {
        node: node.to_owned(),
        summary,
        disk_io: None,
    }
}

fn empty_summary() -> Result<KubeletSummary, ClusterError> {
    Ok(KubeletSummary {
        network: None,
        pods: Vec::new(),
    })
}

fn failed(code: u16) -> Result<KubeletSummary, ClusterError> {
    Err(ClusterError::Api {
        context: "ctx".to_owned(),
        action: KubeletPath::StatsSummary.action(),
        code,
        message: "down".to_owned(),
    })
}

#[test]
fn round_fails_only_when_every_node_fails() {
    let one_ok = round_result(vec![stats("b", failed(503)), stats("a", empty_summary())]);
    assert!(one_ok.is_ok());

    let all_failed = round_result(vec![stats("b", failed(404)), stats("a", failed(503))]);
    assert!(matches!(
        all_failed,
        Err(ClusterError::Api { code: 503, .. })
    ));
}

#[test]
fn round_orders_nodes_by_name() {
    let ordered = round_result(vec![
        stats("node-c", empty_summary()),
        stats("node-a", empty_summary()),
        stats("node-b", failed(503)),
    ])
    .expect("round");
    let names: Vec<_> = ordered.iter().map(|node| node.node.as_str()).collect();
    assert_eq!(names, ["node-a", "node-b", "node-c"]);
}

fn targets(summary: &[&str], disk: &[&str]) -> KubeletTargets {
    KubeletTargets {
        summary_nodes: summary.iter().map(|node| (*node).to_owned()).collect(),
        disk_io_nodes: disk.iter().map(|node| (*node).to_owned()).collect(),
    }
}

/// A fake round that reports when it ran and which targets it received.
struct Calls {
    start: Instant,
    receiver: mpsc::UnboundedReceiver<(Duration, KubeletTargets)>,
}

impl Calls {
    fn next(&mut self) -> (Duration, KubeletTargets) {
        self.receiver.try_recv().expect("a fetch call")
    }

    fn is_empty(&mut self) -> bool {
        self.receiver.try_recv().is_err()
    }
}

fn fake_poll(
    receiver: watch::Receiver<KubeletTargets>,
) -> (impl Stream<Item = WatchUpdate<NodeKubeletStats>>, Calls) {
    let (sender, calls) = mpsc::unbounded_channel();
    let start = Instant::now();
    let stream = poll_targets(receiver, move |targets| {
        let sender = sender.clone();
        async move {
            let _ = sender.send((start.elapsed(), targets));
            Ok(Vec::new())
        }
    });
    (
        stream,
        Calls {
            start,
            receiver: calls,
        },
    )
}

/// Sends `values` one after another, the first after `delay`.
fn send_later(sender: watch::Sender<KubeletTargets>, delay: Duration, values: Vec<KubeletTargets>) {
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        for value in values {
            let _ = sender.send(value);
        }
        // Keeps the channel open for the rest of the test.
        tokio::time::sleep(Duration::from_secs(3600)).await;
    });
}

#[tokio::test(start_paused = true)]
async fn poll_reads_the_newest_targets_each_round() {
    let (sender, receiver) = watch::channel(targets(&["a"], &[]));
    let (stream, mut calls) = fake_poll(receiver);
    let mut stream = pin!(stream);
    send_later(
        sender,
        Duration::from_secs(5),
        vec![targets(&["a", "b"], &[]), targets(&["a", "b", "c"], &["c"])],
    );

    assert!(matches!(
        stream.next().await,
        Some(WatchUpdate::Snapshot(_))
    ));
    assert!(matches!(
        stream.next().await,
        Some(WatchUpdate::Snapshot(_))
    ));

    assert_eq!(calls.next(), (Duration::ZERO, targets(&["a"], &[])));
    assert_eq!(
        calls.next(),
        (Duration::from_secs(5), targets(&["a", "b", "c"], &["c"]))
    );
    assert!(calls.is_empty());
}

#[tokio::test(start_paused = true)]
async fn rounds_repeat_every_interval_without_changes() {
    let (_sender, receiver) = watch::channel(targets(&["a"], &[]));
    let (stream, mut calls) = fake_poll(receiver);
    let mut stream = pin!(stream);
    for _ in 0..3 {
        stream.next().await;
    }
    let times: Vec<_> = (0..3).map(|_| calls.next().0).collect();
    assert_eq!(
        times,
        [
            Duration::ZERO,
            Duration::from_secs(15),
            Duration::from_secs(30)
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn added_node_starts_an_early_round() {
    let (sender, receiver) = watch::channel(targets(&["a"], &[]));
    let (stream, mut calls) = fake_poll(receiver);
    let mut stream = pin!(stream);
    send_later(
        sender,
        Duration::from_secs(5),
        vec![targets(&["a", "b"], &[])],
    );

    stream.next().await;
    stream.next().await;

    assert_eq!(calls.next().0, Duration::ZERO);
    assert_eq!(calls.next().0, Duration::from_secs(5));
}

#[tokio::test(start_paused = true)]
async fn added_disk_node_starts_an_early_round() {
    let (sender, receiver) = watch::channel(targets(&["a", "b"], &[]));
    let (stream, mut calls) = fake_poll(receiver);
    let mut stream = pin!(stream);
    send_later(
        sender,
        Duration::from_secs(4),
        vec![targets(&["a", "b"], &["a"])],
    );

    stream.next().await;
    stream.next().await;

    calls.next();
    assert_eq!(calls.next().0, Duration::from_secs(4));
}

#[tokio::test(start_paused = true)]
async fn early_rounds_keep_the_minimum_gap() {
    let (sender, receiver) = watch::channel(targets(&["a"], &[]));
    let (stream, mut calls) = fake_poll(receiver);
    let mut stream = pin!(stream);
    send_later(
        sender,
        Duration::from_secs(1),
        vec![targets(&["a", "b"], &[])],
    );

    stream.next().await;
    stream.next().await;

    assert_eq!(calls.next().0, Duration::ZERO);
    assert_eq!(calls.next().0, MIN_ROUND_GAP);
}

#[tokio::test(start_paused = true)]
async fn shrinking_targets_wait_for_the_timer() {
    let (sender, receiver) = watch::channel(targets(&["a", "b"], &["a"]));
    let (stream, mut calls) = fake_poll(receiver);
    let mut stream = pin!(stream);
    send_later(sender, Duration::from_secs(5), vec![targets(&["a"], &[])]);

    stream.next().await;
    stream.next().await;

    calls.next();
    let (at, received) = calls.next();
    assert_eq!(at, METRICS_INTERVAL);
    assert_eq!(received, targets(&["a"], &[]));
}

#[tokio::test(start_paused = true)]
async fn empty_targets_make_no_request() {
    let (sender, receiver) = watch::channel(KubeletTargets::default());
    let (stream, mut calls) = fake_poll(receiver);
    let mut stream = pin!(stream);
    send_later(sender, Duration::from_secs(20), vec![targets(&["a"], &[])]);

    stream.next().await;

    let (at, received) = calls.next();
    assert_eq!(at, Duration::from_secs(20));
    assert_eq!(received, targets(&["a"], &[]));
    assert!(calls.is_empty());
    assert_eq!(calls.start.elapsed(), Duration::from_secs(20));
}

#[tokio::test(start_paused = true)]
async fn emptied_targets_idle_without_requests() {
    let (sender, receiver) = watch::channel(targets(&["a"], &[]));
    let (stream, mut calls) = fake_poll(receiver);
    let mut stream = pin!(stream);
    send_later(
        sender,
        Duration::from_secs(5),
        vec![KubeletTargets::default()],
    );

    stream.next().await;
    // The timer round reads the empty targets and idles: no item, no fetch.
    let next = tokio::time::timeout(Duration::from_secs(600), stream.next()).await;

    assert!(next.is_err());
    calls.next();
    assert!(calls.is_empty());
}

#[tokio::test(start_paused = true)]
async fn closed_sender_ends_the_stream() {
    let (sender, receiver) = watch::channel(KubeletTargets::default());
    let (stream, _calls) = fake_poll(receiver);
    let mut stream = pin!(stream);
    drop(sender);

    assert!(stream.next().await.is_none());
}

#[tokio::test(start_paused = true)]
async fn closed_sender_ends_the_stream_while_waiting() {
    let (sender, receiver) = watch::channel(targets(&["a"], &[]));
    let (stream, _calls) = fake_poll(receiver);
    let mut stream = pin!(stream);

    stream.next().await;
    drop(sender);

    assert!(stream.next().await.is_none());
}

#[tokio::test(start_paused = true)]
async fn failed_round_backs_off() {
    let (_sender, receiver) = watch::channel(targets(&["a"], &[]));
    let (times, mut calls) = mpsc::unbounded_channel();
    let start = Instant::now();
    let stream = poll_targets(receiver, move |_| {
        let _ = times.send(start.elapsed());
        async {
            Err(ClusterError::Rendered {
                message: "down".to_owned(),
            })
        }
    });
    let mut stream = pin!(stream);

    assert!(matches!(stream.next().await, Some(WatchUpdate::Failed(_))));
    assert!(matches!(stream.next().await, Some(WatchUpdate::Failed(_))));

    assert_eq!(calls.try_recv().ok(), Some(Duration::ZERO));
    assert_eq!(calls.try_recv().ok(), Some(Duration::from_secs(30)));
}

fn cadvisor_body() -> String {
    let labels = "container=\"app\",id=\"/kubepods/x\",namespace=\"shop\",pod=\"web-1\"";
    format!(
        "# HELP x y\ncontainer_fs_reads_bytes_total{{{labels}}} 10\ncontainer_fs_writes_bytes_total{{{labels}}} 20\n"
    )
}

#[tokio::test]
async fn cadvisor_body_is_read_line_by_line() {
    let body = cadvisor_body();
    let (sample, bytes) = parse_disk_io(Cursor::new(body.as_bytes()), CADVISOR_BYTE_LIMIT)
        .await
        .expect("sample");
    assert_eq!(sample.containers.len(), 1);
    assert_eq!(sample.containers[0].counters.read_bytes, 10);
    assert_eq!(sample.containers[0].counters.write_bytes, 20);
    assert_eq!(bytes, body.len() as u64);
}

#[tokio::test]
async fn cadvisor_body_over_the_limit_is_rejected() {
    let body = cadvisor_body();
    let exact = parse_disk_io(Cursor::new(body.as_bytes()), body.len() as u64).await;
    assert!(exact.is_ok());
    let over = parse_disk_io(Cursor::new(body.as_bytes()), body.len() as u64 - 1).await;
    assert_eq!(over.err(), Some(BODY_TOO_LARGE));
}

#[tokio::test]
async fn cadvisor_single_line_without_newline_is_capped() {
    let body = "x".repeat(1000);
    let result = parse_disk_io(Cursor::new(body.as_bytes()), 100).await;
    assert_eq!(result.err(), Some(BODY_TOO_LARGE));
}

#[tokio::test]
async fn cadvisor_invalid_utf8_is_unreadable() {
    let body: &[u8] = b"container_fs_reads_bytes_total{\xff} 1\n";
    let result = parse_disk_io(Cursor::new(body), CADVISOR_BYTE_LIMIT).await;
    assert_eq!(result.err(), Some(BODY_UNREADABLE));
}

#[test]
fn round_jobs_deduplicate_nodes_and_ignore_foreign_disk_nodes() {
    let jobs = round_jobs(targets(&["b", "a", "b"], &["b", "b", "z"]));
    assert_eq!(jobs, [("a".to_owned(), false), ("b".to_owned(), true)]);
}
