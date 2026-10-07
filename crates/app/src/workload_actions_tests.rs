use std::path::PathBuf;

use cluster::{
    CronJobSummary, CronSchedule, DaemonSetSummary, DeploymentSummary, JobStatus, JobSummary,
    ReplicaSetSummary, StatefulSetSummary, TemplateContainer, WriteOperation,
};

use super::*;
use crate::app_shell::batch_write::CheckedRow;
use crate::write_guard::ActionRisk;

pub(crate) fn test_cluster() -> ClusterRef {
    ClusterRef {
        kubeconfig: PathBuf::from("/home/me/.kube/config"),
        context: "stg-ctx".to_owned(),
    }
}

pub(crate) fn deployment(name: &str) -> DeploymentSummary {
    DeploymentSummary {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired: 3,
        ready: 2,
        up_to_date: 3,
        available: 2,
        strategy: "RollingUpdate".to_owned(),
        max_surge: None,
        max_unavailable: None,
        progress_deadline_seconds: 600,
        is_paused: false,
        generation: 1,
        observed_generation: 1,
        revision: Some("7".to_owned()),
        selector: vec!["app=api".to_owned()],
        containers: Vec::new(),
        conditions: Vec::new(),
        template_change: None,
    }
}

pub(crate) fn stateful_set(name: &str, update_strategy: &str) -> StatefulSetSummary {
    StatefulSetSummary {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired: 3,
        ready: 3,
        current: 3,
        updated: 3,
        service_name: None,
        update_strategy: update_strategy.to_owned(),
        pod_management_policy: "OrderedReady".to_owned(),
        selector: Vec::new(),
        containers: Vec::new(),
        claim_templates: Vec::new(),
        claim_retention: None,
    }
}

fn daemon_set(name: &str, update_strategy: &str) -> DaemonSetSummary {
    DaemonSetSummary {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired: 4,
        current: 4,
        ready: 4,
        up_to_date: 4,
        available: 4,
        misscheduled: 0,
        node_selector: Vec::new(),
        node_affinity_keys: Vec::new(),
        update_strategy: update_strategy.to_owned(),
        selector: Vec::new(),
        containers: Vec::new(),
    }
}

pub(crate) fn cron_job(name: &str, policy: &str, active: usize) -> CronJobSummary {
    CronJobSummary {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        schedule: "*/5 * * * *".to_owned(),
        time_zone: None,
        timetable: CronSchedule::parse("*/5 * * * *", None),
        is_suspended: false,
        concurrency_policy: policy.to_owned(),
        starting_deadline_seconds: None,
        successful_history_limit: None,
        failed_history_limit: None,
        active_jobs: (0..active).map(|index| format!("{name}-{index}")).collect(),
        last_schedule_at: None,
        last_success_at: None,
        containers: Vec::new(),
    }
}

pub(crate) fn job(name: &str) -> JobSummary {
    JobSummary {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        status: JobStatus::Complete,
        completions: Some(1),
        parallelism: Some(1),
        succeeded: 1,
        failed: 0,
        active: 0,
        backoff_limit: None,
        active_deadline_seconds: None,
        ttl_seconds_after_finished: None,
        started_at: None,
        finished_at: None,
        owner: None,
        conditions: Vec::new(),
        containers: Vec::new(),
    }
}

fn now() -> jiff::Timestamp {
    jiff::Timestamp::from_second(1_790_000_000).expect("a valid timestamp")
}

/// The intent of `action` on `object` in the test cluster.
fn intent(action: ResourceAction, object: &KindObject) -> WriteIntent {
    let cluster = test_cluster();
    let scope = WorkloadScope {
        cluster: &cluster,
        cluster_name: "stg-b",
    };
    workload_intent(action, &scope, object, now()).expect("an intent")
}

fn restart_of(kind: ObjectKind) -> ResourceAction {
    ResourceAction::RestartRollout(kind)
}

fn warnings(intent: &WriteIntent) -> Vec<&str> {
    intent.warnings.iter().map(AsRef::as_ref).collect()
}

#[test]
fn restart_names_the_object_and_changes_one_field() {
    let object = KindObject::StatefulSet(stateful_set("kafka", "RollingUpdate"));
    let intent = intent(restart_of(ObjectKind::StatefulSet), &object);
    assert_eq!(intent.label, "Restart rollout of statefulset kafka");
    assert_eq!(intent.button, "Restart");
    assert_eq!(intent.risk, ActionRisk::Change);
    assert_eq!(intent.cluster, test_cluster());
    assert_eq!(intent.cluster_name, "stg-b");
    assert_eq!(intent.request.target().name(), "kafka");
    assert_eq!(intent.request.target().namespace(), Some("team-a"));
    assert!(intent.warnings.is_empty());
}

fn consumer<'a>(namespace: &'a str, name: &'a str, object: Option<&'a KindObject>) -> Consumer<'a> {
    Consumer {
        namespace,
        name,
        object,
    }
}

