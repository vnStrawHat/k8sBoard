//! PromQL text for the Monitor charts (spec 0048). Every query is built here from typed targets:
//! no public function takes query text, and every Kubernetes name is checked and escaped first.

use std::time::Duration;

use crate::dns_name::{is_dns_label, is_dns_subdomain};

/// Most points one range query may ask for.
pub const MAX_POINTS: u64 = 400;

const MIN_STEP: Duration = Duration::from_secs(1);
/// At least four scrapes at a 30 s interval, so a short step never yields an empty `rate`.
const MIN_RATE_WINDOW_SECONDS: u64 = 120;

/// The alphabet of Kubernetes' generated name suffixes (`rand.SafeEncodeString`): no vowels, no
/// `0 1 3`, so CronJob timestamps and most words do not fit.
const SUFFIX_CLASS: &str = "[bcdfghjklmnpqrstvwxz2456789]";

/// The 0011 rule for a node's own uplink: loopback and virtual interfaces are left out.
const NODE_INTERFACES: &str =
    r#"interface!~"lo|(veth|cali|cni|flannel|cilium|lxc|docker|tunl|vxlan|kube-|weave|br-).*""#;
const POD_INTERFACES: &str = r#"interface!="lo""#;
const ALL_CONTAINERS: &str = r#"container!="",container!="POD""#;

/// Range spans with their steps, all within `MAX_POINTS` points.
pub const RANGE_STEPS: [(Duration, Duration); 6] = [
    (Duration::from_secs(15 * 60), Duration::from_secs(15)),
    (Duration::from_secs(60 * 60), Duration::from_secs(15)),
    (Duration::from_secs(6 * 60 * 60), Duration::from_secs(60)),
    (
        Duration::from_secs(24 * 60 * 60),
        Duration::from_secs(5 * 60),
    ),
    (
        Duration::from_secs(7 * 24 * 60 * 60),
        Duration::from_secs(30 * 60),
    ),
    (
        Duration::from_secs(30 * 24 * 60 * 60),
        Duration::from_secs(2 * 60 * 60),
    ),
];

/// What one chart reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsageTarget {
    Pod {
        namespace: String,
        pod: String,
        container: Option<String>,
    },
    Workload {
        namespace: String,
        kind: WorkloadKind,
        name: String,
    },
    Node {
        name: String,
    },
}

/// The workload kinds whose pods are matched by a name pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkloadKind {
    Deployment,
    StatefulSet,
    DaemonSet,
    ReplicaSet,
    Job,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsageMetric {
    Cpu,
    Memory,
    NetworkReceive,
    NetworkTransmit,
    DiskRead,
    DiskWrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RangeError {
    #[error("the step is shorter than 1 s")]
    StepTooShort,
    #[error("the range ends before it starts")]
    EmptyRange,
    #[error("the range has more than {MAX_POINTS} points")]
    TooManyPoints,
    #[error("no step is defined for this range")]
    UnknownSpan,
}

/// A validated range query window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RangeSpec {
    start: jiff::Timestamp,
    end: jiff::Timestamp,
    step: Duration,
}

impl RangeSpec {
    /// Errs when `step` is below 1 s, `end` is not after `start`, or the range has more than
    /// `MAX_POINTS` points.
    pub fn new(
        start: jiff::Timestamp,
        end: jiff::Timestamp,
        step: Duration,
    ) -> Result<Self, RangeError> {
        if step < MIN_STEP {
            return Err(RangeError::StepTooShort);
        }
        let span_seconds = end.as_second().saturating_sub(start.as_second());
        let Ok(span_seconds) = u64::try_from(span_seconds) else {
            return Err(RangeError::EmptyRange);
        };
        if span_seconds == 0 {
            return Err(RangeError::EmptyRange);
        }
        let points = span_seconds / step.as_secs() + 1;
        if points > MAX_POINTS {
            return Err(RangeError::TooManyPoints);
        }
        Ok(Self { start, end, step })
    }

