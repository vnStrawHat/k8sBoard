//! The WHY box of Deployments, DaemonSets, Jobs, Services, PodDisruptionBudgets, HPAs,
//! ResourceQuotas, PVCs, and PVs: what is wrong and, when the pods say so, why. Pure: the drawer
//! reads the live lists and calls `kind_diagnosis`. Pod causes reuse `pod_diagnosis` without
//! events, so probe-failure detail stays in the pod drawer. Condition and status messages are
//! arbitrary text, so nothing here logs them.

use cluster::{
    BindingSummary, BlockCause, BroadGroup, CertificateIssue, ConditionStatus, ContainerKind,
    ContainerState, CronJobSummary, CustomObjectSummary, DaemonSetSummary, DeploymentSummary,
    DisruptionState, EventSummary, EventType, HelmReleaseSummary, HelmStatus,
    HorizontalPodAutoscalerSummary, IngressSummary, JobStatus, JobSummary, NamespacePhase,
    NamespaceSummary, NodeReadiness, NodeSummary, PersistentVolumeClaimSummary,
    PersistentVolumeSummary, PodDisruptionBudgetSummary, PodStatus, PodSummary,
    ResourceQuotaSummary, RoleSummary, SecretDetails, SecretSummary, ServiceAccountSummary,
    ServiceSummary, StatusReason, Subject, SubjectKind, Termination, WorkloadCondition,
};
use jiff::Timestamp;

use crate::access_bindings::{
    BindingIndex, BroadAdmin, broad_admin, is_cluster_admin, service_account_text,
};
use crate::age::format_age;
use crate::batch_rows::{CronState, cron_state_at, skipped_run};
use crate::certificate_expiry::{ExpiryState, date_text, expiry_state};
use crate::custom_rows::is_failing;
use crate::event_rows::message_line;
use crate::ingress_backends::{IngressBackends, backend_problems};
use crate::kind_join::{ServiceHealth, tls_secret_names};
use crate::kind_row::KindObject;
use crate::namespace_rows::STUCK_AFTER;
use crate::network_rows::find_secret;
use crate::pod_diagnosis::{PodDiagnosis, pod_diagnosis};
use crate::policy_rows::{
    fullest_item, is_above_target, is_at_max, is_scaling_disabled, metric_text, quota_text,
};
use crate::quota_room::{QuotaExceeded, quota_exceeded};
use crate::status_tone::{StatusTone, pod_status_label, readiness_text};
use crate::table_selection::ResourceKey;
use crate::workload_rows::{DEADLINE_EXCEEDED, PROGRESSING};

/// The `Job` condition reasons the controller reports when it gives up.
const BACKOFF_LIMIT_EXCEEDED: &str = "BackoffLimitExceeded";
const JOB_DEADLINE_EXCEEDED: &str = "DeadlineExceeded";
/// The API default of a Job's `backoffLimit`.
const DEFAULT_BACKOFF_LIMIT: u32 = 6;

/// The start of the CERTIFICATE box text about a TLS secret that the list does not hold; Topology
/// draws a missing secret itself, so it skips this box.
pub(crate) const NO_TLS_SECRET: &str = "No TLS secret";
/// The title of every certificate box.
pub(crate) const CERTIFICATE_TITLE: &str = "CERTIFICATE";

/// The text of a WHY box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KindDiagnosis {
    /// `Bad` or `Warn`.
    pub(crate) tone: StatusTone,
    /// Upper case, such as `1 OF 3 NOT READY`.
    pub(crate) title: String,
    pub(crate) text: String,
    /// The object the text is about, for the "Open …" link: a pod, or the Secret of an Ingress.
    pub(crate) link: Option<ResourceKey>,
}

