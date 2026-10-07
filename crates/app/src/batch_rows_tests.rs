use cluster::{ControllerRef, CronJobSummary, JobStatus, JobSummary};

use super::*;
use crate::kind_row::{KindObject, LiveContent, PodOwner};
use crate::resource_kind::ResourceKind;
use crate::table_selection::ResourceKey;

fn job() -> JobSummary {
    JobSummary {
        annotations: cluster::AnnotationTerms::default(),
        namespace: "team-a".to_owned(),
        name: "migrate".to_owned(),
        created_at: None,
        labels: Vec::new(),
        status: JobStatus::Running,
        completions: Some(3),
        parallelism: Some(1),
        succeeded: 1,
        failed: 0,
        active: 1,
        backoff_limit: Some(6),
        active_deadline_seconds: None,
        ttl_seconds_after_finished: None,
        started_at: None,
        finished_at: None,
        owner: Some(ControllerRef {
            kind: "CronJob".to_owned(),
            name: "reconcile".to_owned(),
        }),
        conditions: Vec::new(),
        containers: Vec::new(),
    }
}

fn cron_job() -> CronJobSummary {
    CronJobSummary {
        annotations: cluster::AnnotationTerms::default(),
        namespace: "team-a".to_owned(),
        name: "reconcile".to_owned(),
        created_at: None,
        labels: Vec::new(),
        schedule: "*/5 * * * *".to_owned(),
        time_zone: None,
        timetable: cluster::CronSchedule::parse("*/5 * * * *", None),
        is_suspended: false,
        concurrency_policy: "Forbid".to_owned(),
        starting_deadline_seconds: Some(60),
        successful_history_limit: Some(3),
        failed_history_limit: Some(1),
        active_jobs: Vec::new(),
        last_schedule_at: None,
        last_success_at: None,
        containers: Vec::new(),
    }
}

fn at(seconds: i64) -> jiff::Timestamp {
    jiff::Timestamp::from_second(seconds).expect("valid timestamp")
}
#[test]
fn job_completions_follow_kubectl() {
    let completions = |completions, parallelism| {
        let mut job = job();
        job.completions = completions;
        job.parallelism = parallelism;
        job.succeeded = 2;
        job_row(&job).cells.get(1).cloned()
    };
    let text = |text: &str| Some(KindCell::Text(text.to_owned().into()));
    assert_eq!(completions(Some(5), Some(1)), text("2/5"));
    assert_eq!(completions(None, Some(3)), text("2/1 of 3"));
    assert_eq!(completions(None, Some(1)), text("2/1"));
    assert_eq!(completions(None, None), text("2/1"));
}

#[test]
fn job_status_tones() {
    let tone = |status| job_status_label(status).tone;
    assert_eq!(tone(JobStatus::Complete), StatusTone::Ok);
    assert_eq!(tone(JobStatus::Running), StatusTone::Info);
    assert_eq!(tone(JobStatus::Failed), StatusTone::Bad);
    assert_eq!(tone(JobStatus::Failing), StatusTone::Bad);
    assert_eq!(tone(JobStatus::Suspended), StatusTone::Done);
    assert_eq!(job_status_label(JobStatus::Failing).text, "Failing");
}

#[test]
fn job_duration_cell_keeps_start_and_finish() {
    let mut finished = job();
    finished.started_at = Some(at(100));
    finished.finished_at = Some(at(160));
    assert_eq!(
        job_row(&finished).cells.get(2),
        Some(&KindCell::Duration {
            started_at: Some(at(100)),
            finished_at: Some(at(160)),
        })
    );
}

#[test]
fn job_failed_count_is_bad_only_when_positive() {
    let failed_field = |failed| {
        let mut job = job();
        job.failed = failed;
        let row = job_row(&job);
        row.section("Status")
            .expect("status section")
            .rows
            .iter()
            .find_map(|row| match row {
                DetailRow::Field { label, value } if label == "Failed" => Some(value.clone()),
                _ => None,
            })
    };
    assert_eq!(failed_field(0), Some(KindCell::count(0)));
    assert!(
        matches!(failed_field(2), Some(KindCell::Toned(label)) if label.tone == StatusTone::Bad)
    );
}

