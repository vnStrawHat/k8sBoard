use std::future::{self, Ready};
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use serde_json::json;
use tokio::time::Instant;

use super::*;

fn object(value: Value) -> DynamicObject {
    serde_json::from_value(value).expect("test object is a dynamic object")
}

fn api_error() -> ClusterError {
    ClusterError::Api {
        context: "ctx".to_owned(),
        action: POD_ACTION,
        code: 503,
        message: "unavailable".to_owned(),
    }
}

fn pod_json(containers: Value) -> Value {
    json!({
        "metadata": {
            "name": "web-1",
            "namespace": "shop",
            "creationTimestamp": "2026-10-02T09:59:50Z",
            "labels": {"app": "web"},
        },
        "timestamp": "2026-10-02T10:00:00Z",
        "window": "15.012s",
        "containers": containers,
    })
}

#[test]
fn pod_metrics_reads_metrics_server_json() {
    let pod = pod_metrics(&object(pod_json(json!([
        {"name": "app", "usage": {"cpu": "1234567n", "memory": "12345Ki"}},
        {"name": "sidecar", "usage": {"cpu": "2m", "memory": "3Mi"}},
    ]))))
    .expect("pod metrics");

    assert_eq!(pod.namespace, "shop");
    assert_eq!(pod.name, "web-1");
    assert_eq!(
        pod.sampled_at,
        Some("2026-10-02T10:00:00Z".parse().expect("timestamp"))
    );
    let names: Vec<_> = pod.containers.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["app", "sidecar"]);
    assert_eq!(
        pod.containers[0].usage,
        ResourceUsage {
            cpu: CpuAmount::from_nanocores(1_234_567),
            memory: ByteAmount::from_bytes(12_641_280),
        }
    );
    assert_eq!(pod.containers[1].usage.cpu.nanocores(), 2_000_000);
}

#[test]
fn pod_metrics_skips_unparsable_containers() {
    let pod = pod_metrics(&object(pod_json(json!([
        {"name": "no-memory", "usage": {"cpu": "1m"}},
        {"name": "bad-text", "usage": {"cpu": "1m", "memory": "lots"}},
        {"usage": {"cpu": "1m", "memory": "1Mi"}},
        {"name": "kept", "usage": {"cpu": "1m", "memory": "1Mi"}},
    ]))))
    .expect("pod metrics");
    let names: Vec<_> = pod.containers.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["kept"]);
}

#[test]
fn pod_metrics_without_containers_is_none() {
    assert_eq!(pod_metrics(&object(pod_json(json!([])))), None);
    let unreadable = json!([{"name": "a", "usage": {"cpu": "x", "memory": "1Mi"}}]);
    assert_eq!(pod_metrics(&object(pod_json(unreadable))), None);
}

#[test]
fn node_metrics_reads_usage_and_timestamp() {
    let node = node_metrics(&object(json!({
        "metadata": {"name": "worker-1"},
        "timestamp": "2026-10-02T10:00:05Z",
        "window": "20s",
        "usage": {"cpu": "310m", "memory": "8Gi"},
    })))
    .expect("node metrics");
    assert_eq!(node.name, "worker-1");
    assert_eq!(
        node.sampled_at,
        Some("2026-10-02T10:00:05Z".parse().expect("timestamp"))
    );
    assert_eq!(node.usage.cpu.nanocores(), 310_000_000);
    assert_eq!(node.usage.memory.bytes(), 8 << 30);

    let bad_time = node_metrics(&object(json!({
        "metadata": {"name": "worker-2"},
        "timestamp": "yesterday",
        "usage": {"cpu": "1", "memory": "1Ki"},
    })))
    .expect("usage is kept");
    assert_eq!(bad_time.sampled_at, None);
    assert_eq!(bad_time.usage.cpu.nanocores(), 1_000_000_000);
}

