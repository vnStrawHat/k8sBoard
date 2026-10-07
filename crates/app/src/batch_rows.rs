//! Row builders for the batch workloads: Jobs and CronJobs.

use cluster::{CronJobSummary, JobStatus, JobSummary};

use crate::kind_row::{
    DetailRow, DetailSection, JOB_KIND, KindCell, KindObject, KindRow, LiveContent, chips,
};
use crate::status_tone::{StatusLabel, StatusTone};
use crate::workload_rows::{
    condition_row, containers_section, controller_owner, images_cell, non_empty, optional_count,
    owner_row, toned_number,
};

pub(crate) fn job_row(job: &JobSummary) -> KindRow {
    let status = job_status_label(job.status);
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
            images_cell(&job.containers),
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
                    DetailRow::field(
                        "Active deadline",
                        KindCell::text_or_absent(
                            seconds_text(job.active_deadline_seconds).as_deref(),
                        ),
                    ),
                    DetailRow::field(
                        "TTL after finish",
                        KindCell::text_or_absent(
                            seconds_text(job.ttl_seconds_after_finished).as_deref(),
                        ),
                    ),
                    owner_row(&job.namespace, job.owner.as_ref()),
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
        object: KindObject::Job(job.clone()),
    }
}

/// `{n}s`, or `None` when the field is unset.
fn seconds_text(seconds: Option<impl std::fmt::Display>) -> Option<String> {
    seconds.map(|seconds| format!("{seconds}s"))
}

pub(crate) fn cron_job_row(cron_job: &CronJobSummary) -> KindRow {
    // The subtitle is built with the row, so it never claims a time-dependent state: Missed is
    // read at paint time by the Status cell and the diagnosis box.
    let status = cron_state(cron_job).label();
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
    let time_zone = cron_job
        .time_zone
        .as_deref()
        .unwrap_or("Cluster default (UTC assumed)");
    // A suspended CronJob runs nothing, and an invalid schedule has no next run to show.
    let next_run = match &cron_job.timetable {
        Ok(timetable) if !cron_job.is_suspended => KindCell::NextRun(timetable.clone()),
        Ok(_) | Err(_) => KindCell::Absent,
    };
    let starting_deadline = cron_job
        .starting_deadline_seconds
        .map(|seconds| format!("{seconds}s"));
    KindRow {
        namespace: Some(cron_job.namespace.clone()),
        name: cron_job.name.clone(),
        created_at: cron_job.created_at,
        status,
        cells: vec![
            KindCell::CronStatus(Box::new(cron_job.clone())),
            KindCell::Mono(cron_job.schedule.clone().into()),
            suspend.clone(),
            KindCell::count(cron_job.active_jobs.len()),
            last_schedule.clone(),
            next_run,
            images_cell(&cron_job.containers),
            KindCell::age(cron_job.created_at),
        ],
        sections: vec![
            DetailSection {
                title: "Next runs",
                rows: vec![DetailRow::Live(LiveContent::NextRuns)],
            },
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
                    DetailRow::field("Last run", last_schedule),
                    DetailRow::field("Last success", KindCell::age(cron_job.last_success_at)),
                    DetailRow::field(
                        "Active jobs",
                        KindCell::text_or_absent(active_jobs.as_deref()),
                    ),
                ],
            },
            DetailSection {
                title: "Recent jobs",
                rows: vec![DetailRow::Live(LiveContent::RecentJobs)],
            },
            containers_section(&cron_job.containers),
        ],
        event: None,
        related_pods: None,
        labels: chips(&cron_job.labels),
        object: KindObject::CronJob(cron_job.clone()),
    }
}

/// What a CronJob is doing, most important first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CronState {
    Suspended,
    Running,
    /// A scheduled run did not start before its starting deadline ran out. `expected_at` is the
    /// first run that was skipped.
    Missed {
        expected_at: jiff::Timestamp,
    },
    NeverRun,
    LastRunFailed,
    LastRunSucceeded,
}

