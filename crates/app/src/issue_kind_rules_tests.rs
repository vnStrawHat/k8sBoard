use cluster::{
    CertificateInfo, ControllerRef, DaemonSetSummary, DeploymentSummary, EventSummary, EventType,
    HorizontalPodAutoscalerSummary, InvolvedObject, JobStatus, JobSummary, NamespaceScope,
    NamespaceSummary, PodDisruptionBudgetSummary, QuotaItem, ResourceQuotaSummary, SecretSummary,
    WatchUpdate, WorkloadCondition,
};

use super::*;
use crate::issue_feeds::WarningEvents;

const NOW: i64 = 1_000_000;

fn at(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).expect("valid timestamp")
}

fn ago(seconds: i64) -> Timestamp {
    at(NOW - seconds)
}

fn condition(name: &str, is_true: bool, reason: &str, message: &str) -> WorkloadCondition {
    WorkloadCondition {
        name: name.to_owned(),
        is_true,
        reason: Some(reason.to_owned()),
        message: Some(message.to_owned()),
    }
}

fn deployment(conditions: Vec<WorkloadCondition>) -> DeploymentSummary {
    DeploymentSummary {
        namespace: "shop".to_owned(),
        name: "api".to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired: 3,
        ready: 1,
        up_to_date: 1,
        available: 1,
        strategy: String::new(),
        max_surge: None,
        max_unavailable: None,
        progress_deadline_seconds: 600,
        is_paused: false,
        revision: None,
        selector: Vec::new(),
        containers: Vec::new(),
        conditions,
    }
}

fn stalled_deployment() -> DeploymentSummary {
    deployment(vec![condition(
        "Progressing",
        false,
        "ProgressDeadlineExceeded",
        "ReplicaSet has timed out progressing.",
    )])
}

fn daemon_set(desired: u32, current: u32, misscheduled: u32) -> DaemonSetSummary {
    DaemonSetSummary {
        namespace: "kube-system".to_owned(),
        name: "agent".to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired,
        current,
        ready: current,
        up_to_date: current,
        available: current,
        misscheduled,
        node_selector: Vec::new(),
        update_strategy: String::new(),
        selector: Vec::new(),
        containers: Vec::new(),
    }
}

fn job(status: JobStatus, failed: u32, conditions: Vec<WorkloadCondition>) -> JobSummary {
    JobSummary {
        namespace: "shop".to_owned(),
        name: "import".to_owned(),
        created_at: None,
        labels: Vec::new(),
        status,
        completions: Some(1),
        parallelism: Some(1),
        succeeded: 0,
        failed,
        active: 0,
        backoff_limit: None,
        active_deadline_seconds: None,
        ttl_seconds_after_finished: None,
        started_at: None,
        finished_at: None,
        owner: None,
        conditions,
        containers: Vec::new(),
    }
}

fn autoscaler(conditions: Vec<WorkloadCondition>) -> HorizontalPodAutoscalerSummary {
    HorizontalPodAutoscalerSummary {
        namespace: "shop".to_owned(),
        name: "web".to_owned(),
        created_at: None,
        labels: Vec::new(),
        target: ControllerRef {
            kind: "Deployment".to_owned(),
            name: "web".to_owned(),
        },
        min_replicas: 2,
        max_replicas: 10,
        current_replicas: 4,
        desired_replicas: 4,
        metrics: Vec::new(),
        conditions,
        last_scaled_at: None,
    }
}

fn budget() -> PodDisruptionBudgetSummary {
    PodDisruptionBudgetSummary {
        namespace: "shop".to_owned(),
        name: "web".to_owned(),
        created_at: None,
        labels: Vec::new(),
        min_available: Some("2".to_owned()),
        max_unavailable: None,
        selector: None,
        current_healthy: 2,
        desired_healthy: 2,
        expected_pods: 2,
        disruptions_allowed: 0,
        unhealthy_pod_eviction_policy: None,
        conditions: Vec::new(),
        is_status_stale: false,
    }
}