#[test]
fn concat_namespaces_keeps_namespace_order() {
    let merged = concat_namespaces(
        vec![
            (Some("a".to_owned()), Ok(vec!["a1", "a2"])),
            (Some("b".to_owned()), Ok(vec!["b1"])),
        ],
        2,
    );
    assert_eq!(merged.expect("merged"), ["a1", "a2", "b1"]);
}

#[test]
fn concat_namespaces_wraps_the_failing_namespace() {
    let several = concat_namespaces(
        vec![
            (Some("a".to_owned()), Ok(vec![1])),
            (Some("b".to_owned()), Err(api_error())),
        ],
        3,
    );
    let Err(ClusterError::Namespace { namespace, source }) = several else {
        panic!("a failing namespace among several is named");
    };
    assert_eq!(namespace, "b");
    assert!(matches!(*source, ClusterError::Api { code: 503, .. }));

    let single = concat_namespaces::<u32>(vec![(None, Err(api_error()))], 1);
    assert!(matches!(single, Err(ClusterError::Api { code: 503, .. })));
}

#[test]
fn next_delay_backs_off_and_caps() {
    let seconds: Vec<_> = (0..5)
        .map(|failures| next_delay(failures).as_secs())
        .collect();
    assert_eq!(seconds, [15, 30, 60, 120, 120]);
}

type FetchTimes = Arc<Mutex<Vec<Duration>>>;

/// A fetch that fails when the next entry of `outcomes` is `false`, and records when each
/// call happened, relative to the creation of the script.
fn scripted_fetch(
    outcomes: Vec<bool>,
) -> (
    impl FnMut() -> Ready<Result<Vec<usize>, ClusterError>> + Send + 'static,
    FetchTimes,
) {
    let times = FetchTimes::default();
    let recorded = Arc::clone(&times);
    let start = Instant::now();
    let mut calls = 0;
    let fetch = move || {
        if let Ok(mut times) = recorded.lock() {
            times.push(start.elapsed());
        }
        let is_ok = outcomes.get(calls).copied().unwrap_or(true);
        calls += 1;
        future::ready(if is_ok {
            Ok(vec![calls])
        } else {
            Err(api_error())
        })
    };
    (fetch, times)
}

fn fetch_seconds(times: &FetchTimes) -> Vec<u64> {
    let times = times.lock().expect("times lock");
    times.iter().map(Duration::as_secs).collect()
}

#[tokio::test(start_paused = true)]
async fn poll_fetches_at_once_then_every_interval() {
    let (fetch, times) = scripted_fetch(vec![]);
    let mut updates = Box::pin(poll_updates(fetch));
    for _ in 0..3 {
        assert!(matches!(
            updates.next().await,
            Some(WatchUpdate::Snapshot(_))
        ));
    }
    assert_eq!(fetch_seconds(&times), [0, 15, 30]);
}

#[tokio::test(start_paused = true)]
async fn poll_backs_off_then_resets_on_success() {
    let (fetch, times) = scripted_fetch(vec![false, false, true, true]);
    let mut updates = Box::pin(poll_updates(fetch));
    for _ in 0..4 {
        updates.next().await;
    }
    // Waits of 30 s, 60 s, then back to 15 s.
    assert_eq!(fetch_seconds(&times), [0, 30, 90, 105]);
}

#[tokio::test(start_paused = true)]
async fn poll_reports_failures_and_keeps_polling() {
    let (fetch, _) = scripted_fetch(vec![false, true]);
    let mut updates = Box::pin(poll_updates(fetch));
    assert!(matches!(updates.next().await, Some(WatchUpdate::Failed(_))));
    let Some(WatchUpdate::Snapshot(items)) = updates.next().await else {
        panic!("the poll continues after a failure");
    };
    assert_eq!(items, [2]);
}

#[test]
fn concat_namespaces_names_the_failure_when_listing_stopped_early() {
    // Listing stops at the first failure, so only the failing namespace reached `results`.
    let stopped = concat_namespaces::<u32>(vec![(Some("a".to_owned()), Err(api_error()))], 3);
    assert!(matches!(
        stopped,
        Err(ClusterError::Namespace { namespace, .. }) if namespace == "a"
    ));
}
