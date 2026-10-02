//! The WHY box of Deployments, DaemonSets, Jobs, Services, PodDisruptionBudgets, HPAs, and
//! ResourceQuotas: what is wrong and, when the pods say so, why. Pure: the drawer reads the live
//! lists and calls `kind_diagnosis`. Pod causes reuse
//! `pod_diagnosis` without events, so probe-failure detail stays in the pod drawer. Condition and
//! status messages are arbitrary text, so nothing here logs them.

use cluster::{
    BlockCause, ContainerKind, ContainerState, DaemonSetSummary, DeploymentSummary,
    DisruptionState, HorizontalPodAutoscalerSummary, JobStatus, JobSummary, NodeReadiness,
    NodeSummary, PodDisruptionBudgetSummary, PodStatus, PodSummary, ResourceQuotaSummary,
    ServiceSummary, StatusReason, Termination, WorkloadCondition,
};
use jiff::Timestamp;

use crate::kind_join::ServiceHealth;
use crate::kind_row::KindObject;
use crate::pod_diagnosis::{PodDiagnosis, pod_diagnosis};
use crate::policy_rows::{
    fullest_item, is_above_target, is_at_max, is_scaling_disabled, metric_text, quota_text,
};
use crate::status_tone::{StatusTone, pod_status_label, readiness_text};
use crate::table_selection::ResourceKey;
use crate::workload_rows::{DEADLINE_EXCEEDED, PROGRESSING};

/// The `Job` condition reasons the controller reports when it gives up.
const BACKOFF_LIMIT_EXCEEDED: &str = "BackoffLimitExceeded";
const JOB_DEADLINE_EXCEEDED: &str = "DeadlineExceeded";
/// The API default of a Job's `backoffLimit`.
const DEFAULT_BACKOFF_LIMIT: u32 = 6;

/// The text of a WHY box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KindDiagnosis {
    /// `Bad` or `Warn`.
    pub(crate) tone: StatusTone,
    /// Upper case, such as `1 OF 3 NOT READY`.
    pub(crate) title: String,
    pub(crate) text: String,
    /// The pod the text is about, for the "Open pod" link.
    pub(crate) pod: Option<ResourceKey>,
}

pub(crate) struct DiagnosisInputs<'a> {
    /// The pods the object owns, in snapshot order; `None` while the pods list has not loaded.
    pub(crate) pods: Option<&'a [&'a PodSummary]>,
    pub(crate) nodes: &'a [NodeSummary],
    /// Services only: what the pods and endpoint slices say. `pods` then holds the matching pods.
    pub(crate) service: Option<ServiceHealth>,
    pub(crate) now: Timestamp,
}

/// The WHY box of an object, or `None` when it needs none. Rules that need pods wait for them;
/// the rules that read only the object's own conditions (marked * in the spec) never do.
pub(crate) fn kind_diagnosis(
    object: &KindObject,
    inputs: &DiagnosisInputs,
) -> Option<KindDiagnosis> {
    match object {
        KindObject::Deployment(deployment) => deployment_diagnosis(deployment, inputs),
        KindObject::DaemonSet(set) => daemon_set_diagnosis(set, inputs),
        KindObject::Job(job) => job_diagnosis(job, inputs),
        KindObject::Service(service) => service_diagnosis(service, inputs),
        KindObject::PodDisruptionBudget(budget) => pod_disruption_budget_diagnosis(budget),
        KindObject::HorizontalPodAutoscaler(hpa) => horizontal_pod_autoscaler_diagnosis(hpa),
        KindObject::ResourceQuota(quota) => resource_quota_diagnosis(quota),
        KindObject::Plain
        | KindObject::CronJob(_)
        | KindObject::StatefulSet(_)
        | KindObject::ReplicaSet(_)
        | KindObject::Ingress(_)
        | KindObject::ConfigMap(_)
        | KindObject::NetworkPolicy(_) => None,
    }
}

/// The first owned pod (snapshot order) that `pod_diagnosis` finds a cause for.
fn unhealthy_pod<'a>(inputs: &DiagnosisInputs<'a>) -> Option<(&'a PodSummary, PodDiagnosis)> {
    inputs.pods?.iter().find_map(|pod| {
        let diagnosis = pod_diagnosis(pod, None, inputs.now)?;
        Some((*pod, diagnosis))
    })
}

