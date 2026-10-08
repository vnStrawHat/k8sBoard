use cluster::{
    ControllerRef, MetricsScheme, MetricsSourceFields, NodeReadiness, NodeScheduling, NodeStatus,
    NodeSummary, NodeSystemInfo, PodStatus, PodSummary, ReadyCount, StatusReason,
};

use super::*;
use crate::kubelet_metrics::KubeletFeed;
use crate::metrics_history::{NodeUsageHistory, PodUsageHistory};
use crate::table_selection::ResourceKey;

const STEP: Duration = Duration::from_secs(1_800);

fn at(seconds: i64) -> jiff::Timestamp {
    jiff::Timestamp::from_second(seconds).expect("valid timestamp")
}

fn source() -> MetricsSource {
    MetricsSource::new(&MetricsSourceFields {
        namespace: "monitoring".to_owned(),
        service: "vmselect".to_owned(),
        port: "8481".to_owned(),
        scheme: MetricsScheme::Http,
        prefix: "/select/0/prometheus".to_owned(),
    })
    .expect("valid source")
}

fn pod(name: &str, controller: Option<(&str, &str)>, host_network: bool) -> PodSummary {
    PodSummary {
        annotations: cluster::AnnotationTerms::default(),
        is_finished: false,
        namespace: "shop".to_owned(),
        name: name.to_owned(),
        status: PodStatus::Reason(StatusReason::Running),
        ready: ReadyCount { ready: 1, total: 1 },
        restarts: 0,
        node_name: Some("worker-1".to_owned()),
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
        host_network,
        image_pull_secrets: Vec::new(),
        node_selector: Vec::new(),
        node_affinity: Vec::new(),
        containers: Vec::new(),
    }
}

/// A series of `values` from `at(1_000_000)`, one per `STEP`.
fn series(values: &[Option<f64>]) -> SourceSeries {
    SourceSeries {
        points: values
            .iter()
            .enumerate()
            .map(|(index, value)| (at(1_000_000 + 1_800 * index as i64), *value))
            .collect(),
        was_cut: false,
    }
}

fn full(value: f64) -> SourceSeries {
    series(&[Some(value), Some(value * 2.0), Some(value * 3.0)])
}

fn result(metrics: Vec<(UsageMetric, Result<SourceSeries, MetricsError>)>) -> SourceResult {
    SourceResult {
        end: at(1_000_000 + 3_600),
        step: STEP,
        oom: Vec::new(),
        metrics,
    }
}

fn all_ok() -> Vec<(UsageMetric, Result<SourceSeries, MetricsError>)> {
    vec![
        (UsageMetric::Cpu, Ok(full(1.0))),
        (UsageMetric::Memory, Ok(full(100.0))),
        (UsageMetric::NetworkReceive, Ok(full(10.0))),
        (UsageMetric::NetworkTransmit, Ok(full(20.0))),
        (UsageMetric::DiskRead, Ok(full(30.0))),
        (UsageMetric::DiskWrite, Ok(full(40.0))),
    ]
}

fn view_of(subject: MonitorSubject<'_>, pods: &[PodSummary], result: &SourceResult) -> SourceView {
    view_in_range(MonitorRange::Hours6, subject, pods, result)
}

fn view_in_range(
    range: MonitorRange,
    subject: MonitorSubject<'_>,
    pods: &[PodSummary],
    result: &SourceResult,
) -> SourceView {
    let (pod_history, node_history, kubelet) = (
        PodUsageHistory::default(),
        NodeUsageHistory::default(),
        KubeletFeed::new(),
    );
    let scope = MonitorScope::Total;
    source_monitor_data(
        &MonitorInput {
            subject,
            scope: &scope,
            range,
            pods,
            pod_history: &pod_history,
            node_history: &node_history,
            kubelet: &kubelet,
            nodes: &[],
            is_all_namespaces: true,
        },
        result,
    )
}

fn charts_of(view: SourceView) -> (MonitorData, bool) {
    match view {
        SourceView::Charts { data, was_cut } => (data, was_cut),
        SourceView::Fallback(reason) => panic!("expected charts, got a fallback: {reason}"),
    }
}

fn node(name: &str) -> NodeSummary {
    NodeSummary {
        annotations: cluster::AnnotationTerms::default(),
        name: name.to_owned(),
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
        resources: Vec::new(),
        labels: Vec::new(),
    }
}

