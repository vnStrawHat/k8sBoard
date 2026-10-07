//! The rules over the condition feeds: workloads, autoscalers, budgets, quotas, claims, stuck
//! namespaces, and certificates. Pure: the WHY rules of 0012 to 0018 read each object on its own
//! (no pods), and this module maps what they find to a finding. Condition and event messages are
//! arbitrary text, so nothing here logs them.

use cluster::{
    BlockCause, DisruptionState, IngressSummary, JobStatus, NamespacePhase, NodeSummary,
    PersistentVolumeClaimSummary, SecretDetails,
};
use jiff::{SignedDuration, Timestamp};

use crate::age::format_age;
use crate::certificate_expiry::{EXPIRY_WARNING, ExpiryState, date_text, expiry_state};
use crate::event_rows::message_line;
use crate::issue::{Finding, IssueAction, IssueObject, IssueRule, IssueSeverity};
use crate::issue_board::IssueInputs;
use crate::kind_diagnosis::{DiagnosisInputs, KindDiagnosis, kind_diagnosis};
use crate::kind_row::KindObject;
use crate::policy_rows::{quota_ratio, quota_text, quota_tone};
use crate::resource_kind::ResourceKind;
use crate::status_tone::StatusTone;
use crate::usage_format::format_percent;

/// A DaemonSet or a failing Job attempt is a normal part of a rollout for this long.
const ROLLOUT_GRACE: SignedDuration = SignedDuration::from_mins(5);
/// A claim waits this long for its volume before a Warning event makes it an issue.
pub(crate) const PVC_PENDING_GRACE: SignedDuration = SignedDuration::from_mins(5);

/// Characters of an event message a claim cause quotes.
const CAUSE_MESSAGE_CHARS: usize = 200;

/// Every finding of the namespaces and of the Ready condition feeds. A feed that is not in
/// `inputs.objects` has not loaded, and its rules are skipped.
pub(crate) fn condition_findings(inputs: &IssueInputs) -> Vec<Finding> {
    // A namespace is a cluster object, but one outside the picked namespaces is not this view's
    // business.
    let is_in_scope = |name: &str| {
        let picked = inputs.scope.namespaces();
        picked.is_empty() || picked.iter().any(|namespace| namespace == name)
    };
    let mut findings: Vec<Finding> = inputs
        .namespaces
        .into_iter()
        .flatten()
        .filter_map(|namespace| {
            // Only a Terminating namespace can be stuck, so only that one is cloned.
            (namespace.phase == NamespacePhase::Terminating && is_in_scope(&namespace.name))
                .then(|| stuck_namespace_finding(namespace, inputs.now))
                .flatten()
        })
        .collect();
    let nodes = inputs.nodes.unwrap_or_default();
    for (kind, objects) in inputs.objects {
        for object in *objects {
            if is_superseded_job(object, objects) {
                continue;
            }
            let finding = object_finding(*kind, object, nodes, inputs.now)
                .or_else(|| quota_near_limit_finding(*kind, object))
                .or_else(|| pending_claim_finding(object, inputs))
                .or_else(|| certificate_finding(object, inputs.now));
            findings.extend(finding);
        }
    }
    findings.extend(unrouted_service_findings(inputs));
    findings
}

/// How long a Service nothing selects may be young before an Ingress routing to it is an issue: the
/// pods of a Deployment applied together with it are still being made.
pub(crate) const SERVICE_NO_PODS_GRACE: SignedDuration = SignedDuration::from_mins(2);

