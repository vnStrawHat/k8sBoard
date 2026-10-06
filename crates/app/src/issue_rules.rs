//! The rules that turn pods, nodes, volume usage, and Warning events into findings. Pure: each
//! rule reads `IssueInputs` and returns what it saw; the board groups, orders, and delays them.
//! Event messages are arbitrary text, so nothing here logs them.

use std::cmp::Reverse;
use std::collections::HashMap;

use cluster::{
    ByteAmount, ConditionStatus, ContainerKind, ContainerState, ContainerSummary, CpuAmount,
    EventSummary, InitStatus, NodeCondition, NodeReadiness, NodeSummary, PodStatus, PodSummary,
    StatusReason, Termination,
};
use jiff::{SignedDuration, Timestamp};

use crate::age::format_age;
use crate::event_rows::message_line;
use crate::issue::{Finding, IssueAction, IssueObject, IssueRule, IssueSeverity};
use crate::issue_board::IssueInputs;
use crate::kind_row::{JOB_KIND, pod_workload};
use crate::node_usage::node_usage;
use crate::pod_diagnosis::{DiagnosisCause, PodDiagnosis, is_diagnosis_skipped, pod_diagnosis};
use crate::status_tone::{PRESSURE_CONDITIONS, StatusTone, active_pressures, node_condition_tone};
use crate::usage_format::{Measure, format_percent};

pub(crate) const UNSCHEDULABLE_GRACE: SignedDuration = SignedDuration::from_mins(2);
pub(crate) const NOT_READY_GRACE: SignedDuration = SignedDuration::from_mins(5);
pub(crate) const STARTUP_FALLBACK_GRACE: SignedDuration = SignedDuration::from_mins(10);
pub(crate) const STUCK_STARTING_AFTER: SignedDuration = SignedDuration::from_mins(10);
pub(crate) const NODE_UNKNOWN_GRACE: SignedDuration = SignedDuration::from_mins(1);
pub(crate) const RESTART_WINDOW: SignedDuration = SignedDuration::from_hours(1);
pub(crate) const RESTART_WARN: u32 = 3;
pub(crate) const EVICTION_WINDOW: SignedDuration = SignedDuration::from_hours(24);
pub(crate) const EVENT_WINDOW: SignedDuration = SignedDuration::from_mins(15);
pub(crate) const WARNING_BURST: u32 = 10;
/// The ratio at which `usage_tone` turns a usage bar red.
pub(crate) const USAGE_WARN_RATIO: f64 = 0.9;

/// Characters of an event message an issue quotes.
const CAUSE_MESSAGE_CHARS: usize = 200;

const MEMORY: &str = "memory";
const CPU: &str = "cpu";

/// Whether `at` is known and not older than `window`.
fn is_within(at: Option<Timestamp>, window: SignedDuration, now: Timestamp) -> bool {
    at.is_some_and(|at| now.duration_since(at) <= window)
}

/// One line of an event message, cut to `CAUSE_MESSAGE_CHARS`.
fn quoted(message: &str) -> String {
    let line = message_line(message);
    if line.chars().count() <= CAUSE_MESSAGE_CHARS {
        return line;
    }
    let mut cut: String = line.chars().take(CAUSE_MESSAGE_CHARS - 1).collect();
    cut.push('…');
    cut
}

/// `{reason}: {message}`, or whichever of the two exists.
fn reason_and_message(reason: Option<&str>, message: Option<&str>, fallback: &str) -> String {
    match (reason, message) {
        (Some(reason), Some(message)) => format!("{reason}: {}", quoted(message)),
        (Some(text), None) => text.to_owned(),
        (None, Some(message)) => quoted(message),
        (None, None) => fallback.to_owned(),
    }
}

// ---- pods ----

/// The first pod rule that matches, which makes the pod's one finding.
pub(crate) fn pod_finding(pod: &PodSummary, inputs: &IssueInputs) -> Option<Finding> {
    if is_diagnosis_skipped(pod) {
        return None;
    }
    let events = inputs
        .events
        .and_then(|events| events.of(&IssueObject::pod(&pod.namespace, &pod.name)));
    match pod_diagnosis(pod, events, inputs.now) {
        Some(diagnosis) => diagnosed_finding(pod, &diagnosis, inputs),
        None => stuck_finding(pod, events, inputs.now)
            .or_else(|| restart_finding(pod, inputs.now))
            .or_else(|| memory_finding(pod, inputs))
            .or_else(|| cpu_finding(pod, inputs)),
    }
}