fn restart_batch(
    kind: ObjectKind,
    consumers: &[Consumer<'_>],
) -> Result<crate::app_shell::batch_write::BatchIntent, SharedString> {
    let cluster = test_cluster();
    let scope = WorkloadScope {
        cluster: &cluster,
        cluster_name: "stg-b",
    };
    consumer_restart_batch(&scope, kind, consumers, "web-config", now())
}

#[test]
fn a_restart_of_named_consumers_lists_them_and_says_their_state_is_unchecked() {
    let batch = restart_batch(
        ObjectKind::StatefulSet,
        &[
            consumer("team-a", "db", None),
            consumer("team-b", "kafka", None),
        ],
    )
    .expect("a batch");
    assert_eq!(batch.label, "Restart 2 statefulsets that read web-config");
    assert_eq!(batch.action, restart_of(ObjectKind::StatefulSet));
    let objects: Vec<&str> = batch.plan.items.iter().map(|i| i.object.as_ref()).collect();
    assert_eq!(objects, ["team-a/db", "team-b/kafka"]);
    assert_eq!(batch.warnings.len(), 1);
    assert!(batch.warnings[0].contains("not loaded"));
    let invalid = restart_batch(ObjectKind::Deployment, &[consumer("team-a", "", None)]);
    assert!(invalid.is_err());
}

#[test]
fn the_title_of_one_consumer_is_singular() {
    let object = KindObject::Deployment(deployment("web"));
    let batch = restart_batch(
        ObjectKind::Deployment,
        &[consumer("team-a", "web", Some(&object))],
    )
    .expect("a batch");
    assert_eq!(batch.label, "Restart 1 deployment that reads web-config");
}

#[test]
fn a_loaded_consumer_is_checked_so_the_unchecked_line_goes() {
    let object = KindObject::Deployment(deployment("web"));
    let batch = restart_batch(
        ObjectKind::Deployment,
        &[consumer("team-a", "web", Some(&object))],
    )
    .expect("a batch");
    assert!(batch.warnings.is_empty(), "{:?}", batch.warnings);
}

#[test]
fn a_paused_consumer_is_skipped_and_a_batch_of_only_paused_ones_says_why() {
    let mut paused = deployment("web");
    paused.is_paused = true;
    let paused = KindObject::Deployment(paused);
    let running = KindObject::Deployment(deployment("api"));
    let batch = restart_batch(
        ObjectKind::Deployment,
        &[
            consumer("team-a", "web", Some(&paused)),
            consumer("team-a", "api", Some(&running)),
        ],
    )
    .expect("a batch");
    assert_eq!(batch.plan.items.len(), 1);
    assert_eq!(batch.plan.skipped.len(), 1);
    assert_eq!(batch.plan.skipped[0].object, "team-a/web");
    let only_paused = restart_batch(
        ObjectKind::Deployment,
        &[consumer("team-a", "web", Some(&paused))],
    );
    assert_eq!(
        only_paused.err().as_deref(),
        Some("Resume the rollout first")
    );
}

#[test]
fn a_helm_managed_consumer_gets_the_helm_warning() {
    let mut helm = deployment("web");
    helm.labels = vec!["app.kubernetes.io/managed-by=Helm".to_owned()];
    let helm = KindObject::Deployment(helm);
    let batch = restart_batch(
        ObjectKind::Deployment,
        &[consumer("team-a", "web", Some(&helm))],
    )
    .expect("a batch");
    assert_eq!(batch.warnings, [HELM_MANAGED_WARNING]);
    // Several: one summary line.
    let plain = KindObject::Deployment(deployment("api"));
    let batch = restart_batch(
        ObjectKind::Deployment,
        &[
            consumer("team-a", "web", Some(&helm)),
            consumer("team-a", "api", Some(&plain)),
        ],
    )
    .expect("a batch");
    assert_eq!(
        batch.warnings,
        ["1 of them are managed by Helm: the next upgrade replaces this change"]
    );
}

#[test]
fn an_on_delete_consumer_and_an_unloaded_one_each_get_their_line() {
    let on_delete = KindObject::StatefulSet(stateful_set("db", "OnDelete"));
    let batch = restart_batch(
        ObjectKind::StatefulSet,
        &[
            consumer("team-a", "db", Some(&on_delete)),
            consumer("team-a", "kafka", None),
        ],
    )
    .expect("a batch");
    assert_eq!(batch.warnings.len(), 2);
    assert!(batch.warnings[0].starts_with("The state of 1 of them is not loaded"));
    assert!(batch.warnings[1].contains("OnDelete"));
}

#[test]
fn restart_timestamp_is_whole_seconds() {
    let object = KindObject::Deployment(deployment("api"));
    let cluster = test_cluster();
    let scope = WorkloadScope {
        cluster: &cluster,
        cluster_name: "stg-b",
    };
    // A fraction of a second must not reach the request: the dry-run and the commit share it.
    let fractional = jiff::Timestamp::new(1_790_000_000, 999_000_000).expect("a timestamp");
    let intent = workload_intent(
        restart_of(ObjectKind::Deployment),
        &scope,
        &object,
        fractional,
    )
    .expect("an intent");
    let WriteOperation::RestartRollout { restarted_at } = intent.request.operation() else {
        panic!("a restart operation");
    };
    assert_eq!(restarted_at.subsec_nanosecond(), 0);
    assert_eq!(restarted_at.as_second(), 1_790_000_000);
}

#[test]
fn on_delete_warning_for_restart() {
    for (object, kind) in [
        (
            KindObject::StatefulSet(stateful_set("kafka", "OnDelete")),
            ObjectKind::StatefulSet,
        ),
        (
            KindObject::DaemonSet(daemon_set("agent", "OnDelete")),
            ObjectKind::DaemonSet,
        ),
    ] {
        let intent = intent(restart_of(kind), &object);
        assert_eq!(
            warnings(&intent),
            ["Strategy OnDelete: pods restart only when deleted"]
        );
    }
    let rolling = KindObject::DaemonSet(daemon_set("agent", "RollingUpdate"));
    assert!(
        intent(restart_of(ObjectKind::DaemonSet), &rolling)
            .warnings
            .is_empty()
    );
}

#[test]
fn pause_and_resume_follow_the_state_of_the_row() {
    let mut paused = deployment("api");
    let running = KindObject::Deployment(paused.clone());
    let pause = intent(ResourceAction::PauseRollout, &running);
    assert_eq!(pause.label, "Pause rollout of deployment api");
    assert_eq!(pause.button, "Pause");
    assert_eq!(
        pause.request.operation(),
        &WriteOperation::SetRolloutPaused { paused: true }
    );
    paused.is_paused = true;
    let resume = intent(
        ResourceAction::PauseRollout,
        &KindObject::Deployment(paused),
    );
    assert_eq!(resume.label, "Resume rollout of deployment api");
    assert_eq!(resume.button, "Resume");
    assert_eq!(
        resume.request.operation(),
        &WriteOperation::SetRolloutPaused { paused: false }
    );
}

#[test]
fn suspend_and_resume_follow_the_state_of_the_row() {
    let mut cron = cron_job("reconcile", "Allow", 0);
    let suspend = intent(
        ResourceAction::SuspendCronJob,
        &KindObject::CronJob(cron.clone()),
    );
    assert_eq!(suspend.label, "Suspend cronjob reconcile");
    assert_eq!(
        suspend.request.operation(),
        &WriteOperation::SetCronJobSuspended { suspended: true }
    );
    cron.is_suspended = true;
    let resume = intent(ResourceAction::SuspendCronJob, &KindObject::CronJob(cron));
    assert_eq!(resume.label, "Resume cronjob reconcile");
    assert_eq!(resume.button, "Resume");
    assert_eq!(
        resume.request.operation(),
        &WriteOperation::SetCronJobSuspended { suspended: false }
    );
}

#[test]
fn only_a_resume_names_the_next_run() {
    let mut cron = cron_job("reconcile", "Allow", 0);
    let suspend = intent(
        ResourceAction::SuspendCronJob,
        &KindObject::CronJob(cron.clone()),
    );
    assert!(warnings(&suspend).is_empty());
    cron.is_suspended = true;
    let resume = intent(ResourceAction::SuspendCronJob, &KindObject::CronJob(cron));
    let notes = warnings(&resume);
    assert_eq!(notes.len(), 1);
    // `*/5 * * * *` runs within five minutes of any instant.
    assert!(notes[0].starts_with("Next run: "), "{}", notes[0]);
    assert!(notes[0].contains(" · in "), "{}", notes[0]);
}

#[test]
fn a_resume_of_an_invalid_schedule_names_no_run() {
    let mut cron = cron_job("reconcile", "Allow", 0);
    cron.is_suspended = true;
    cron.timetable = CronSchedule::parse("not a schedule", None);
    let resume = intent(ResourceAction::SuspendCronJob, &KindObject::CronJob(cron));
    assert!(warnings(&resume).is_empty());
}

#[test]
fn trigger_warnings_follow_policy_and_active_jobs() {
    const RUNNING: &str = "1 job(s) of this CronJob are running; this run starts anyway";
    const FORBID: &str =
        "While this run is active, scheduled runs are skipped (concurrency Forbid)";
    const REPLACE: &str =
        "A scheduled run replaces this job if it is still running (concurrency Replace)";
    let cases: [(&str, usize, Vec<&str>); 6] = [
        ("Allow", 0, vec![]),
        ("Allow", 1, vec![RUNNING]),
        ("Forbid", 0, vec![FORBID]),
        ("Forbid", 1, vec![RUNNING, FORBID]),
        ("Replace", 0, vec![REPLACE]),
        ("Replace", 1, vec![RUNNING, REPLACE]),
    ];
    for (policy, active, expected) in cases {
        let object = KindObject::CronJob(cron_job("reconcile", policy, active));
        let intent = intent(ResourceAction::TriggerCronJob, &object);
        assert_eq!(warnings(&intent), expected, "{policy} with {active} active");
    }
}

#[test]
fn trigger_and_rerun_create_a_job() {
    let trigger = intent(
        ResourceAction::TriggerCronJob,
        &KindObject::CronJob(cron_job("reconcile", "Allow", 0)),
    );
    assert_eq!(trigger.label, "Trigger cronjob reconcile now");
    assert_eq!(trigger.button, "Trigger now");
    assert_eq!(trigger.request.operation(), &WriteOperation::TriggerCronJob);
    let rerun = intent(
        ResourceAction::RerunJob,
        &KindObject::Job(job("etl-nightly-29312400")),
    );
    assert_eq!(rerun.label, "Re-run job etl-nightly-29312400");
    assert_eq!(rerun.button, "Re-run");
    assert_eq!(rerun.request.operation(), &WriteOperation::RerunJob);
}

#[test]
fn an_action_for_another_kind_has_no_intent() {
    let cluster = test_cluster();
    let scope = WorkloadScope {
        cluster: &cluster,
        cluster_name: "stg-b",
    };
    let cron = KindObject::CronJob(cron_job("reconcile", "Allow", 0));
    // A CronJob does not restart, and a Deployment does not re-run.
    assert!(workload_intent(restart_of(ObjectKind::Deployment), &scope, &cron, now()).is_none());
    let deployment = KindObject::Deployment(deployment("api"));
    assert!(workload_intent(ResourceAction::RerunJob, &scope, &deployment, now()).is_none());
    assert!(workload_intent(ResourceAction::Cordon, &scope, &deployment, now()).is_none());
}

#[test]
fn a_name_that_cannot_form_a_path_has_no_intent() {
    let cluster = test_cluster();
    let scope = WorkloadScope {
        cluster: &cluster,
        cluster_name: "stg-b",
    };
    let object = KindObject::Deployment(deployment("../secrets"));
    assert!(workload_intent(restart_of(ObjectKind::Deployment), &scope, &object, now()).is_none());
}

#[test]
fn row_block_refuses_a_paused_restart_only() {
    let mut paused = deployment("api");
    paused.is_paused = true;
    let restart = restart_of(ObjectKind::Deployment);
    assert_eq!(
        row_block(restart, &KindObject::Deployment(paused.clone()), None).as_deref(),
        Some("Resume the rollout first")
    );
    assert!(row_block(restart, &KindObject::Deployment(deployment("api")), None).is_none());
    // Resuming a paused rollout is the way out, so it is never blocked.
    assert!(
        row_block(
            ResourceAction::PauseRollout,
            &KindObject::Deployment(paused),
            None
        )
        .is_none()
    );
    let set = KindObject::StatefulSet(stateful_set("kafka", "RollingUpdate"));
    assert!(row_block(restart_of(ObjectKind::StatefulSet), &set, None).is_none());
}

#[test]
fn state_labels_flip_the_verb() {
    let mut paused = deployment("api");
    let running = KindObject::Deployment(paused.clone());
    assert_eq!(
        state_label(ResourceAction::PauseRollout, "Pause rollout", &running),
        "Pause rollout"
    );
    paused.is_paused = true;
    assert_eq!(
        state_label(
            ResourceAction::PauseRollout,
            "Pause rollout",
            &KindObject::Deployment(paused)
        ),
        "Resume rollout"
    );
    let mut cron = cron_job("reconcile", "Allow", 0);
    cron.is_suspended = true;
    assert_eq!(
        state_label(
            ResourceAction::SuspendCronJob,
            "Suspend",
            &KindObject::CronJob(cron)
        ),
        "Resume"
    );
    assert_eq!(
        state_label(
            ResourceAction::TriggerCronJob,
            "Trigger now",
            &KindObject::CronJob(cron_job("reconcile", "Allow", 0))
        ),
        "Trigger now"
    );
}

// ---- Scale ----

fn hpa(name: &str, target_kind: &str, target_name: &str) -> KindObject {
    KindObject::HorizontalPodAutoscaler(cluster::HorizontalPodAutoscalerSummary {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        target: cluster::ControllerRef {
            kind: target_kind.to_owned(),
            name: target_name.to_owned(),
        },
        min_replicas: 2,
        max_replicas: 8,
        current_replicas: 3,
        desired_replicas: 3,
        metrics: Vec::new(),
        conditions: Vec::new(),
        last_scaled_at: None,
    })
}

fn deployment_target(hpas: &[KindObject]) -> ScaleTarget {
    ScaleTarget::of(&KindObject::Deployment(deployment("api")), hpas).expect("a scale target")
}

fn scale_to(target: &ScaleTarget, replicas: u32) -> WriteIntent {
    let cluster = test_cluster();
    let scope = WorkloadScope {
        cluster: &cluster,
        cluster_name: "stg-b",
    };
    scale_intent(&scope, target, replicas).expect("an intent")
}

#[test]
fn scale_names_the_change_and_changes_one_field() {
    let intent = scale_to(&deployment_target(&[]), 5);
    assert_eq!(intent.label, "Scale deployment api from 3 to 5");
    assert_eq!(intent.button, "Scale");
    assert_eq!(intent.risk, ActionRisk::Change);
    assert_eq!(
        intent.request.operation(),
        &WriteOperation::ScaleWorkload { replicas: 5 }
    );
    assert!(intent.warnings.is_empty());
    let sets = ScaleTarget::of(
        &KindObject::StatefulSet(stateful_set("kafka", "OnDelete")),
        &[],
    )
    .expect("a stateful set scales");
    assert_eq!(
        scale_to(&sets, 4).label,
        "Scale statefulset kafka from 3 to 4"
    );
}

#[test]
fn scale_to_zero_is_destructive() {
    let target = deployment_target(&[]);
    assert_eq!(scale_to(&target, 0).risk, ActionRisk::Destructive);
    assert_eq!(scale_to(&target, 1).risk, ActionRisk::Change);
}

#[test]
fn scale_down_warns() {
    let target = deployment_target(&[]);
    assert_eq!(
        warnings(&scale_to(&target, 1)),
        ["Scaling down from 3 to 1"]
    );
    assert_eq!(
        warnings(&scale_to(&target, 0)),
        ["Scaling down from 3 to 0"]
    );
    // Scaling up, or to the same number, says nothing.
    assert!(scale_to(&target, 4).warnings.is_empty());
    assert!(scale_to(&target, 3).warnings.is_empty());
}

#[test]
fn hpa_warning_only_when_targeting_and_loaded() {
    let targeting = hpa("api-hpa", "Deployment", "api");
    let target = deployment_target(std::slice::from_ref(&targeting));
    assert_eq!(
        warnings(&scale_to(&target, 5)),
        ["HPA api-hpa manages replicas (2–8); it will override this"]
    );
    // The warning follows the scale-down line; a value outside the range says it is reverted.
    assert_eq!(
        warnings(&scale_to(&target, 1)),
        [
            "Scaling down from 3 to 1",
            "HPA api-hpa keeps 2–8; a value outside is reverted"
        ]
    );
    assert_eq!(
        warnings(&scale_to(&target, 9)),
        ["HPA api-hpa keeps 2–8; a value outside is reverted"]
    );
    // The bounds themselves are inside.
    assert_eq!(
        warnings(&scale_to(&target, 8)),
        ["HPA api-hpa manages replicas (2–8); it will override this"]
    );
    // An HPA of another workload or another kind is not this workload's.
    for other in [
        hpa("web-hpa", "Deployment", "web"),
        hpa("api-hpa", "StatefulSet", "api"),
    ] {
        assert!(deployment_target(&[other]).hpa.is_none());
    }
    // The list is not loaded yet: no warning, and nothing starts to load it.
    assert!(deployment_target(&[]).hpa.is_none());
}

#[test]
fn only_deployments_and_stateful_sets_scale() {
    assert!(
        ScaleTarget::of(
            &KindObject::DaemonSet(daemon_set("agent", "RollingUpdate")),
            &[]
        )
        .is_none()
    );
    assert!(ScaleTarget::of(&KindObject::Job(job("etl")), &[]).is_none());
    let target = deployment_target(&[]);
    assert_eq!(target.subject_text(), "deployment/api");
    assert_eq!(target.state_text(), "Now 3 desired · 2 ready");
}

#[test]
fn replicas_must_be_a_new_whole_number() {
    assert_eq!(replicas_input("5", 3), ReplicasInput::Set(5));
    assert_eq!(replicas_input(" 5 ", 3), ReplicasInput::Set(5));
    assert_eq!(replicas_input("0", 3), ReplicasInput::Set(0));
    assert_eq!(replicas_input("3", 3), ReplicasInput::Unchanged);
    for text in ["", " ", "-1", "+2", "2.5", "1e3", "abc", "99999999999"] {
        assert_eq!(replicas_input(text, 3), ReplicasInput::Invalid, "{text:?}");
    }
    // The API takes an int32: the largest count is accepted, one more is not.
    assert_eq!(parse_replicas("2147483647"), Some(2_147_483_647));
    assert_eq!(parse_replicas("2147483648"), None);
}

#[test]
fn the_dialog_names_the_row_cluster_for_a_scale() {
    let intent = scale_to(&deployment_target(&[]), 5);
    assert_eq!(intent.cluster, test_cluster());
    assert_eq!(intent.cluster_name, "stg-b");
    assert_eq!(intent.request.target().namespace(), Some("team-a"));
}

// ---- Roll back ----

pub(crate) fn replica_set(
    name: &str,
    revision: Option<&str>,
    owner: Option<&str>,
    image: &str,
) -> ReplicaSetSummary {
    ReplicaSetSummary {
        namespace: "team-a".to_owned(),
        name: name.to_owned(),
        created_at: None,
        labels: Vec::new(),
        desired: 3,
        current: 3,
        ready: 3,
        owner: owner.map(|owner| cluster::ControllerRef {
            kind: "Deployment".to_owned(),
            name: owner.to_owned(),
        }),
        revision: revision.map(str::to_owned),
        change_cause: None,
        selector: Vec::new(),
        containers: vec![cluster::TemplateContainer {
            resources: Vec::new(),
            name: "web".to_owned(),
            image: image.to_owned(),
            ports: Vec::new(),
        }],
    }
}

/// `api` runs revision 7; 6 and 5 are older, 8 is newer than the Deployment knows (a stale row).
fn revisions() -> Vec<ReplicaSetSummary> {
    vec![
        replica_set("api-7d", Some("7"), Some("api"), "api:2.14.0"),
        replica_set("api-6c1e2a", Some("6"), Some("api"), "api:2.13.4"),
        replica_set("api-5b", Some("5"), Some("api"), "api:2.12.0"),
        replica_set("api-8f", Some("8"), Some("api"), "api:2.15.0"),
    ]
}

#[test]
fn previous_revision_is_the_highest_below_current() {
    let target = previous_revision(&deployment("api"), &revisions()).expect("a previous revision");
    assert_eq!(
        target,
        RevisionTarget {
            replica_set: "api-6c1e2a".to_owned(),
            revision: 6,
            tag: Some("2.13.4".to_owned()),
        }
    );
    assert_eq!(target.text(), "rev 6 (2.13.4)");
}

#[test]
fn a_foreign_or_unnumbered_replica_set_is_never_the_previous_revision() {
    let sets = vec![
        // Another Deployment's, a name that merely starts alike, and one without a number.
        replica_set("api-6-other", Some("6"), Some("api-canary"), "api:1"),
        replica_set("api-6-orphan", Some("6"), None, "api:1"),
        replica_set("api-x", None, Some("api"), "api:1"),
        replica_set("api-nan", Some("six"), Some("api"), "api:1"),
    ];
    assert_eq!(previous_revision(&deployment("api"), &sets), None);
    // A Deployment that does not know its own revision has nothing to compare with.
    let mut unknown = deployment("api");
    unknown.revision = None;
    assert_eq!(previous_revision(&unknown, &revisions()), None);
}

#[test]
fn an_image_without_a_tag_names_the_revision_alone() {
    let sets = vec![replica_set("api-6", Some("6"), Some("api"), "")];
    let target = previous_revision(&deployment("api"), &sets).expect("a revision");
    assert_eq!(target.tag, None);
    assert_eq!(target.text(), "rev 6");
}

#[test]
fn roll_back_waits_for_loaded_revisions_and_reads_the_deployment_state() {
    let sets = revisions();
    let running = deployment("api");
    assert_eq!(roll_back_choice(&running, None), RollBackChoice::NotLoaded);
    assert!(matches!(
        roll_back_choice(&running, Some(&sets)),
        RollBackChoice::To(_)
    ));
    assert_eq!(
        roll_back_choice(&running, Some(&sets[..1])),
        RollBackChoice::NoEarlier
    );
    let mut paused = deployment("api");
    paused.is_paused = true;
    // Paused wins over everything else, loaded or not.
    assert_eq!(roll_back_choice(&paused, None), RollBackChoice::Paused);
    assert_eq!(
        roll_back_choice(&paused, Some(&sets)),
        RollBackChoice::Paused
    );
}

#[test]
fn roll_back_row_block_reasons() {
    let sets = revisions();
    let block = |deployment: DeploymentSummary, sets: Option<&[ReplicaSetSummary]>| {
        row_block(
            ResourceAction::RollBack,
            &KindObject::Deployment(deployment),
            sets,
        )
        .map(|reason| reason.to_string())
    };
    assert_eq!(block(deployment("api"), Some(&sets)), None);
    // Revisions that are not loaded yet do not block: the item opens the drawer, which loads them.
    assert_eq!(block(deployment("api"), None), None);
    assert_eq!(
        block(deployment("api"), Some(&sets[..1])).as_deref(),
        Some("No earlier revision")
    );
    let mut paused = deployment("api");
    paused.is_paused = true;
    assert_eq!(
        block(paused, Some(&sets)).as_deref(),
        Some("Resume the rollout first")
    );
}

#[test]
fn roll_back_intent_names_the_revision() {
    let cluster = test_cluster();
    let scope = WorkloadScope {
        cluster: &cluster,
        cluster_name: "stg-b",
    };
    let target = previous_revision(&deployment("api"), &revisions()).expect("a revision");
    let intent = roll_back_intent(&scope, &deployment("api"), &target).expect("an intent");
    assert_eq!(intent.label, "Roll back deployment api to rev 6 (2.13.4)");
    assert_eq!(intent.button, "Roll back");
    assert_eq!(intent.risk, ActionRisk::Change);
    assert_eq!(
        intent.request.operation(),
        &WriteOperation::RollBackDeployment {
            replica_set: "api-6c1e2a".to_owned(),
            revision: 6,
        }
    );
    // The dialog shows names and numbers only, never the template.
    let fields = intent.request.changed_fields();
    assert_eq!(fields[0].value.as_deref(), Some("rev 6 (api-6c1e2a)"));
}

// ---- Bulk ----

fn bulk_objects_for(objects: &[KindObject]) -> Vec<CheckedRow<'_>> {
    // One cluster for every row: a batch never spans two.
    let cluster: &'static ClusterRef = Box::leak(Box::new(test_cluster()));
    objects
        .iter()
        .map(|object| CheckedRow { cluster, object })
        .collect()
}

