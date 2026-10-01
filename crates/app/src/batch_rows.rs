//! Row builders for the batch workloads: Jobs and CronJobs.

use cluster::{CronJobSummary, JobStatus, JobSummary};

use crate::kind_row::{DetailRow, DetailSection, JOB_KIND, KindCell, KindRow, chips};
use crate::status_tone::{StatusLabel, StatusTone};
use crate::workload_rows::{
    condition_row, containers_section, controller_owner, non_empty, optional_count, owner_text,
    toned_number,
};

pub(crate) fn job_row(job: &JobSummary) -> KindRow {
    let status = job_status_label(job.status);
    let owner = owner_text(job.owner.as_ref());
    let completions = KindCell::Text(completions_text(job).into());
    let duration = KindCell::Duration {
        started_at: job.started_at,
        finished_at: job.finished_at,
    };
    let failed = if job.failed > 0 {
        toned_number(job.failed, StatusTone::Bad)
    } else {
        KindCell::count(job.failed)
    };
    KindRow {
        namespace: Some(job.namespace.clone()),
        name: job.name.clone(),
        created_at: job.created_at,
        status: status.clone(),
        cells: vec![
            KindCell::Toned(status.clone()),
            completions.clone(),
            duration.clone(),
            KindCell::age(job.created_at),
        ],
        sections: vec![
            DetailSection {
                title: "Status",
                rows: vec![
                    DetailRow::field("Status", KindCell::Toned(status)),
                    DetailRow::field("Completions", completions),
                    DetailRow::field("Parallelism", optional_count(job.parallelism)),
                    DetailRow::field("Active", KindCell::count(job.active)),
                    DetailRow::field("Succeeded", KindCell::count(job.succeeded)),
                    DetailRow::field("Failed", failed),
                    DetailRow::field("Backoff limit", optional_count(job.backoff_limit)),
                    DetailRow::field("Started", KindCell::age(job.started_at)),
                    DetailRow::field("Duration", duration),
                    DetailRow::field("Owner", KindCell::text_or_absent(owner.as_deref())),
                ],
            },
            DetailSection {
                title: "Conditions",
                rows: job.conditions.iter().map(condition_row).collect(),
            },
            containers_section(&job.containers),
        ],
        event: None,
        related_pods: controller_owner(&job.namespace, JOB_KIND, &job.name),
        labels: chips(&job.labels),
    }
}

pub(crate) fn cron_job_row(cron_job: &CronJobSummary) -> KindRow {
    let (status_text, status_tone) = if cron_job.is_suspended {
        ("Suspended", StatusTone::Done)
    } else {
        match last_run(cron_job) {
            LastRun::Running => ("Running", StatusTone::Info),
            LastRun::Failed => ("Last run failed", StatusTone::Warn),
            LastRun::Succeeded => ("Last run succeeded", StatusTone::Ok),
            LastRun::NeverRun => ("Not run yet", StatusTone::Info),
        }
    };
    let last_schedule = KindCell::Age {
        at: cron_job.last_schedule_at,
        tone: last_run_tone(cron_job),
    };
    let suspend = if cron_job.is_suspended {
        KindCell::Toned(StatusLabel {
            text: "Yes".into(),
            tone: StatusTone::Done,
        })
    } else {
        KindCell::Text("No".into())
    };
    let active_jobs = (!cron_job.active_jobs.is_empty()).then(|| cron_job.active_jobs.join(", "));
    let time_zone = cron_job.time_zone.as_deref().unwrap_or("Cluster default");
    let starting_deadline = cron_job
        .starting_deadline_seconds
        .map(|seconds| format!("{seconds}s"));
    KindRow {
        namespace: Some(cron_job.namespace.clone()),
        name: cron_job.name.clone(),
        created_at: cron_job.created_at,
        status: StatusLabel {
            text: status_text.into(),
            tone: status_tone,
        },
        cells: vec![
            KindCell::Mono(cron_job.schedule.clone().into()),
            suspend.clone(),
            KindCell::count(cron_job.active_jobs.len()),
            last_schedule.clone(),
            KindCell::age(cron_job.created_at),
        ],
        sections: vec![
            DetailSection {
                title: "Schedule",
                rows: vec![
                    DetailRow::field("Schedule", KindCell::Mono(cron_job.schedule.clone().into())),
                    DetailRow::field("Time zone", KindCell::Text(time_zone.to_owned().into())),
                    DetailRow::field("Suspend", suspend),
                    DetailRow::field(
                        "Concurrency policy",
                        KindCell::text_or_absent(non_empty(&cron_job.concurrency_policy)),
                    ),
                    DetailRow::field(
                        "Starting deadline",
                        KindCell::text_or_absent(starting_deadline.as_deref()),
                    ),
                    DetailRow::field("History limits", history_limits(cron_job)),
                ],
            },
            DetailSection {
                title: "Runs",
                rows: vec![
                    DetailRow::field("Last schedule", last_schedule),
                    DetailRow::field("Last success", KindCell::age(cron_job.last_success_at)),
                    DetailRow::field(
                        "Active jobs",
                        KindCell::text_or_absent(active_jobs.as_deref()),
                    ),
                ],
            },
            containers_section(&cron_job.containers),
        ],
        event: None,
        related_pods: None,
        labels: chips(&cron_job.labels),
    }
}