#[test]
fn cron_last_run_outcomes() {
    let mut cron = cron_job();
    assert_eq!(last_run(&cron), CronState::NeverRun);
    assert_eq!(last_run_tone(&cron), None);

    cron.last_schedule_at = Some(at(100));
    assert_eq!(last_run(&cron), CronState::LastRunFailed);
    assert_eq!(last_run_tone(&cron), Some(StatusTone::Warn));

    cron.last_success_at = Some(at(100));
    assert_eq!(last_run(&cron), CronState::LastRunSucceeded);
    assert_eq!(last_run_tone(&cron), Some(StatusTone::Ok));

    cron.last_success_at = Some(at(50));
    assert_eq!(last_run(&cron), CronState::LastRunFailed);

    cron.active_jobs = vec!["reconcile-1".to_owned()];
    assert_eq!(last_run(&cron), CronState::Running);
    assert_eq!(last_run_tone(&cron), Some(StatusTone::Info));
}

#[test]
fn cron_last_schedule_cell_is_toned_by_outcome() {
    let mut cron = cron_job();
    cron.last_schedule_at = Some(at(100));
    cron.last_success_at = Some(at(120));
    assert_eq!(
        cron_job_row(&cron).cells.get(4),
        Some(&KindCell::Age {
            at: Some(at(100)),
            tone: Some(StatusTone::Ok),
        })
    );
}

#[test]
fn cron_drawer_defaults_time_zone_and_formats_history_limits() {
    let row = cron_job_row(&cron_job());
    let schedule = row.section("Schedule").expect("schedule section");
    assert!(schedule.rows.contains(&DetailRow::field(
        "Time zone",
        KindCell::Text("Cluster default (UTC assumed)".into())
    )));
    assert!(schedule.rows.contains(&DetailRow::field(
        "Starting deadline",
        KindCell::Text("60s".into())
    )));
    assert!(schedule.rows.contains(&DetailRow::field(
        "History limits",
        KindCell::Text("3 succeeded · 1 failed".into())
    )));
    assert_eq!(row.related_pods, None);
}

#[test]
fn batch_row_cells_match_column_count() {
    assert_eq!(
        job_row(&job()).cells.len(),
        ResourceKind::Jobs.columns().len()
    );
    assert_eq!(
        cron_job_row(&cron_job()).cells.len(),
        ResourceKind::CronJobs.columns().len()
    );
}

#[test]
fn job_related_pods_name_the_job_controller() {
    assert_eq!(
        job_row(&job()).related_pods,
        Some(PodOwner::Controller {
            namespace: "team-a".to_owned(),
            kind: JOB_KIND,
            name: "migrate".to_owned(),
        })
    );
}

#[test]
fn history_limits_name_only_the_limits_that_are_set() {
    let mut cron = cron_job();
    assert_eq!(
        history_limits(&cron),
        KindCell::Text("3 succeeded · 1 failed".into())
    );
    cron.failed_history_limit = None;
    assert_eq!(history_limits(&cron), KindCell::Text("3 succeeded".into()));
    cron.successful_history_limit = None;
    cron.failed_history_limit = Some(1);
    assert_eq!(history_limits(&cron), KindCell::Text("1 failed".into()));
    cron.failed_history_limit = None;
    assert_eq!(history_limits(&cron), KindCell::Absent);
}

#[test]
fn cron_failed_last_run_has_the_same_tone_in_cell_and_subtitle() {
    let mut cron = cron_job();
    cron.last_schedule_at = Some(at(100));
    let row = cron_job_row(&cron);
    assert_eq!(
        row.cells.get(4),
        Some(&KindCell::Age {
            at: Some(at(100)),
            tone: Some(StatusTone::Warn),
        })
    );
    assert_eq!(row.status.tone, StatusTone::Warn);
}