fn quota(items: &[(&str, &str, &str)]) -> ResourceQuotaSummary {
    ResourceQuotaSummary {
        namespace: "shop".to_owned(),
        name: "compute".to_owned(),
        created_at: None,
        labels: Vec::new(),
        items: items
            .iter()
            .map(|(resource, hard, used)| QuotaItem {
                resource: (*resource).to_owned(),
                hard: (*hard).to_owned(),
                used: Some((*used).to_owned()),
            })
            .collect(),
        scopes: Vec::new(),
    }
}

fn claim(phase: &str) -> cluster::PersistentVolumeClaimSummary {
    cluster::PersistentVolumeClaimSummary {
        namespace: "shop".to_owned(),
        name: "data".to_owned(),
        created_at: Some(ago(3_600)),
        labels: Vec::new(),
        phase: phase.to_owned(),
        is_terminating: false,
        volume: Some("pv-1".to_owned()),
        capacity: None,
        requested: None,
        access_modes: Vec::new(),
        storage_class: None,
        volume_mode: None,
        conditions: Vec::new(),
    }
}

fn certificate(not_before: i64, not_after: i64) -> KindObject {
    KindObject::Secret(SecretSummary {
        namespace: "shop".to_owned(),
        name: "shop-tls".to_owned(),
        created_at: None,
        labels: Vec::new(),
        secret_type: "kubernetes.io/tls".to_owned(),
        keys: Vec::new(),
        details: SecretDetails::Certificate {
            chain: vec![CertificateInfo {
                subject: "CN=shop".to_owned(),
                issuer: "CN=ca".to_owned(),
                alt_names: Vec::new(),
                not_before: at(not_before),
                not_after: at(not_after),
            }],
        },
        is_immutable: false,
        is_owned: false,
    })
}

fn inputs() -> IssueInputs<'static> {
    IssueInputs {
        pods: None,
        nodes: None,
        scope: &NamespaceScope::All,
        namespaces: None,
        events: None,
        objects: &[],
        pod_usage: None,
        node_usage: None,
        kubelet: None,
        is_job_feed_live: false,
        now: at(NOW),
    }
}

/// The finding of `object` on its feed, with no events.
fn finding(kind: ResourceKind, object: KindObject) -> Option<Finding> {
    let objects = [object];
    let feeds = [(kind, &objects[..])];
    condition_findings(&IssueInputs {
        objects: &feeds,
        ..inputs()
    })
    .into_iter()
    .next()
}

fn warning_event(kind: &str, name: &str, reason: &str, message: &str) -> EventSummary {
    EventSummary {
        namespace: "shop".to_owned(),
        name: format!("{name}.{reason}"),
        event_type: EventType::Warning,
        reason: reason.to_owned(),
        object: InvolvedObject {
            kind: kind.to_owned(),
            namespace: Some("shop".to_owned()),
            name: name.to_owned(),
        },
        message: message.to_owned(),
        count: 1,
        first_seen: Some(ago(600)),
        last_seen: Some(ago(60)),
        source: None,
        container: None,
    }
}

fn stuck_namespace(deleting_ago: i64) -> NamespaceSummary {
    NamespaceSummary {
        name: "coroot".to_owned(),
        phase: NamespacePhase::Terminating,
        labels: Vec::new(),
        created_at: None,
        deleting_since: Some(ago(deleting_ago)),
        deletion_conditions: vec![cluster::NamespaceDeletionCondition {
            name: "NamespaceContentRemaining".to_owned(),
            reason: None,
            message: Some(
                "Some resources are remaining: pods. has 2 resource instances".to_owned(),
            ),
        }],
    }
}

#[test]
fn rollout_stalled_is_critical() {
    let found = finding(
        ResourceKind::Deployments,
        KindObject::Deployment(stalled_deployment()),
    )
    .expect("a finding");
    assert_eq!(found.rule, IssueRule::KindRollout);
    assert_eq!(found.severity, IssueSeverity::Critical);
    assert_eq!(found.reason, "Rollout stalled");
    assert!(
        found.cause.starts_with("No progress for 600s."),
        "{}",
        found.cause
    );
    assert_eq!(
        found.object,
        IssueObject::new("Deployment", Some("shop"), "api")
    );
    assert_eq!(found.grace, None);
    assert_eq!(found.action, IssueAction::Open);
    assert_eq!(found.workload, None);
}

