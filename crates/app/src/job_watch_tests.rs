use cluster::{
    ContainerKind, ContainerProbes, ContainerState, ContainerSummary, ControllerRef, PodStatus,
    ReadyCount, StatusReason, Termination, WorkloadCondition,
};

use super::*;

fn time(text: &str) -> jiff::Timestamp {
    text.parse().expect("valid timestamp")
}

fn job(status: JobStatus) -> JobSummary {
    JobSummary {
        namespace: "lab-batch".to_owned(),
        annotations: cluster::AnnotationTerms::default(),
        name: "report-failed-manual-x1".to_owned(),
        created_at: None,
        labels: Vec::new(),
        status,
        completions: Some(1),
        parallelism: Some(1),
        succeeded: 0,
        failed: 0,
        active: 0,
        backoff_limit: Some(0),
        active_deadline_seconds: None,
        ttl_seconds_after_finished: None,
        started_at: Some(time("2024-10-04T10:00:00Z")),
        finished_at: Some(time("2024-10-04T10:00:12Z")),
        owner: None,
        conditions: Vec::new(),
        containers: Vec::new(),
    }
}

fn failed_condition(reason: &str) -> WorkloadCondition {
    WorkloadCondition {
        name: "Failed".to_owned(),
        is_true: true,
        reason: Some(reason.to_owned()),
        message: None,
        last_transition: None,
    }
}

fn job_pod(name: &str, owner: &str, created: &str, exit_code: i32) -> PodSummary {
    PodSummary {
        namespace: "lab-batch".to_owned(),
        annotations: cluster::AnnotationTerms::default(),
        name: name.to_owned(),
        status: PodStatus::Reason(StatusReason::Error),
        ready: ReadyCount { ready: 0, total: 1 },
        restarts: 0,
        node_name: None,
        created_at: Some(time(created)),
        pod_ip: None,
        qos_class: None,
        service_account: None,
        controller: Some(ControllerRef {
            kind: "Job".to_owned(),
            name: owner.to_owned(),
        }),
        conditions: Vec::new(),
        containers: vec![ContainerSummary {
            terminal: cluster::ContainerTerminal::None,
            name: "main".to_owned(),
            image: "busybox".to_owned(),
            kind: ContainerKind::Main,
            state: ContainerState::Terminated(Termination {
                reason: Some(StatusReason::Error),
                exit_code,
                signal: None,
                started_at: None,
                finished_at: None,
            }),
            is_ready: false,
            restart_count: 0,
            last_termination: None,
            image_digest: None,
            pull_policy: None,
            is_started: None,
            ports: Vec::new(),
            resources: Vec::new(),
            probes: ContainerProbes::default(),
            env: Vec::new(),
            env_from: Vec::new(),
            mounts: Vec::new(),
        }],
        status_message: None,
        labels: Vec::new(),
        host_network: false,
        image_pull_secrets: Vec::new(),
        node_selector: Vec::new(),
        node_affinity: Vec::new(),
        is_finished: true,
    }
}

#[test]
fn a_running_job_has_no_end() {
    for status in [JobStatus::Running, JobStatus::Failing, JobStatus::Suspended] {
        assert_eq!(job_end_text(&job(status), None), None, "{status:?}");
    }
}

#[test]
fn a_complete_job_says_how_long_it_took() {
    assert_eq!(
        job_end_text(&job(JobStatus::Complete), None),
        Some((
            "Job report-failed-manual-x1 succeeded in 12s".to_owned(),
            true
        ))
    );
    let mut unstarted = job(JobStatus::Complete);
    unstarted.started_at = None;
    assert_eq!(
        job_end_text(&unstarted, None).map(|(text, _)| text),
        Some("Job report-failed-manual-x1 succeeded".to_owned())
    );
}

#[test]
fn a_failed_job_names_the_exit_code_first() {
    let mut failed = job(JobStatus::Failed);
    failed.conditions = vec![failed_condition("BackoffLimitExceeded")];
    assert_eq!(
        job_end_text(&failed, Some(3)),
        Some((
            "Job report-failed-manual-x1 failed: exit 3".to_owned(),
            false
        ))
    );
    assert_eq!(
        job_end_text(&failed, None).map(|(text, _)| text),
        Some("Job report-failed-manual-x1 failed: backoff limit reached".to_owned())
    );
}

