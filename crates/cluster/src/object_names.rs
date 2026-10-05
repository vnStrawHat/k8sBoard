//! The names of every object of a kind, for the command palette's name search (spec 0056). Only
//! namespace and name are read out of the response: the LIST is metadata-only, and nothing else of
//! an object is kept, logged, or traced.

use std::fmt::Debug;

use k8s_openapi::NamespaceResourceScope;
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler;
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{
    ConfigMap, Event, Namespace, Node, PersistentVolume, PersistentVolumeClaim, Pod, ResourceQuota,
    Secret, Service, ServiceAccount,
};
use k8s_openapi::api::networking::v1::{Ingress, NetworkPolicy};
use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use k8s_openapi::api::rbac::v1::{ClusterRole, ClusterRoleBinding, Role, RoleBinding};
use k8s_openapi::api::storage::v1::StorageClass;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use k8s_openapi::serde::de::DeserializeOwned;
use kube::Api;
use kube::api::ListParams;

use crate::connection::{ClusterConnection, ClusterError, LIST_PAGE_SIZE};
use crate::namespace::NamespaceScope;
use crate::object_yaml::ObjectKind;

/// The most names kept per kind; a kind with more is cut here and reported as truncated.
const NAME_LIMIT: usize = 5_000;

const ACTION: &str = "listing object names";

/// One object's identity: `namespace` is `None` for a cluster-scoped kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectName {
    pub namespace: Option<String>,
    pub name: String,
}

/// The names of one kind in a scope.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NameList {
    pub names: Vec<ObjectName>,
    /// More objects exist than `NAME_LIMIT`: paging stopped at the limit.
    pub is_truncated: bool,
}

impl ClusterConnection {
    /// Metadata-only LIST in pages of 500, stopped at 5,000 names per kind: one list per namespace
    /// of a `Several` scope, one for `All`, one for a cluster-scoped kind. GET only.
    pub async fn list_object_names(
        &self,
        kind: ObjectKind,
        scope: &NamespaceScope,
    ) -> Result<NameList, ClusterError> {
        match kind {
            ObjectKind::Pod => self.names_namespaced::<Pod>(scope).await,
            ObjectKind::Node => self.names_cluster::<Node>().await,
            ObjectKind::Namespace => self.names_cluster::<Namespace>().await,
            ObjectKind::Event => self.names_namespaced::<Event>(scope).await,
            ObjectKind::Deployment => self.names_namespaced::<Deployment>(scope).await,
            ObjectKind::StatefulSet => self.names_namespaced::<StatefulSet>(scope).await,
            ObjectKind::DaemonSet => self.names_namespaced::<DaemonSet>(scope).await,
            ObjectKind::ReplicaSet => self.names_namespaced::<ReplicaSet>(scope).await,
            ObjectKind::Job => self.names_namespaced::<Job>(scope).await,
            ObjectKind::CronJob => self.names_namespaced::<CronJob>(scope).await,
            ObjectKind::Service => self.names_namespaced::<Service>(scope).await,
            ObjectKind::Ingress => self.names_namespaced::<Ingress>(scope).await,
            ObjectKind::ConfigMap => self.names_namespaced::<ConfigMap>(scope).await,
            ObjectKind::NetworkPolicy => self.names_namespaced::<NetworkPolicy>(scope).await,
            ObjectKind::HorizontalPodAutoscaler => {
                self.names_namespaced::<HorizontalPodAutoscaler>(scope)
                    .await
            }
            ObjectKind::ResourceQuota => self.names_namespaced::<ResourceQuota>(scope).await,
            ObjectKind::PodDisruptionBudget => {
                self.names_namespaced::<PodDisruptionBudget>(scope).await
            }
            ObjectKind::PersistentVolumeClaim => {
                self.names_namespaced::<PersistentVolumeClaim>(scope).await
            }
            ObjectKind::PersistentVolume => self.names_cluster::<PersistentVolume>().await,
            ObjectKind::StorageClass => self.names_cluster::<StorageClass>().await,
            ObjectKind::ServiceAccount => self.names_namespaced::<ServiceAccount>(scope).await,
            ObjectKind::Secret => self.names_namespaced::<Secret>(scope).await,
            ObjectKind::Role => self.names_namespaced::<Role>(scope).await,
            ObjectKind::ClusterRole => self.names_cluster::<ClusterRole>().await,
            ObjectKind::RoleBinding => self.names_namespaced::<RoleBinding>(scope).await,
            ObjectKind::ClusterRoleBinding => self.names_cluster::<ClusterRoleBinding>().await,
            ObjectKind::CustomResourceDefinition => {
                self.names_cluster::<CustomResourceDefinition>().await
            }
        }
    }

    async fn names_namespaced<K>(&self, scope: &NamespaceScope) -> Result<NameList, ClusterError>
    where
        K: kube::Resource<Scope = NamespaceResourceScope> + Clone + DeserializeOwned + Debug,
        K::DynamicType: Default,
    {
        let apis = self
            .scoped_apis::<K>(scope)
            .into_iter()
            .map(|(_, api)| api)
            .collect();
        self.collect_names(apis).await
    }

    async fn names_cluster<K>(&self) -> Result<NameList, ClusterError>
    where
        K: kube::Resource + Clone + DeserializeOwned + Debug,
        K::DynamicType: Default,
    {
        self.collect_names(vec![Api::<K>::all(self.client().clone())])
            .await
    }

    /// Pages through each API in turn. `resource_version` stays `None` on purpose (as in
    /// `count_params`): `resourceVersion=0` is answered from the watch cache, which ignores `limit`.
    /// A 410 on a later page is not retried: the caller runs the list again on its next turn.
    async fn collect_names<K>(&self, apis: Vec<Api<K>>) -> Result<NameList, ClusterError>
    where
        K: Clone + DeserializeOwned + Debug,
    {
        let mut list = NameList::default();
        let api_count = apis.len();
        for (index, api) in apis.iter().enumerate() {
            let mut continue_token: Option<String> = None;
            loop {
                let mut params = ListParams::default().limit(LIST_PAGE_SIZE);
                if let Some(token) = &continue_token {
                    params = params.continue_token(token);
                }
                let page = self.run(ACTION, api.list_metadata(&params)).await?;
                list.names.extend(page.items.into_iter().filter_map(|item| {
                    Some(ObjectName {
                        namespace: item.metadata.namespace,
                        name: item.metadata.name?,
                    })
                }));
                continue_token = page.metadata.continue_.filter(|token| !token.is_empty());
                if list.names.len() >= NAME_LIMIT {
                    // Reached exactly at the end of a page, a further page or API may be empty;
                    // the list says "first 5,000", which stays true.
                    let has_more = list.names.len() > NAME_LIMIT
                        || continue_token.is_some()
                        || index + 1 < api_count;
                    list.names.truncate(NAME_LIMIT);
                    list.is_truncated = has_more;
                    return Ok(list);
                }
                if continue_token.is_none() {
                    break;
                }
            }
        }
        Ok(list)
    }
}

#[cfg(test)]
#[path = "object_names_tests.rs"]
mod object_names_tests;