fn pod_base(
    pod: &PodSummary,
    rule: IssueRule,
    severity: IssueSeverity,
    reason: String,
    cause: String,
) -> Finding {
    Finding {
        rule,
        severity,
        object: IssueObject::pod(&pod.namespace, &pod.name),
        reason: reason.into(),
        cause,
        container: None,
        onset: None,
        grace: None,
        action: IssueAction::Open,
        workload: pod_workload(&pod.namespace, pod.controller.as_ref()),
    }
}

/// The pod `Ready` condition's transition time while it is false: when the pod stopped serving.
fn ready_since(pod: &PodSummary) -> Option<Timestamp> {
    pod.conditions
        .iter()
        .find(|condition| condition.name == "Ready" && !condition.is_true)
        .and_then(|condition| condition.changed_at)
}

fn is_job_pod(pod: &PodSummary) -> bool {
    pod.controller
        .as_ref()
        .is_some_and(|controller| controller.kind == JOB_KIND)
}

fn diagnosed_finding(
    pod: &PodSummary,
    diagnosis: &PodDiagnosis,
    inputs: &IssueInputs,
) -> Option<Finding> {
    let ready_since = ready_since(pod);
    // The Job rule owns the pods of a Job while its feed is live.
    let is_left_to_job = is_job_pod(pod) && inputs.is_job_feed_live;
    let finding = |rule, severity, reason: String| Finding {
        container: diagnosis.container.clone(),
        ..pod_base(pod, rule, severity, reason, diagnosis.text.clone())
    };
    let view_logs = IssueAction::ViewLogs {
        container: diagnosis.container.clone(),
    };
    match &diagnosis.cause {
        DiagnosisCause::ImagePull(reason) => Some(finding(
            IssueRule::PodImage,
            IssueSeverity::Critical,
            reason.to_string(),
        )),
        DiagnosisCause::CrashLoop => Some(Finding {
            onset: ready_since,
            action: view_logs,
            ..finding(
                IssueRule::PodCrash,
                IssueSeverity::Critical,
                StatusReason::CrashLoopBackOff.to_string(),
            )
        }),
        DiagnosisCause::Waiting(reason) => Some(finding(
            IssueRule::PodWaiting,
            IssueSeverity::Critical,
            reason.to_string(),
        )),
        DiagnosisCause::Unschedulable { since } => Some(Finding {
            onset: since.or(pod.created_at),
            grace: Some(UNSCHEDULABLE_GRACE),
            ..finding(
                IssueRule::PodUnschedulable,
                IssueSeverity::Warning,
                StatusReason::Pending.to_string(),
            )
        }),
        // A deliberate wait, not an outage.
        DiagnosisCause::SchedulingGated => None,
        DiagnosisCause::PodFailed => {
            let onset = ready_since.or(pod.created_at);
            let is_old_eviction = pod.status == PodStatus::Reason(StatusReason::Evicted)
                && !is_within(onset, EVICTION_WINDOW, inputs.now);
            if is_left_to_job || is_old_eviction {
                return None;
            }
            Some(Finding {
                onset,
                ..finding(
                    IssueRule::PodFailed,
                    IssueSeverity::Warning,
                    pod.status.to_string(),
                )
            })
        }
        DiagnosisCause::Exited { reason } => {
            if is_left_to_job {
                return None;
            }
            Some(Finding {
                onset: ready_since,
                action: view_logs,
                ..finding(
                    IssueRule::PodExited,
                    IssueSeverity::Warning,
                    reason
                        .as_ref()
                        .map_or_else(|| StatusReason::Error.to_string(), ToString::to_string),
                )
            })
        }
        DiagnosisCause::StartupPending => {
            let container = diagnosis.container.as_deref().and_then(|name| {
                pod.containers
                    .iter()
                    .find(|container| container.name == name)
            });
            Some(Finding {
                onset: container.and_then(started_at),
                grace: Some(container.map_or(STARTUP_FALLBACK_GRACE, startup_grace)),
                ..finding(
                    IssueRule::PodStartup,
                    IssueSeverity::Warning,
                    "Startup probe".to_owned(),
                )
            })
        }
        DiagnosisCause::NotReady => Some(Finding {
            onset: ready_since,
            grace: Some(NOT_READY_GRACE),
            ..finding(
                IssueRule::PodNotReady,
                IssueSeverity::Warning,
                "Not ready".to_owned(),
            )
        }),
    }
}

