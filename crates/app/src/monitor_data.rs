//! What the Monitor tab shows for one subject: the series, the request and limit lines, and the
//! OOM kills. Pure: it reads the histories and the pods list it is given, so tests need no
//! session.

use std::rc::Rc;
use std::time::Duration;

use cluster::{
    ByteAmount, ContainerKind, ContainerResource, ContainerSummary, CpuAmount, NodeSummary,
    PodSummary, ResourceUsage,
};

use crate::drawer::{MonitorRange, MonitorScope};
use crate::history_rings::{Resolution, Timelines};
use crate::kind_row::{PodOwner, owns_pod};
use crate::kubelet_history::{RateKind, RatePair, RateSeries};
use crate::kubelet_metrics::KubeletFeed;
use crate::metrics_history::{NodeUsageHistory, PodUsageHistory, UsageSeries};
use crate::monitor_notices::{COLLECTING, kubelet_notice};
use crate::node_usage::{node_allocatable, node_requests, takes_room};
use crate::usage_chart::{ChartSeries, ReferenceKind, ReferenceLine, UsageChartModel};
use crate::usage_format::Measure;

/// A server timestamp that has not advanced for this many polls in a row reads as a stalled
/// metrics-server. Counting polls keeps it independent of the clock and the range.
const STALE_AFTER_TICKS: u32 = 4;
/// Requests and limits this close (a share of the limit) count as equal: a Guaranteed pod.
const EQUAL_SHARE: f64 = 0.005;

