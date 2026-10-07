use std::fmt;

use futures::Stream;
use k8s_openapi::api::batch::v1::{Job, JobCondition};
use kube::Api;
use kube::runtime::watcher;

use crate::connection::{ClusterConnection, ClusterError};
use crate::namespace::NamespaceScope;
use crate::pod_status::non_negative;
use crate::resource_watch::{WatchUpdate, selected_summary_watch, summary_watch};
use crate::workload::{
    AnnotationTerms, ControllerRef, TemplateContainer, WorkloadCondition, annotation_terms,
    condition, controller_ref, label_terms, optional_count, template_containers,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobSummary {
    pub namespace: String,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// `key=value` terms in key order, for the drawer's folded Annotations section: see
    /// `annotation_terms` for what is cut and hidden.
    pub annotations: AnnotationTerms,
    pub status: JobStatus,
    pub completions: Option<u32>,
    pub parallelism: Option<u32>,
    pub succeeded: u32,
    pub failed: u32,
    pub active: u32,
    pub backoff_limit: Option<u32>,
    /// `spec.activeDeadlineSeconds`.
    pub active_deadline_seconds: Option<u64>,
    /// `spec.ttlSecondsAfterFinished`.
    pub ttl_seconds_after_finished: Option<u32>,
    pub started_at: Option<jiff::Timestamp>,
    /// `status.completionTime`, else the transition time of the true `Failed` condition,
    /// so a failed job's duration does not grow forever.
    pub finished_at: Option<jiff::Timestamp>,
    pub owner: Option<ControllerRef>,
    pub conditions: Vec<WorkloadCondition>,
    pub containers: Vec<TemplateContainer>,
}

/// The first true condition in the order `Failed`, `Complete`, `FailureTarget`,
/// `Suspended`; `Running` when none is true.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStatus {
    Running,
    Complete,
    Failed,
    /// `FailureTarget` is true: the job is being failed but has not finished.
    Failing,
    Suspended,
}

impl fmt::Display for JobStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Running => "Running",
            Self::Complete => "Complete",
            Self::Failed => "Failed",
            Self::Failing => "Failing",
            Self::Suspended => "Suspended",
        };
        formatter.write_str(text)
    }
}

impl ClusterConnection {
    /// Watches jobs in `scope`. Yields batched snapshots ordered by (namespace, name).
    pub fn watch_jobs(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<JobSummary>> + Send + 'static {
        summary_watch(self, self.scoped_apis(&scope), "watching jobs", job_summary)
    }

    /// One job by one GET: the app follows a job it started to its end without a list of jobs. A
    /// 404 is `ClusterError::Api { code: 404 }`.
    pub async fn job(&self, namespace: &str, name: &str) -> Result<JobSummary, ClusterError> {
        let api = Api::<Job>::namespaced(self.client().clone(), namespace);
        let job = self.run("reading a job", api.get(name)).await?;
        Ok(job_summary(&job))
    }

    /// Watches every job of `namespace` as one drawer-scoped watch; the caller keeps the
    /// jobs owned by its cron job.
    pub fn watch_namespace_jobs(
        &self,
        namespace: &str,
    ) -> impl Stream<Item = WatchUpdate<JobSummary>> + Send + 'static {
        let scope = NamespaceScope::Named(namespace.to_owned());
        selected_summary_watch(
            self,
            self.scoped_apis(&scope),
            watcher::Config::default(),
            "watching namespace jobs",
            job_summary,
        )
    }
}

pub(crate) fn job_summary(job: &Job) -> JobSummary {
    let spec = job.spec.as_ref();
    let status = job.status.as_ref();
    let api_conditions = status
        .and_then(|status| status.conditions.as_deref())
        .unwrap_or_default();
    JobSummary {
        namespace: job.metadata.namespace.clone().unwrap_or_default(),
        name: job.metadata.name.clone().unwrap_or_default(),
        created_at: job.metadata.creation_timestamp.as_ref().map(|time| time.0),
        labels: label_terms(&job.metadata),
        annotations: annotation_terms(&job.metadata),
        status: job_status(api_conditions),
        completions: spec.and_then(|spec| spec.completions).map(non_negative),
        parallelism: spec.and_then(|spec| spec.parallelism).map(non_negative),
        succeeded: optional_count(status.and_then(|status| status.succeeded)),
        failed: optional_count(status.and_then(|status| status.failed)),
        active: optional_count(status.and_then(|status| status.active)),
        backoff_limit: spec.and_then(|spec| spec.backoff_limit).map(non_negative),
        active_deadline_seconds: spec
            .and_then(|spec| spec.active_deadline_seconds)
            .and_then(|seconds| u64::try_from(seconds).ok()),
        ttl_seconds_after_finished: spec
            .and_then(|spec| spec.ttl_seconds_after_finished)
            .map(non_negative),
        started_at: status
            .and_then(|status| status.start_time.as_ref())
            .map(|time| time.0),
        finished_at: finished_at(job, api_conditions),
        owner: controller_ref(&job.metadata),
        conditions: api_conditions
            .iter()
            .map(|item| {
                condition(
                    &item.type_,
                    &item.status,
                    item.reason.as_deref(),
                    item.message.as_deref(),
                    item.last_transition_time.as_ref().map(|time| time.0),
                )
            })
            .collect(),
        containers: spec.map_or_else(Vec::new, |spec| template_containers(&spec.template)),
    }
}

/// Mirrors kubectl's `printJob` precedence; `Terminating` is not modelled.
fn job_status(conditions: &[JobCondition]) -> JobStatus {
    const BY_PRIORITY: [(&str, JobStatus); 4] = [
        ("Complete", JobStatus::Complete),
        ("Failed", JobStatus::Failed),
        ("Suspended", JobStatus::Suspended),
        ("FailureTarget", JobStatus::Failing),
    ];
    BY_PRIORITY
        .into_iter()
        .find(|(type_, _)| true_condition(conditions, type_).is_some())
        .map_or(JobStatus::Running, |(_, status)| status)
}

fn true_condition<'a>(conditions: &'a [JobCondition], type_: &str) -> Option<&'a JobCondition> {
    conditions
        .iter()
        .find(|item| item.type_ == type_ && item.status == "True")
}

fn finished_at(job: &Job, conditions: &[JobCondition]) -> Option<jiff::Timestamp> {
    if let Some(completed) = job
        .status
        .as_ref()
        .and_then(|status| status.completion_time.as_ref())
    {
        return Some(completed.0);
    }
    true_condition(conditions, "Failed")?
        .last_transition_time
        .as_ref()
        .map(|time| time.0)
}

#[cfg(test)]
#[path = "job_tests.rs"]
mod job_tests;