#[test]
fn kind_rules_ignore_pod_derived_boxes() {
    // D3 (not ready), S1 and S2 (pods of the set) read pods, which the condition feeds never have.
    let unready = deployment(Vec::new());
    assert!(finding(ResourceKind::Deployments, KindObject::Deployment(unready)).is_none());
    let set = daemon_set(3, 3, 0);
    let struggling = DaemonSetSummary { ready: 0, ..set };
    assert!(finding(ResourceKind::DaemonSets, KindObject::DaemonSet(struggling)).is_none());
}

#[test]
fn kind_graces() {
    let backoff = condition("Failed", true, "BackoffLimitExceeded", "limit");
    let cases = [
        (
            "DaemonSet S3",
            ResourceKind::DaemonSets,
            KindObject::DaemonSet(daemon_set(4, 3, 0)),
            Some(ROLLOUT_GRACE),
        ),
        (
            "DaemonSet S4",
            ResourceKind::DaemonSets,
            KindObject::DaemonSet(daemon_set(3, 3, 1)),
            Some(ROLLOUT_GRACE),
        ),
        (
            "Job J4",
            ResourceKind::Jobs,
            KindObject::Job(job(JobStatus::Running, 2, Vec::new())),
            Some(ROLLOUT_GRACE),
        ),
        (
            "Deployment D1",
            ResourceKind::Deployments,
            KindObject::Deployment(stalled_deployment()),
            None,
        ),
        (
            "Job J1",
            ResourceKind::Jobs,
            KindObject::Job(job(JobStatus::Failed, 7, vec![backoff])),
            None,
        ),
    ];
    for (name, kind, object, grace) in cases {
        let found = finding(kind, object).unwrap_or_else(|| panic!("{name} has a finding"));
        assert_eq!(found.grace, grace, "{name}");
    }
}

#[test]
fn job_backoff_limit_finding() {
    let failed = job(
        JobStatus::Failed,
        7,
        vec![condition("Failed", true, "BackoffLimitExceeded", "limit")],
    );
    let found = finding(ResourceKind::Jobs, KindObject::Job(failed)).expect("a finding");
    assert_eq!(found.rule, IssueRule::KindJob);
    assert_eq!(found.severity, IssueSeverity::Critical);
    assert_eq!(found.reason, "Backoff limit reached");
    assert_eq!(found.cause, "7 attempts failed.");
}

#[test]
fn hpa_at_max_is_capped_at_warning() {
    let capped = autoscaler(vec![condition(
        "ScalingLimited",
        true,
        "TooManyReplicas",
        "",
    )]);
    let capped = cluster::HorizontalPodAutoscalerSummary {
        current_replicas: 10,
        desired_replicas: 10,
        metrics: vec![cluster::HpaMetric {
            name: "cpu".to_owned(),
            source: cluster::MetricSource::Resource,
            target: cluster::MetricValue::Utilization(70),
            current: Some(cluster::MetricValue::Utilization(92)),
        }],
        ..capped
    };
    let found = finding(
        ResourceKind::HorizontalPodAutoscalers,
        KindObject::HorizontalPodAutoscaler(capped),
    )
    .expect("a finding");
    // The WHY box is Bad; a limit is a constraint, not an outage.
    assert_eq!(found.rule, IssueRule::KindAutoscaler);
    assert_eq!(found.severity, IssueSeverity::Warning);
    assert_eq!(found.reason, "At max replicas");
}

#[test]
fn failed_metrics_are_a_warning_too() {
    let broken = autoscaler(vec![condition(
        "ScalingActive",
        false,
        "FailedGetExternalMetric",
        "unable to get external metric",
    )]);
    let found = finding(
        ResourceKind::HorizontalPodAutoscalers,
        KindObject::HorizontalPodAutoscaler(broken),
    )
    .expect("a finding");
    assert_eq!(found.severity, IssueSeverity::Warning);
    assert_eq!(found.reason, "Metrics unavailable");
    assert!(found.cause.contains("unable to get external metric"));
}

#[test]
fn pdb_blocks_drain_is_warning() {
    let found = finding(
        ResourceKind::PodDisruptionBudgets,
        KindObject::PodDisruptionBudget(budget()),
    )
    .expect("a finding");
    assert_eq!(found.rule, IssueRule::KindDisruptionBudget);
    assert_eq!(found.severity, IssueSeverity::Warning);
    assert_eq!(found.reason, "Blocks drain");
}