/// The first owned pod whose cause is Bad: the rules that say "not ready" need a real failure.
fn failing_pod<'a>(inputs: &DiagnosisInputs<'a>) -> Option<(&'a PodSummary, PodDiagnosis)> {
    inputs.pods?.iter().find_map(|pod| {
        let diagnosis = pod_diagnosis(pod, None, inputs.now)?;
        (diagnosis.tone == StatusTone::Bad).then_some((*pod, diagnosis))
    })
}

pub(crate) fn find_condition<'a>(
    conditions: &'a [WorkloadCondition],
    name: &str,
) -> Option<&'a WorkloadCondition> {
    conditions.iter().find(|condition| condition.name == name)
}

/// `{reason}: {message}`, either part may be missing; `None` when both are.
fn reason_and_message(condition: &WorkloadCondition) -> Option<String> {
    match (&condition.reason, &condition.message) {
        (Some(reason), Some(message)) => Some(format!("{reason}: {message}")),
        (Some(text), None) | (None, Some(text)) => Some(text.clone()),
        (None, None) => None,
    }
}

fn plural<'a>(count: u32, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}

// ---- PodDisruptionBudgets ----

/// BLOCKS DRAIN: the budget refuses every eviction, so a node drain that reaches its pods waits.
/// Reads only the budget, so it shows while the pods list loads.
fn pod_disruption_budget_diagnosis(budget: &PodDisruptionBudgetSummary) -> Option<KindDiagnosis> {
    let DisruptionState::Blocked(cause) = budget.disruption_state() else {
        return None;
    };
    let limit = match (&budget.min_available, &budget.max_unavailable) {
        (Some(value), _) => format!("minAvailable is {value}"),
        (None, Some(value)) => format!("maxUnavailable is {value}"),
        (None, None) => "the budget allows no disruption".to_owned(),
    };
    let text = match cause {
        BlockCause::SyncFailed => {
            let detail = find_condition(&budget.conditions, "DisruptionAllowed")
                .and_then(|condition| condition.message.as_deref().or(condition.reason.as_deref()))
                .unwrap_or("no detail");
            format!(
                "The disruption controller cannot compute this budget ({detail}). Evictions of \
                 the selected pods are refused, so draining a node that runs them will wait."
            )
        }
        BlockCause::UnhealthyPods => format!(
            "Only {} of {} pods are healthy and {limit}. Draining any node that runs these pods \
             will wait.",
            budget.current_healthy, budget.expected_pods
        ),
        BlockCause::NoRoom if budget.expected_pods == 1 => format!(
            "{limit} and the only pod must stay up, so it cannot be evicted. Draining the node \
             that runs this pod will wait until the budget changes."
        ),
        BlockCause::NoRoom => format!(
            "{limit} and all {} pods must stay up, so no pod can be evicted. Draining any node \
             that runs these pods will wait until the budget changes.",
            budget.expected_pods
        ),
    };
    Some(KindDiagnosis {
        tone: StatusTone::Bad,
        title: "BLOCKS DRAIN".to_owned(),
        text,
        pod: None,
    })
}

// ---- HorizontalPodAutoscalers ----

/// Whether `reason` names a failed metric read (`FailedGetResourceMetric`, ...).
fn is_metric_failure(reason: &str) -> bool {
    (reason.starts_with("FailedGet") && reason.ends_with("Metric"))
        || reason == "InvalidMetricSourceType"
}

fn is_scale_failure(reason: &str) -> bool {
    matches!(reason, "FailedGetScale" | "FailedUpdateScale")
}

/// The WHY box of an HPA, first match: a failed metric read, a failed scale call, an inactive or
/// unable controller, then the max-replicas cap. Reads only the object. A target scaled to zero
/// by hand has no box.
fn horizontal_pod_autoscaler_diagnosis(
    hpa: &HorizontalPodAutoscalerSummary,
) -> Option<KindDiagnosis> {
    if is_scaling_disabled(hpa) {
        return None;
    }
    let failed =
        |name: &str| find_condition(&hpa.conditions, name).filter(|condition| !condition.is_true);
    let (active, able) = (failed("ScalingActive"), failed("AbleToScale"));
    let reason_is = |is_match: fn(&str) -> bool| {
        [active, able]
            .into_iter()
            .flatten()
            .find(|condition| condition.reason.as_deref().is_some_and(is_match))
    };
    let (title, condition) = if let Some(condition) = reason_is(is_metric_failure) {
        ("METRICS UNAVAILABLE", condition)
    } else if let Some(condition) = reason_is(is_scale_failure) {
        ("CANNOT SCALE", condition)
    } else if let Some(condition) = active {
        ("SCALING INACTIVE", condition)
    } else if let Some(condition) = able {
        ("CANNOT SCALE", condition)
    } else {
        return at_max_diagnosis(hpa);
    };
    Some(KindDiagnosis {
        tone: StatusTone::Bad,
        title: title.to_owned(),
        text: reason_and_message(condition).unwrap_or_else(|| "No detail was given.".to_owned()),
        pod: None,
    })
}