fn bulk(action: ResourceAction, objects: &[KindObject]) -> Result<BatchIntent, SharedString> {
    let rows = bulk_objects_for(objects);
    let inputs = BulkInputs {
        cluster_name: "stg-b",
        rows: &rows,
        now: now(),
        hpas: &[],
    };
    bulk_intent(action, &inputs)
}

fn item_names(batch: &BatchIntent) -> Vec<&str> {
    batch
        .plan
        .items
        .iter()
        .map(|item| item.object.as_ref())
        .collect()
}

#[test]
fn bulk_restart_names_the_count_and_the_kind() {
    let objects: Vec<KindObject> = ["api", "web", "worker", "cron"]
        .into_iter()
        .map(|name| KindObject::Deployment(deployment(name)))
        .collect();
    let batch = bulk(restart_of(ObjectKind::Deployment), &objects).expect("a batch");
    assert_eq!(batch.label, "Restart 4 deployments");
    assert_eq!(batch.verb, "Restart");
    assert_eq!(batch.button, "Restart");
    assert_eq!(batch.confirm_label(0), "Restart 4");
    assert_eq!(batch.risk, ActionRisk::Change);
    assert_eq!(batch.cluster, test_cluster());
    assert_eq!(
        item_names(&batch),
        ["team-a/api", "team-a/web", "team-a/worker", "team-a/cron"]
    );
    // Each item is the same request the single action sends, so its dry-run and audit line match.
    assert_eq!(
        batch.plan.items[0].label,
        "Restart rollout of deployment api"
    );
    let WriteOperation::RestartRollout { restarted_at } = batch.plan.items[2].request.operation()
    else {
        panic!("a restart");
    };
    assert_eq!(restarted_at.as_second(), 1_790_000_000);
}