#[test]
fn quota_near_limit_names_first_item() {
    let near = quota(&[
        ("pods", "10", "5"),
        ("requests.cpu", "10", "9"),
        ("limits.cpu", "20", "19"),
    ]);
    let found = finding(
        ResourceKind::ResourceQuotas,
        KindObject::ResourceQuota(near),
    )
    .expect("a finding");
    assert_eq!(found.rule, IssueRule::QuotaNearLimit);
    assert_eq!(found.severity, IssueSeverity::Warning);
    assert_eq!(found.reason, "Quota near limit");
    assert!(found.cause.starts_with("requests.cpu "), "{}", found.cause);
    assert!(found.cause.contains("(90%)"), "{}", found.cause);
    assert!(
        found.cause.ends_with(" 1 more near their limit."),
        "{}",
        found.cause
    );
}

#[test]
fn at_quota_wins_over_near_limit() {
    let full = quota(&[("pods", "10", "10"), ("requests.cpu", "10", "9")]);
    let found = finding(
        ResourceKind::ResourceQuotas,
        KindObject::ResourceQuota(full),
    )
    .expect("a finding");
    assert_eq!(found.rule, IssueRule::KindQuota);
    assert_eq!(found.reason, "At quota");
    let calm = quota(&[("pods", "10", "5")]);
    assert!(
        finding(
            ResourceKind::ResourceQuotas,
            KindObject::ResourceQuota(calm)
        )
        .is_none()
    );
}

fn pending_finding(
    claim: &cluster::PersistentVolumeClaimSummary,
    events: Option<&WarningEvents>,
) -> Option<Finding> {
    let objects = [KindObject::PersistentVolumeClaim(claim.clone())];
    let feeds = [(ResourceKind::PersistentVolumeClaims, &objects[..])];
    condition_findings(&IssueInputs {
        objects: &feeds,
        events,
        ..inputs()
    })
    .into_iter()
    .next()
}

fn event_feed(events: Vec<EventSummary>) -> WarningEvents {
    let mut feed = WarningEvents::default();
    feed.apply(WatchUpdate::Snapshot(events));
    feed
}

#[test]
fn pvc_pending_needs_warning_event() {
    let pending = claim("Pending");
    let events = event_feed(vec![warning_event(
        "PersistentVolumeClaim",
        "data",
        "ProvisioningFailed",
        "storageclass \"fast\" not found",
    )]);
    let found = pending_finding(&pending, Some(&events)).expect("a finding");
    assert_eq!(found.rule, IssueRule::PvcPending);
    assert_eq!(found.severity, IssueSeverity::Warning);
    assert_eq!(found.reason, "Pending");
    assert_eq!(
        found.cause,
        "Pending for 1h. ProvisioningFailed: storageclass \"fast\" not found"
    );
    assert_eq!(found.onset, pending.created_at);
    assert_eq!(found.grace, Some(PVC_PENDING_GRACE));
    // A claim that waits for its first consumer has only Normal events: no Warning, no issue.
    let calm = event_feed(Vec::new());
    assert!(pending_finding(&pending, Some(&calm)).is_none());
    // And a bound claim is no issue whatever its events say.
    assert!(pending_finding(&claim("Bound"), Some(&events)).is_none());
}

#[test]
fn pvc_pending_quiet_without_warning_feed() {
    assert!(pending_finding(&claim("Pending"), None).is_none());
}

#[test]
fn pvc_lost_is_critical() {
    let found = finding(
        ResourceKind::PersistentVolumeClaims,
        KindObject::PersistentVolumeClaim(claim("Lost")),
    )
    .expect("a finding");
    assert_eq!(found.rule, IssueRule::KindClaim);
    assert_eq!(found.severity, IssueSeverity::Critical);
    assert_eq!(found.reason, "Volume lost");
    assert_eq!(
        found.object,
        IssueObject::new("PersistentVolumeClaim", Some("shop"), "data")
    );
}