fn at_max_diagnosis(hpa: &HorizontalPodAutoscalerSummary) -> Option<KindDiagnosis> {
    if !is_at_max(hpa) {
        return None;
    }
    let mut text = format!(
        "Running {} of max {} replicas and the metrics ask for more.",
        hpa.current_replicas, hpa.max_replicas
    );
    if let Some(metric) = hpa
        .metrics
        .iter()
        .find(|metric| is_above_target(metric) == Some(true))
    {
        text.push_str(&format!(" {} is above target.", metric_text(metric)));
    }
    text.push_str(" Raise maxReplicas or reduce the load.");
    Some(KindDiagnosis {
        tone: StatusTone::Bad,
        title: "AT MAX REPLICAS".to_owned(),
        text,
        pod: None,
    })
}

// ---- ResourceQuotas ----

fn resource_quota_diagnosis(quota: &ResourceQuotaSummary) -> Option<KindDiagnosis> {
    // The same item the status names: the fullest one, when it is at its limit.
    let (item, _) = fullest_item(quota).filter(|(_, ratio)| *ratio >= 1.0)?;
    let usage = quota_text(item);
    Some(KindDiagnosis {
        tone: StatusTone::Bad,
        title: "AT QUOTA".to_owned(),
        text: format!(
            "{} is at its limit ({usage}). New objects that need it are rejected; see Blocked \
             creations.",
            item.resource
        ),
        pod: None,
    })
}

// ---- Deployments ----

fn deployment_diagnosis(
    deployment: &DeploymentSummary,
    inputs: &DiagnosisInputs,
) -> Option<KindDiagnosis> {
    // A scaled-to-zero or paused Deployment is not expected to progress.
    if deployment.desired == 0 || deployment.is_paused {
        return None;
    }
    let is_stalled = find_condition(&deployment.conditions, PROGRESSING).is_some_and(|condition| {
        !condition.is_true && condition.reason.as_deref() == Some(DEADLINE_EXCEEDED)
    });
    if is_stalled {
        // A stall that long explains itself with any cause, a warning included.
        let unhealthy = unhealthy_pod(inputs);
        let mut text = format!("No progress for {}s.", deployment.progress_deadline_seconds);
        if let Some((pod, diagnosis)) = &unhealthy {
            text.push_str(&format!(" Pod {}: {}", pod.name, diagnosis.text));
        }
        return Some(KindDiagnosis {
            tone: StatusTone::Bad,
            title: "ROLLOUT STALLED".to_owned(),
            text,
            pod: unhealthy.map(|(pod, _)| ResourceKey::of_pod(pod)),
        });
    }
    if let Some(condition) = find_condition(&deployment.conditions, "ReplicaFailure")
        && condition.is_true
    {
        return Some(KindDiagnosis {
            tone: StatusTone::Bad,
            title: "REPLICA FAILURE".to_owned(),
            text: reason_and_message(condition)
                .unwrap_or_else(|| "The controller cannot create pods.".to_owned()),
            pod: None,
        });
    }
    if deployment.ready >= deployment.desired {
        return None;
    }
    // Only a Bad cause: a pod that is merely warming up (running, not ready yet) is a normal
    // rollout, and a long stall is caught by ROLLOUT STALLED.
    let (pod, diagnosis) = failing_pod(inputs)?;
    // A container cause reads "Pod x is CrashLoopBackOff: ..."; a pod-level one already says it.
    let text = match diagnosis.container {
        Some(_) => format!(
            "Pod {} is {}: {}",
            pod.name,
            pod_status_label(pod).text,
            diagnosis.text
        ),
        None => format!("Pod {}: {}", pod.name, diagnosis.text),
    };
    Some(KindDiagnosis {
        tone: if deployment.ready == 0 {
            StatusTone::Bad
        } else {
            StatusTone::Warn
        },
        title: format!(
            "{} OF {} NOT READY",
            deployment.desired - deployment.ready,
            deployment.desired
        ),
        text,
        pod: Some(ResourceKey::of_pod(pod)),
    })
}

// ---- DaemonSets ----

/// A pod that is not running with every container ready.
pub(crate) fn is_pod_not_ready(pod: &PodSummary) -> bool {
    pod.status != PodStatus::Reason(StatusReason::Running) || pod.ready.ready < pod.ready.total
}

