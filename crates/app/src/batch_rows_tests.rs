use cluster::{ControllerRef, CronJobSummary, JobStatus, JobSummary};

use super::*;
use crate::kind_row::{KindObject, LiveContent, PodOwner};
use crate::resource_kind::ResourceKind;
use crate::table_selection::ResourceKey;

fn job() -> JobSummary {
    JobSummary {
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
    assert_eq!(last_run(&cron), LastRun::NeverRun);
    assert_eq!(last_run_tone(&cron), None);

    cron.last_schedule_at = Some(at(100));
    assert_eq!(last_run(&cron), LastRun::Failed);
    assert_eq!(last_run_tone(&cron), Some(StatusTone::Warn));

    cron.last_success_at = Some(at(100));
    assert_eq!(last_run(&cron), LastRun::Succeeded);
    assert_eq!(last_run_tone(&cron), Some(StatusTone::Ok));

    cron.last_success_at = Some(at(50));
    assert_eq!(last_run(&cron), LastRun::Failed);

    cron.active_jobs = vec!["reconcile-1".to_owned()];
    assert_eq!(last_run(&cron), LastRun::Running);
    assert_eq!(last_run_tone(&cron), Some(StatusTone::Info));
}

#[test]
fn cron_last_schedule_cell_is_toned_by_outcome() {
    let mut cron = cron_job();
    cron.last_schedule_at = Some(at(100));
    cron.last_success_at = Some(at(120));
    assert_eq!(
        cron_job_row(&cron).cells.get(3),
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
        row.cells.get(3),
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
        cron_job_row(&cron).cells.get(1),
        Some(&KindCell::Text("No".into()))
    );
    cron.is_suspended = true;
    assert_eq!(
        cron_job_row(&cron).cells.get(1),
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
    assert_eq!(next_run, 4);
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
        cron_job_row(&suspended).cells.get(4),
        Some(&KindCell::Absent)
    );
    let mut invalid = cron_job();
    invalid.timetable = cluster::CronSchedule::parse("61 * * * *", None);
    assert_eq!(cron_job_row(&invalid).cells.get(4), Some(&KindCell::Absent));
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