pub(crate) struct DiagnosisInputs<'a> {
    /// The pods the object owns, in snapshot order; `None` while the pods list has not loaded.
    pub(crate) pods: Option<&'a [&'a PodSummary]>,
    pub(crate) nodes: &'a [NodeSummary],
    /// Services only: what the pods and endpoint slices say. `pods` then holds the matching pods.
    pub(crate) service: Option<ServiceHealth>,
    /// ClusterRoles: the bindings of a ready Bindings companion; `None` while it is not ready.
    pub(crate) bindings: Option<&'a BindingIndex<'a>>,
    /// Ingresses: the TLS secrets of a ready companion; `None` while it is not ready or denied.
    pub(crate) tls_secrets: Option<&'a [SecretSummary]>,
    /// PVCs: the events of the open drawer's object once they have loaded; `None` before that.
    pub(crate) events: Option<&'a [EventSummary]>,
    /// Ingresses: the Services of the namespace and the pods, once both lists have loaded.
    pub(crate) backends: Option<IngressBackends<'a>>,
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
        KindObject::CronJob(cron_job) => cron_job_diagnosis(cron_job, inputs.now),
        KindObject::Service(service) => service_diagnosis(service, inputs),
        KindObject::PodDisruptionBudget(budget) => pod_disruption_budget_diagnosis(budget),
        KindObject::HorizontalPodAutoscaler(hpa) => horizontal_pod_autoscaler_diagnosis(hpa),
        KindObject::ResourceQuota(quota) => resource_quota_diagnosis(quota),
        KindObject::PersistentVolumeClaim(claim) => claim_diagnosis(claim, inputs.events),
        KindObject::PersistentVolume(volume) => volume_diagnosis(volume),
        KindObject::Role(role) => role_diagnosis(role, inputs.bindings),
        KindObject::Binding(binding) => binding_diagnosis(binding),
        KindObject::ServiceAccount(account) => service_account_diagnosis(account, inputs.bindings),
        KindObject::Secret(secret) => secret_diagnosis(secret, inputs.now),
        KindObject::Ingress(ingress) => ingress_diagnosis(ingress, inputs),
        KindObject::HelmRelease(release) => helm_release_diagnosis(release, inputs.now),
        KindObject::Custom(summary) => custom_object_diagnosis(summary),
        KindObject::Namespace(namespace) => namespace_diagnosis(namespace, inputs.now),
        KindObject::Plain
        | KindObject::StatefulSet(_)
        | KindObject::ReplicaSet(_)
        | KindObject::ConfigMap(_)
        | KindObject::NetworkPolicy(_)
        | KindObject::StorageClass(_)
        | KindObject::Crd(_) => None,
    }
}

/// A namespace that has been Terminating for more than `STUCK_AFTER`: Bad when the controller
/// reports a failure, else Warn. The text is the controller's own messages, which can name
/// resource types and finalizers, never logged.
fn namespace_diagnosis(namespace: &NamespaceSummary, now: Timestamp) -> Option<KindDiagnosis> {
    if namespace.phase != NamespacePhase::Terminating {
        return None;
    }
    let since = namespace.deleting_since?;
    if now.duration_since(since) <= STUCK_AFTER {
        return None;
    }
    let has_failure = namespace
        .deletion_conditions
        .iter()
        .any(|condition| condition.name.ends_with("Failure"));
    let messages: Vec<&str> = namespace
        .deletion_conditions
        .iter()
        .filter_map(|condition| condition.message.as_deref())
        .collect();
    let reasons = if messages.is_empty() {
        "The namespace reports no reason.".to_owned()
    } else {
        messages.join(" ")
    };
    Some(KindDiagnosis {
        tone: if has_failure {
            StatusTone::Bad
        } else {
            StatusTone::Warn
        },
        title: "STUCK".to_owned(),
        text: format!(
            "Terminating for {}. {reasons}",
            format_age(Some(since), now)
        ),
        link: None,
    })
}