/// The state of the node `pod` runs on when that node is not Ready; `None` for a Ready node, an
/// unscheduled pod, or a node the list does not know.
pub(crate) fn unready_node<'a>(
    pod: &PodSummary,
    nodes: &'a [NodeSummary],
) -> Option<(&'a str, NodeReadiness)> {
    let name = pod.node_name.as_deref()?;
    let node = nodes.iter().find(|node| node.name == name)?;
    match node.status.readiness {
        NodeReadiness::Ready => None,
        readiness @ (NodeReadiness::NotReady | NodeReadiness::Unknown) => {
            Some((node.name.as_str(), readiness))
        }
    }
}

fn daemon_set_diagnosis(set: &DaemonSetSummary, inputs: &DiagnosisInputs) -> Option<KindDiagnosis> {
    if set.desired == 0 {
        return None;
    }
    if let Some(pods) = inputs.pods {
        // S1: pods that wait on a node that is down.
        let stranded: Vec<(&PodSummary, &str, NodeReadiness)> = pods
            .iter()
            .filter(|pod| is_pod_not_ready(pod))
            .filter_map(|pod| {
                let (node, readiness) = unready_node(pod, inputs.nodes)?;
                Some((*pod, node, readiness))
            })
            .collect();
        if let Some(&(pod, node, readiness)) = stranded.first() {
            let count = u32::try_from(stranded.len()).unwrap_or(u32::MAX);
            let state = if stranded.iter().all(|(_, _, other)| *other == readiness) {
                readiness_text(readiness)
            } else {
                "not Ready"
            };
            let text = if count == 1 {
                format!("The pod on {node} is not ready because the node is {state}.")
            } else {
                format!(
                    "Pods on {node} and {} more nodes are not ready because their nodes are {state}.",
                    count - 1
                )
            };
            return Some(KindDiagnosis {
                tone: StatusTone::Warn,
                title: format!("{count} {} MISSING", plural(count, "NODE", "NODES")),
                text,
                pod: Some(ResourceKey::of_pod(pod)),
            });
        }
        // S2: a pod on a healthy node that has its own problem.
        if set.ready < set.desired
            && let Some((pod, diagnosis)) = failing_pod(inputs)
        {
            let place = pod
                .node_name
                .as_deref()
                .map(|node| format!(" on {node}"))
                .unwrap_or_default();
            return Some(KindDiagnosis {
                tone: StatusTone::Warn,
                title: format!("{} OF {} NOT READY", set.desired - set.ready, set.desired),
                text: format!("Pod {}{place}: {}", pod.name, diagnosis.text),
                pod: Some(ResourceKey::of_pod(pod)),
            });
        }
    }
    // S3
    if set.current < set.desired {
        let missing = set.desired - set.current;
        return Some(KindDiagnosis {
            tone: StatusTone::Warn,
            title: format!(
                "{missing} {} WITHOUT A POD",
                plural(missing, "NODE", "NODES")
            ),
            text: format!(
                "{} {} should run a pod; {} do.",
                set.desired,
                plural(set.desired, "node", "nodes"),
                set.current
            ),
            pod: None,
        });
    }
    // S4
    if set.misscheduled > 0 {
        let text = if set.misscheduled == 1 {
            "1 pod runs on a node the DaemonSet no longer targets.".to_owned()
        } else {
            format!(
                "{} pods run on nodes the DaemonSet no longer targets.",
                set.misscheduled
            )
        };
        return Some(KindDiagnosis {
            tone: StatusTone::Warn,
            title: "MISSCHEDULED".to_owned(),
            text,
            pod: None,
        });
    }
    None
}

// ---- Jobs ----

/// The termination of the pod's first main container: where it is now, else its last one. The
/// Attempts section shows its exit code, and the BACKOFF LIMIT box quotes it.
pub(crate) fn first_main_termination(pod: &PodSummary) -> Option<&Termination> {
    let container = pod
        .containers
        .iter()
        .find(|container| container.kind == ContainerKind::Main)?;
    match &container.state {
        ContainerState::Terminated(termination) => Some(termination),
        ContainerState::Waiting { .. }
        | ContainerState::Running { .. }
        | ContainerState::NotReported => container.last_termination.as_ref(),
    }
}