#[test]
fn stuck_namespace_uses_kind_diagnosis() {
    let namespaces = [stuck_namespace(3_600)];
    let findings = condition_findings(&IssueInputs {
        namespaces: Some(&namespaces),
        ..inputs()
    });
    let [found] = findings.as_slice() else {
        panic!("one finding, got {findings:?}");
    };
    assert_eq!(found.rule, IssueRule::NamespaceStuck);
    assert_eq!(found.reason, "Stuck terminating");
    assert_eq!(found.object, IssueObject::new("Namespace", None, "coroot"));
    assert_eq!(found.onset, Some(ago(3_600)));
    assert!(
        found.cause.starts_with("Terminating for 1h."),
        "{}",
        found.cause
    );
}

fn namespace_findings(namespace: NamespaceSummary) -> usize {
    let namespaces = [namespace];
    condition_findings(&IssueInputs {
        namespaces: Some(&namespaces),
        ..inputs()
    })
    .len()
}

#[test]
fn a_namespace_deleting_for_a_minute_is_not_stuck_yet() {
    assert_eq!(namespace_findings(stuck_namespace(60)), 0);
}

#[test]
fn an_active_namespace_is_never_stuck() {
    let active = NamespaceSummary {
        phase: NamespacePhase::Active,
        ..stuck_namespace(3_600)
    };
    assert_eq!(namespace_findings(active), 0);
}

#[test]
fn cert_expiring_uses_w3_wording() {
    let soon = certificate(NOW - 86_400, NOW + 5 * 86_400);
    let found = finding(ResourceKind::Secrets, soon).expect("a finding");
    assert_eq!(found.rule, IssueRule::CertExpiring);
    assert_eq!(found.severity, IssueSeverity::Warning);
    assert_eq!(found.reason, "Cert expiring");
    assert!(
        found.cause.starts_with("Expires in 5d ("),
        "{}",
        found.cause
    );
    assert_eq!(
        found.object,
        IssueObject::new("Secret", Some("shop"), "shop-tls")
    );
    assert!(
        found.object.target().is_some(),
        "the Secrets screen has the row"
    );
}

#[test]
fn cert_expired_is_critical() {
    let lapsed = certificate(NOW - 90 * 86_400, NOW - 2 * 86_400);
    let found = finding(ResourceKind::Secrets, lapsed).expect("a finding");
    assert_eq!(found.rule, IssueRule::CertExpired);
    assert_eq!(found.severity, IssueSeverity::Critical);
    assert_eq!(found.reason, "Cert expired");
    assert!(found.cause.starts_with("Expired "), "{}", found.cause);
    assert!(found.cause.ends_with("(2d ago)."), "{}", found.cause);
}

#[test]
fn unparsed_certificate_is_not_an_issue() {
    let secret = |details| {
        KindObject::Secret(SecretSummary {
            namespace: "shop".to_owned(),
            name: "shop-tls".to_owned(),
            created_at: None,
            labels: Vec::new(),
            secret_type: "kubernetes.io/tls".to_owned(),
            keys: Vec::new(),
            details,
            is_immutable: false,
            is_owned: false,
        })
    };
    assert!(finding(ResourceKind::Secrets, secret(SecretDetails::None)).is_none());
    // Valid for a year, and one that begins in the future: neither is an issue.
    assert!(
        finding(
            ResourceKind::Secrets,
            certificate(NOW - 86_400, NOW + 365 * 86_400)
        )
        .is_none()
    );
    assert!(
        finding(
            ResourceKind::Secrets,
            certificate(NOW + 86_400, NOW + 400 * 86_400)
        )
        .is_none()
    );
}

#[test]
fn title_becomes_sentence_case() {
    assert_eq!(sentence_case_title("ROLLOUT STALLED"), "Rollout stalled");
    assert_eq!(
        sentence_case_title("3 FAILED ATTEMPTS"),
        "3 failed attempts"
    );
    assert_eq!(sentence_case_title(""), "");
}

#[test]
fn stuck_namespace_outside_the_picked_namespaces_is_not_listed() {
    let namespaces = [stuck_namespace(3_600)];
    let in_scope = |scope: NamespaceScope| {
        condition_findings(&IssueInputs {
            scope: &scope,
            namespaces: Some(&namespaces),
            ..inputs()
        })
        .len()
    };
    assert_eq!(in_scope(NamespaceScope::All), 1);
    assert_eq!(in_scope(NamespaceScope::Named("coroot".to_owned())), 1);
    assert_eq!(in_scope(NamespaceScope::Named("shop".to_owned())), 0);
}