#[test]
fn cron_suspended_status_wins_over_last_run() {
    let mut cron = cron_job();
    cron.last_schedule_at = Some(at(100));
    cron.is_suspended = true;
    let status = cron_job_row(&cron).status;
    assert_eq!(status.text, "Suspended");
    assert_eq!(status.tone, StatusTone::Done);
}

#[test]
fn cron_suspend_cell_reads_yes_toned_or_plain_no() {
    let mut cron = cron_job();
    assert_eq!(
        cron_job_row(&cron).cells.get(2),
        Some(&KindCell::Text("No".into()))
    );
    cron.is_suspended = true;
    assert_eq!(
        cron_job_row(&cron).cells.get(2),
        Some(&KindCell::Toned(StatusLabel {
            text: "Yes".into(),
            tone: StatusTone::Done,
        }))
    );
}

#[test]
fn job_owner_is_a_link() {
    let row = job_row(&job());
    let status = row.section("Status").expect("status section");
    assert!(status.rows.contains(&DetailRow::Link {
        label: "Owner".into(),
        text: "cronjob/reconcile".into(),
        target: ResourceKey::Kind {
            kind: ResourceKind::CronJobs,
            namespace: Some("team-a".to_owned()),
            name: "reconcile".to_owned(),
        },
    }));
    // An owner kind without a screen stays text; no owner is a dash.
    let mut other = job();
    other.owner = Some(ControllerRef {
        kind: "Workflow".to_owned(),
        name: "nightly".to_owned(),
    });
    let status = job_row(&other).section("Status").cloned().expect("status");
    assert!(status.rows.contains(&DetailRow::field(
        "Owner",
        KindCell::Text("workflow/nightly".into())
    )));
    let mut orphan = job();
    orphan.owner = None;
    let status = job_row(&orphan).section("Status").cloned().expect("status");
    assert!(
        status
            .rows
            .contains(&DetailRow::field("Owner", KindCell::Absent))
    );
}

#[test]
fn job_status_shows_deadline_and_ttl() {
    let mut limited = job();
    limited.active_deadline_seconds = Some(3_600);
    limited.ttl_seconds_after_finished = Some(86_400);
    let row = job_row(&limited);
    let status = row.section("Status").expect("status section");
    assert!(status.rows.contains(&DetailRow::field(
        "Active deadline",
        KindCell::Text("3600s".into())
    )));
    assert!(status.rows.contains(&DetailRow::field(
        "TTL after finish",
        KindCell::Text("86400s".into())
    )));
    let row = job_row(&job());
    let status = row.section("Status").expect("status section");
    assert!(
        status
            .rows
            .contains(&DetailRow::field("Active deadline", KindCell::Absent))
    );
    assert!(
        status
            .rows
            .contains(&DetailRow::field("TTL after finish", KindCell::Absent))
    );
}

#[test]
fn cron_job_row_has_next_run_cell() {
    let row = cron_job_row(&cron_job());
    let columns = ResourceKind::CronJobs.columns();
    let next_run = columns
        .iter()
        .position(|column| column.name == "Next run")
        .expect("a Next run column");
    assert_eq!(next_run, 5);
    assert!(matches!(
        row.cells.get(next_run),
        Some(KindCell::NextRun(_))
    ));
    assert_eq!(columns[next_run].align, crate::resource_kind::Align::Right);
    assert!(matches!(row.object, KindObject::CronJob(_)));
}

#[test]
fn suspended_cron_job_has_no_next_run() {
    let mut suspended = cron_job();
    suspended.is_suspended = true;
    assert_eq!(
        cron_job_row(&suspended).cells.get(5),
        Some(&KindCell::Absent)
    );
    let mut invalid = cron_job();
    invalid.timetable = cluster::CronSchedule::parse("61 * * * *", None);
    assert_eq!(cron_job_row(&invalid).cells.get(5), Some(&KindCell::Absent));
}