fn started_at(container: &ContainerSummary) -> Option<Timestamp> {
    match container.state {
        ContainerState::Running { started_at } => started_at,
        _ => None,
    }
}

/// The time a container may take to start: the probe's initial delay plus the time its checks
/// may fail before the kubelet restarts it. A probe without a period or threshold gets the
/// fallback.
fn startup_grace(container: &ContainerSummary) -> SignedDuration {
    container
        .probes
        .startup
        .as_ref()
        .and_then(|probe| {
            let checks = i64::from(probe.period_seconds) * i64::from(probe.failure_threshold);
            (checks > 0).then(|| checks + i64::from(probe.initial_delay_seconds))
        })
        .map_or(STARTUP_FALLBACK_GRACE, SignedDuration::from_secs)
}

fn stuck_finding(
    pod: &PodSummary,
    events: Option<&[EventSummary]>,
    now: Timestamp,
) -> Option<Finding> {
    let is_starting = matches!(
        pod.status,
        PodStatus::Reason(
            StatusReason::ContainerCreating | StatusReason::PodInitializing | StatusReason::Pending
        ) | PodStatus::Init(InitStatus::Progress { .. })
    );
    if !is_starting {
        return None;
    }
    let status = pod.status.to_string();
    let mut cause = format!("Stuck in {status} for {}.", format_age(pod.created_at, now));
    // The slice is newest first.
    if let Some(event) = events.and_then(<[EventSummary]>::first) {
        cause.push_str(&format!(" {}: {}", event.reason, quoted(&event.message)));
    }
    Some(Finding {
        onset: pod.created_at,
        grace: Some(STUCK_STARTING_AFTER),
        ..pod_base(
            pod,
            IssueRule::PodStuck,
            IssueSeverity::Warning,
            status,
            cause,
        )
    })
}

/// The containers that run workloads: init containers have finished or are the pod's problem.
fn serving_containers(pod: &PodSummary) -> impl Iterator<Item = &ContainerSummary> {
    pod.containers
        .iter()
        .filter(|container| container.kind != ContainerKind::Init)
}

// ponytail: looks back one hour from the last exit only, because `restart_count` is a lifetime
// total and the API keeps no restart history; upgrade path: count `last_termination` changes per
// tick in the metrics history.
fn restart_finding(pod: &PodSummary, now: Timestamp) -> Option<Finding> {
    serving_containers(pod)
        .filter(|container| matches!(container.state, ContainerState::Running { .. }))
        .find_map(|container| {
            let termination = container.last_termination.as_ref()?;
            let finished_at = termination.finished_at?;
            if !is_within(Some(finished_at), RESTART_WINDOW, now) {
                return None;
            }
            let is_oom = termination.reason == Some(StatusReason::OomKilled);
            if !is_oom && container.restart_count < RESTART_WARN {
                return None;
            }
            let (reason, cause) = if is_oom {
                ("OOMKilled", oom_cause(container, finished_at, now))
            } else {
                ("Restarting", restart_cause(container, termination, now))
            };
            Some(Finding {
                container: Some(container.name.clone()),
                onset: Some(finished_at),
                action: IssueAction::ViewLogs {
                    container: Some(container.name.clone()),
                },
                ..pod_base(
                    pod,
                    IssueRule::PodRestarts,
                    IssueSeverity::Warning,
                    reason.to_owned(),
                    cause,
                )
            })
        })
}

fn oom_cause(container: &ContainerSummary, finished_at: Timestamp, now: Timestamp) -> String {
    let age = format_age(Some(finished_at), now);
    match resource_limit(container, MEMORY) {
        Some(limit) => format!("OOMKilled {age} ago; memory limit {limit}."),
        None => format!("OOMKilled {age} ago. No memory limit is set."),
    }
}

/// The restart count is a lifetime total, so the text says so.
fn restart_cause(
    container: &ContainerSummary,
    termination: &Termination,
    now: Timestamp,
) -> String {
    let exit = termination.reason.as_ref().map_or_else(
        || format!("code {}", termination.exit_code),
        ToString::to_string,
    );
    format!(
        "Restarted {} times in total; last exit {exit} {} ago.",
        container.restart_count,
        format_age(termination.finished_at, now)
    )
}

fn resource_limit<'a>(container: &'a ContainerSummary, name: &str) -> Option<&'a str> {
    container
        .resources
        .iter()
        .find(|resource| resource.name == name)
        .and_then(|resource| resource.limit.as_deref())
}