#[test]
fn source_target_per_subject_and_scope() {
    let api = pod("api-1", Some(("ReplicaSet", "api-7d9f8c")), false);
    let total = MonitorScope::Total;
    let part = |name: &str| MonitorScope::Part(name.to_owned());
    let pod_target = |container: Option<&str>| UsageTarget::Pod {
        namespace: "shop".to_owned(),
        pod: "api-1".to_owned(),
        container: container.map(str::to_owned),
    };
    assert_eq!(
        source_target(&MonitorSubject::Pod(&api), &total),
        Some(pod_target(None))
    );
    assert_eq!(
        source_target(&MonitorSubject::Pod(&api), &part("app")),
        Some(pod_target(Some("app")))
    );
    assert_eq!(
        source_target(
            &MonitorSubject::Container {
                pod: &api,
                container: "side"
            },
            &total
        ),
        Some(pod_target(Some("side")))
    );
    let deployment = PodOwner::Deployment {
        namespace: "shop".to_owned(),
        name: "api".to_owned(),
    };
    assert_eq!(
        source_target(&MonitorSubject::Workload(&deployment), &total),
        Some(UsageTarget::Workload {
            namespace: "shop".to_owned(),
            kind: WorkloadKind::Deployment,
            name: "api".to_owned()
        })
    );
    assert_eq!(
        source_target(&MonitorSubject::Workload(&deployment), &part("api-1")),
        Some(pod_target(None))
    );
    let stateful = PodOwner::Controller {
        namespace: "shop".to_owned(),
        kind: "StatefulSet",
        name: "db".to_owned(),
    };
    assert!(matches!(
        source_target(&MonitorSubject::Workload(&stateful), &total),
        Some(UsageTarget::Workload {
            kind: WorkloadKind::StatefulSet,
            ..
        })
    ));
    let cron = PodOwner::Controller {
        namespace: "shop".to_owned(),
        kind: "CronJob",
        name: "nightly".to_owned(),
    };
    assert_eq!(
        source_target(&MonitorSubject::Workload(&cron), &total),
        None
    );
    let worker = node("worker-1");
    assert_eq!(
        source_target(&MonitorSubject::Node(&worker), &total),
        Some(UsageTarget::Node {
            name: "worker-1".to_owned()
        })
    );
    assert_eq!(
        source_target(
            &MonitorSubject::Workload(&PodOwner::Node {
                name: "worker-1".to_owned()
            }),
            &total
        ),
        None
    );
}

#[test]
fn a_node_asks_no_disk_metric() {
    let node = UsageTarget::Node {
        name: "worker-1".to_owned(),
    };
    assert_eq!(source_metrics(&node).len(), 4);
    assert!(!source_metrics(&node).contains(&UsageMetric::DiskRead));
    let pod = UsageTarget::Pod {
        namespace: "shop".to_owned(),
        pod: "api-1".to_owned(),
        container: None,
    };
    assert_eq!(source_metrics(&pod).len(), 6);
}

#[test]
fn ranges_follow_the_source() {
    assert_eq!(MonitorRange::SOURCE[5], MonitorRange::Days30);
    assert_eq!(MonitorRange::SOURCE[..4], MonitorRange::SAMPLER);
}

#[test]
fn refresh_after_per_range() {
    assert_eq!(
        refresh_after(MonitorRange::Minutes15),
        Duration::from_secs(30)
    );
    assert_eq!(refresh_after(MonitorRange::Hour1), Duration::from_secs(30));
    for range in [
        MonitorRange::Hours6,
        MonitorRange::Hours24,
        MonitorRange::Days7,
        MonitorRange::Days30,
    ] {
        assert_eq!(refresh_after(range), Duration::from_secs(300), "{range:?}");
    }
}

fn key() -> SourceKey {
    SourceKey {
        subject: ClusterObject::new(
            crate::cluster_registry::ClusterRef {
                kubeconfig: std::path::PathBuf::from("a.yaml"),
                context: "ctx".to_owned(),
            },
            ResourceKey::Pod {
                namespace: "shop".to_owned(),
                name: "api-1".to_owned(),
            },
        ),
        container: None,
        scope: MonitorScope::Total,
        range: MonitorRange::Days30,
        source: source(),
    }
}

#[test]
fn a_fresh_answer_is_not_due_and_a_stale_one_is() {
    let mut fetch = SourceFetch::new(key());
    assert!(!fetch.is_due(), "nothing has landed yet");
    fetch.view = Some(SourceView::Fallback("x".to_owned()));
    assert!(!fetch.is_due());
    let Some(long_ago) = Instant::now().checked_sub(Duration::from_secs(301)) else {
        return;
    };
    fetch.started = long_ago;
    assert!(fetch.is_due());
    fetch._task = Some(Task::ready(()));
    assert!(!fetch.is_due(), "a running fetch is not started again");
}

