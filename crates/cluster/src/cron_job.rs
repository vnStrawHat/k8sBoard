use futures::Stream;
use k8s_openapi::api::batch::v1::CronJob;

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::pod_status::non_negative;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::{TemplateContainer, label_terms, non_empty, template_containers};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CronJobSummary {
    pub namespace: String,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    pub schedule: String,
    /// `spec.timeZone`; empty is `None`.
    pub time_zone: Option<String>,
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
            self.scoped_api(scope),
            "watching cron jobs",
            cron_job_summary,
        )
    }
}

pub(crate) fn cron_job_summary(cron_job: &CronJob) -> CronJobSummary {
    let spec = cron_job.spec.as_ref();
    let status = cron_job.status.as_ref();
    let job_spec = spec.and_then(|spec| spec.job_template.spec.as_ref());
    CronJobSummary {
        namespace: cron_job.metadata.namespace.clone().unwrap_or_default(),
        name: cron_job.metadata.name.clone().unwrap_or_default(),
        created_at: cron_job
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
        labels: label_terms(&cron_job.metadata),
        schedule: spec.map(|spec| spec.schedule.clone()).unwrap_or_default(),
        time_zone: non_empty(spec.and_then(|spec| spec.time_zone.as_deref())),
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
        last_schedule_at: status
            .and_then(|status| status.last_schedule_time.as_ref())
            .map(|time| time.0),
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