pub(crate) enum MonitorSubject<'a> {
    Pod(&'a PodSummary),
    Container {
        pod: &'a PodSummary,
        container: &'a str,
    },
    Node(&'a NodeSummary),
    Workload(&'a PodOwner),
}

/// Plain data, so tests need no session; the caller fills it from `LiveCluster`.
pub(crate) struct MonitorInput<'a> {
    pub(crate) subject: MonitorSubject<'a>,
    pub(crate) scope: &'a MonitorScope,
    pub(crate) range: MonitorRange,
    pub(crate) pods: &'a [PodSummary],
    pub(crate) pod_history: &'a PodUsageHistory,
    pub(crate) node_history: &'a NodeUsageHistory,
    pub(crate) kubelet: &'a KubeletFeed,
    pub(crate) nodes: &'a [NodeSummary],
    pub(crate) is_all_namespaces: bool,
}

pub(crate) struct MonitorData {
    /// CPU, then Memory; none before the metrics feed has a sample.
    pub(crate) charts: Vec<Rc<UsageChartModel>>,
    /// Network, then Disk I/O.
    pub(crate) kubelet_charts: Vec<Rc<UsageChartModel>>,
    pub(crate) choices: Vec<ScopeChoice>,
    /// When the server last sampled the series, once that has not changed for four polls.
    pub(crate) stale_since: Option<jiff::Timestamp>,
    /// How much history the app has; `None` before the first tick.
    pub(crate) span: Option<Duration>,
    /// The scope the charts show: the asked one, or `Total` when it is no longer offered.
    pub(crate) scope: MonitorScope,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScopeChoice {
    pub(crate) label: String,
    pub(crate) scope: MonitorScope,
}

impl MonitorData {
    /// Whether the app has less history than `range` asks for: the data starts when it connected.
    pub(crate) fn is_short_for(&self, range: MonitorRange) -> bool {
        let step = Timelines::step(range.resolution());
        // A history one step short still covers the range: the newest tick sits on the right edge.
        self.span.is_none_or(|span| span + step < range.duration())
    }
}

pub(crate) fn monitor_data(input: &MonitorInput) -> MonitorData {
    let choices = scope_choices(input);
    let scope = if choices.iter().any(|choice| choice.scope == *input.scope) {
        input.scope.clone()
    } else {
        MonitorScope::Total
    };
    let resolution = input.range.resolution();
    let (series, history_span, newest) = read_series(input, &scope, resolution);
    let kubelet = &input.kubelet.history;
    // CPU and Memory end at the metrics feed's newest tick; the kubelet cards share that end so
    // the four line up, and use their own newest tick when the metrics feed has none.
    let metrics_end = newest.or_else(|| series.points.last().map(|(at, _)| *at));
    let end = metrics_end
        .or_else(|| kubelet.newest_tick())
        .unwrap_or_else(jiff::Timestamp::now);
    let duration = input.range.duration();
    let start = end - jiff::SignedDuration::try_from(duration).unwrap_or_default();
    let network = read_rates(input, &scope, RateKind::Network, resolution);
    let disk = read_rates(input, &scope, RateKind::DiskIo, resolution);
    let kubelet_charts = vec![
        kubelet_chart(input, &scope, RateKind::Network, &network, (start, end)),
        kubelet_chart(input, &scope, RateKind::DiskIo, &disk, (start, end)),
    ];
    let span = history_span.or_else(|| kubelet.span());
    if metrics_end.is_none() {
        return MonitorData {
            charts: Vec::new(),
            kubelet_charts,
            choices,
            stale_since: None,
            span,
            scope,
        };
    }
    let kept: Vec<(jiff::Timestamp, Option<ResourceUsage>)> = series
        .points
        .iter()
        .filter(|(at, _)| *at >= start)
        .copied()
        .collect();
    let markers: Vec<jiff::Timestamp> = series
        .oom
        .iter()
        .copied()
        .filter(|at| *at >= start && *at <= end)
        .collect();
    let (cpu_lines, memory_lines) = reference_lines(input, &scope);
    let name = series_name(input, &scope, series.pod_count);
    let chart = |id: &str, title: &str, unit: Measure, value: fn(ResourceUsage) -> f64, lines| {
        Rc::new(UsageChartModel {
            id: id.to_owned().into(),
            title: title.to_owned().into(),
            unit,
            step: series.step,
            start,
            end,
            series: vec![ChartSeries {
                name: name.clone().into(),
                points: kept
                    .iter()
                    .map(|(at, usage)| (*at, usage.map(value)))
                    .collect(),
            }],
            references: lines,
            markers: markers.clone(),
            notice: None,
        })
    };
    let charts = vec![
        chart(
            "monitor-cpu",
            "CPU",
            Measure::Cpu,
            |usage| usage.cpu.cores(),
            cpu_lines,
        ),
        chart(
            "monitor-memory",
            "Memory",
            Measure::Bytes,
            |usage| usage.memory.bytes() as f64,
            memory_lines,
        ),
    ];
    MonitorData {
        charts,
        kubelet_charts,
        choices,
        stale_since: series
            .sampled_at
            .filter(|_| series.stalled_ticks >= STALE_AFTER_TICKS),
        span,
        scope,
    }
}

/// The kubelet rates of the subject and scope.
pub(crate) fn read_rates(
    input: &MonitorInput,
    scope: &MonitorScope,
    kind: RateKind,
    resolution: Resolution,
) -> RateSeries {
    let history = &input.kubelet.history;
    let part = match scope {
        MonitorScope::Total => None,
        MonitorScope::Part(name) => Some(name.as_str()),
    };
    match &input.subject {
        MonitorSubject::Pod(pod) => {
            history.pod_rates(kind, &pod.namespace, &pod.name, part, resolution)
        }
        MonitorSubject::Container { pod, container } => {
            history.pod_rates(kind, &pod.namespace, &pod.name, Some(container), resolution)
        }
        MonitorSubject::Workload(owner) => history.owner_rates(kind, owner, part, resolution),
        MonitorSubject::Node(node) => history.node_rates(kind, &node.name, resolution),
    }
}

/// One kubelet card: two rate series in the window, and why it may be empty.
pub(crate) fn kubelet_chart(
    input: &MonitorInput,
    scope: &MonitorScope,
    kind: RateKind,
    rates: &RateSeries,
    (start, end): (jiff::Timestamp, jiff::Timestamp),
) -> Rc<UsageChartModel> {
    let (id, title, names) = match kind {
        RateKind::Network => ("monitor-network", "Network", ["receive", "transmit"]),
        RateKind::DiskIo => ("monitor-disk", "Disk I/O", ["read", "write"]),
    };
    let points = |field: fn(&RatePair<u64>) -> u64| -> Vec<(jiff::Timestamp, Option<f64>)> {
        rates
            .points
            .iter()
            .filter(|(at, _)| *at >= start && *at <= end)
            .map(|(at, rate)| (*at, rate.as_ref().map(|rate| field(rate) as f64)))
            .collect()
    };
    let has_rate = rates
        .points
        .iter()
        .any(|(at, rate)| *at >= start && *at <= end && rate.is_some());
    // Nothing to draw and nothing else to say: the first rate is still to come.
    let notice =
        kubelet_notice(input, scope, kind).or_else(|| (!has_rate).then(|| COLLECTING.to_owned()));
    Rc::new(UsageChartModel {
        id: id.into(),
        title: title.into(),
        unit: Measure::Rate,
        step: rates.step,
        start,
        end,
        series: vec![
            ChartSeries {
                name: names[0].into(),
                points: points(|rate| rate.first),
            },
            ChartSeries {
                name: names[1].into(),
                points: points(|rate| rate.second),
            },
        ],
        references: Vec::new(),
        markers: Vec::new(),
        notice: notice.map(Into::into),
    })
}

/// The series of the subject and scope, the history's span, and the newest tick.
pub(crate) fn read_series(
    input: &MonitorInput,
    scope: &MonitorScope,
    resolution: Resolution,
) -> (UsageSeries, Option<Duration>, Option<jiff::Timestamp>) {
    let history = input.pod_history;
    let part = match scope {
        MonitorScope::Total => None,
        MonitorScope::Part(name) => Some(name.as_str()),
    };
    let series = match &input.subject {
        MonitorSubject::Pod(pod) => history.pod_series(&pod.namespace, &pod.name, part, resolution),
        MonitorSubject::Container { pod, container } => {
            history.pod_series(&pod.namespace, &pod.name, Some(container), resolution)
        }
        MonitorSubject::Workload(owner) => history.owner_series(owner, part, resolution),
        MonitorSubject::Node(node) => {
            let series = input.node_history.node_series(&node.name, resolution);
            return (
                series,
                input.node_history.span(),
                input.node_history.newest_tick(),
            );
        }
    };
    (series, history.span(), history.newest_tick())
}

/// A pod's containers that run beside its main ones: init containers never share the node.
fn running_containers(pod: &PodSummary) -> impl Iterator<Item = &ContainerSummary> {
    pod.containers
        .iter()
        .filter(|container| container.kind != ContainerKind::Init)
}

pub(crate) fn scope_choices(input: &MonitorInput) -> Vec<ScopeChoice> {
    let total = |label: &str| ScopeChoice {
        label: label.to_owned(),
        scope: MonitorScope::Total,
    };
    let part = |label: String, name: &str| ScopeChoice {
        label,
        scope: MonitorScope::Part(name.to_owned()),
    };
    match &input.subject {
        MonitorSubject::Pod(pod) => {
            std::iter::once(total("Pod total"))
                .chain(running_containers(pod).map(|container| {
                    part(format!("Container: {}", container.name), &container.name)
                }))
                .collect()
        }
        MonitorSubject::Workload(owner) => std::iter::once(total("All pods"))
            .chain(
                input
                    .pods
                    .iter()
                    .filter(|pod| owns_pod(owner, pod))
                    .map(|pod| part(format!("Pod: {}", pod.name), &pod.name)),
            )
            .collect(),
        MonitorSubject::Node(_) => vec![total("Node total")],
        MonitorSubject::Container { .. } => vec![total("Container")],
    }
}

pub(crate) fn series_name(input: &MonitorInput, scope: &MonitorScope, pod_count: usize) -> String {
    match (&input.subject, scope) {
        (MonitorSubject::Pod(_), MonitorScope::Total) => "pod total".to_owned(),
        (MonitorSubject::Workload(_), MonitorScope::Total) => match pod_count {
            1 => "1 pod".to_owned(),
            count => format!("{count} pods"),
        },
        (MonitorSubject::Container { container, .. }, _) => (*container).to_owned(),
        (_, MonitorScope::Part(name)) => name.clone(),
        (MonitorSubject::Node(_), MonitorScope::Total) => "used".to_owned(),
    }
}

/// The request and limit of the containers a chart covers, for one resource.
struct ContainerLines {
    request: Option<f64>,
    /// Some containers set no request, so the line understates.
    is_request_partial: bool,
    /// Only when every container sets one: one without a limit is unbounded.
    limit: Option<f64>,
}

fn resource_value(
    container: &ContainerSummary,
    name: &str,
    pick: fn(&ContainerResource) -> Option<&String>,
) -> Option<f64> {
    let resource = container
        .resources
        .iter()
        .find(|resource| resource.name == name)?;
    let text = pick(resource)?;
    match name {
        "cpu" => CpuAmount::parse(text).map(CpuAmount::cores),
        _ => ByteAmount::parse(text).map(|bytes| bytes.bytes() as f64),
    }
}

fn container_lines<'a>(
    containers: impl Iterator<Item = &'a ContainerSummary> + Clone,
    name: &str,
) -> ContainerLines {
    let count = containers.clone().count();
    let requests: Vec<f64> = containers
        .clone()
        .filter_map(|container| {
            resource_value(container, name, |resource| resource.request.as_ref())
        })
        .collect();
    let limits: Vec<f64> = containers
        .filter_map(|container| resource_value(container, name, |resource| resource.limit.as_ref()))
        .collect();
    ContainerLines {
        request: (!requests.is_empty()).then(|| requests.iter().sum()),
        is_request_partial: requests.len() < count,
        limit: (count > 0 && limits.len() == count).then(|| limits.iter().sum()),
    }
}

fn lines_to_references(lines: &ContainerLines) -> Vec<ReferenceLine> {
    let mut references = Vec::new();
    // Request equal to limit (a Guaranteed pod) is one line: two on top of each other hide.
    if let (Some(request), Some(limit), false) =
        (lines.request, lines.limit, lines.is_request_partial)
        && (request - limit).abs() <= limit * EQUAL_SHARE
    {
        references.push(ReferenceLine {
            kind: ReferenceKind::Limit,
            label: "request = limit".into(),
            value: limit,
        });
        return references;
    }
    if let Some(value) = lines.request {
        let label = if lines.is_request_partial {
            "request (partial)"
        } else {
            "request"
        };
        references.push(ReferenceLine {
            kind: ReferenceKind::Request,
            label: label.into(),
            value,
        });
    }
    if let Some(value) = lines.limit {
        references.push(ReferenceLine {
            kind: ReferenceKind::Limit,
            label: "limit".into(),
            value,
        });
    }
    references
}

/// The reference lines of the CPU chart and of the Memory chart.
pub(crate) fn reference_lines(
    input: &MonitorInput,
    scope: &MonitorScope,
) -> (Vec<ReferenceLine>, Vec<ReferenceLine>) {
    let both = |containers: &[&ContainerSummary]| {
        (
            lines_to_references(&container_lines(containers.iter().copied(), "cpu")),
            lines_to_references(&container_lines(containers.iter().copied(), "memory")),
        )
    };
    match (&input.subject, scope) {
        (MonitorSubject::Node(node), _) => node_references(node, input),
        (MonitorSubject::Container { pod, container }, _) => {
            let own: Vec<&ContainerSummary> = pod
                .containers
                .iter()
                .filter(|candidate| candidate.name == *container)
                .collect();
            both(&own)
        }
        (MonitorSubject::Pod(pod), MonitorScope::Total) => {
            both(&running_containers(pod).collect::<Vec<_>>())
        }
        (MonitorSubject::Pod(pod), MonitorScope::Part(name)) => {
            let own: Vec<&ContainerSummary> = pod
                .containers
                .iter()
                .filter(|candidate| candidate.name == *name)
                .collect();
            both(&own)
        }
        (MonitorSubject::Workload(owner), scope) => {
            let containers: Vec<&ContainerSummary> = input
                .pods
                .iter()
                .filter(|pod| owns_pod(owner, pod) && takes_room(pod))
                .filter(|pod| match scope {
                    MonitorScope::Total => true,
                    MonitorScope::Part(name) => pod.name == *name,
                })
                .flat_map(running_containers)
                .collect();
            both(&containers)
        }
    }
}

/// `allocatable` always; `requested` only when the pods list covers every namespace.
fn node_references(
    node: &NodeSummary,
    input: &MonitorInput,
) -> (Vec<ReferenceLine>, Vec<ReferenceLine>) {
    let (cpu, memory) = node_allocatable(node);
    let mut cpu_lines = Vec::new();
    let mut memory_lines = Vec::new();
    if input.is_all_namespaces {
        let (cpu_request, memory_request) = node_requests(&node.name, input.pods);
        cpu_lines.push(ReferenceLine {
            kind: ReferenceKind::Request,
            label: "requested".into(),
            value: cpu_request.cores(),
        });
        memory_lines.push(ReferenceLine {
            kind: ReferenceKind::Request,
            label: "requested".into(),
            value: memory_request.bytes() as f64,
        });
    }
    if let Some(cpu) = cpu {
        cpu_lines.push(ReferenceLine {
            kind: ReferenceKind::Allocatable,
            label: "allocatable".into(),
            value: cpu.cores(),
        });
    }
    if let Some(memory) = memory {
        memory_lines.push(ReferenceLine {
            kind: ReferenceKind::Allocatable,
            label: "allocatable".into(),
            value: memory.bytes() as f64,
        });
    }
    (cpu_lines, memory_lines)
}

#[cfg(test)]
#[path = "monitor_data_tests.rs"]
mod monitor_data_tests;