/// A custom object: its own conditions, read at once. The first match wins: Ready or Available
/// False is Bad, Ready Unknown is Warn, then a failing condition is Warn. The messages are the
/// controller's text and can quote a field value, so they are shown here and never logged.
fn custom_object_diagnosis(summary: &CustomObjectSummary) -> Option<KindDiagnosis> {
    let find = |name: &str| {
        summary
            .conditions
            .iter()
            .find(|condition| condition.name == name)
    };
    for (name, title) in [("Ready", "NOT READY"), ("Available", "UNAVAILABLE")] {
        let Some(condition) = find(name).filter(|c| c.status == ConditionStatus::False) else {
            continue;
        };
        let text = match (&condition.message, &condition.reason) {
            (Some(message), _) => message.clone(),
            (None, Some(reason)) => format!("{name} is False: {reason}"),
            (None, None) => format!("{name} is False"),
        };
        return Some(KindDiagnosis {
            tone: StatusTone::Bad,
            title: title.to_owned(),
            text,
            link: None,
        });
    }
    if let Some(condition) = find("Ready").filter(|c| c.status == ConditionStatus::Unknown) {
        let text = condition
            .message
            .clone()
            .unwrap_or_else(|| "Ready is Unknown".to_owned());
        return Some(KindDiagnosis {
            tone: StatusTone::Warn,
            title: "READY UNKNOWN".to_owned(),
            text,
            link: None,
        });
    }
    let failing = summary.conditions.iter().find(|c| is_failing(c))?;
    let detail = failing
        .message
        .as_deref()
        .or(failing.reason.as_deref())
        .unwrap_or_default();
    Some(KindDiagnosis {
        tone: StatusTone::Warn,
        title: format!("{} FAILING", failing.name.to_uppercase()),
        text: format!("{}: {detail}", failing.name),
        link: None,
    })
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

/// `FailedCreate: quota team-quota: limits.memory 600Mi of 640Mi used, needs 150Mi`: the numbers
/// of the admission failure instead of the pod name and the raw message.
fn quota_failure_text(condition: &WorkloadCondition, quota: &QuotaExceeded) -> String {
    let lines = quota.lines.join("; ");
    match &condition.reason {
        Some(reason) => format!("{reason}: quota {}: {lines}", quota.quota),
        None => format!("Quota {}: {lines}", quota.quota),
    }
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
        link: None,
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
        link: None,
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
        link: None,
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
        link: None,
    })
}

// ---- Storage ----

/// VOLUME LOST: the volume a claim was bound to is gone (reads only the claim). PENDING: the
/// newest Warning event of a claim nothing provisions.
fn claim_diagnosis(
    claim: &PersistentVolumeClaimSummary,
    events: Option<&[EventSummary]>,
) -> Option<KindDiagnosis> {
    match claim.phase.as_str() {
        "Lost" => Some(lost_claim_diagnosis(claim)),
        "Pending" => pending_claim_diagnosis(events?),
        _ => None,
    }
}

/// A claim that waits for its first consumer has only Normal events, so it gets no box.
fn pending_claim_diagnosis(events: &[EventSummary]) -> Option<KindDiagnosis> {
    let newest = newest_warning(events)?;
    Some(KindDiagnosis {
        tone: StatusTone::Warn,
        title: "PENDING".to_owned(),
        text: format!("{}: {}", newest.reason, message_line(&newest.message)),
        link: None,
    })
}

fn newest_warning(events: &[EventSummary]) -> Option<&EventSummary> {
    events
        .iter()
        .filter(|event| event.event_type == EventType::Warning)
        .max_by_key(|event| event.last_seen)
}

/// The claim's StorageClass when the newest Warning event says the cluster has no such class
/// (`storageclass.storage.k8s.io "fast-ssd" not found`). The StorageClasses list is not loaded on
/// the claim screens, so the provisioner's own event is the evidence.
pub(crate) fn missing_storage_class<'a>(
    claim: &'a PersistentVolumeClaimSummary,
    events: &[EventSummary],
) -> Option<&'a str> {
    let class = claim.storage_class.as_deref()?;
    let newest = newest_warning(events)?;
    newest
        .message
        .contains(&format!("\"{class}\" not found"))
        .then_some(class)
}

fn lost_claim_diagnosis(claim: &PersistentVolumeClaimSummary) -> KindDiagnosis {
    let gone = claim.volume.as_deref().map_or_else(
        || "The bound volume no longer exists.".to_owned(),
        |volume| format!("The bound volume {volume} no longer exists."),
    );
    KindDiagnosis {
        tone: StatusTone::Bad,
        title: "VOLUME LOST".to_owned(),
        text: format!("{gone} The data on it is gone or unreachable."),
        link: None,
    }
}

/// RELEASED: the claim is gone but the volume is not; RECLAIM FAILED: the reclaim policy could not
/// run. Reads only the volume.
fn volume_diagnosis(volume: &PersistentVolumeSummary) -> Option<KindDiagnosis> {
    match volume.phase.as_str() {
        "Released" => Some(released_diagnosis(volume)),
        "Failed" => Some(KindDiagnosis {
            tone: StatusTone::Bad,
            title: "RECLAIM FAILED".to_owned(),
            text: reclaim_failure_text(volume),
            link: None,
        }),
        _ => None,
    }
}