#[test]
fn cron_job_sections_start_with_next_runs() {
    let row = cron_job_row(&cron_job());
    let titles: Vec<&str> = row.sections.iter().map(|section| section.title).collect();
    assert_eq!(
        titles,
        ["Next runs", "Schedule", "Runs", "Recent jobs", "Containers"]
    );
    assert_eq!(
        row.section("Next runs").map(|section| section.rows.clone()),
        Some(vec![DetailRow::Live(LiveContent::NextRuns)])
    );
    assert_eq!(
        row.section("Recent jobs")
            .map(|section| section.rows.clone()),
        Some(vec![DetailRow::Live(LiveContent::RecentJobs)])
    );
}

fn time(text: &str) -> jiff::Timestamp {
    text.parse().expect("valid timestamp")
}

/// Every five minutes, last started at 10:00, so the 10:05 run is the next expected one.
fn cron_ran_at_ten() -> CronJobSummary {
    let mut cron = cron_job();
    cron.last_schedule_at = Some(time("2024-10-04T10:00:00Z"));
    cron.last_success_at = cron.last_schedule_at;
    cron
}

#[test]
fn cron_run_is_missed_only_once_the_starting_deadline_has_passed() {
    let cron = cron_ran_at_ten();
    // The deadline is 60 s: the 10:05 run may still start until 10:06.
    assert_eq!(
        cron_state_at(&cron, time("2024-10-04T10:06:00Z")),
        CronState::LastRunSucceeded
    );
    assert_eq!(
        cron_state_at(&cron, time("2024-10-04T10:06:01Z")),
        CronState::Missed {
            expected_at: time("2024-10-04T10:05:00Z")
        }
    );
    // The label is the Warn state the cell and the box read.
    let label = cron_state_at(&cron, time("2024-10-04T10:30:00Z")).label();
    assert_eq!(label.text, "Missed schedule");
    assert_eq!(label.tone, StatusTone::Warn);
}

#[test]
fn cron_run_deadline_defaults_to_one_hundred_seconds() {
    let mut cron = cron_ran_at_ten();
    cron.starting_deadline_seconds = None;
    assert_eq!(
        cron_state_at(&cron, time("2024-10-04T10:06:40Z")),
        CronState::LastRunSucceeded
    );
    assert!(matches!(
        cron_state_at(&cron, time("2024-10-04T10:06:41Z")),
        CronState::Missed { .. }
    ));
}

#[test]
fn a_resume_meets_the_latest_run_that_came_due() {
    let cron = cron_ran_at_ten();
    // 10:05 and 10:10 came due; the latest, 10:10, is 30 s old and inside the 60 s deadline.
    assert_eq!(
        missed_run(&cron, time("2024-10-04T10:10:30Z")),
        Some(MissedRun {
            at: time("2024-10-04T10:10:00Z"),
            starts: true
        })
    );
    // 90 s old: the controller skips it.
    assert_eq!(
        missed_run(&cron, time("2024-10-04T10:11:30Z")),
        Some(MissedRun {
            at: time("2024-10-04T10:10:00Z"),
            starts: false
        })
    );
    // Nothing came due yet.
    assert_eq!(missed_run(&cron, time("2024-10-04T10:04:00Z")), None);
}

#[test]
fn a_run_without_a_starting_deadline_always_starts() {
    let mut cron = cron_ran_at_ten();
    cron.starting_deadline_seconds = None;
    let missed = missed_run(&cron, time("2024-10-04T12:00:00Z")).expect("a run came due");
    assert_eq!(missed.at, time("2024-10-04T12:00:00Z"));
    assert!(missed.starts);
}