impl CronState {
    pub(crate) fn label(self) -> StatusLabel {
        let (text, tone) = match self {
            Self::Suspended => ("Suspended", StatusTone::Done),
            Self::Running => ("Running", StatusTone::Info),
            Self::Missed { .. } => ("Missed schedule", StatusTone::Warn),
            Self::NeverRun => ("Not run yet", StatusTone::Info),
            Self::LastRunFailed => ("Last run failed", StatusTone::Warn),
            Self::LastRunSucceeded => ("Last run succeeded", StatusTone::Ok),
        };
        StatusLabel {
            text: text.into(),
            tone,
        }
    }
}

/// What the CronJob's own fields say, without a clock.
pub(crate) fn cron_state(cron_job: &CronJobSummary) -> CronState {
    if cron_job.is_suspended {
        return CronState::Suspended;
    }
    last_run(cron_job)
}

fn last_run(cron_job: &CronJobSummary) -> CronState {
    // A Job that succeeds sets `lastSuccessfulTime` after its schedule time, so a schedule newer
    // than the last success, with no active job, means that run did not succeed.
    let Some(scheduled_at) = cron_job.last_schedule_at else {
        return CronState::NeverRun;
    };
    if !cron_job.active_jobs.is_empty() {
        return CronState::Running;
    }
    match cron_job.last_success_at {
        Some(succeeded_at) if succeeded_at >= scheduled_at => CronState::LastRunSucceeded,
        Some(_) | None => CronState::LastRunFailed,
    }
}

/// The controller starts a Job at most `startingDeadlineSeconds` late, and 100 s when unset.
const DEFAULT_STARTING_DEADLINE_SECONDS: i64 = 100;

/// `cron_state` plus the one rule that needs the clock: the first run after the last schedule
/// (after creation when it never ran) is overdue past its starting deadline. An `@every` schedule
/// counts from the last run, so it has no calendar to miss, and a running Job may hold a new run
/// back under the concurrency policy.
pub(crate) fn cron_state_at(cron_job: &CronJobSummary, now: jiff::Timestamp) -> CronState {
    let state = cron_state(cron_job);
    if matches!(state, CronState::Suspended | CronState::Running) {
        return state;
    }
    let Ok(timetable) = &cron_job.timetable else {
        return state;
    };
    if timetable.is_every() {
        return state;
    }
    let Some(since) = cron_job.last_schedule_at.or(cron_job.created_at) else {
        return state;
    };
    let Some(expected) = timetable.next_after(since) else {
        return state;
    };
    let expected_at = expected.timestamp();
    let deadline = cron_job
        .starting_deadline_seconds
        .unwrap_or(DEFAULT_STARTING_DEADLINE_SECONDS)
        .max(0);
    if now.as_second() <= expected_at.as_second().saturating_add(deadline) {
        return state;
    }
    CronState::Missed { expected_at }
}

/// The first run due after the active Job started, once it is past: a CronJob with `concurrencyPolicy:
/// Forbid` skips it while that Job still runs. `None` unless a Forbid CronJob has an active Job and a
/// run came due since its schedule time, so a Job that is simply busy within its interval says nothing.
pub(crate) fn skipped_run(
    cron_job: &CronJobSummary,
    now: jiff::Timestamp,
) -> Option<jiff::Timestamp> {
    if cron_job.is_suspended
        || cron_job.concurrency_policy != FORBID
        || cron_job.active_jobs.is_empty()
    {
        return None;
    }
    let timetable = cron_job.timetable.as_ref().ok()?;
    let expected_at = timetable
        .next_after(cron_job.last_schedule_at?)?
        .timestamp();
    (expected_at <= now).then_some(expected_at)
}

/// The concurrency policy that skips a run while the last one is still active.
const FORBID: &str = "Forbid";

fn last_run_tone(cron_job: &CronJobSummary) -> Option<StatusTone> {
    match last_run(cron_job) {
        CronState::Suspended | CronState::NeverRun | CronState::Missed { .. } => None,
        CronState::Running => Some(StatusTone::Info),
        CronState::LastRunSucceeded => Some(StatusTone::Ok),
        // Warn, like the "Last run failed" subtitle: the CronJob itself is fine, its last run was not.
        CronState::LastRunFailed => Some(StatusTone::Warn),
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

pub(crate) fn job_status_label(status: JobStatus) -> StatusLabel {
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