fn released_diagnosis(volume: &PersistentVolumeSummary) -> KindDiagnosis {
    let claim = volume.claim.as_ref().map_or_else(
        || "The claim".to_owned(),
        |claim| format!("Claim {}/{}", claim.namespace, claim.name),
    );
    let text = if volume.reclaim_policy == "Retain" {
        format!(
            "{claim} was deleted. Reclaim policy Retain keeps the data on the volume. Delete the \
             PV and its backing volume to free the space, or clear claimRef to bind it again."
        )
    } else {
        format!(
            "{claim} was deleted and reclaim policy is {}, but the volume still exists. Check the \
             Events tab for reclaim errors.",
            volume.reclaim_policy
        )
    };
    KindDiagnosis {
        tone: StatusTone::Warn,
        title: "RELEASED".to_owned(),
        text,
        link: None,
    }
}

/// `{reason}: {message}`, either part may be missing.
fn reclaim_failure_text(volume: &PersistentVolumeSummary) -> String {
    match (&volume.reason, &volume.message) {
        (Some(reason), Some(message)) => format!("{reason}: {message}"),
        (Some(text), None) | (None, Some(text)) => text.clone(),
        (None, None) => "The volume could not be reclaimed. Check the Events tab.".to_owned(),
    }
}

// ---- Access control ----