#[test]
fn a_failed_job_without_a_reason_points_to_its_pods() {
    assert_eq!(
        job_end_text(&job(JobStatus::Failed), None).map(|(text, _)| text),
        Some("Job report-failed-manual-x1 failed: see its pods".to_owned())
    );
    let mut late = job(JobStatus::Failed);
    late.conditions = vec![failed_condition("DeadlineExceeded")];
    assert_eq!(
        job_end_text(&late, None).map(|(text, _)| text),
        Some("Job report-failed-manual-x1 failed: active deadline exceeded".to_owned())
    );
}

#[test]
fn the_exit_code_comes_from_the_newest_failed_pod_of_the_job() {
    let job = job(JobStatus::Failed);
    let pods = [
        job_pod("old", "report-failed-manual-x1", "2024-10-04T10:00:00Z", 1),
        job_pod("new", "report-failed-manual-x1", "2024-10-04T10:00:05Z", 3),
        job_pod("other", "another-job", "2024-10-04T10:00:09Z", 9),
    ];
    assert_eq!(failed_exit_code(&pods, &job), Some(3));
    assert_eq!(failed_exit_code(&[], &job), None);
}

fn cron_job() -> CronJobSummary {
    CronJobSummary {
        namespace: "lab-batch".to_owned(),
        annotations: cluster::AnnotationTerms::default(),
        name: "cleanup-suspended".to_owned(),
        created_at: None,
        labels: Vec::new(),
        schedule: "*/2 * * * *".to_owned(),
        time_zone: None,
        timetable: cluster::CronSchedule::parse("*/2 * * * *", None),
        is_suspended: false,
        concurrency_policy: "Allow".to_owned(),
        starting_deadline_seconds: Some(60),
        successful_history_limit: None,
        failed_history_limit: None,
        active_jobs: Vec::new(),
        last_schedule_at: Some(time("2024-10-04T10:00:00Z")),
        last_success_at: None,
        containers: Vec::new(),
    }
}

#[test]
fn a_resume_finds_the_job_once_the_cron_job_scheduled_the_missed_run() {
    let run = MissedRun {
        at: time("2024-10-04T10:06:00Z"),
        starts: true,
    };
    let mut cron = cron_job();
    // Not scheduled yet: the last schedule is the old one.
    assert_eq!(started_job(&cron, run), None);
    cron.last_schedule_at = Some(run.at);
    // Scheduled and the Job is not listed any more.
    assert_eq!(started_job(&cron, run), Some(None));
    cron.active_jobs = vec!["cleanup-1".to_owned(), "cleanup-2".to_owned()];
    assert_eq!(started_job(&cron, run), Some(Some("cleanup-2")));
}

#[test]
fn the_resume_toast_names_the_job_the_next_run_or_the_skipped_run() {
    let name = "cleanup-suspended";
    assert_eq!(
        resume_end_text(
            name,
            &ResumeEnd::Started {
                job: Some("cleanup-suspended-29".to_owned())
            },
            Some("22:24 UTC")
        ),
        "Started job cleanup-suspended-29 (missed run)"
    );
    assert_eq!(
        resume_end_text(name, &ResumeEnd::Started { job: None }, Some("22:24 UTC")),
        "Resumed cronjob cleanup-suspended; the missed run started; next run 22:24 UTC"
    );
    assert_eq!(
        resume_end_text(
            name,
            &ResumeEnd::Resumed { skipped: None },
            Some("22:24 UTC")
        ),
        "Resumed cronjob cleanup-suspended; next run 22:24 UTC"
    );
    assert_eq!(
        resume_end_text(
            name,
            &ResumeEnd::Resumed {
                skipped: Some("22:22 UTC".to_owned())
            },
            None
        ),
        "Resumed cronjob cleanup-suspended; the 22:22 UTC run was past its deadline and is skipped"
    );
}