/// ServiceNoPods: a Service whose selector matches no pod while an Ingress routes to it, so the
/// route answers 503. A Service nothing routes to is the owner's business (the Service drawer says
/// so); one with no selector, or an ExternalName, has endpoints of its own.
fn unrouted_service_findings(inputs: &IssueInputs) -> Vec<Finding> {
    let objects_of = |kind: ResourceKind| {
        inputs
            .objects
            .iter()
            .find(|(feed, _)| *feed == kind)
            .map(|(_, objects)| *objects)
    };
    let (Some(pods), Some(services), Some(ingresses)) = (
        inputs.pods,
        objects_of(ResourceKind::Services),
        objects_of(ResourceKind::Ingresses),
    ) else {
        return Vec::new();
    };
    let ingresses: Vec<&IngressSummary> = ingresses
        .iter()
        .filter_map(|object| match object {
            KindObject::Ingress(ingress) => Some(ingress),
            _ => None,
        })
        .collect();
    services
        .iter()
        .filter_map(|object| match object {
            KindObject::Service(service) => Some(service),
            _ => None,
        })
        .filter(|service| !service.selector.is_empty() && service.service_type != "ExternalName")
        .filter(|service| {
            !pods.iter().any(|pod| {
                pod.namespace == service.namespace
                    && service
                        .selector
                        .iter()
                        .all(|term| pod.labels.contains(term))
            })
        })
        .filter_map(|service| {
            let routes: Vec<&str> = ingresses
                .iter()
                .filter(|ingress| ingress.namespace == service.namespace)
                .filter(|ingress| routes_to(ingress, &service.name))
                .map(|ingress| ingress.name.as_str())
                .collect();
            let (first, more) = routes.split_first()?;
            let by = match more.len() {
                0 => format!("Ingress {first} routes"),
                count => format!("Ingress {first} and {count} more route"),
            };
            Some(Finding {
                rule: IssueRule::ServiceNoPods,
                severity: IssueSeverity::Critical,
                object: IssueObject::new(
                    ResourceKind::Services.object_kind(),
                    Some(&service.namespace),
                    &service.name,
                ),
                reason: "No matching pods".into(),
                cause: format!(
                    "No pod in {} has the labels {}; {by} to it.",
                    service.namespace,
                    service.selector.join(", ")
                ),
                container: None,
                onset: service.created_at,
                grace: Some(SERVICE_NO_PODS_GRACE),
                action: IssueAction::Open,
                workload: None,
            })
        })
        .collect()
}

/// Whether a path of the Ingress, or its default backend, names the Service.
fn routes_to(ingress: &IngressSummary, service: &str) -> bool {
    ingress.default_service.as_deref() == Some(service)
        || ingress
            .rules
            .iter()
            .any(|rule| rule.service.as_deref() == Some(service))
}

/// A failed Job that a newer Job of the same owner has since completed: the CronJob recovered, so
/// the old failure is history.
fn is_superseded_job(object: &KindObject, objects: &[KindObject]) -> bool {
    let KindObject::Job(job) = object else {
        return false;
    };
    if !matches!(job.status, JobStatus::Failed | JobStatus::Failing) || job.owner.is_none() {
        return false;
    }
    objects.iter().any(|other| {
        matches!(
            other,
            KindObject::Job(newer)
                if newer.status == JobStatus::Complete
                    && newer.namespace == job.namespace
                    && newer.owner == job.owner
                    && newer.created_at > job.created_at
        )
    })
}

fn namespace_object(name: &str) -> IssueObject {
    IssueObject::new("Namespace", None, name)
}

/// NamespaceStuck: the 0018 STUCK box of a namespace that has been Terminating for a while.
fn stuck_namespace_finding(
    namespace: &cluster::NamespaceSummary,
    now: Timestamp,
) -> Option<Finding> {
    let object = KindObject::Namespace(namespace.clone());
    let diagnosis = diagnose(&object, &[], now)?;
    Some(Finding {
        rule: IssueRule::NamespaceStuck,
        severity: severity_of(&diagnosis, Cap::None),
        object: namespace_object(&namespace.name),
        reason: "Stuck terminating".into(),
        cause: diagnosis.text,
        container: None,
        // The namespace says when its deletion began.
        onset: namespace.deleting_since,
        grace: None,
        action: IssueAction::Open,
        workload: None,
    })
}

/// The WHY box of one object with no pods: only the rules that read the object itself can fire.
fn diagnose(object: &KindObject, nodes: &[NodeSummary], now: Timestamp) -> Option<KindDiagnosis> {
    kind_diagnosis(
        object,
        &DiagnosisInputs {
            pods: None,
            nodes,
            service: None,
            bindings: None,
            tls_secrets: None,
            events: None,
            backends: None,
            storage_classes: None,
            now,
        },
    )
}

/// Whether a kind's problems are limits and constraints, never outages.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Cap {
    None,
    Warning,
}

fn severity_of(diagnosis: &KindDiagnosis, cap: Cap) -> IssueSeverity {
    match (diagnosis.tone, cap) {
        (StatusTone::Bad, Cap::None) => IssueSeverity::Critical,
        _ => IssueSeverity::Warning,
    }
}