fn plural_count(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// VERY BROAD: a role with a rule for every verb on every resource. A ClusterRole also says who
/// holds it once the bindings are known.
fn role_diagnosis(role: &RoleSummary, bindings: Option<&BindingIndex>) -> Option<KindDiagnosis> {
    if !role.grants_everything() {
        return None;
    }
    let text = match &role.namespace {
        Some(namespace) => format!("Grants every verb on every resource in {namespace}."),
        None => {
            let mut text = "Grants every verb on every resource.".to_owned();
            if let Some(bindings) = bindings {
                text.push_str(&bound_to_text(&bindings.bindings_of_role(role)));
            }
            text
        }
    };
    Some(KindDiagnosis {
        tone: StatusTone::Warn,
        title: "VERY BROAD".to_owned(),
        text,
        link: None,
    })
}

/// ` Bound to 3 subjects, including sa kube-system/tiller.`
fn bound_to_text(bindings: &[&BindingSummary]) -> String {
    // The same subject in two bindings is one subject.
    let mut subjects: Vec<&Subject> = Vec::new();
    for subject in bindings.iter().flat_map(|binding| &binding.subjects) {
        if !subjects.contains(&subject) {
            subjects.push(subject);
        }
    }
    let count = subjects.len();
    let mut text = format!(" Bound to {}", plural_count(count, "subject", "subjects"));
    if let Some(account) = subjects
        .iter()
        .find(|subject| subject.kind == SubjectKind::ServiceAccount)
    {
        text.push_str(&format!(", including sa {}", service_account_text(account)));
    }
    text.push('.');
    text
}

/// CLUSTER ADMIN: a role the account holds, directly or through its groups, is cluster-admin. A
/// ClusterRoleBinding is named before a RoleBinding because it reaches the whole cluster. Waits
/// for the bindings, so it shows only once they are all loaded.
fn service_account_diagnosis(
    account: &ServiceAccountSummary,
    bindings: Option<&BindingIndex>,
) -> Option<KindDiagnosis> {
    let roles = bindings?.roles_held(&account.namespace, &account.name);
    let admin: Vec<_> = roles
        .into_iter()
        .filter(|bound| is_cluster_admin(&bound.role))
        .collect();
    let through = admin
        .iter()
        .find(|bound| bound.binding_namespace().is_none())
        .or(admin.first())?;
    let text = match through.binding_namespace() {
        None => format!(
            "This service account has full access to the cluster through {}.",
            through.binding_text
        ),
        Some(namespace) => format!(
            "This service account has full access to namespace {namespace} through {}.",
            through.binding_text
        ),
    };
    Some(KindDiagnosis {
        tone: StatusTone::Warn,
        title: "CLUSTER ADMIN".to_owned(),
        text,
        link: None,
    })
}

/// REVIEW: cluster-admin handed to a broad group or to service accounts. A group is checked first
/// because it reaches the most callers.
fn binding_diagnosis(binding: &BindingSummary) -> Option<KindDiagnosis> {
    let admin = broad_admin(binding)?;
    let place = match &binding.namespace {
        Some(namespace) => format!("namespace {namespace}"),
        None => "the cluster".to_owned(),
    };
    let groups: Vec<_> = binding
        .subjects
        .iter()
        .filter_map(|subject| Some((subject, subject.broad_group()?)))
        .collect();
    let broadest = groups
        .iter()
        .find(|(_, group)| {
            matches!(
                group,
                BroadGroup::Authenticated | BroadGroup::Unauthenticated
            )
        })
        .or(groups.first());
    if let Some((subject, group)) = broadest {
        let who = match group {
            BroadGroup::Authenticated => "every signed-in user and service account".to_owned(),
            BroadGroup::Unauthenticated => "anonymous requests".to_owned(),
            BroadGroup::AllServiceAccounts => "every service account".to_owned(),
            BroadGroup::NamespaceServiceAccounts(namespace) => {
                format!("every service account in {namespace}")
            }
        };
        let tone = match admin {
            BroadAdmin::Everyone => StatusTone::Bad,
            BroadAdmin::ServiceAccounts => StatusTone::Warn,
        };
        return Some(KindDiagnosis {
            tone,
            title: "REVIEW".to_owned(),
            text: format!("Group {} gives {who} full access to {place}.", subject.name),
            link: None,
        });
    }
    let accounts: Vec<String> = binding
        .subjects
        .iter()
        .filter(|subject| subject.kind == SubjectKind::ServiceAccount)
        .map(service_account_text)
        .collect();
    let first = accounts.first()?;
    let text = match (accounts.len(), &binding.namespace) {
        (1, Some(_)) => {
            format!("Service account {first} has full access to {place}. Consider a narrower Role.")
        }
        (1, None) => format!(
            "Service account {first} has full access to {place}. Consider a namespaced Role instead."
        ),
        (count, _) => {
            format!("{count} service accounts, including {first}, have full access to {place}.")
        }
    };
    Some(KindDiagnosis {
        tone: StatusTone::Warn,
        title: "REVIEW".to_owned(),
        text,
        link: None,
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
            link: unhealthy.map(|(pod, _)| ResourceKey::of_pod(pod)),
        });
    }
    if let Some(condition) = find_condition(&deployment.conditions, "ReplicaFailure")
        && condition.is_true
    {
        let quota = condition.message.as_deref().and_then(quota_exceeded);
        return Some(KindDiagnosis {
            tone: StatusTone::Bad,
            title: "REPLICA FAILURE".to_owned(),
            text: match &quota {
                Some(quota) => quota_failure_text(condition, quota),
                None => reason_and_message(condition)
                    .unwrap_or_else(|| "The controller cannot create pods.".to_owned()),
            },
            link: quota.and_then(|quota| {
                ResourceKey::of_object("ResourceQuota", Some(&deployment.namespace), &quota.quota)
            }),
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
        link: Some(ResourceKey::of_pod(pod)),
    })
}

/// The box of a Deployment whose rollout is still going: the newest ReplicaSet does not hold every
/// replica yet, or not all of them are available. While old pods keep the Deployment green, this is
/// the only place the drawer says so. It is not a problem by itself, so the issue feeds and the
/// topology checks never read it; the drawer asks for it when `kind_diagnosis` has nothing. A pod
/// with a Bad cause among the new pods (created after the last write of the pod template) is named,
/// so a broken release shows its reason before the progress deadline passes. Needs no pods list.
pub(crate) fn rollout_progress(
    deployment: &DeploymentSummary,
    inputs: &DiagnosisInputs,
) -> Option<KindDiagnosis> {
    if deployment.desired == 0 || deployment.is_paused {
        return None;
    }
    let is_complete =
        deployment.up_to_date >= deployment.desired && deployment.available >= deployment.desired;
    if is_complete {
        return None;
    }
    let mut text = format!(
        "{} of {} pods run the new template; {} of {} are available.",
        deployment.up_to_date, deployment.desired, deployment.available, deployment.desired
    );
    let newest_write = deployment.template_change.as_ref().map(|writer| writer.at);
    let problem = inputs.pods.into_iter().flatten().find_map(|pod| {
        let is_new = match (pod.created_at, newest_write) {
            (Some(created), Some(written)) => created >= written,
            _ => true,
        };
        let diagnosis = pod_diagnosis(pod, None, inputs.now)
            .filter(|diagnosis| is_new && diagnosis.tone == StatusTone::Bad)?;
        Some((*pod, diagnosis))
    });
    if let Some((pod, diagnosis)) = &problem {
        let new_pod = match diagnosis.container {
            Some(_) => format!(
                "New pod {} is {}: {}",
                pod.name,
                pod_status_label(pod).text,
                diagnosis.text
            ),
            None => format!("New pod {}: {}", pod.name, diagnosis.text),
        };
        text.push(' ');
        text.push_str(&new_pod);
    }
    Some(KindDiagnosis {
        tone: StatusTone::Warn,
        title: "ROLLOUT IN PROGRESS".to_owned(),
        text,
        link: problem.map(|(pod, _)| ResourceKey::of_pod(pod)),
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
                link: Some(ResourceKey::of_pod(pod)),
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
                link: Some(ResourceKey::of_pod(pod)),
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
            link: None,
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
            link: None,
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
                link: None,
            })
        }
        JobStatus::Running | JobStatus::Complete | JobStatus::Suspended => None,
    }
}