#[test]
fn step_text_uses_the_largest_whole_unit() {
    for (seconds, text) in [
        (15, "15s"),
        (60, "1m"),
        (300, "5m"),
        (1_800, "30m"),
        (7_200, "2h"),
        (90, "90s"),
    ] {
        assert_eq!(step_text(Duration::from_secs(seconds)), text);
    }
}

#[test]
fn a_full_answer_builds_four_cards() {
    let api = pod("api-1", None, false);
    let (data, was_cut) = charts_of(view_of(MonitorSubject::Pod(&api), &[], &result(all_ok())));
    assert!(!was_cut);
    let ids: Vec<&str> = data
        .charts
        .iter()
        .chain(&data.kubelet_charts)
        .map(|chart| chart.id.as_ref())
        .collect();
    assert_eq!(
        ids,
        [
            "monitor-cpu",
            "monitor-memory",
            "monitor-network",
            "monitor-disk"
        ]
    );
    assert!(
        data.kubelet_charts
            .iter()
            .all(|chart| chart.notice.is_none())
    );
    assert_eq!(data.charts[0].step, STEP);
    assert_eq!(data.charts[0].series[0].name.as_ref(), "pod total");
}

#[test]
fn the_oom_marker_is_on_the_memory_chart() {
    let api = pod("api-1", None, false);
    let mut answers = result(all_ok());
    // 600 s after the middle point of the range.
    answers.oom = vec![at(1_000_000 + 1_800 + 600)];
    let (data, _) = charts_of(view_of(MonitorSubject::Pod(&api), &[], &answers));
    assert_eq!(
        data.charts[1].markers, answers.oom,
        "the marker is on the memory chart"
    );
}

#[test]
fn an_oom_outside_the_range_is_dropped() {
    let api = pod("api-1", None, false);
    let mut answers = result(all_ok());
    answers.oom = vec![at(1_000_000 - 10 * 86_400)];
    let (data, _) = charts_of(view_of(MonitorSubject::Pod(&api), &[], &answers));
    assert!(data.charts[1].markers.is_empty());
}

#[test]
fn empty_cpu_falls_back() {
    let api = pod("api-1", None, false);
    let mut answers = all_ok();
    answers[0].1 = Ok(series(&[None, None, None]));
    match view_of(MonitorSubject::Pod(&api), &[], &result(answers)) {
        SourceView::Fallback(reason) => {
            assert_eq!(reason, "the source has no CPU series for this subject");
        }
        SourceView::Charts { .. } => panic!("an empty CPU answer cannot draw"),
    }
}

#[test]
fn a_failed_memory_query_falls_back_with_its_error() {
    let api = pod("api-1", None, false);
    let mut answers = all_ok();
    answers[1].1 = Err(MetricsError::TimedOut);
    match view_of(MonitorSubject::Pod(&api), &[], &result(answers)) {
        SourceView::Fallback(reason) => {
            assert_eq!(reason, "the metrics backend did not answer within 20 s");
        }
        SourceView::Charts { .. } => panic!("a failed Memory query cannot draw"),
    }
}

#[test]
fn a_missing_answer_falls_back() {
    let api = pod("api-1", None, false);
    let answers = result(Vec::new());
    assert!(matches!(
        view_of(MonitorSubject::Pod(&api), &[], &answers),
        SourceView::Fallback(_)
    ));
}

#[test]
fn failed_network_keeps_other_charts() {
    let api = pod("api-1", None, false);
    let mut answers = all_ok();
    answers[2].1 = Err(MetricsError::NoApiAtPrefix);
    let (data, _) = charts_of(view_of(MonitorSubject::Pod(&api), &[], &result(answers)));
    assert_eq!(data.charts.len(), 2);
    assert_eq!(
        data.kubelet_charts[0].notice.as_deref(),
        Some("nothing answers at this path prefix; check the prefix and port")
    );
    assert!(data.kubelet_charts[1].notice.is_none(), "disk is untouched");
    assert!(data.charts.iter().all(|chart| chart.notice.is_none()));
}

#[test]
fn a_card_without_values_says_the_source_has_none() {
    let api = pod("api-1", None, false);
    let mut answers = all_ok();
    answers[4].1 = Ok(series(&[None, None, None]));
    answers[5].1 = Ok(series(&[None, None, None]));
    let (data, _) = charts_of(view_of(MonitorSubject::Pod(&api), &[], &result(answers)));
    assert_eq!(
        data.kubelet_charts[1].notice.as_deref(),
        Some("No disk I/O series in the source")
    );
    assert!(data.kubelet_charts[0].notice.is_none());
}

