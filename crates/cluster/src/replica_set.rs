use futures::future::Either;
use futures::{Stream, future, stream};
use k8s_openapi::api::apps::v1::ReplicaSet;
use kube::api::{Api, ListParams};
use kube::runtime::watcher;

use crate::connection::{ClusterConnection, ClusterError};
use crate::namespace::NamespaceScope;
use crate::object_yaml::{ObjectKind, ObjectRef};
use crate::pod_status::non_negative;
use crate::resource_watch::{WatchUpdate, selected_summary_watch, summary_watch};
use crate::workload::{
    AnnotationTerms, ControllerRef, TemplateContainer, annotation_terms, change_cause,
    controller_ref, label_terms, optional_count, revision, selector_terms, template_containers,
};

/// The action text of `deployment_revisions`.
const REVISIONS_ACTION: &str = "listing the revisions of a deployment";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaSetSummary {
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
    /// `status.replicas`.
    pub current: u32,
    pub ready: u32,
    pub owner: Option<ControllerRef>,
    /// The `deployment.kubernetes.io/revision` annotation.
    pub revision: Option<String>,
    /// The `kubernetes.io/change-cause` annotation: what `kubectl rollout history` lists.
    pub change_cause: Option<String>,
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
            self.scoped_apis(&scope),
            "watching replica sets",
            replica_set_summary,
        )
    }

    /// Watches the replica sets of `namespace` matching `selector` (kubectl selector syntax,
    /// for example `app=api,tier in (a,b)`), as one drawer-scoped watch. An empty selector,
    /// which would select every replica set, or one with an `<invalid>` term, yields one empty
    /// snapshot and ends.
    pub fn watch_selected_replica_sets(
        &self,
        namespace: &str,
        selector: &str,
    ) -> impl Stream<Item = WatchUpdate<ReplicaSetSummary>> + Send + 'static {
        // `<invalid>` is what `Selector::terms` prints for a requirement that never matches.
        if selector.is_empty() || selector.contains("<invalid>") {
            return Either::Left(stream::once(future::ready(WatchUpdate::Snapshot(
                Vec::new(),
            ))));
        }
        let scope = NamespaceScope::Named(namespace.to_owned());
        Either::Right(selected_summary_watch(
            self,
            self.scoped_apis(&scope),
            watcher::Config::default().labels(selector),
            "watching selected replica sets",
            replica_set_summary,
        ))
    }

    /// One LIST of `deployment`'s namespace's ReplicaSets with `labelSelector = selector` (kubectl
    /// syntax, as `watch_selected_replica_sets`), kept when their controller owner is that
    /// Deployment. An empty selector, or one with `<invalid>`, is `Ok(vec![])` with no request.
    /// Order is as returned. Read-only; `deployment` must be a Deployment.
    pub async fn deployment_revisions(
        &self,
        deployment: &ObjectRef,
        selector: &str,
    ) -> Result<Vec<ReplicaSetSummary>, ClusterError> {
        let (Some(ObjectKind::Deployment), Some(namespace)) =
            (deployment.builtin_kind(), deployment.namespace())
        else {
            return Err(ClusterError::UnexpectedResponse {
                context: self.context().to_owned(),
                action: REVISIONS_ACTION,
                source: "the object is not a deployment".into(),
            });
        };
        if selector.is_empty() || selector.contains("<invalid>") {
            return Ok(Vec::new());
        }
        let api = Api::<ReplicaSet>::namespaced(self.client().clone(), namespace);
        let list = self
            .run(
                REVISIONS_ACTION,
                api.list(&ListParams::default().labels(selector)),
            )
            .await?;
        Ok(list
            .items
            .iter()
            .map(replica_set_summary)
            .filter(|summary| {
                summary.owner.as_ref().is_some_and(|owner| {
                    owner.kind == "Deployment" && owner.name == deployment.name()
                })
            })
            .collect())
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
        annotations: annotation_terms(&replica_set.metadata),
        desired: spec.and_then(|spec| spec.replicas).map_or(1, non_negative),
        current: status.map_or(0, |status| non_negative(status.replicas)),
        ready: optional_count(status.and_then(|status| status.ready_replicas)),
        owner: controller_ref(&replica_set.metadata),
        revision: revision(&replica_set.metadata),
        change_cause: change_cause(&replica_set.metadata),
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
    fn replica_set_summary_reads_the_change_cause() {
        let with = |cause: Option<&str>| {
            let annotations = cause.map(|text| {
                BTreeMap::from([("kubernetes.io/change-cause".to_owned(), text.to_owned())])
            });
            replica_set_summary(&ReplicaSet {
                metadata: ObjectMeta {
                    annotations,
                    ..Default::default()
                },
                ..Default::default()
            })
        };
        assert_eq!(
            with(Some("release test")).change_cause.as_deref(),
            Some("release test")
        );
        // An empty annotation is no cause.
        assert_eq!(with(Some("")).change_cause, None);
        assert_eq!(with(None).change_cause, None);
    }

    #[test]
    fn replica_set_desired_defaults_to_one() {
        let summary = replica_set_summary(&ReplicaSet::default());
        assert_eq!((summary.desired, summary.current, summary.ready), (1, 0, 0));
        assert_eq!(summary.owner, None);
    }
}

#[cfg(test)]
#[path = "replica_set_tests.rs"]
mod replica_set_tests;