/// A CronJob that is suspended, missed a run, or whose last run failed, with the next step. It
/// reads only the CronJob and the clock, so it shows while the lists load; the failed run is read
/// from Recent jobs below, because the CronJob names no finished Job.
fn cron_job_diagnosis(cron_job: &CronJobSummary, now: Timestamp) -> Option<KindDiagnosis> {
    if let Some(expected_at) = skipped_run(cron_job, now) {
        return Some(skipping_runs(cron_job, expected_at, now));
    }
    let (tone, title, text) = match cron_state_at(cron_job, now) {
        CronState::Suspended => (
            StatusTone::Warn,
            "SUSPENDED",
            concat!(
                "Suspend is on, so no run is scheduled. Resume the CronJob to run on schedule ",
                "again, or Trigger now for a single run."
            )
            .to_owned(),
        ),
        CronState::Missed { expected_at } => {
            let deadline = cron_job.starting_deadline_seconds.unwrap_or(100);
            (
                StatusTone::Warn,
                "SCHEDULE MISSED",
                format!(
                    concat!(
                        "No job started for the run due {} ago, and its starting deadline of ",
                        "{}s has passed. The CronJob controller may be down or too busy, or the ",
                        "deadline too short. Trigger now to run it once."
                    ),
                    format_age(Some(expected_at), now),
                    deadline
                ),
            )
        }
        CronState::LastRunFailed => (
            StatusTone::Warn,
            "LAST RUN FAILED",
            format!(
                concat!(
                    "The run started {} ago did not succeed. Open its job under Recent jobs for ",
                    "the reason, or Trigger now to retry."
                ),
                format_age(cron_job.last_schedule_at, now)
            ),
        ),
        CronState::Running | CronState::NeverRun | CronState::LastRunSucceeded => return None,
    };
    Some(KindDiagnosis {
        tone,
        title: title.to_owned(),
        text,
        link: None,
    })
}

/// SKIPPING RUNS: the active Job holds the CronJob, so the run due `expected_at` and every later one
/// is skipped until it ends. The job started at the last schedule time.
fn skipping_runs(
    cron_job: &CronJobSummary,
    expected_at: Timestamp,
    now: Timestamp,
) -> KindDiagnosis {
    let job = cron_job.active_jobs.first().map_or("A job", String::as_str);
    let more = match cron_job.active_jobs.len() {
        0 | 1 => String::new(),
        others => format!(" and {} more", others - 1),
    };
    KindDiagnosis {
        tone: StatusTone::Warn,
        title: "SKIPPING RUNS".to_owned(),
        text: format!(
            concat!(
                "{}{} still active since {} ago, concurrencyPolicy Forbid: the run due {} ago ",
                "was skipped, and so is every run until it ends. Delete the job to let the ",
                "schedule run again."
            ),
            job,
            more,
            format_age(cron_job.last_schedule_at, now),
            format_age(Some(expected_at), now)
        ),
        link: None,
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
                link: last.map(|(pod, _)| ResourceKey::of_pod(pod)),
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
            link: None,
        }),
        _ => Some(KindDiagnosis {
            tone: StatusTone::Bad,
            title: "JOB FAILED".to_owned(),
            text: reason_and_message(condition).unwrap_or_else(|| "The job failed.".to_owned()),
            link: None,
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
            link: None,
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
        link: unhealthy.map(|(pod, _)| ResourceKey::of_pod(pod)),
    })
}

// ---- Helm releases ----

/// What the box says when Helm gave no description.
const HELM_NO_REASON: &str = "Helm gave no reason.";