#[test]
fn batch_skips_rows_with_a_row_block() {
    let mut paused = deployment("web");
    paused.is_paused = true;
    let objects = vec![
        KindObject::Deployment(deployment("api")),
        KindObject::Deployment(paused),
    ];
    let batch = bulk(restart_of(ObjectKind::Deployment), &objects).expect("a batch");
    assert_eq!(item_names(&batch), ["team-a/api"]);
    assert_eq!(batch.plan.skipped.len(), 1);
    assert_eq!(batch.plan.skipped[0].object, "team-a/web");
    assert_eq!(batch.plan.skipped[0].reason, "Resume the rollout first");
    assert_eq!(batch.label, "Restart 1 deployments");
}

#[test]
fn bulk_restart_says_on_delete_once() {
    let objects = vec![
        KindObject::StatefulSet(stateful_set("kafka", "OnDelete")),
        KindObject::StatefulSet(stateful_set("redis", "OnDelete")),
        KindObject::StatefulSet(stateful_set("pg", "RollingUpdate")),
    ];
    let batch = bulk(restart_of(ObjectKind::StatefulSet), &objects).expect("a batch");
    assert_eq!(
        batch.warnings,
        ["2 use update strategy OnDelete: their pods restart only when deleted"]
    );
}

#[test]
fn bulk_rerun_and_trigger_name_their_words() {
    let jobs = vec![KindObject::Job(job("etl-1")), KindObject::Job(job("etl-2"))];
    let rerun = bulk(ResourceAction::RerunJob, &jobs).expect("a batch");
    assert_eq!(rerun.label, "Re-run 2 jobs");
    assert_eq!(rerun.button, "Re-run");
    let crons = vec![
        KindObject::CronJob(cron_job("a", "Allow", 0)),
        KindObject::CronJob(cron_job("b", "Allow", 0)),
    ];
    let trigger = bulk(ResourceAction::TriggerCronJob, &crons).expect("a batch");
    assert_eq!(trigger.label, "Run 2 cronjobs now");
    assert_eq!(trigger.verb, "Run");
    assert_eq!(trigger.button, "Trigger now");
    assert_eq!(trigger.confirm_label(0), "Run 2");
}