/// What the last schedule of a CronJob came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LastRun {
    NeverRun,
    Running,
    Succeeded,
    Failed,
}

/// A Job that succeeds sets `lastSuccessfulTime` after its schedule time, so a schedule newer
/// than the last success, with no active job, means that run did not succeed.
fn last_run(cron_job: &CronJobSummary) -> LastRun {
    let Some(scheduled_at) = cron_job.last_schedule_at else {
        return LastRun::NeverRun;
    };
    if !cron_job.active_jobs.is_empty() {
        return LastRun::Running;
    }
    match cron_job.last_success_at {
        Some(succeeded_at) if succeeded_at >= scheduled_at => LastRun::Succeeded,
        Some(_) | None => LastRun::Failed,
    }
}

fn last_run_tone(cron_job: &CronJobSummary) -> Option<StatusTone> {
    match last_run(cron_job) {
        LastRun::NeverRun => None,
        LastRun::Running => Some(StatusTone::Info),
        LastRun::Succeeded => Some(StatusTone::Ok),
        // Warn, like the "Last run failed" subtitle: the CronJob itself is fine, its last run was not.
        LastRun::Failed => Some(StatusTone::Warn),
    }
}

/// `{successful} succeeded · {failed} failed`, naming only the limits the CronJob sets.
fn history_limits(cron_job: &CronJobSummary) -> KindCell {
    let successful = cron_job
        .successful_history_limit
        .map(|limit| format!("{limit} succeeded"));
    let failed = cron_job
        .failed_history_limit
        .map(|limit| format!("{limit} failed"));
    let parts: Vec<String> = successful.into_iter().chain(failed).collect();
    if parts.is_empty() {
        return KindCell::Absent;
    }
    KindCell::Text(parts.join(" · ").into())
}

fn job_status_label(status: JobStatus) -> StatusLabel {
    let tone = match status {
        JobStatus::Complete => StatusTone::Ok,
        JobStatus::Running => StatusTone::Info,
        JobStatus::Failed | JobStatus::Failing => StatusTone::Bad,
        JobStatus::Suspended => StatusTone::Done,
    };
    StatusLabel {
        text: status.to_string().into(),
        tone,
    }
}

/// kubectl's COMPLETIONS: `{succeeded}/{completions}`, and `{succeeded}/1 of {parallelism}` for
/// a work-queue job that sets no completion count.
fn completions_text(job: &JobSummary) -> String {
    match (job.completions, job.parallelism) {
        (Some(completions), _) => format!("{}/{completions}", job.succeeded),
        (None, Some(parallelism)) if parallelism > 1 => {
            format!("{}/1 of {parallelism}", job.succeeded)
        }
        (None, _) => format!("{}/1", job.succeeded),
    }
}

#[cfg(test)]
#[path = "batch_rows_tests.rs"]
mod batch_rows_tests;
