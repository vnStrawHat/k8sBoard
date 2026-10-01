use futures::Stream;
use k8s_openapi::api::apps::v1::ReplicaSet;

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::pod_status::non_negative;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::{
    ControllerRef, TemplateContainer, controller_ref, label_terms, optional_count, revision,
    selector_terms, template_containers,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaSetSummary {
    pub namespace: String,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// `spec.replicas`, defaulting to 1 as the API server does.
    pub desired: u32,
    /// `status.replicas`.
    pub current: u32,
    pub ready: u32,
    pub owner: Option<ControllerRef>,
    /// The `deployment.kubernetes.io/revision` annotation.
    pub revision: Option<String>,
    pub selector: Vec<String>,
    pub containers: Vec<TemplateContainer>,
}

impl ClusterConnection {
    /// Watches replica sets in `scope`. Yields batched snapshots ordered by (namespace, name).
    pub fn watch_replica_sets(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<ReplicaSetSummary>> + Send + 'static {
        summary_watch(
            self,
            self.scoped_api(scope),
            "watching replica sets",
            replica_set_summary,
        )
    }
}

pub(crate) fn replica_set_summary(replica_set: &ReplicaSet) -> ReplicaSetSummary {
    let spec = replica_set.spec.as_ref();
    let status = replica_set.status.as_ref();
    ReplicaSetSummary {
        namespace: replica_set.metadata.namespace.clone().unwrap_or_default(),
        name: replica_set.metadata.name.clone().unwrap_or_default(),
        created_at: replica_set
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
        labels: label_terms(&replica_set.metadata),
        desired: spec.and_then(|spec| spec.replicas).map_or(1, non_negative),
        current: status.map_or(0, |status| non_negative(status.replicas)),
        ready: optional_count(status.and_then(|status| status.ready_replicas)),
        owner: controller_ref(&replica_set.metadata),
        revision: revision(&replica_set.metadata),
        selector: spec.map_or_else(Vec::new, |spec| selector_terms(&spec.selector)),
        containers: spec
            .and_then(|spec| spec.template.as_ref())
            .map_or_else(Vec::new, template_containers),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::api::apps::v1::{ReplicaSetSpec, ReplicaSetStatus};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};

    use super::*;

    #[test]
    fn replica_set_summary_reads_owner_and_counts() {
        let replica_set = ReplicaSet {
            metadata: ObjectMeta {
                annotations: Some(BTreeMap::from([(
                    "deployment.kubernetes.io/revision".to_owned(),
                    "4".to_owned(),
                )])),
                owner_references: Some(vec![OwnerReference {
                    kind: "Deployment".to_owned(),
                    name: "api".to_owned(),
                    controller: Some(true),
                    ..Default::default()
                }]),
                ..Default::default()
            },
            spec: Some(ReplicaSetSpec {
                replicas: Some(0),
                ..Default::default()
            }),
            status: Some(ReplicaSetStatus {
                replicas: 2,
                ready_replicas: Some(1),
                ..Default::default()
            }),
        };
        let summary = replica_set_summary(&replica_set);
        assert_eq!((summary.desired, summary.current, summary.ready), (0, 2, 1));
        assert_eq!(
            summary.owner,
            Some(ControllerRef {
                kind: "Deployment".to_owned(),
                name: "api".to_owned(),
            })
        );
        assert_eq!(summary.revision.as_deref(), Some("4"));
    }

    #[test]
    fn replica_set_desired_defaults_to_one() {
        let summary = replica_set_summary(&ReplicaSet::default());
        assert_eq!((summary.desired, summary.current, summary.ready), (1, 0, 0));
        assert_eq!(summary.owner, None);
    }
}