/// A failed release (the title follows the prefix Helm writes in the description), or one that is
/// pending, uninstalling, or in a status this build does not know. Reads the release alone, so it
/// shows at once. The description is Helm's text and can quote a field value: it is shown here and
/// never logged.
fn helm_release_diagnosis(release: &HelmReleaseSummary, now: Timestamp) -> Option<KindDiagnosis> {
    let since = || match release.updated_at {
        Some(_) => format!("{} ago", format_age(release.updated_at, now)),
        None => "a while ago".to_owned(),
    };
    let (tone, title, text) = match &release.status {
        HelmStatus::Failed => {
            let description = release.description.as_deref().unwrap_or(HELM_NO_REASON);
            if description.starts_with("Rollback \"") {
                (StatusTone::Bad, "ROLLBACK FAILED", description.to_owned())
            } else if description.starts_with("Upgrade \"") {
                let text = match release.deployed_revision {
                    Some(deployed) => {
                        format!("{description} Revision {deployed} is still deployed.")
                    }
                    None => description.to_owned(),
                };
                (StatusTone::Bad, "UPGRADE FAILED", text)
            } else {
                (StatusTone::Bad, "INSTALL FAILED", description.to_owned())
            }
        }
        HelmStatus::PendingInstall | HelmStatus::PendingUpgrade | HelmStatus::PendingRollback => {
            let title = match release.status {
                HelmStatus::PendingInstall => "PENDING INSTALL",
                HelmStatus::PendingUpgrade => "PENDING UPGRADE",
                _ => "PENDING ROLLBACK",
            };
            let text = format!(
                "Helm started this operation {} and has not finished. A pending release blocks \
                 the next helm upgrade.",
                since()
            );
            (StatusTone::Warn, title, text)
        }
        HelmStatus::Uninstalling => (
            StatusTone::Warn,
            "UNINSTALLING",
            format!("Helm started uninstalling this release {}.", since()),
        ),
        HelmStatus::Unknown(status) => (
            StatusTone::Warn,
            "UNKNOWN STATUS",
            format!("Helm reports status \"{status}\"."),
        ),
        HelmStatus::Deployed | HelmStatus::Uninstalled | HelmStatus::Superseded => return None,
    };
    Some(KindDiagnosis {
        tone,
        title: title.to_owned(),
        text,
        link: None,
    })
}

// ---- Secrets ----

/// A certificate worth a box: the tone and the text, for a Secret and for the Ingresses that
/// name it. `None` for a valid certificate and for a secret without one.
pub(crate) fn certificate_verdict(
    details: &SecretDetails,
    now: Timestamp,
) -> Option<(StatusTone, String)> {
    match details {
        SecretDetails::Certificate { chain } => {
            let leaf = chain.first()?;
            match expiry_state(leaf, now) {
                ExpiryState::Expired => Some((
                    StatusTone::Bad,
                    format!(
                        "Expired {} ({} ago).",
                        date_text(leaf.not_after),
                        format_age(Some(leaf.not_after), now)
                    ),
                )),
                ExpiryState::ExpiringSoon => Some((
                    StatusTone::Warn,
                    format!(
                        "Expires {} (in {}).",
                        date_text(leaf.not_after),
                        format_age(Some(now), leaf.not_after)
                    ),
                )),
                ExpiryState::NotYetValid => Some((
                    StatusTone::Warn,
                    format!("Not valid until {}.", date_text(leaf.not_before)),
                )),
                ExpiryState::Valid => None,
            }
        }
        SecretDetails::NoCertificate(CertificateIssue::Unparsed) => Some((
            StatusTone::Warn,
            "tls.crt could not be parsed as an X.509 certificate.".to_owned(),
        )),
        SecretDetails::NoCertificate(CertificateIssue::Missing) => {
            Some((StatusTone::Warn, "The secret has no tls.crt.".to_owned()))
        }
        SecretDetails::None
        | SecretDetails::Registries(_)
        | SecretDetails::ServiceAccountToken { .. } => None,
    }
}

/// CERTIFICATE: the leaf is expired, about to expire, not valid yet, or unusable. Reads only the
/// secret, so it shows while the lists load.
fn secret_diagnosis(secret: &SecretSummary, now: Timestamp) -> Option<KindDiagnosis> {
    let (tone, text) = certificate_verdict(&secret.details, now)?;
    Some(KindDiagnosis {
        tone,
        title: CERTIFICATE_TITLE.to_owned(),
        text,
        link: None,
    })
}

/// How long an Ingress may lack a load-balancer address before the box says no controller took it.
const NO_ADDRESS_GRACE: jiff::SignedDuration = jiff::SignedDuration::from_mins(5);