/// How a kind's WHY box becomes a finding.
struct KindRule {
    rule: IssueRule,
    cap: Cap,
    /// Whether the finding waits `ROLLOUT_GRACE`: every DaemonSet finding, a Job's Warn box, and a
    /// budget that blocks because pods are unhealthy (a rollout does that for a while).
    has_grace: fn(StatusTone, &KindObject) -> bool,
}

fn kind_rule(kind: ResourceKind) -> Option<KindRule> {
    let (rule, cap, has_grace): (_, _, fn(StatusTone, &KindObject) -> bool) = match kind {
        ResourceKind::Deployments => (IssueRule::KindRollout, Cap::None, |_, _| false),
        ResourceKind::DaemonSets => (IssueRule::KindRollout, Cap::None, |_, _| true),
        ResourceKind::Jobs => (IssueRule::KindJob, Cap::None, |tone, _| {
            tone == StatusTone::Warn
        }),
        ResourceKind::HorizontalPodAutoscalers => {
            (IssueRule::KindAutoscaler, Cap::Warning, |_, _| false)
        }
        ResourceKind::PodDisruptionBudgets => (
            IssueRule::KindDisruptionBudget,
            Cap::Warning,
            is_unhealthy_block,
        ),
        ResourceKind::ResourceQuotas => (IssueRule::KindQuota, Cap::Warning, |_, _| false),
        ResourceKind::PersistentVolumeClaims => (IssueRule::KindClaim, Cap::None, |_, _| false),
        _ => return None,
    };
    Some(KindRule {
        rule,
        cap,
        has_grace,
    })
}

/// A budget that blocks because its pods are unhealthy: a rollout does that for a while.
fn is_unhealthy_block(_: StatusTone, object: &KindObject) -> bool {
    matches!(
        object,
        KindObject::PodDisruptionBudget(budget)
            if budget.disruption_state() == DisruptionState::Blocked(BlockCause::UnhealthyPods)
    )
}

/// The object a condition summary names.
fn object_of(kind: ResourceKind, object: &KindObject) -> Option<IssueObject> {
    let (namespace, name) = match object {
        KindObject::Deployment(item) => (&item.namespace, &item.name),
        KindObject::DaemonSet(item) => (&item.namespace, &item.name),
        KindObject::Job(item) => (&item.namespace, &item.name),
        KindObject::HorizontalPodAutoscaler(item) => (&item.namespace, &item.name),
        KindObject::PodDisruptionBudget(item) => (&item.namespace, &item.name),
        KindObject::ResourceQuota(item) => (&item.namespace, &item.name),
        KindObject::PersistentVolumeClaim(item) => (&item.namespace, &item.name),
        KindObject::Secret(item) => (&item.namespace, &item.name),
        _ => return None,
    };
    Some(IssueObject::new(kind.object_kind(), Some(namespace), name))
}

/// `ROLLOUT STALLED` as `Rollout stalled`.
fn sentence_case_title(title: &str) -> String {
    let lower = title.to_lowercase();
    let mut characters = lower.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(characters).collect()
    })
}

/// One finding per object whose WHY box fires with no pods: only the condition rules can. A failed
/// Job has the time it gave up; the other summaries carry no transition time, so their
/// age is the first sighting.
fn object_finding(
    kind: ResourceKind,
    object: &KindObject,
    nodes: &[NodeSummary],
    now: Timestamp,
) -> Option<Finding> {
    let rule = kind_rule(kind)?;
    let diagnosis = diagnose(object, nodes, now)?;
    let has_grace = (rule.has_grace)(diagnosis.tone, object);
    Some(Finding {
        rule: rule.rule,
        severity: severity_of(&diagnosis, rule.cap),
        object: object_of(kind, object)?,
        reason: sentence_case_title(&diagnosis.title).into(),
        cause: diagnosis.text,
        container: None,
        onset: failed_at(object),
        grace: has_grace.then_some(ROLLOUT_GRACE),
        // A Job's WHY is about its pods' exits, which the logs of its newest pod show.
        action: match object {
            KindObject::Job(_) => IssueAction::ViewLogs { container: None },
            _ => IssueAction::Open,
        },
        workload: None,
    })
}