#[test]
fn bulk_suspend_label_reads_resume_when_all_suspended() {
    let mut first = cron_job("a", "Allow", 0);
    first.is_suspended = true;
    let mut second = cron_job("b", "Allow", 0);
    second.is_suspended = true;
    let all = vec![KindObject::CronJob(first), KindObject::CronJob(second)];
    assert!(all_suspended(&bulk_objects_for(&all)));
    let resume = bulk(ResourceAction::SuspendCronJob, &all).expect("a batch");
    assert_eq!(resume.label, "Resume 2 cronjobs");
    assert_eq!(resume.button, "Resume");
    for item in &resume.plan.items {
        assert_eq!(
            item.request.operation(),
            &WriteOperation::SetCronJobSuspended { suspended: false }
        );
    }
    // A mix suspends the running ones and lists the suspended ones as skipped.
    let mut done = cron_job("c", "Allow", 0);
    done.is_suspended = true;
    let mix = vec![
        KindObject::CronJob(cron_job("d", "Allow", 0)),
        KindObject::CronJob(done),
    ];
    assert!(!all_suspended(&bulk_objects_for(&mix)));
    let suspend = bulk(ResourceAction::SuspendCronJob, &mix).expect("a batch");
    assert_eq!(suspend.label, "Suspend 1 cronjobs");
    assert_eq!(item_names(&suspend), ["team-a/d"]);
    assert_eq!(suspend.plan.skipped[0].reason, "already suspended");
}