/// An Ingress whose rules route to a broken backend first, then one with no address after
/// `NO_ADDRESS_GRACE` (nothing serves it), else its CERTIFICATE box. NO ADDRESS needs no list, so it
/// shows while the TLS secrets load.
fn ingress_diagnosis(ingress: &IngressSummary, inputs: &DiagnosisInputs) -> Option<KindDiagnosis> {
    // A broken backend is the cause of a 502 whatever the controller reports, so it comes first.
    ingress_backend_diagnosis(ingress, inputs)
        .or_else(|| no_address_diagnosis(ingress, inputs.now))
        .or_else(|| ingress_certificate_diagnosis(ingress, inputs))
}

/// BACKEND UNREACHABLE: a rule routes to a Service that is missing, lacks the port, or has no ready
/// pod. One line per rule, and a link to the Service the first problem is about. Waits for the
/// Services and the pods.
fn ingress_backend_diagnosis(
    ingress: &IngressSummary,
    inputs: &DiagnosisInputs,
) -> Option<KindDiagnosis> {
    let (lines, service) = backend_problems(ingress, inputs.backends.as_ref()?);
    if lines.is_empty() {
        return None;
    }
    Some(KindDiagnosis {
        tone: StatusTone::Bad,
        title: "BACKEND UNREACHABLE".to_owned(),
        text: lines.join(
            "
",
        ),
        link: service.and_then(|service| {
            ResourceKey::of_object("Service", Some(&service.namespace), &service.name)
        }),
    })
}

/// NO ADDRESS: the Ingress has been there for a while and no controller wrote an address into its
/// status. The cluster's default IngressClass is not read here, so a classless Ingress is told
/// what it needs.
fn no_address_diagnosis(ingress: &IngressSummary, now: Timestamp) -> Option<KindDiagnosis> {
    let age = now.duration_since(ingress.created_at?);
    if !ingress.addresses.is_empty() || age <= NO_ADDRESS_GRACE {
        return None;
    }
    let cause = match &ingress.class {
        Some(class) => format!("class {class}: no controller reports it"),
        None => "no ingressClassName; only a default IngressClass would pick it up".to_owned(),
    };
    Some(KindDiagnosis {
        tone: StatusTone::Warn,
        title: "NO ADDRESS".to_owned(),
        text: format!("No address: no ingress controller has picked it up ({cause})."),
        link: None,
    })
}

/// CERTIFICATE of an Ingress: the worst of the TLS secrets it names. An expired certificate comes
/// first, then a missing secret, then the rest (expiring, not yet valid, unusable), the earliest
/// not-after first. The text is the Secret box text prefixed with the secret's name. It waits for
/// the TLS secrets companion, and links to the Secret it is about.
fn ingress_certificate_diagnosis(
    ingress: &IngressSummary,
    inputs: &DiagnosisInputs,
) -> Option<KindDiagnosis> {
    let secrets = inputs.tls_secrets?;
    let mut worst: Option<((u8, i64, &str), KindDiagnosis)> = None;
    for name in tls_secret_names(ingress) {
        let secret = find_secret(secrets, &ingress.namespace, name);
        let (rank, tone, text) = match secret {
            None => (
                1,
                StatusTone::Warn,
                format!("{NO_TLS_SECRET} {name} in {}.", ingress.namespace),
            ),
            Some(secret) => {
                // A valid certificate says nothing; the other names still count.
                let Some((tone, text)) = certificate_verdict(&secret.details, inputs.now) else {
                    continue;
                };
                let rank = if tone == StatusTone::Bad { 0 } else { 2 };
                (rank, tone, format!("{name}: {text}"))
            }
        };
        let not_after = secret
            .and_then(|secret| match &secret.details {
                SecretDetails::Certificate { chain } => chain.first(),
                _ => None,
            })
            .map_or(i64::MAX, |leaf| leaf.not_after.as_second());
        let key = (rank, not_after, name);
        if worst.as_ref().is_some_and(|(known, _)| *known <= key) {
            continue;
        }
        let diagnosis = KindDiagnosis {
            tone,
            title: CERTIFICATE_TITLE.to_owned(),
            text,
            link: ResourceKey::of_object("Secret", Some(&ingress.namespace), name),
        };
        worst = Some((key, diagnosis));
    }
    worst.map(|(_, diagnosis)| diagnosis)
}

#[cfg(test)]
#[path = "kind_diagnosis_tests.rs"]
mod kind_diagnosis_tests;
