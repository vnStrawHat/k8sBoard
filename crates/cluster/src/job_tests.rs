use k8s_openapi::api::batch::v1::{JobSpec, JobStatus as ApiJobStatus};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;

use super::*;

fn timestamp(text: &str) -> jiff::Timestamp {
    text.parse().expect("valid timestamp")
}

fn api_condition(type_: &str, status: &str) -> JobCondition {
    JobCondition {
        type_: type_.to_owned(),
        status: status.to_owned(),
        ..Default::default()
    }
}

fn job_with_conditions(conditions: Vec<JobCondition>) -> Job {
    Job {
        status: Some(ApiJobStatus {
            conditions: Some(conditions),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[test]
fn job_status_follows_condition_order() {
    // Listed lowest priority first, so each shorter prefix drops the winner before it.
    let types = ["FailureTarget", "Suspended", "Failed", "Complete"];
    let cases = [
        (4, JobStatus::Complete),
        (3, JobStatus::Failed),
        (2, JobStatus::Suspended),
        (1, JobStatus::Failing),
        (0, JobStatus::Running),
    ];
    for (count, expected) in cases {
        let conditions = types[..count]
            .iter()
            .map(|type_| api_condition(type_, "True"))
            .collect();
        assert_eq!(
            job_summary(&job_with_conditions(conditions)).status,
            expected
        );
    }
}

#[test]
fn job_status_ignores_false_conditions() {
    let job = job_with_conditions(vec![
        api_condition("Failed", "False"),
        api_condition("Complete", "Unknown"),
    ]);
    assert_eq!(job_summary(&job).status, JobStatus::Running);
    assert_eq!(job_summary(&Job::default()).status, JobStatus::Running);
}

#[test]
fn job_status_displays_kubectl_wording() {
    let texts: Vec<_> = [
        JobStatus::Running,
        JobStatus::Complete,
        JobStatus::Failed,
        JobStatus::Failing,
        JobStatus::Suspended,
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    assert_eq!(
        texts,
        ["Running", "Complete", "Failed", "Failing", "Suspended"]
    );
}

#[test]
fn job_finished_at_falls_back_to_failed_condition_time() {
    let completed = timestamp("2024-05-01T10:05:00Z");
    let failed_at = timestamp("2024-05-01T10:09:00Z");
    let mut failed = api_condition("Failed", "True");
    failed.last_transition_time = Some(Time(failed_at));

    let mut job = job_with_conditions(vec![failed]);
    assert_eq!(job_summary(&job).finished_at, Some(failed_at));

    if let Some(status) = job.status.as_mut() {
        status.completion_time = Some(Time(completed));
    }
    assert_eq!(job_summary(&job).finished_at, Some(completed));

    assert_eq!(job_summary(&Job::default()).finished_at, None);
}

#[test]
fn job_summary_reads_counts_and_spec() {
    let started = timestamp("2024-05-01T10:00:00Z");
    let job = Job {
        spec: Some(JobSpec {
            completions: Some(5),
            parallelism: Some(2),
            backoff_limit: Some(6),
            ..Default::default()
        }),
        status: Some(ApiJobStatus {
            active: Some(1),
            succeeded: Some(3),
            failed: Some(2),
            start_time: Some(Time(started)),
            ..Default::default()
        }),
        ..Default::default()
    };
    let summary = job_summary(&job);
    assert_eq!(
        (
            summary.completions,
            summary.parallelism,
            summary.backoff_limit
        ),
        (Some(5), Some(2), Some(6))
    );
    assert_eq!(
        (summary.active, summary.succeeded, summary.failed),
        (1, 3, 2)
    );
    assert_eq!(summary.started_at, Some(started));
}

#[test]
fn job_keeps_deadline_and_ttl() {
    let job = Job {
        spec: Some(JobSpec {
            active_deadline_seconds: Some(3_600),
            ttl_seconds_after_finished: Some(86_400),
            ..Default::default()
        }),
        ..Default::default()
    };
    let summary = job_summary(&job);
    assert_eq!(summary.active_deadline_seconds, Some(3_600));
    assert_eq!(summary.ttl_seconds_after_finished, Some(86_400));
    let bare = job_summary(&Job::default());
    assert_eq!(
        (
            bare.active_deadline_seconds,
            bare.ttl_seconds_after_finished
        ),
        (None, None)
    );
}
