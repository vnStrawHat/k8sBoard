use futures::Stream;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::pod_status::non_negative;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::{
    AnnotationTerms, TemplateContainer, WorkloadCondition, annotation_terms, condition,
    int_or_string_text, label_terms, optional_count, revision, selector_terms, template_containers,
};

/// The API server default for `spec.progressDeadlineSeconds`.
const DEFAULT_PROGRESS_DEADLINE_SECONDS: u32 = 600;

/// The newest writer of the pod template, from `metadata.managedFields`: the field manager name
/// (what kubectl, Argo CD, Helm, or a CI tool set), not a user identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldWriter {
    pub manager: String,
    pub at: jiff::Timestamp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeploymentSummary {
    pub namespace: String,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// `key=value` terms in key order, for the drawer's folded Annotations section: see
    /// `annotation_terms` for what is cut and hidden.
    pub annotations: AnnotationTerms,
    /// `spec.replicas`, defaulting to 1 as the API server does.
    pub desired: u32,
    pub ready: u32,
    pub up_to_date: u32,
    pub available: u32,
    /// `spec.strategy.type`, or empty when absent.
    pub strategy: String,
    pub max_surge: Option<String>,
    pub max_unavailable: Option<String>,
    /// `spec.progressDeadlineSeconds`.
    pub progress_deadline_seconds: u32,
    pub is_paused: bool,
    /// `metadata.generation`: it grows with every change of the spec, a rollout's start included.
    pub generation: i64,
    /// `status.observedGeneration`: equals `generation` once the controller has seen the latest
    /// change.
    pub observed_generation: i64,
    /// The `deployment.kubernetes.io/revision` annotation.
    pub revision: Option<String>,
    pub selector: Vec<String>,
    pub containers: Vec<TemplateContainer>,
    pub conditions: Vec<WorkloadCondition>,
    /// The newest manager that wrote `spec.template`.
    pub template_change: Option<FieldWriter>,
}

impl ClusterConnection {
    /// Watches deployments in `scope`. Yields batched snapshots ordered by (namespace, name).
    pub fn watch_deployments(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<DeploymentSummary>> + Send + 'static {
        summary_watch(
            self,
            self.scoped_apis(&scope),
            "watching deployments",
            deployment_summary,
        )
    }
}

pub(crate) fn deployment_summary(deployment: &Deployment) -> DeploymentSummary {
    let spec = deployment.spec.as_ref();
    let status = deployment.status.as_ref();
    let strategy = spec.and_then(|spec| spec.strategy.as_ref());
    let rolling_update = strategy.and_then(|strategy| strategy.rolling_update.as_ref());
    DeploymentSummary {
        namespace: deployment.metadata.namespace.clone().unwrap_or_default(),
        name: deployment.metadata.name.clone().unwrap_or_default(),
        created_at: deployment
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
        labels: label_terms(&deployment.metadata),
        annotations: annotation_terms(&deployment.metadata),
        desired: spec.and_then(|spec| spec.replicas).map_or(1, non_negative),
        ready: optional_count(status.and_then(|status| status.ready_replicas)),
        up_to_date: optional_count(status.and_then(|status| status.updated_replicas)),
        available: optional_count(status.and_then(|status| status.available_replicas)),
        strategy: strategy
            .and_then(|strategy| strategy.type_.clone())
            .unwrap_or_default(),
        max_surge: rolling_update
            .and_then(|update| update.max_surge.as_ref())
            .map(int_or_string_text),
        max_unavailable: rolling_update
            .and_then(|update| update.max_unavailable.as_ref())
            .map(int_or_string_text),
        is_paused: spec.and_then(|spec| spec.paused) == Some(true),
        generation: deployment.metadata.generation.unwrap_or_default(),
        observed_generation: status
            .and_then(|status| status.observed_generation)
            .unwrap_or_default(),
        progress_deadline_seconds: spec
            .and_then(|spec| spec.progress_deadline_seconds)
            .map_or(DEFAULT_PROGRESS_DEADLINE_SECONDS, non_negative),
        revision: revision(&deployment.metadata),
        selector: spec.map_or_else(Vec::new, |spec| selector_terms(&spec.selector)),
        containers: spec.map_or_else(Vec::new, |spec| template_containers(&spec.template)),
        conditions: status
            .into_iter()
            .flat_map(|status| status.conditions.iter().flatten())
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
        template_change: template_writer(&deployment.metadata),
    }
}

/// Among the main-resource `Update` and `Apply` entries that own `f:spec` > `f:template`, the
/// newest by time (a tie goes to the later entry). Only the name and time are kept; the
/// `fieldsV1` tree is dropped here.
fn template_writer(metadata: &ObjectMeta) -> Option<FieldWriter> {
    metadata
        .managed_fields
        .iter()
        .flatten()
        .filter(|entry| entry.subresource.as_deref().is_none_or(str::is_empty))
        .filter(|entry| matches!(entry.operation.as_deref(), Some("Update" | "Apply")))
        .filter(|entry| {
            entry
                .fields_v1
                .as_ref()
                .is_some_and(|fields| fields.0.pointer("/f:spec/f:template").is_some())
        })
        .filter_map(|entry| {
            let manager = entry.manager.as_deref().filter(|name| !name.is_empty())?;
            Some(FieldWriter {
                manager: manager.to_owned(),
                at: entry.time.as_ref()?.0,
            })
        })
        .max_by_key(|writer| writer.at)
}

#[cfg(test)]
#[path = "deployment_tests.rs"]
mod deployment_tests;