fn memory_finding(pod: &PodSummary, inputs: &IssueInputs) -> Option<Finding> {
    let usage = inputs.pod_usage?;
    serving_containers(pod).find_map(|container| {
        let limit = ByteAmount::parse(resource_limit(container, MEMORY)?)?.bytes();
        if limit == 0 {
            return None;
        }
        let [previous, newest] =
            usage.latest_container_pair(&pod.namespace, &pod.name, &container.name)?;
        // Two samples in a row, so one spike while a request is served does not raise it.
        let is_near = |bytes: u64| bytes as f64 >= USAGE_WARN_RATIO * limit as f64;
        if !is_near(previous.memory.bytes()) || !is_near(newest.memory.bytes()) {
            return None;
        }
        let cause = format!(
            "Container {} uses {} of its {} memory limit.",
            container.name,
            Measure::Bytes.format(newest.memory.bytes() as f64),
            Measure::Bytes.format(limit as f64),
        );
        Some(usage_finding(
            pod,
            container,
            IssueRule::PodMemory,
            "Near memory limit",
            cause,
        ))
    })
}

/// The newest CPU sample against the limit. Throttling is not measured, so the text claims none.
fn cpu_finding(pod: &PodSummary, inputs: &IssueInputs) -> Option<Finding> {
    let usage = inputs.pod_usage?;
    serving_containers(pod).find_map(|container| {
        let limit = CpuAmount::parse(resource_limit(container, CPU)?)?.cores();
        if limit <= 0. {
            return None;
        }
        let newest = usage.latest_container(&pod.namespace, &pod.name, &container.name)?;
        if newest.cpu.cores() < USAGE_WARN_RATIO * limit {
            return None;
        }
        let cause = format!(
            "Container {} uses {} of its {} CPU limit.",
            container.name,
            Measure::Cpu.format(newest.cpu.cores()),
            Measure::Cpu.format(limit),
        );
        Some(usage_finding(
            pod,
            container,
            IssueRule::PodCpu,
            "At CPU limit",
            cause,
        ))
    })
}

fn usage_finding(
    pod: &PodSummary,
    container: &ContainerSummary,
    rule: IssueRule,
    reason: &str,
    cause: String,
) -> Finding {
    Finding {
        container: Some(container.name.clone()),
        ..pod_base(pod, rule, IssueSeverity::Warning, reason.to_owned(), cause)
    }
}

// ---- nodes ----

const READY: &str = "Ready";
const NETWORK_UNAVAILABLE: &str = "NetworkUnavailable";

/// The first node rule that matches. A cordoned node is not an issue: someone chose that.
pub(crate) fn node_finding(node: &NodeSummary, inputs: &IssueInputs) -> Option<Finding> {
    not_ready_finding(node, inputs.now)
        .or_else(|| network_finding(node))
        .or_else(|| pressure_finding(node))
        .or_else(|| other_condition_finding(node))
        .or_else(|| node_usage_finding(node, inputs))
}

fn node_base(
    node: &NodeSummary,
    rule: IssueRule,
    severity: IssueSeverity,
    reason: &str,
    cause: String,
) -> Finding {
    Finding {
        rule,
        severity,
        object: IssueObject::node(&node.name),
        reason: reason.to_owned().into(),
        cause,
        container: None,
        onset: None,
        grace: None,
        action: IssueAction::Open,
        workload: None,
    }
}

fn condition<'a>(node: &'a NodeSummary, name: &str) -> Option<&'a NodeCondition> {
    node.conditions
        .iter()
        .find(|condition| condition.name == name)
}

fn is_true(condition: &NodeCondition) -> bool {
    condition.status == ConditionStatus::True
}

fn not_ready_finding(node: &NodeSummary, now: Timestamp) -> Option<Finding> {
    let (reason, status, grace) = match node.status.readiness {
        NodeReadiness::Ready => return None,
        NodeReadiness::NotReady => ("NotReady", "False", None),
        NodeReadiness::Unknown => ("Unknown", "Unknown", Some(NODE_UNKNOWN_GRACE)),
    };
    let ready = condition(node, READY);
    let changed_at = ready.and_then(|ready| ready.changed_at);
    let mut cause = match changed_at {
        Some(_) => format!("Ready is {status} for {}.", format_age(changed_at, now)),
        None => format!("Ready is {status}."),
    };
    if let Some(message) = ready.and_then(|ready| ready.message.as_deref()) {
        cause.push(' ');
        cause.push_str(&quoted(message));
    }
    Some(Finding {
        onset: changed_at,
        grace,
        ..node_base(
            node,
            IssueRule::NodeNotReady,
            IssueSeverity::Critical,
            reason,
            cause,
        )
    })
}