#[test]
fn failed_job_ages_from_the_moment_it_gave_up() {
    let failed = JobSummary {
        finished_at: Some(ago(7_200)),
        ..job(
            JobStatus::Failed,
            7,
            vec![condition("Failed", true, "BackoffLimitExceeded", "limit")],
        )
    };
    let found = finding(ResourceKind::Jobs, KindObject::Job(failed)).expect("a finding");
    assert_eq!(found.onset, Some(ago(7_200)));
    // A job that still retries has no such time.
    let retrying = finding(
        ResourceKind::Jobs,
        KindObject::Job(job(JobStatus::Running, 2, Vec::new())),
    )
    .expect("a finding");
    assert_eq!(retrying.onset, None);
}

#[test]
fn cert_onsets_come_from_the_not_after_date() {
    let lapsed = finding(
        ResourceKind::Secrets,
        certificate(NOW - 90 * 86_400, NOW - 2 * 86_400),
    )
    .expect("a finding");
    assert_eq!(lapsed.onset, Some(ago(2 * 86_400)));
    let soon = finding(
        ResourceKind::Secrets,
        certificate(NOW - 86_400, NOW + 5 * 86_400),
    )
    .expect("a finding");
    // It came within reach of the warning when 14 days were left.
    let warning_days = EXPIRY_WARNING.as_secs();
    assert_eq!(soon.onset, Some(at(NOW + 5 * 86_400 - warning_days)));
}

fn owned_job(name: &str, status: JobStatus, created_ago: i64, owner: Option<&str>) -> KindObject {
    let failing = vec![condition("Failed", true, "BackoffLimitExceeded", "limit")];
    let conditions = if status == JobStatus::Failed {
        failing
    } else {
        Vec::new()
    };
    KindObject::Job(JobSummary {
        name: name.to_owned(),
        created_at: Some(ago(created_ago)),
        owner: owner.map(|name| ControllerRef {
            kind: "CronJob".to_owned(),
            name: name.to_owned(),
        }),
        ..job(status, 7, conditions)
    })
}

fn job_findings(objects: &[KindObject]) -> Vec<Finding> {
    let feeds = [(ResourceKind::Jobs, objects)];
    condition_findings(&IssueInputs {
        objects: &feeds,
        ..inputs()
    })
}

#[test]
fn failed_job_is_dropped_when_a_newer_one_of_the_owner_completed() {
    let recovered = [
        owned_job("report-1", JobStatus::Failed, 7_200, Some("report")),
        owned_job("report-2", JobStatus::Complete, 3_600, Some("report")),
    ];
    assert!(job_findings(&recovered).is_empty());
    // An older completion does not hide a newer failure.
    let regressed = [
        owned_job("report-1", JobStatus::Complete, 7_200, Some("report")),
        owned_job("report-2", JobStatus::Failed, 3_600, Some("report")),
    ];
    assert_eq!(job_findings(&regressed).len(), 1);
    // Another owner's success says nothing, and neither does a Job without an owner.
    let unrelated = [
        owned_job("report-1", JobStatus::Failed, 7_200, Some("report")),
        owned_job("other-2", JobStatus::Complete, 3_600, Some("other")),
        owned_job("adhoc", JobStatus::Failed, 7_200, None),
        owned_job("adhoc-2", JobStatus::Complete, 3_600, None),
    ];
    assert_eq!(job_findings(&unrelated).len(), 2);
}

#[test]
fn pdb_blocked_by_unhealthy_pods_waits_but_a_full_budget_does_not() {
    let unhealthy = PodDisruptionBudgetSummary {
        current_healthy: 1,
        expected_pods: 3,
        ..budget()
    };
    let found = finding(
        ResourceKind::PodDisruptionBudgets,
        KindObject::PodDisruptionBudget(unhealthy),
    )
    .expect("a finding");
    assert_eq!(found.grace, Some(ROLLOUT_GRACE));
    // No room to evict although every pod is healthy: that stays until the budget changes.
    let no_room = finding(
        ResourceKind::PodDisruptionBudgets,
        KindObject::PodDisruptionBudget(budget()),
    )
    .expect("a finding");
    assert_eq!(no_room.grace, None);
}