#[test]
fn an_every_schedule_misses_no_run() {
    let mut cron = cron_ran_at_ten();
    cron.schedule = "@every 5m".to_owned();
    cron.timetable = cluster::CronSchedule::parse("@every 5m", None)
        .map(|timetable| timetable.anchored_at(cron.last_schedule_at));
    assert_eq!(missed_run(&cron, time("2024-10-04T12:00:00Z")), None);
}

#[test]
fn cron_never_run_counts_from_creation() {
    let mut cron = cron_job();
    cron.created_at = Some(time("2024-10-04T10:01:00Z"));
    assert_eq!(
        cron_state_at(&cron, time("2024-10-04T10:05:30Z")),
        CronState::NeverRun
    );
    assert_eq!(
        cron_state_at(&cron, time("2024-10-04T10:07:00Z")),
        CronState::Missed {
            expected_at: time("2024-10-04T10:05:00Z")
        }
    );
    // Without a creation time there is nothing to count from.
    cron.created_at = None;
    assert_eq!(
        cron_state_at(&cron, time("2024-10-04T10:07:00Z")),
        CronState::NeverRun
    );
}

#[test]
fn cron_suspended_running_and_every_are_never_missed() {
    let late = time("2024-10-04T12:00:00Z");
    let mut suspended = cron_ran_at_ten();
    suspended.is_suspended = true;
    assert_eq!(cron_state_at(&suspended, late), CronState::Suspended);
    let mut running = cron_ran_at_ten();
    running.active_jobs = vec!["reconcile-1".to_owned()];
    assert_eq!(cron_state_at(&running, late), CronState::Running);
    let mut every = cron_ran_at_ten();
    every.timetable = cluster::CronSchedule::parse("@every 5m", None);
    assert_eq!(cron_state_at(&every, late), CronState::LastRunSucceeded);
    let mut invalid = cron_ran_at_ten();
    invalid.timetable = cluster::CronSchedule::parse("not a schedule", None);
    assert_eq!(cron_state_at(&invalid, late), CronState::LastRunSucceeded);
}

#[test]
fn cron_failed_run_that_was_also_missed_reads_missed() {
    let mut cron = cron_ran_at_ten();
    cron.last_success_at = None;
    assert_eq!(
        cron_state_at(&cron, time("2024-10-04T10:05:30Z")),
        CronState::LastRunFailed
    );
    assert!(matches!(
        cron_state_at(&cron, time("2024-10-04T10:20:00Z")),
        CronState::Missed { .. }
    ));
}

#[test]
fn cron_status_cell_is_painted_from_the_whole_cron_job() {
    let cron = cron_ran_at_ten();
    assert_eq!(
        cron_job_row(&cron).cells.first(),
        Some(&KindCell::CronStatus(Box::new(cron)))
    );
}

#[test]
fn a_forbid_cron_job_skips_the_run_due_while_its_job_is_active() {
    let mut cron = cron_job();
    cron.active_jobs = vec!["reconcile-1".to_owned()];
    cron.last_schedule_at = Some(at(1_700_000_000));
    // `*/5`: the next run after the schedule time is five minutes (or less) later.
    let due = skipped_run(&cron, at(1_700_000_400)).expect("a skipped run");
    assert!(due.as_second() > 1_700_000_000 && due.as_second() <= 1_700_000_300);
    // Not yet due, no active job, another policy, or suspended: nothing is skipped.
    assert_eq!(skipped_run(&cron, at(1_700_000_001)), None);
    let mut idle = cron.clone();
    idle.active_jobs.clear();
    assert_eq!(skipped_run(&idle, at(1_700_000_400)), None);
    let mut allowing = cron.clone();
    allowing.concurrency_policy = "Allow".to_owned();
    assert_eq!(skipped_run(&allowing, at(1_700_000_400)), None);
    cron.is_suspended = true;
    assert_eq!(skipped_run(&cron, at(1_700_000_400)), None);
}
