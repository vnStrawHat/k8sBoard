//! The Monitor tab's read of a Prometheus-compatible metrics source (spec 0048): what to query for a
//! subject, when to query again, and the charts and rows built from the answers. Pure: the fetch
//! itself runs in `app_shell_monitor_source.rs`.

use std::rc::Rc;
use std::time::{Duration, Instant};

use cluster::{
    MetricsError, MetricsSource, UsageMetric, UsageSeries as SourceSeries, UsageTarget,
    WorkloadKind,
};
use gpui_kit::Task;

use crate::drawer::{MonitorRange, MonitorScope};
use crate::kind_row::{PodOwner, owns_pod};
use crate::kubelet_history::{RateKind, RatePair};
use crate::monitor_data::{
    MonitorData, MonitorInput, MonitorRow, MonitorSubject, kubelet_chart, nearest_rate, read_rates,
    reference_lines, scope_choices, series_name,
};
use crate::monitor_notices::source_network_notice;
use crate::node_usage::takes_room;
use crate::table_selection::ClusterObject;
use crate::usage_chart::{ChartSeries, ReferenceLine, UsageChartModel};
use crate::usage_format::Measure;

/// Every metric a pod or workload view reads; one query each.
const ALL_METRICS: [UsageMetric; 6] = [
    UsageMetric::Cpu,
    UsageMetric::Memory,
    UsageMetric::NetworkReceive,
    UsageMetric::NetworkTransmit,
    UsageMetric::DiskRead,
    UsageMetric::DiskWrite,
];
/// A node's disk I/O stays on the kubelet feed (decision 17).
const NODE_METRICS: [UsageMetric; 4] = [
    UsageMetric::Cpu,
    UsageMetric::Memory,
    UsageMetric::NetworkReceive,
    UsageMetric::NetworkTransmit,
];

/// What a fetch was made for. An equal key with a fresh result needs no new fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceKey {
    pub(crate) subject: ClusterObject,
    pub(crate) container: Option<String>,
    pub(crate) scope: MonitorScope,
    pub(crate) range: MonitorRange,
    pub(crate) source: MetricsSource,
}

/// The answers of one fetch, with what the charts need besides them.
pub(crate) struct SourceResult {
    pub(crate) end: jiff::Timestamp,
    pub(crate) step: Duration,
    /// OOM kills the app knows of, for the markers (the source has no spec or status history).
    pub(crate) oom: Vec<jiff::Timestamp>,
    pub(crate) metrics: Vec<(UsageMetric, Result<SourceSeries, MetricsError>)>,
}

pub(crate) enum SourceView {
    Charts {
        data: MonitorData,
        /// A matrix had more than 64 series; the first 64 were summed.
        was_cut: bool,
    },
    /// CPU or Memory failed or is empty: the reason.
    Fallback(String),
}

/// The query of the shown Monitor and what it produced. Dropping it aborts the requests and the
/// refresh timer.
pub(crate) struct SourceFetch {
    pub(crate) key: SourceKey,
    pub(crate) started: Instant,
    /// `None` until the first fetch lands.
    pub(crate) view: Option<SourceView>,
    /// A refresh failed while `view` (an older answer) stands.
    pub(crate) last_failure: Option<String>,
    /// `Some` exactly while a fetch runs.
    pub(crate) _task: Option<Task<()>>,
    pub(crate) _refresh: Option<Task<()>>,
}

impl SourceFetch {
    /// A new key with nothing fetched yet.
    pub(crate) fn new(key: SourceKey) -> Self {
        Self {
            key,
            started: Instant::now(),
            view: None,
            last_failure: None,
            _task: None,
            _refresh: None,
        }
    }

    /// Whether a landed answer is older than the range refreshes after, and no fetch runs.
    pub(crate) fn is_due(&self) -> bool {
        self._task.is_none()
            && self.view.is_some()
            && self.started.elapsed() >= refresh_after(self.key.range)
    }
}

/// How long an answer stays fresh: the short ranges move, the long ones barely do.
pub(crate) fn refresh_after(range: MonitorRange) -> Duration {
    match range {
        MonitorRange::Minutes15 | MonitorRange::Hour1 => Duration::from_secs(30),
        MonitorRange::Hours6
        | MonitorRange::Hours24
        | MonitorRange::Days7
        | MonitorRange::Days30 => Duration::from_secs(300),
    }
}

/// The metrics a subject's fetch asks for.
pub(crate) fn source_metrics(target: &UsageTarget) -> &'static [UsageMetric] {
    match target {
        UsageTarget::Node { .. } => &NODE_METRICS,
        UsageTarget::Pod { .. } | UsageTarget::Workload { .. } => &ALL_METRICS,
    }
}