    /// The window of one `RANGE_STEPS` span: `end` is `now` rounded down to the step (so a refresh
    /// reuses the backend cache) and `start` is `end - span`.
    pub fn ending_at(now: jiff::Timestamp, span: Duration) -> Result<Self, RangeError> {
        let Some((_, step)) = RANGE_STEPS.iter().find(|(known, _)| *known == span) else {
            return Err(RangeError::UnknownSpan);
        };
        let step_seconds = i64::try_from(step.as_secs()).unwrap_or(i64::MAX);
        let span_seconds = i64::try_from(span.as_secs()).unwrap_or(i64::MAX);
        let end_second = now.as_second().div_euclid(step_seconds) * step_seconds;
        let start_second = end_second.saturating_sub(span_seconds);
        let (Ok(start), Ok(end)) = (
            jiff::Timestamp::from_second(start_second),
            jiff::Timestamp::from_second(end_second),
        ) else {
            return Err(RangeError::EmptyRange);
        };
        Self::new(start, end, *step)
    }

    pub fn start(&self) -> jiff::Timestamp {
        self.start
    }

    pub fn end(&self) -> jiff::Timestamp {
        self.end
    }

    pub fn step(&self) -> Duration {
        self.step
    }

    /// Number of samples a full answer holds: `start`, then one per step up to `end`.
    pub fn points(&self) -> usize {
        let span = self.end.as_second().saturating_sub(self.start.as_second());
        let steps = u64::try_from(span).unwrap_or(0) / self.step.as_secs();
        usize::try_from(steps)
            .unwrap_or(usize::MAX)
            .saturating_add(1)
    }

    fn rate_window(&self) -> String {
        format!("{}s", self.step.as_secs().max(MIN_RATE_WINDOW_SECONDS))
    }
}

/// A name failed its DNS check; no query is built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InvalidName;

/// A PromQL double-quoted string: `\` becomes `\\`, `"` becomes `\"`, a newline becomes `\n`, and
/// other control characters are dropped. The result includes its quotes.
pub(crate) fn string_literal(value: &str) -> String {
    let mut text = String::with_capacity(value.len() + 2);
    text.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => text.push_str("\\\\"),
            '"' => text.push_str("\\\""),
            '\n' => text.push_str("\\n"),
            ch if ch.is_control() => {}
            ch => text.push(ch),
        }
    }
    text.push('"');
    text
}

/// Gives every RE2 metacharacter of `value` a `\`, so a name matches only itself in `=~` (PromQL
/// regexes are fully anchored). The caller then puts the pattern through `string_literal`, so
/// `a.b` is written `"a\\.b"` in the query.
fn regex_escape(value: &str) -> String {
    let mut text = String::with_capacity(value.len());
    for ch in value.chars() {
        if "\\.+*?()|[]{}^$".contains(ch) {
            text.push('\\');
        }
        text.push(ch);
    }
    text
}

/// The query text for `metric` of `target`; `range` sets the rate window.
pub(crate) fn usage_query(
    target: &UsageTarget,
    metric: UsageMetric,
    range: &RangeSpec,
) -> Result<String, InvalidName> {
    let selector = Selector::of(target)?;
    let window = range.rate_window();
    let query = match metric {
        UsageMetric::Cpu => format!(
            "sum(rate(container_cpu_usage_seconds_total{{{}}}[{window}]))",
            selector.with_containers()
        ),
        UsageMetric::Memory => format!(
            "sum(container_memory_working_set_bytes{{{}}})",
            selector.with_containers()
        ),
        UsageMetric::NetworkReceive => network_query("receive", &selector, &window),
        UsageMetric::NetworkTransmit => network_query("transmit", &selector, &window),
        UsageMetric::DiskRead => format!(
            "sum(rate(container_fs_reads_bytes_total{{{}}}[{window}]))",
            selector.with_containers()
        ),
        UsageMetric::DiskWrite => format!(
            "sum(rate(container_fs_writes_bytes_total{{{}}}[{window}]))",
            selector.with_containers()
        ),
    };
    Ok(query)
}