#[test]
fn host_network_pod_shows_the_network_notice() {
    let host = pod("agent-1", None, true);
    let (data, _) = charts_of(view_of(MonitorSubject::Pod(&host), &[], &result(all_ok())));
    let network = &data.kubelet_charts[0];
    assert_eq!(
        network.notice.as_deref(),
        Some("Host network: traffic is the node's (see the node's Monitor tab)")
    );
    assert!(
        network.series.iter().all(|series| series.points.is_empty()),
        "no source data behind the notice"
    );
    assert!(
        data.kubelet_charts[1].notice.is_none(),
        "other cards keep the source"
    );
    assert!(!data.charts[0].series[0].points.is_empty());
}

#[test]
fn a_workload_with_a_host_network_pod_shows_the_network_notice() {
    let owner = PodOwner::Controller {
        namespace: "shop".to_owned(),
        kind: "DaemonSet",
        name: "agent".to_owned(),
    };
    let pods = [
        pod("agent-aaaaa", Some(("DaemonSet", "agent")), true),
        pod("agent-bbbbb", Some(("DaemonSet", "agent")), false),
    ];
    let (data, _) = charts_of(view_of(
        MonitorSubject::Workload(&owner),
        &pods,
        &result(all_ok()),
    ));
    assert_eq!(
        data.kubelet_charts[0].notice.as_deref(),
        Some("1 host-network pod not counted")
    );
    assert_eq!(data.charts[0].series[0].name.as_ref(), "2 pods");
}

#[test]
fn cut_series_are_reported() {
    let api = pod("api-1", None, false);
    let mut answers = all_ok();
    if let Ok(series) = &mut answers[0].1 {
        series.was_cut = true;
    }
    let (_, was_cut) = charts_of(view_of(MonitorSubject::Pod(&api), &[], &result(answers)));
    assert!(was_cut);
}

#[test]
fn a_node_takes_its_disk_card_from_the_kubelet_feed() {
    let worker = node("worker-1");
    let mut answers = all_ok();
    answers.truncate(4);
    let (data, _) = charts_of(view_of(
        MonitorSubject::Node(&worker),
        &[],
        &result(answers),
    ));
    let disk = &data.kubelet_charts[1];
    assert_eq!(disk.id.as_ref(), "monitor-disk");
    assert_eq!(
        disk.notice.as_deref(),
        Some("Collecting… rates need two samples"),
        "no source disk series: the kubelet feed has not sampled"
    );
    assert_eq!(data.charts[0].series[0].name.as_ref(), "used");
}

#[test]
fn an_unknown_scope_falls_back_to_the_total() {
    let api = pod("api-1", None, false);
    let (pod_history, node_history, kubelet) = (
        PodUsageHistory::default(),
        NodeUsageHistory::default(),
        KubeletFeed::new(),
    );
    let gone = MonitorScope::Part("gone".to_owned());
    let view = source_monitor_data(
        &MonitorInput {
            subject: MonitorSubject::Pod(&api),
            scope: &gone,
            range: MonitorRange::Hours6,
            pods: &[],
            pod_history: &pod_history,
            node_history: &node_history,
            kubelet: &kubelet,
            nodes: &[],
            is_all_namespaces: true,
        },
        &result(all_ok()),
    );
    let (data, _) = charts_of(view);
    assert_eq!(data.scope, MonitorScope::Total);
}

#[test]
fn the_node_disk_card_follows_the_range_resolution() {
    let worker = node("worker-1");
    let mut answers = all_ok();
    answers.truncate(4);
    for (range, step) in [
        (MonitorRange::Minutes15, Duration::from_secs(15)),
        (MonitorRange::Hour1, Duration::from_secs(15)),
        (MonitorRange::Hours6, Duration::from_secs(300)),
        (MonitorRange::Days30, Duration::from_secs(300)),
    ] {
        let view = view_in_range(
            range,
            MonitorSubject::Node(&worker),
            &[],
            &result(answers.clone()),
        );
        let (data, _) = charts_of(view);
        assert_eq!(data.kubelet_charts[1].step, step, "{range:?}");
    }
}

#[test]
fn fallback_note_names_the_source() {
    assert_eq!(
        fallback_note("the metrics backend did not answer within 20 s"),
        "Metrics source: the metrics backend did not answer within 20 s. Showing k8sBoard samples."
    );
}