/// What the source is asked about for a subject and scope. `None` for a workload kind with no pod
/// name pattern (a CronJob's pods belong to its Jobs) and for a node's own pod list.
pub(crate) fn source_target(subject: &MonitorSubject, scope: &MonitorScope) -> Option<UsageTarget> {
    let part = match scope {
        MonitorScope::Total => None,
        MonitorScope::Part(name) => Some(name.clone()),
    };
    match subject {
        MonitorSubject::Pod(pod) => Some(UsageTarget::Pod {
            namespace: pod.namespace.clone(),
            pod: pod.name.clone(),
            container: part,
        }),
        MonitorSubject::Container { pod, container } => Some(UsageTarget::Pod {
            namespace: pod.namespace.clone(),
            pod: pod.name.clone(),
            container: Some((*container).to_owned()),
        }),
        MonitorSubject::Node(node) => Some(UsageTarget::Node {
            name: node.name.clone(),
        }),
        MonitorSubject::Workload(owner) => {
            let (namespace, kind, name) = workload_of(owner)?;
            match part {
                None => Some(UsageTarget::Workload {
                    namespace,
                    kind,
                    name,
                }),
                // One pod of the workload: the pod itself.
                Some(pod) => Some(UsageTarget::Pod {
                    namespace,
                    pod,
                    container: None,
                }),
            }
        }
    }
}

fn workload_of(owner: &PodOwner) -> Option<(String, WorkloadKind, String)> {
    match owner {
        PodOwner::Deployment { namespace, name } => {
            Some((namespace.clone(), WorkloadKind::Deployment, name.clone()))
        }
        PodOwner::Controller {
            namespace,
            kind,
            name,
        } => {
            let kind = match *kind {
                "StatefulSet" => WorkloadKind::StatefulSet,
                "DaemonSet" => WorkloadKind::DaemonSet,
                "ReplicaSet" => WorkloadKind::ReplicaSet,
                "Job" => WorkloadKind::Job,
                _ => return None,
            };
            Some((namespace.clone(), kind, name.clone()))
        }
        PodOwner::Node { .. } => None,
    }
}

/// `15s`, `1m`, `30m`, `2h`: the step in its largest whole unit.
pub(crate) fn step_text(step: Duration) -> String {
    let seconds = step.as_secs();
    if seconds >= 3_600 && seconds.is_multiple_of(3_600) {
        format!("{}h", seconds / 3_600)
    } else if seconds >= 60 && seconds.is_multiple_of(60) {
        format!("{}m", seconds / 60)
    } else {
        format!("{seconds}s")
    }
}

/// The muted line above the sampler charts when a short range could not be served by the source.
pub(crate) fn fallback_note(reason: &str) -> String {
    format!("Metrics source: {reason}. Showing k8sBoard samples.")
}

type Points = Vec<(jiff::Timestamp, Option<f64>)>;

fn answer(
    result: &SourceResult,
    metric: UsageMetric,
) -> Option<&Result<SourceSeries, MetricsError>> {
    result
        .metrics
        .iter()
        .find(|(candidate, _)| *candidate == metric)
        .map(|(_, answer)| answer)
}

fn has_values(series: &SourceSeries) -> bool {
    series.points.iter().any(|(_, value)| value.is_some())
}