fn network_finding(node: &NodeSummary) -> Option<Finding> {
    let network = condition(node, NETWORK_UNAVAILABLE).filter(|condition| is_true(condition))?;
    let cause = network
        .message
        .as_deref()
        .map_or_else(|| "The node network is not configured.".to_owned(), quoted);
    Some(Finding {
        onset: network.changed_at,
        ..node_base(
            node,
            IssueRule::NodeNetwork,
            IssueSeverity::Critical,
            NETWORK_UNAVAILABLE,
            cause,
        )
    })
}

fn pressure_finding(node: &NodeSummary) -> Option<Finding> {
    let mut pressures = active_pressures(&node.conditions);
    let first = pressures.next()?;
    let others: Vec<&str> = pressures.map(|condition| condition.name.as_str()).collect();
    let mut cause = first
        .message
        .as_deref()
        .map_or_else(|| format!("The kubelet reports {}.", first.name), quoted);
    if !others.is_empty() {
        // Kubelet messages end without a full stop.
        if !cause.ends_with(['.', '!', '?']) {
            cause.push('.');
        }
        cause.push_str(&format!(" Also {}.", others.join(", ")));
    }
    Some(Finding {
        onset: first.changed_at,
        ..node_base(
            node,
            IssueRule::NodePressure,
            IssueSeverity::Warning,
            &first.name,
            cause,
        )
    })
}

/// Conditions that node-problem-detector and similar agents add.
fn other_condition_finding(node: &NodeSummary) -> Option<Finding> {
    let known = |name: &str| {
        name == READY || name == NETWORK_UNAVAILABLE || PRESSURE_CONDITIONS.contains(&name)
    };
    let problem = node.conditions.iter().find(|condition| {
        !known(&condition.name) && node_condition_tone(condition) == StatusTone::Bad
    })?;
    let cause = reason_and_message(
        problem.reason.as_deref(),
        problem.message.as_deref(),
        &format!("The node reports {}.", problem.name),
    );
    Some(Finding {
        onset: problem.changed_at,
        ..node_base(
            node,
            IssueRule::NodeCondition,
            IssueSeverity::Warning,
            &problem.name,
            cause,
        )
    })
}

fn node_usage_finding(node: &NodeSummary, inputs: &IssueInputs) -> Option<Finding> {
    let latest = inputs.node_usage?.latest(&node.name);
    let usage = node_usage(node, latest);
    let finding = |rule, reason, resource, ratio: f64| {
        node_base(
            node,
            rule,
            IssueSeverity::Warning,
            reason,
            format!("Uses {} of allocatable {resource}.", format_percent(ratio)),
        )
    };
    usage
        .memory
        .filter(|ratio| *ratio >= USAGE_WARN_RATIO)
        .map(|ratio| finding(IssueRule::NodeMemory, "High memory", MEMORY, ratio))
        .or_else(|| {
            usage
                .cpu
                .filter(|ratio| *ratio >= USAGE_WARN_RATIO)
                .map(|ratio| finding(IssueRule::NodeCpu, "High CPU", "CPU", ratio))
        })
}

// ---- volume usage ----

/// A claim whose volume is nearly full, from the kubelet summaries.
pub(crate) fn volume_findings(inputs: &IssueInputs) -> Vec<Finding> {
    let Some(kubelet) = inputs.kubelet else {
        return Vec::new();
    };
    kubelet
        .pvc_usages()
        .filter_map(|usage| {
            let capacity = usage.capacity?.bytes();
            // What the filesystem has left is what a write meets; `used` can miss reserved blocks.
            let used = match usage.available {
                Some(available) => capacity.saturating_sub(available.bytes()),
                None => usage.used?.bytes(),
            };
            let ratio = used as f64 / capacity.max(1) as f64;
            (capacity > 0 && ratio >= USAGE_WARN_RATIO).then(|| Finding {
                rule: IssueRule::VolumeFull,
                severity: IssueSeverity::Warning,
                object: IssueObject::new(
                    "PersistentVolumeClaim",
                    Some(&usage.namespace),
                    &usage.claim,
                ),
                reason: "Volume almost full".into(),
                cause: format!(
                    "{} of {} used ({}).",
                    Measure::Bytes.format(used as f64),
                    Measure::Bytes.format(capacity as f64),
                    format_percent(ratio),
                ),
                container: None,
                onset: None,
                grace: None,
                action: IssueAction::Open,
                workload: None,
            })
        })
        .collect()
}