#[test]
fn bulk_scale_down_line_counts_the_items_of_the_batch() {
    // A row whose name cannot be sent is skipped, so it is in neither number of the line.
    let objects = vec![
        KindObject::Deployment(deployment("api")),
        KindObject::Deployment(deployment("")),
    ];
    let rows = bulk_objects_for(&objects);
    let inputs = BulkInputs {
        cluster_name: "stg-b",
        rows: &rows,
        now: now(),
        hpas: &[],
    };
    let batch = bulk_scale_intent(&inputs, 1, ObjectKind::Deployment).expect("a batch");
    assert_eq!(item_names(&batch), ["team-a/api"]);
    assert_eq!(batch.warnings, ["Scaling down 1 of 1"]);
}

#[test]
fn bulk_scale_sets_one_count_and_skips_rows_that_have_it() {
    let mut four = deployment("web");
    four.desired = 4;
    let objects = vec![
        KindObject::Deployment(deployment("api")),
        KindObject::Deployment(four),
    ];
    let rows = bulk_objects_for(&objects);
    let inputs = BulkInputs {
        cluster_name: "stg-b",
        rows: &rows,
        now: now(),
        hpas: &[],
    };
    let batch = bulk_scale_intent(&inputs, 4, ObjectKind::Deployment).expect("a batch");
    assert_eq!(batch.label, "Scale 1 deployments to 4");
    assert_eq!(item_names(&batch), ["team-a/api"]);
    assert_eq!(batch.plan.skipped[0].reason, "already 4");
    assert_eq!(batch.risk, ActionRisk::Change);
    assert!(batch.warnings.is_empty());
    // Zero takes workloads down, so it is the destructive tier, with a scale-down line.
    let zero = bulk_scale_intent(&inputs, 0, ObjectKind::Deployment).expect("a batch");
    assert_eq!(zero.risk, ActionRisk::Destructive);
    assert_eq!(zero.warnings, ["Scaling down 2 of 2"]);
    // Every row at the count already: nothing to do, and the button says why.
    let same = vec![KindObject::Deployment(deployment("api"))];
    let rows = bulk_objects_for(&same);
    let inputs = BulkInputs {
        cluster_name: "stg-b",
        rows: &rows,
        now: now(),
        hpas: &[],
    };
    assert_eq!(
        bulk_scale_intent(&inputs, 3, ObjectKind::Deployment)
            .err()
            .as_deref(),
        Some("already 3")
    );
}