/// The charts and rows of a result, or why CPU and Memory (the two cards every view needs) cannot
/// be drawn.
pub(crate) fn source_monitor_data(input: &MonitorInput, result: &SourceResult) -> SourceView {
    let mut required = Vec::new();
    for (label, metric) in [("CPU", UsageMetric::Cpu), ("Memory", UsageMetric::Memory)] {
        match answer(result, metric) {
            Some(Ok(series)) if has_values(series) => required.push(series),
            Some(Ok(_)) => {
                return SourceView::Fallback(format!(
                    "the source has no {label} series for this subject"
                ));
            }
            Some(Err(error)) => return SourceView::Fallback(error.to_string()),
            None => return SourceView::Fallback("the query did not run".to_owned()),
        }
    }
    let (cpu, memory) = (required[0], required[1]);
    let choices = scope_choices(input);
    let scope = if choices.iter().any(|choice| choice.scope == *input.scope) {
        input.scope.clone()
    } else {
        MonitorScope::Total
    };
    let end = result.end;
    let start = end - jiff::SignedDuration::try_from(input.range.duration()).unwrap_or_default();
    let markers: Vec<jiff::Timestamp> = result
        .oom
        .iter()
        .copied()
        .filter(|at| *at >= start && *at <= end)
        .collect();
    let (cpu_lines, memory_lines) = reference_lines(input, &scope);
    let pod_count = match &input.subject {
        MonitorSubject::Workload(owner) => input
            .pods
            .iter()
            .filter(|pod| owns_pod(owner, pod) && takes_room(pod))
            .count(),
        _ => 1,
    };
    let name = series_name(input, &scope, pod_count);
    let frame = Frame {
        step: result.step,
        start,
        end,
    };
    let charts = vec![
        frame.chart(
            ("monitor-cpu", "CPU", Measure::Cpu),
            vec![(name.as_str(), cpu.points.clone())],
            Extras {
                references: cpu_lines,
                markers: markers.clone(),
                notice: None,
            },
        ),
        frame.chart(
            ("monitor-memory", "Memory", Measure::Bytes),
            vec![(name.as_str(), memory.points.clone())],
            Extras {
                references: memory_lines,
                markers: markers.clone(),
                notice: None,
            },
        ),
    ];
    let network = pair_chart(
        &frame,
        (
            "monitor-network",
            "Network",
            "network",
            ["receive", "transmit"],
        ),
        source_network_notice(input, &scope),
        [
            answer(result, UsageMetric::NetworkReceive),
            answer(result, UsageMetric::NetworkTransmit),
        ],
    );
    let is_node = matches!(input.subject, MonitorSubject::Node(_));
    let disk_rates = read_rates(input, &scope, RateKind::DiskIo, input.range.resolution());
    let disk = if is_node {
        // Device-mapper aliases make a node sum from the source count one disk twice.
        kubelet_chart(input, &scope, RateKind::DiskIo, &disk_rates, (start, end))
    } else {
        pair_chart(
            &frame,
            ("monitor-disk", "Disk I/O", "disk I/O", ["read", "write"]),
            None,
            [
                answer(result, UsageMetric::DiskRead),
                answer(result, UsageMetric::DiskWrite),
            ],
        )
    };
    let pair_at = |first: UsageMetric, second: UsageMetric, index: usize| {
        let value = |metric| match answer(result, metric) {
            Some(Ok(series)) => series.points.get(index).and_then(|(_, value)| *value),
            _ => None,
        };
        Some(RatePair {
            first: value(first)?,
            second: value(second)?,
        })
    };
    let half_step = result.step / 2;
    let rows = cpu
        .points
        .iter()
        .enumerate()
        .rev()
        .map(|(index, (at, cpu_value))| MonitorRow {
            offset: end.duration_since(*at).as_secs().max(0).unsigned_abs(),
            cpu: *cpu_value,
            memory: memory.points.get(index).and_then(|(_, value)| *value),
            network: pair_at(
                UsageMetric::NetworkReceive,
                UsageMetric::NetworkTransmit,
                index,
            ),
            disk: if is_node {
                nearest_rate(&disk_rates, *at)
            } else {
                pair_at(UsageMetric::DiskRead, UsageMetric::DiskWrite, index)
            },
            is_oom: markers
                .iter()
                .any(|mark| mark.duration_since(*at).unsigned_abs() <= half_step),
        })
        .collect();
    let was_cut = result
        .metrics
        .iter()
        .any(|(_, answer)| answer.as_ref().is_ok_and(|series| series.was_cut));
    SourceView::Charts {
        data: MonitorData {
            charts,
            kubelet_charts: vec![network, disk],
            rows,
            choices,
            stale_since: None,
            span: None,
            scope,
        },
        was_cut,
    }
}

/// The time frame every chart of one view shares.
struct Frame {
    step: Duration,
    start: jiff::Timestamp,
    end: jiff::Timestamp,
}

impl Frame {
    fn chart(
        &self,
        (id, title, unit): (&str, &str, Measure),
        series: Vec<(&str, Points)>,
        extras: Extras,
    ) -> Rc<UsageChartModel> {
        Rc::new(UsageChartModel {
            id: id.to_owned().into(),
            title: title.to_owned().into(),
            unit,
            step: self.step,
            start: self.start,
            end: self.end,
            series: series
                .into_iter()
                .map(|(name, points)| ChartSeries {
                    name: name.to_owned().into(),
                    points,
                })
                .collect(),
            references: extras.references,
            markers: extras.markers,
            notice: extras.notice.map(Into::into),
        })
    }
}

/// What a chart carries besides its series.
#[derive(Default)]
struct Extras {
    references: Vec<ReferenceLine>,
    markers: Vec<jiff::Timestamp>,
    notice: Option<String>,
}