/// Network is counted at pod level, so a container filter is ignored.
fn network_query(direction: &str, selector: &Selector, window: &str) -> String {
    let interfaces = match selector.level {
        Level::Node => NODE_INTERFACES,
        Level::Pods => POD_INTERFACES,
    };
    format!(
        "sum(rate(container_network_{direction}_bytes_total{{{},{interfaces}}}[{window}]))",
        selector.pods
    )
}

enum Level {
    Pods,
    Node,
}

/// The label matchers of one target: the pod selector `P` and the container filter `C`.
struct Selector {
    pods: String,
    containers: Option<String>,
    level: Level,
}

impl Selector {
    fn of(target: &UsageTarget) -> Result<Self, InvalidName> {
        match target {
            UsageTarget::Pod {
                namespace,
                pod,
                container,
            } => {
                check(is_dns_label(namespace))?;
                check(is_dns_subdomain(pod))?;
                let containers = match container {
                    Some(name) => {
                        check(is_dns_label(name))?;
                        format!("container={}", string_literal(name))
                    }
                    None => ALL_CONTAINERS.to_owned(),
                };
                Ok(Self {
                    pods: format!(
                        "namespace={},pod={}",
                        string_literal(namespace),
                        string_literal(pod)
                    ),
                    containers: Some(containers),
                    level: Level::Pods,
                })
            }
            UsageTarget::Workload {
                namespace,
                kind,
                name,
            } => {
                check(is_dns_label(namespace))?;
                check(is_dns_subdomain(name))?;
                Ok(Self {
                    pods: format!(
                        "namespace={},pod=~{}",
                        string_literal(namespace),
                        string_literal(&pod_name_pattern(*kind, name))
                    ),
                    containers: Some(ALL_CONTAINERS.to_owned()),
                    level: Level::Pods,
                })
            }
            UsageTarget::Node { name } => {
                check(is_dns_subdomain(name))?;
                // The root cgroup: `id="/"` where the source keeps the `id` label, else (the UAT
                // VictoriaMetrics scrape drops it) the series with no pod. `id=~"/|"` also matches
                // an absent label. See as-built.md for the UAT fact.
                Ok(Self {
                    pods: format!(r#"id=~"/|",pod="",node={}"#, string_literal(name)),
                    containers: None,
                    level: Level::Node,
                })
            }
        }
    }

    fn with_containers(&self) -> String {
        match &self.containers {
            Some(containers) => format!("{},{containers}", self.pods),
            None => self.pods.clone(),
        }
    }
}

fn check(is_valid: bool) -> Result<(), InvalidName> {
    if is_valid { Ok(()) } else { Err(InvalidName) }
}

/// The regex (unquoted, name escaped) matching the pods a workload of `kind` creates.
// ponytail: another workload whose name plus a dash and a word of the suffix alphabet fits the
// pattern (`api` and `api-v2x`) also counts; upgrade path: a `kube_pod_owner` join when
// kube-state-metrics is present (decision 11).
fn pod_name_pattern(kind: WorkloadKind, name: &str) -> String {
    let name = regex_escape(name);
    match kind {
        WorkloadKind::Deployment => format!("{name}-{SUFFIX_CLASS}{{1,10}}-{SUFFIX_CLASS}{{5}}"),
        WorkloadKind::StatefulSet => format!("{name}-[0-9]+"),
        WorkloadKind::DaemonSet | WorkloadKind::ReplicaSet | WorkloadKind::Job => {
            format!("{name}-{SUFFIX_CLASS}{{5}}")
        }
    }
}

#[cfg(test)]
#[path = "promql_tests.rs"]
mod promql_tests;
