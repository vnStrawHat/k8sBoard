use futures::Stream;
use k8s_openapi::api::batch::v1::CronJob;

use crate::connection::ClusterConnection;
use crate::cron_schedule::{CronSchedule, ScheduleError};
use crate::namespace::NamespaceScope;
use crate::pod_status::non_negative;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::{
    AnnotationTerms, TemplateContainer, annotation_terms, label_terms, non_empty,
    template_containers,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CronJobSummary {
    pub namespace: String,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// `key=value` terms in key order, for the drawer's folded Annotations section: see
    /// `annotation_terms` for what is cut and hidden.
    pub annotations: AnnotationTerms,
    pub schedule: String,
    /// `spec.timeZone`; empty is `None`.
    pub time_zone: Option<String>,
    /// `schedule` in `time_zone`; an `@every` schedule is anchored at `last_schedule_at`,
    /// else `created_at`. With `startingDeadlineSeconds` the controller may count from
    /// `now - startingDeadlineSeconds` instead, so an `@every` next run can differ from this
    /// anchor.
    pub timetable: Result<CronSchedule, ScheduleError>,
    pub is_suspended: bool,
    pub concurrency_policy: String,
    pub starting_deadline_seconds: Option<i64>,
    pub successful_history_limit: Option<u32>,
    pub failed_history_limit: Option<u32>,
    /// The names in `status.active`.
    pub active_jobs: Vec<String>,
    pub last_schedule_at: Option<jiff::Timestamp>,
    pub last_success_at: Option<jiff::Timestamp>,
    /// From `spec.jobTemplate.spec.template`.
    pub containers: Vec<TemplateContainer>,
}

impl ClusterConnection {
    /// Watches cron jobs in `scope`. Yields batched snapshots ordered by (namespace, name).
    pub fn watch_cron_jobs(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<CronJobSummary>> + Send + 'static {
        summary_watch(
            self,
            self.scoped_apis(&scope),
            "watching cron jobs",
            cron_job_summary,
        )
    }
}

pub(crate) fn cron_job_summary(cron_job: &CronJob) -> CronJobSummary {
    let spec = cron_job.spec.as_ref();
    let status = cron_job.status.as_ref();
    let job_spec = spec.and_then(|spec| spec.job_template.spec.as_ref());
    let created_at = cron_job
        .metadata
        .creation_timestamp
        .as_ref()
        .map(|time| time.0);
    let last_schedule_at = status
        .and_then(|status| status.last_schedule_time.as_ref())
        .map(|time| time.0);
    let schedule = spec.map(|spec| spec.schedule.clone()).unwrap_or_default();
    let time_zone = non_empty(spec.and_then(|spec| spec.time_zone.as_deref()));
    let timetable = CronSchedule::parse(&schedule, time_zone.as_deref())
        .map(|timetable| timetable.anchored_at(last_schedule_at.or(created_at)));
    CronJobSummary {
        namespace: cron_job.metadata.namespace.clone().unwrap_or_default(),
        name: cron_job.metadata.name.clone().unwrap_or_default(),
        created_at,
        labels: label_terms(&cron_job.metadata),
        annotations: annotation_terms(&cron_job.metadata),
        schedule,
        time_zone,
        timetable,
        is_suspended: spec.and_then(|spec| spec.suspend) == Some(true),
        concurrency_policy: spec
            .and_then(|spec| spec.concurrency_policy.clone())
            .unwrap_or_default(),
        starting_deadline_seconds: spec.and_then(|spec| spec.starting_deadline_seconds),
        successful_history_limit: spec
            .and_then(|spec| spec.successful_jobs_history_limit)
            .map(non_negative),
        failed_history_limit: spec
            .and_then(|spec| spec.failed_jobs_history_limit)
            .map(non_negative),
        active_jobs: status
            .into_iter()
            .flat_map(|status| status.active.iter().flatten())
            .filter_map(|job| job.name.clone())
            .collect(),
        last_schedule_at,
        last_success_at: status
            .and_then(|status| status.last_successful_time.as_ref())
            .map(|time| time.0),
        containers: job_spec.map_or_else(Vec::new, |job| template_containers(&job.template)),
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::batch::v1::{CronJobSpec, CronJobStatus, JobSpec, JobTemplateSpec};
    use k8s_openapi::api::core::v1::{Container, ObjectReference, PodSpec, PodTemplateSpec};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;

    use super::*;

    fn timestamp(text: &str) -> jiff::Timestamp {
        text.parse().expect("valid timestamp")
    }

    #[test]
    fn cron_job_summary_reads_schedule_and_history() {
        let last_schedule = timestamp("2024-05-01T10:00:00Z");
        let last_success = timestamp("2024-05-01T09:00:00Z");
        let cron_job = CronJob {
            spec: Some(CronJobSpec {
                schedule: "*/5 * * * *".to_owned(),
                time_zone: Some("Europe/Berlin".to_owned()),
                suspend: Some(true),
                concurrency_policy: Some("Forbid".to_owned()),
                starting_deadline_seconds: Some(120),
                successful_jobs_history_limit: Some(3),
                failed_jobs_history_limit: Some(1),
                job_template: JobTemplateSpec {
                    spec: Some(JobSpec {
                        template: PodTemplateSpec {
                            spec: Some(PodSpec {
                                containers: vec![Container {
                                    name: "report".to_owned(),
                                    image: Some("reports:1".to_owned()),
                                    ..Default::default()
                                }],
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            }),
            status: Some(CronJobStatus {
                active: Some(vec![
                    ObjectReference {
                        name: Some("report-2841".to_owned()),
                        ..Default::default()
                    },
                    ObjectReference::default(),
                ]),
                last_schedule_time: Some(Time(last_schedule)),
                last_successful_time: Some(Time(last_success)),
            }),
            ..Default::default()
        };
        let summary = cron_job_summary(&cron_job);
        assert_eq!(summary.schedule, "*/5 * * * *");
        assert_eq!(summary.time_zone.as_deref(), Some("Europe/Berlin"));
        assert!(summary.is_suspended);
        assert_eq!(summary.concurrency_policy, "Forbid");
        assert_eq!(summary.starting_deadline_seconds, Some(120));
        assert_eq!(
            (
                summary.successful_history_limit,
                summary.failed_history_limit
            ),
            (Some(3), Some(1))
        );
        assert_eq!(summary.active_jobs, ["report-2841"]);
        assert_eq!(summary.last_schedule_at, Some(last_schedule));
        assert_eq!(summary.last_success_at, Some(last_success));
        assert_eq!(summary.containers.len(), 1);
        assert_eq!(summary.containers[0].image, "reports:1");
    }

    fn cron_job_with_schedule(schedule: &str, time_zone: Option<&str>) -> CronJob {
        CronJob {
            spec: Some(CronJobSpec {
                schedule: schedule.to_owned(),
                time_zone: time_zone.map(str::to_owned),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn cron_job_summary_parses_timetable() {
        let summary =
            cron_job_summary(&cron_job_with_schedule("30 9 * * *", Some("Europe/Berlin")));
        let timetable = summary.timetable.expect("valid schedule");
        assert_eq!(timetable.zone_name(), Some("Europe/Berlin"));
        let next = timetable
            .next_after(timestamp("2024-05-01T00:00:00Z"))
            .expect("next run");
        assert_eq!(next.timestamp(), timestamp("2024-05-01T07:30:00Z"));
    }

    #[test]
    fn invalid_schedule_keeps_error() {
        let summary = cron_job_summary(&cron_job_with_schedule("61 * * * *", None));
        assert_eq!(summary.schedule, "61 * * * *");
        assert!(matches!(
            summary.timetable,
            Err(ScheduleError::Value {
                field: "minute",
                ..
            })
        ));
        let summary = cron_job_summary(&CronJob::default());
        assert_eq!(summary.timetable, Err(ScheduleError::FieldCount(0)));
    }

    #[test]
    fn every_schedule_is_anchored_at_last_schedule() {
        let created = timestamp("2024-05-01T00:15:00Z");
        let last_schedule = timestamp("2024-05-01T10:00:00Z");
        let mut cron_job = cron_job_with_schedule("@every 1h", None);
        cron_job.metadata.creation_timestamp = Some(Time(created));
        let after = timestamp("2024-05-01T10:30:00Z");
        let next_of = |cron_job: &CronJob| {
            let summary = cron_job_summary(cron_job);
            summary.timetable.expect("valid schedule").next_after(after)
        };
        assert_eq!(
            next_of(&cron_job).map(|next| next.timestamp()),
            Some(timestamp("2024-05-01T11:15:00Z"))
        );
        cron_job.status = Some(CronJobStatus {
            last_schedule_time: Some(Time(last_schedule)),
            ..Default::default()
        });
        cron_job.metadata.creation_timestamp = None;
        assert_eq!(
            next_of(&cron_job).map(|next| next.timestamp()),
            Some(timestamp("2024-05-01T11:00:00Z"))
        );
        cron_job.status = None;
        assert_eq!(next_of(&cron_job), None);
    }

    #[test]
    fn cron_job_without_spec_is_empty() {
        let summary = cron_job_summary(&CronJob::default());
        assert_eq!(summary.schedule, "");
        assert!(!summary.is_suspended);
        assert_eq!(summary.time_zone, None);
        assert!(summary.active_jobs.is_empty());
        assert!(summary.containers.is_empty());
    }
}