#[test]
fn bulk_scale_warns_about_hpa_managed_rows() {
    let objects = vec![
        KindObject::Deployment(deployment("api")),
        KindObject::Deployment(deployment("web")),
    ];
    let hpas = vec![hpa("api-hpa", "Deployment", "api")];
    let rows = bulk_objects_for(&objects);
    let inputs = BulkInputs {
        cluster_name: "stg-b",
        rows: &rows,
        now: now(),
        hpas: &hpas,
    };
    let batch = bulk_scale_intent(&inputs, 5, ObjectKind::Deployment).expect("a batch");
    assert_eq!(
        batch.warnings,
        ["1 of them are managed by an HPA, which will override this"]
    );
}

#[test]
fn bulk_refuses_an_action_with_no_bulk_form() {
    let objects = vec![KindObject::Deployment(deployment("api"))];
    assert_eq!(
        bulk(ResourceAction::RollBack, &objects).err().as_deref(),
        Some("Not a bulk action")
    );
}

fn helm_managed(mut summary: DeploymentSummary) -> DeploymentSummary {
    summary
        .labels
        .push("app.kubernetes.io/managed-by=Helm".to_owned());
    summary
}

#[test]
fn scale_warns_when_helm_manages_the_workload() {
    let object = KindObject::Deployment(helm_managed(deployment("api")));
    let target = ScaleTarget::of(&object, &[]).expect("a scale target");
    let intent = scale_to(&target, 5);
    assert_eq!(intent.warnings, [HELM_MANAGED_WARNING]);
    // A workload that Helm does not manage has no such line.
    assert!(scale_to(&deployment_target(&[]), 5).warnings.is_empty());
}

#[test]
fn roll_back_warns_when_helm_manages_the_deployment() {
    let cluster = test_cluster();
    let scope = WorkloadScope {
        cluster: &cluster,
        cluster_name: "stg-b",
    };
    let target = RevisionTarget {
        replica_set: "api-6c".to_owned(),
        revision: 6,
        tag: None,
    };
    let helm =
        roll_back_intent(&scope, &helm_managed(deployment("api")), &target).expect("an intent");
    assert_eq!(helm.warnings, [HELM_MANAGED_WARNING]);
    let plain = roll_back_intent(&scope, &deployment("api"), &target).expect("an intent");
    assert!(plain.warnings.is_empty());
}

// ---- Resume: pending changes ----

fn web_container(image: &str) -> cluster::TemplateContainer {
    cluster::TemplateContainer {
        name: "web".to_owned(),
        image: image.to_owned(),
        ports: Vec::new(),
        resources: Vec::new(),
    }
}

/// The ReplicaSet that runs now (`desired` 3) and an old one (`desired` 0).
fn running_and_old(image: &str) -> Vec<ReplicaSetSummary> {
    let mut running = replica_set("api-7d", Some("7"), Some("api"), image);
    running.desired = 3;
    let mut old = replica_set("api-6c", Some("6"), Some("api"), "api:1.0.0");
    old.desired = 0;
    vec![old, running]
}

#[test]
fn resume_lists_the_image_changes_made_while_paused() {
    let mut paused = deployment("api");
    paused.is_paused = true;
    paused.containers = vec![web_container("repo.example.com/api:2.15.0")];
    let sets = running_and_old("repo.example.com/api:2.14.0");
    assert_eq!(
        pending_changes_note(&paused, Some(&sets)),
        "Rolls out 1 pending image change: image web 2.14.0 → 2.15.0"
    );
    // A new repository names both references.
    paused.containers = vec![web_container("other.example.com/api:2.14.0")];
    assert_eq!(
        pending_changes_note(&paused, Some(&sets)),
        "Rolls out 1 pending image change: image web repo.example.com/api:2.14.0 → \
         other.example.com/api:2.14.0"
    );
}

#[test]
fn resume_counts_added_and_removed_containers() {
    let mut paused = deployment("api");
    paused.containers = vec![web_container("api:2.14.0"), {
        let mut sidecar = web_container("proxy:1");
        sidecar.name = "proxy".to_owned();
        sidecar
    }];
    let sets = running_and_old("api:2.14.0");
    assert_eq!(
        pending_changes_note(&paused, Some(&sets)),
        "Rolls out 1 pending image change: container proxy added"
    );
}

#[test]
fn resume_without_a_running_replica_set_or_an_image_change_stays_general() {
    let mut paused = deployment("api");
    paused.containers = vec![web_container("api:2.14.0")];
    assert_eq!(
        pending_changes_note(&paused, None),
        "Rolls out the pod template changes made while paused"
    );
    assert_eq!(
        pending_changes_note(&paused, Some(&running_and_old("api:2.14.0"))),
        "Rolls out the pod template changes made while paused; no image changed"
    );
}

// ---- Set image ----

fn container(name: &str, image: &str) -> TemplateContainer {
    TemplateContainer {
        name: name.to_owned(),
        image: image.to_owned(),
        ports: Vec::new(),
        resources: Vec::new(),
    }
}

fn web_deployment(containers: Vec<TemplateContainer>) -> KindObject {
    let mut summary = deployment("web");
    summary.containers = containers;
    KindObject::Deployment(summary)
}

fn set_image_to(target: &ImageTarget, container: &str, image: &str, cause: &str) -> WriteIntent {
    let cluster = test_cluster();
    let scope = WorkloadScope {
        cluster: &cluster,
        cluster_name: "stg-b",
    };
    set_image_intent(&scope, target, container, image, cause).expect("an intent")
}

#[test]
fn the_tag_is_what_the_popover_selects() {
    let selected = |image: &str| image[tag_range(image)].to_owned();
    assert_eq!(selected("nginx:1.27-alpine"), "1.27-alpine");
    assert_eq!(selected("repo.example.com:5000/library/nginx:1.27"), "1.27");
    assert_eq!(selected("nginx@sha256:abc123"), "sha256:abc123");
    // Without a tag the cursor lands at the end, where a tag is typed.
    assert_eq!(selected("repo.example.com:5000/nginx"), "");
    assert_eq!(tag_range("nginx"), 5..5);
}