// ---- events ----

/// What an event is worth as an issue.
struct EventVerdict {
    rule: IssueRule,
    severity: IssueSeverity,
    reason: String,
    cause: String,
}

fn event_verdict(event: &EventSummary, now: Timestamp) -> Option<EventVerdict> {
    if !is_within(event.last_seen, EVENT_WINDOW, now) {
        return None;
    }
    let (kind, reason) = (event.object.kind.as_str(), event.reason.as_str());
    let message = quoted(&event.message);
    if reason == "FailedCreate"
        && matches!(kind, "ReplicaSet" | "StatefulSet" | "DaemonSet" | "Job")
    {
        return Some(EventVerdict {
            rule: IssueRule::EventFailedCreate,
            severity: IssueSeverity::Critical,
            reason: reason.to_owned(),
            cause: message,
        });
    }
    if kind == JOB_KIND && matches!(reason, "BackoffLimitExceeded" | "DeadlineExceeded") {
        return Some(EventVerdict {
            rule: IssueRule::EventJobFailed,
            severity: IssueSeverity::Warning,
            reason: reason.to_owned(),
            cause: message,
        });
    }
    // The pod rules already explain a pod that cannot be scheduled.
    if reason == "FailedScheduling" {
        return None;
    }
    // A series that began long ago and fired every now and then is not a burst.
    let is_burst = event.count >= WARNING_BURST && is_within(event.first_seen, EVENT_WINDOW, now);
    is_burst.then(|| EventVerdict {
        rule: IssueRule::EventBurst,
        severity: IssueSeverity::Warning,
        reason: reason.to_owned(),
        cause: format!(
            "{message} (×{} in {}).",
            event.count,
            format_age(event.first_seen, now)
        ),
    })
}

/// One finding per involved object: its event with the highest count. An event about a Pod or a
/// Node that is gone is dropped.
pub(crate) fn event_findings(inputs: &IssueInputs) -> Vec<Finding> {
    let Some(events) = inputs.events else {
        return Vec::new();
    };
    let mut best: HashMap<IssueObject, (&EventSummary, EventVerdict)> = HashMap::new();
    for event in events.recent() {
        let Some(verdict) = event_verdict(event, inputs.now) else {
            continue;
        };
        let object = IssueObject::of_involved(&event.object);
        if is_gone(&object, inputs) {
            continue;
        }
        // The most specific rule wins, then the highest count: a FailedCreate must not hide
        // behind a burst of another warning on the same object.
        let rank = |rule: IssueRule, count: u32| (rule, Reverse(count));
        let is_better = best.get(&object).is_none_or(|(known, known_verdict)| {
            rank(verdict.rule, event.count) < rank(known_verdict.rule, known.count)
        });
        if is_better {
            best.insert(object, (event, verdict));
        }
    }
    let mut findings: Vec<Finding> = best
        .into_iter()
        .map(|(object, (event, verdict))| Finding {
            rule: verdict.rule,
            severity: verdict.severity,
            object,
            reason: verdict.reason.into(),
            cause: verdict.cause,
            container: None,
            onset: event.first_seen,
            grace: None,
            action: IssueAction::Open,
            workload: None,
        })
        .collect();
    findings.sort_by(|left, right| (left.rule, &left.object).cmp(&(right.rule, &right.object)));
    findings
}

/// Whether the Pod or Node an event is about is missing from a list that has loaded. A list that
/// has not loaded says nothing.
fn is_gone(object: &IssueObject, inputs: &IssueInputs) -> bool {
    match object.kind.as_str() {
        "Pod" => inputs.pods.is_some_and(|pods| {
            let key = (
                object.namespace.as_deref().unwrap_or_default(),
                object.name.as_str(),
            );
            // Pod snapshots are ordered by (namespace, name).
            pods.binary_search_by(|pod| (pod.namespace.as_str(), pod.name.as_str()).cmp(&key))
                .is_err()
        }),
        "Node" => inputs
            .nodes
            .is_some_and(|nodes| nodes.iter().all(|node| node.name != object.name)),
        _ => false,
    }
}

#[cfg(test)]
#[path = "issue_rules_tests.rs"]
mod issue_rules_tests;