/// A card of two rate series. With a `host_notice` (the 0011 rule: such counters are the node's) it
/// shows that notice and no data; a failed query names its error; a card with no value says the
/// source has no such series.
fn pair_chart(
    frame: &Frame,
    (id, title, what, names): (&str, &str, &str, [&str; 2]),
    host_notice: Option<String>,
    answers: [Option<&Result<SourceSeries, MetricsError>>; 2],
) -> Rc<UsageChartModel> {
    if let Some(notice) = host_notice {
        let empty = names.map(|name| (name, Vec::new()));
        let extras = Extras {
            notice: Some(notice),
            ..Extras::default()
        };
        return frame.chart((id, title, Measure::Rate), empty.into(), extras);
    }
    let error = answers.iter().find_map(|answer| match answer {
        Some(Err(error)) => Some(error.to_string()),
        _ => None,
    });
    let series: Vec<(&str, Points)> = names
        .into_iter()
        .zip(answers)
        .map(|(name, answer)| {
            let points = match answer {
                Some(Ok(series)) => series.points.clone(),
                _ => Vec::new(),
            };
            (name, points)
        })
        .collect();
    let has_value = series
        .iter()
        .any(|(_, points)| points.iter().any(|(_, value)| value.is_some()));
    let notice = error.or_else(|| (!has_value).then(|| format!("No {what} series in the source")));
    let extras = Extras {
        notice,
        ..Extras::default()
    };
    frame.chart((id, title, Measure::Rate), series, extras)
}

/// The source of the `pod-monitor-source-fixture` screen: the UAT VictoriaMetrics service.
#[cfg(feature = "screenshot")]
pub(crate) fn fixture_source() -> Option<MetricsSource> {
    use cluster::{MetricsScheme, MetricsSourceFields};
    MetricsSource::new(&MetricsSourceFields {
        namespace: "monitoring".to_owned(),
        service: "vmselect-vm-victoria-metrics-k8s-stack".to_owned(),
        port: "8481".to_owned(),
        scheme: MetricsScheme::Http,
        prefix: "/select/0/prometheus".to_owned(),
    })
    .ok()
}

/// Synthetic answers for `spec`: a CPU wave, a memory sawtooth, and network and disk pairs.
#[cfg(feature = "screenshot")]
pub(crate) fn fixture_answers(
    spec: &cluster::RangeSpec,
) -> Vec<(UsageMetric, Result<SourceSeries, MetricsError>)> {
    let points = |value: &dyn Fn(f64) -> f64| -> SourceSeries {
        let step = i64::try_from(spec.step().as_secs()).unwrap_or(1);
        SourceSeries {
            points: (0..spec.points())
                .map(|index| {
                    let at = jiff::Timestamp::from_second(
                        spec.start().as_second() + step * i64::try_from(index).unwrap_or(0),
                    )
                    .unwrap_or(spec.end());
                    (at, Some(value(index as f64)))
                })
                .collect(),
            was_cut: false,
        }
    };
    let mib = 1_048_576.0;
    vec![
        (
            UsageMetric::Cpu,
            Ok(points(&|i| {
                0.32 + 0.22 * (i / 18.0).sin() + 0.05 * (i / 3.1).sin()
            })),
        ),
        (
            UsageMetric::Memory,
            Ok(points(&|i| (180.0 + (i % 90.0) * 3.0) * mib)),
        ),
        (
            UsageMetric::NetworkReceive,
            Ok(points(&|i| 24_000.0 + 9_000.0 * (i / 11.0).sin())),
        ),
        (
            UsageMetric::NetworkTransmit,
            Ok(points(&|i| 9_000.0 + 4_000.0 * (i / 7.0).cos())),
        ),
        (
            UsageMetric::DiskRead,
            Ok(points(&|i| 3_000.0 + 2_000.0 * (i / 23.0).sin().abs())),
        ),
        (
            UsageMetric::DiskWrite,
            Ok(points(&|i| 14_000.0 + 6_000.0 * (i / 13.0).sin())),
        ),
    ]
}

/// The one OOM kill of the fixture: where the memory sawtooth falls back the second time.
#[cfg(feature = "screenshot")]
pub(crate) fn fixture_oom(spec: &cluster::RangeSpec) -> jiff::Timestamp {
    let step = i64::try_from(spec.step().as_secs()).unwrap_or(1);
    jiff::Timestamp::from_second(spec.start().as_second() + step * 180).unwrap_or(spec.end())
}

#[cfg(test)]
#[path = "monitor_source_tests.rs"]
mod monitor_source_tests;