#[test]
fn an_image_is_unchanged_set_or_invalid() {
    assert_eq!(image_input("nginx:1", "nginx:1"), ImageInput::Unchanged);
    assert_eq!(image_input(" nginx:1 ", "nginx:1"), ImageInput::Unchanged);
    assert_eq!(
        image_input("nginx:2", "nginx:1"),
        ImageInput::Set("nginx:2".to_owned())
    );
    assert_eq!(image_input("", "nginx:1"), ImageInput::Invalid);
    assert_eq!(image_input("nginx 2", "nginx:1"), ImageInput::Invalid);
}

#[test]
fn only_pod_template_workloads_with_a_container_take_set_image() {
    let one = web_deployment(vec![container("web", "nginx:1")]);
    assert!(ImageTarget::of(&one).is_some());
    assert!(ImageTarget::of(&web_deployment(Vec::new())).is_none());
    assert!(ImageTarget::of(&KindObject::Job(job("etl"))).is_none());
}

#[test]
fn set_image_lists_the_old_and_the_new_image_and_the_cause() {
    let object = web_deployment(vec![container("web", "nginx:1.27-alpine")]);
    let target = ImageTarget::of(&object).expect("a target");
    let intent = set_image_to(&target, "web", "nginx:1.26-alpine", " release test ");
    assert_eq!(intent.label, "Set image of deployment web");
    assert_eq!(intent.button, "Set image");
    assert_eq!(intent.risk, ActionRisk::Change);
    assert_eq!(
        intent.action,
        ResourceAction::SetImage(ObjectKind::Deployment)
    );
    assert_eq!(
        intent.change_lines,
        [
            "image: nginx:1.27-alpine → nginx:1.26-alpine",
            "change cause: release test"
        ]
    );
}

#[test]
fn set_image_names_the_container_when_the_template_has_several() {
    let object = web_deployment(vec![
        container("web", "nginx:1"),
        container("sidecar", "busybox:1"),
    ]);
    let target = ImageTarget::of(&object).expect("a target");
    let intent = set_image_to(&target, "sidecar", "busybox:2", "");
    assert_eq!(
        intent.change_lines,
        [
            "image (sidecar): busybox:1 → busybox:2",
            "change cause: none (an earlier one is cleared)"
        ]
    );
}

#[test]
fn set_image_of_a_container_that_left_the_template_is_refused() {
    let object = web_deployment(vec![container("web", "nginx:1")]);
    let target = ImageTarget::of(&object).expect("a target");
    let cluster = test_cluster();
    let scope = WorkloadScope {
        cluster: &cluster,
        cluster_name: "stg-b",
    };
    assert!(set_image_intent(&scope, &target, "gone", "nginx:2", "").is_none());
}

#[test]
fn set_image_warns_about_helm_and_a_paused_rollout() {
    let mut summary = deployment("web");
    summary.containers = vec![container("web", "nginx:1")];
    summary.is_paused = true;
    summary.labels = vec!["app.kubernetes.io/managed-by=Helm".to_owned()];
    let target = ImageTarget::of(&KindObject::Deployment(summary)).expect("a target");
    let intent = set_image_to(&target, "web", "nginx:2", "");
    assert_eq!(
        warnings(&intent),
        [
            "The rollout is paused: the pods change after Resume",
            HELM_MANAGED_WARNING
        ]
    );
}

#[test]
fn set_image_needs_the_patch_right_of_its_kind() {
    for kind in [
        ObjectKind::Deployment,
        ObjectKind::StatefulSet,
        ObjectKind::DaemonSet,
    ] {
        let object = match kind {
            ObjectKind::Deployment => web_deployment(vec![container("web", "nginx:1")]),
            ObjectKind::StatefulSet => {
                let mut set = stateful_set("web", "RollingUpdate");
                set.containers = vec![container("web", "nginx:1")];
                KindObject::StatefulSet(set)
            }
            _ => {
                let mut set = daemon_set("web", "RollingUpdate");
                set.containers = vec![container("web", "nginx:1")];
                KindObject::DaemonSet(set)
            }
        };
        let target = ImageTarget::of(&object).expect("a target");
        let intent = set_image_to(&target, "web", "nginx:2", "");
        assert_eq!(intent.request.target().kind_name(), kind.name());
    }
}

fn quota_in(namespace: &str, resource: &str, hard: &str, used: &str) -> KindObject {
    KindObject::ResourceQuota(cluster::ResourceQuotaSummary {
        namespace: namespace.to_owned(),
        name: "team-quota".to_owned(),
        created_at: None,
        labels: Vec::new(),
        items: vec![cluster::QuotaItem {
            resource: resource.to_owned(),
            hard: hard.to_owned(),
            used: Some(used.to_owned()),
        }],
        scopes: Vec::new(),
    })
}

fn deployment_limiting_memory(limit: &str) -> KindObject {
    let mut api = deployment("api");
    api.containers = vec![TemplateContainer {
        resources: vec![cluster::ContainerResource {
            name: "memory".to_owned(),
            request: None,
            limit: Some(limit.to_owned()),
        }],
        ..web_container("web:1")
    }];
    KindObject::Deployment(api)
}

#[test]
fn a_scale_past_the_namespace_quota_says_which_pods_will_not_start() {
    let quotas = [quota_in("team-a", "limits.memory", "640Mi", "600Mi")];
    let target = ScaleTarget::of(&deployment_limiting_memory("150Mi"), &[])
        .expect("a scale target")
        .with_quotas(&quotas);
    assert_eq!(
        scale_to(&target, 4).warnings,
        ["needs 150Mi limits.memory per pod, team-quota has 40Mi left: the new pod will not start"]
    );
    assert_eq!(
        scale_to(&target, 5).warnings,
        [
            "needs 150Mi limits.memory per pod, team-quota has 40Mi left: none of the 2 new pods will start"
        ]
    );
    // Scaling down adds no pod.
    assert!(
        scale_to(&target, 2)
            .warnings
            .iter()
            .all(|w| !w.contains("quota"))
    );
}

#[test]
fn a_quota_of_another_namespace_or_none_loaded_says_nothing() {
    let other = [quota_in("team-b", "limits.memory", "640Mi", "600Mi")];
    let object = deployment_limiting_memory("150Mi");
    let with_other = ScaleTarget::of(&object, &[])
        .expect("a scale target")
        .with_quotas(&other);
    assert!(with_other.quotas.is_empty());
    assert!(scale_to(&with_other, 5).warnings.is_empty());
    let unloaded = ScaleTarget::of(&object, &[]).expect("a scale target");
    assert!(scale_to(&unloaded, 5).warnings.is_empty());
}