/// QuotaNearLimit: no item is at its limit (that is the AT QUOTA box), but one is at 90 % or more.
fn quota_near_limit_finding(kind: ResourceKind, object: &KindObject) -> Option<Finding> {
    let KindObject::ResourceQuota(quota) = object else {
        return None;
    };
    let mut near = quota.items.iter().filter_map(|item| {
        let ratio = quota_ratio(item)?;
        (quota_tone(ratio) == Some(StatusTone::Warn)).then_some((item, ratio))
    });
    let (item, ratio) = near.next()?;
    let mut cause = format!(
        "{} {} ({}).",
        item.resource,
        quota_text(item),
        format_percent(ratio)
    );
    let more = near.count();
    if more > 0 {
        cause.push_str(&format!(" {more} more near their limit."));
    }
    Some(Finding {
        rule: IssueRule::QuotaNearLimit,
        severity: IssueSeverity::Warning,
        object: object_of(kind, object)?,
        reason: "Quota near limit".into(),
        cause,
        container: None,
        onset: None,
        grace: None,
        action: IssueAction::Open,
        workload: None,
    })
}

/// PvcPending: a claim that is Pending and has a Warning event. A claim that waits for its first
/// consumer has only Normal events, so it stays quiet; so does the rule while the events load.
fn pending_claim_finding(object: &KindObject, inputs: &IssueInputs) -> Option<Finding> {
    let KindObject::PersistentVolumeClaim(claim) = object else {
        return None;
    };
    if claim.phase != "Pending" {
        return None;
    }
    let events = inputs.events?.of(&claim_object(claim))?;
    let newest = events.first()?;
    Some(Finding {
        rule: IssueRule::PvcPending,
        severity: IssueSeverity::Warning,
        object: claim_object(claim),
        reason: "Pending".into(),
        cause: format!(
            "Pending for {}. {}: {}",
            format_age(claim.created_at, inputs.now),
            newest.reason,
            quoted(&newest.message)
        ),
        container: None,
        onset: claim.created_at,
        grace: Some(PVC_PENDING_GRACE),
        action: IssueAction::Open,
        workload: None,
    })
}

fn claim_object(claim: &PersistentVolumeClaimSummary) -> IssueObject {
    IssueObject::new(
        ResourceKind::PersistentVolumeClaims.object_kind(),
        Some(&claim.namespace),
        &claim.name,
    )
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

/// CertExpired and CertExpiring: the leaf of a TLS secret. A certificate that is not valid yet or
/// could not be read is not an issue, and neither is a secret that is no certificate.
fn certificate_finding(object: &KindObject, now: Timestamp) -> Option<Finding> {
    let KindObject::Secret(secret) = object else {
        return None;
    };
    let SecretDetails::Certificate { chain } = &secret.details else {
        return None;
    };
    let leaf = chain.first()?;
    // The age counts from the lapse, or from the day the certificate came within reach of it.
    let (rule, severity, reason, cause, onset) = match expiry_state(leaf, now) {
        ExpiryState::Expired => (
            IssueRule::CertExpired,
            IssueSeverity::Critical,
            "Cert expired",
            format!(
                "Expired {} ({} ago).",
                date_text(leaf.not_after),
                format_age(Some(leaf.not_after), now)
            ),
            leaf.not_after,
        ),
        ExpiryState::ExpiringSoon => (
            IssueRule::CertExpiring,
            IssueSeverity::Warning,
            "Cert expiring",
            format!(
                "Expires in {} ({}).",
                format_age(Some(now), leaf.not_after),
                date_text(leaf.not_after)
            ),
            leaf.not_after
                .checked_sub(EXPIRY_WARNING)
                .unwrap_or(leaf.not_after),
        ),
        ExpiryState::Valid | ExpiryState::NotYetValid => return None,
    };
    // A certificate cannot be a problem before it is valid or before the secret that holds it
    // exists, so a short-lived or freshly uploaded one does not borrow an older onset.
    let onset = onset
        .max(leaf.not_before)
        .max(secret.created_at.unwrap_or(onset));
    Some(Finding {
        rule,
        severity,
        object: object_of(ResourceKind::Secrets, object)?,
        reason: reason.into(),
        cause,
        container: None,
        onset: Some(onset),
        grace: None,
        action: IssueAction::Open,
        workload: None,
    })
}

/// When a failed Job finished: the moment it gave up. A running Job has no such time.
fn failed_at(object: &KindObject) -> Option<Timestamp> {
    match object {
        KindObject::Job(job) if matches!(job.status, JobStatus::Failed) => job.finished_at,
        _ => None,
    }
}

#[cfg(test)]
#[path = "issue_kind_rules_tests.rs"]
mod issue_kind_rules_tests;
