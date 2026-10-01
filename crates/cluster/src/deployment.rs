use futures::Stream;
use k8s_openapi::api::apps::v1::Deployment;

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::pod_status::non_negative;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::{
    TemplateContainer, WorkloadCondition, condition, int_or_string_text, label_terms,
    optional_count, revision, selector_terms, template_containers,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeploymentSummary {
    pub namespace: String,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// `spec.replicas`, defaulting to 1 as the API server does.
    pub desired: u32,
    pub ready: u32,
    pub up_to_date: u32,
    pub available: u32,
    /// `spec.strategy.type`, or empty when absent.
    pub strategy: String,
    pub max_surge: Option<String>,
    pub max_unavailable: Option<String>,
    pub is_paused: bool,
    /// The `deployment.kubernetes.io/revision` annotation.
    pub revision: Option<String>,
    pub selector: Vec<String>,
    pub containers: Vec<TemplateContainer>,
    pub conditions: Vec<WorkloadCondition>,
}

impl ClusterConnection {
    /// Watches deployments in `scope`. Yields batched snapshots ordered by (namespace, name).
    pub fn watch_deployments(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<DeploymentSummary>> + Send + 'static {
        summary_watch(
            self,
            self.scoped_api(scope),
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
        revision: revision(&deployment.metadata),
        selector: spec.map_or_else(Vec::new, |spec| selector_terms(&spec.selector)),
        containers: spec.map_or_else(Vec::new, |spec| template_containers(&spec.template)),
        conditions: status
            .into_iter()
            .flat_map(|status| status.conditions.iter().flatten())
            .map(|item| condition(&item.type_, &item.status, item.reason.as_deref()))
            .collect(),
    }
}

#[cfg(test)]
#[path = "deployment_tests.rs"]
mod deployment_tests;