fn job_diagnosis(job: &JobSummary, inputs: &DiagnosisInputs) -> Option<KindDiagnosis> {
    match job.status {
        JobStatus::Failed | JobStatus::Failing => failed_job_diagnosis(job, inputs),
        JobStatus::Running if job.failed > 0 => {
            let limit = job.backoff_limit.unwrap_or(DEFAULT_BACKOFF_LIMIT);
            Some(KindDiagnosis {
                tone: StatusTone::Warn,
                title: format!(
                    "{} FAILED {}",
                    job.failed,
                    plural(job.failed, "ATTEMPT", "ATTEMPTS")
                ),
                text: format!(
                    "Retrying; the job fails after {} failed attempts.",
                    limit.saturating_add(1)
                ),
                pod: None,
            })
        }
        JobStatus::Running | JobStatus::Complete | JobStatus::Suspended => None,
    }
}

fn failed_job_diagnosis(job: &JobSummary, inputs: &DiagnosisInputs) -> Option<KindDiagnosis> {
    let condition = job.conditions.iter().find(|condition| {
        matches!(condition.name.as_str(), "Failed" | "FailureTarget") && condition.is_true
    })?;
    match condition.reason.as_deref() {
        Some(BACKOFF_LIMIT_EXCEEDED) => {
            let last = last_failed_pod(inputs);
            let mut text = format!(
                "{} {} failed.",
                job.failed,
                plural(job.failed, "attempt", "attempts")
            );
            if let Some((_, termination)) = last {
                text.push_str(&format!(
                    " Last pod exited with code {}{}.",
                    termination.exit_code,
                    termination
                        .reason
                        .as_ref()
                        .map(|reason| format!(" ({reason})"))
                        .unwrap_or_default()
                ));
            }
            Some(KindDiagnosis {
                tone: StatusTone::Bad,
                title: "BACKOFF LIMIT REACHED".to_owned(),
                text,
                pod: last.map(|(pod, _)| ResourceKey::of_pod(pod)),
            })
        }
        Some(JOB_DEADLINE_EXCEEDED) => Some(KindDiagnosis {
            tone: StatusTone::Bad,
            title: "DEADLINE EXCEEDED".to_owned(),
            text: match job.active_deadline_seconds {
                Some(seconds) => {
                    format!("The job ran longer than its active deadline of {seconds}s.")
                }
                None => "The job ran longer than its active deadline.".to_owned(),
            },
            pod: None,
        }),
        _ => Some(KindDiagnosis {
            tone: StatusTone::Bad,
            title: "JOB FAILED".to_owned(),
            text: reason_and_message(condition).unwrap_or_else(|| "The job failed.".to_owned()),
            pod: None,
        }),
    }
}

/// The newest owned pod whose first main container ended with a failure, and that termination.
fn last_failed_pod<'a>(inputs: &DiagnosisInputs<'a>) -> Option<(&'a PodSummary, &'a Termination)> {
    inputs
        .pods?
        .iter()
        .filter_map(|pod| {
            let termination = first_main_termination(pod).filter(|term| term.exit_code != 0)?;
            Some((*pod, termination))
        })
        .max_by_key(|(pod, _)| pod.created_at)
}

// ---- Services ----

fn service_diagnosis(service: &ServiceSummary, inputs: &DiagnosisInputs) -> Option<KindDiagnosis> {
    let health = inputs.service?;
    // V1: the selector finds no pod. The health leaves `matching_pods` unknown for a Service that
    // has no selector or is an ExternalName, so neither gets this box.
    if health.matching_pods == Some(0) {
        return Some(KindDiagnosis {
            tone: StatusTone::Bad,
            title: "NO MATCHING PODS".to_owned(),
            text: format!(
                "No pod in {} has the labels {}.",
                service.namespace,
                service.selector.join(", ")
            ),
            pod: None,
        });
    }
    // V2: endpoints exist and none takes traffic. Like every rule that reads pods, it waits for
    // the pods list.
    inputs.pods?;
    let counts = health.endpoints.filter(|_| health.is_unserved())?;
    let unhealthy = unhealthy_pod(inputs);
    let mut text = format!(
        "{} {}, none ready.",
        counts.total,
        if counts.total == 1 {
            "endpoint"
        } else {
            "endpoints"
        }
    );
    if let Some((pod, diagnosis)) = &unhealthy {
        text.push_str(&format!(" Pod {}: {}", pod.name, diagnosis.text));
    }
    Some(KindDiagnosis {
        tone: StatusTone::Bad,
        title: "NO READY ENDPOINTS".to_owned(),
        text,
        pod: unhealthy.map(|(pod, _)| ResourceKey::of_pod(pod)),
    })
}

#[cfg(test)]
#[path = "kind_diagnosis_tests.rs"]
mod kind_diagnosis_tests;
