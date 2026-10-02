use std::fmt::Debug;

use k8s_openapi::NamespaceResourceScope;
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler;
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{
    ConfigMap, Event, Namespace, Node, PersistentVolume, PersistentVolumeClaim, Pod, ResourceQuota,
    Service,
};
use k8s_openapi::api::networking::v1::{Ingress, NetworkPolicy};
use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use k8s_openapi::api::storage::v1::StorageClass;
use k8s_openapi::serde::de::DeserializeOwned;
use kube::Api;
use kube::api::ListParams;

use crate::connection::{ClusterConnection, ClusterError};
use crate::namespace::NamespaceScope;
use crate::object_yaml::ObjectKind;

impl ClusterConnection {
    /// The object count of `kind` in `scope`: one `list` with `limit=1` per namespace (one
    /// for `All` and for cluster-scoped kinds), `items.len() + remainingItemCount`. `None`
    /// when a list has a continue token but no remaining count.
    pub async fn count_objects(
        &self,
        kind: ObjectKind,
        scope: &NamespaceScope,
    ) -> Result<Option<u64>, ClusterError> {
        match kind {
            ObjectKind::Pod => self.count_namespaced::<Pod>(scope, "counting pods").await,
            ObjectKind::Node => self.count_cluster::<Node>("counting nodes").await,
            ObjectKind::Namespace => self.count_cluster::<Namespace>("counting namespaces").await,
            ObjectKind::Event => {
                self.count_namespaced::<Event>(scope, "counting events")
                    .await
            }
            ObjectKind::Deployment => {
                self.count_namespaced::<Deployment>(scope, "counting deployments")
                    .await
            }
            ObjectKind::StatefulSet => {
                self.count_namespaced::<StatefulSet>(scope, "counting stateful sets")
                    .await
            }
            ObjectKind::DaemonSet => {
                self.count_namespaced::<DaemonSet>(scope, "counting daemon sets")
                    .await
            }
            ObjectKind::ReplicaSet => {
                self.count_namespaced::<ReplicaSet>(scope, "counting replica sets")
                    .await
            }
            ObjectKind::Job => self.count_namespaced::<Job>(scope, "counting jobs").await,
            ObjectKind::CronJob => {
                self.count_namespaced::<CronJob>(scope, "counting cron jobs")
                    .await
            }
            ObjectKind::Service => {
                self.count_namespaced::<Service>(scope, "counting services")
                    .await
            }
            ObjectKind::Ingress => {
                self.count_namespaced::<Ingress>(scope, "counting ingresses")
                    .await
            }
            ObjectKind::ConfigMap => {
                self.count_namespaced::<ConfigMap>(scope, "counting config maps")
                    .await
            }
            ObjectKind::NetworkPolicy => {
                self.count_namespaced::<NetworkPolicy>(scope, "counting network policies")
                    .await
            }
            ObjectKind::HorizontalPodAutoscaler => {
                self.count_namespaced::<HorizontalPodAutoscaler>(
                    scope,
                    "counting horizontal pod autoscalers",
                )
                .await
            }
            ObjectKind::ResourceQuota => {
                self.count_namespaced::<ResourceQuota>(scope, "counting resource quotas")
                    .await
            }
            ObjectKind::PodDisruptionBudget => {
                self.count_namespaced::<PodDisruptionBudget>(
                    scope,
                    "counting pod disruption budgets",
                )
                .await
            }
            ObjectKind::PersistentVolumeClaim => {
                self.count_namespaced::<PersistentVolumeClaim>(
                    scope,
                    "counting persistent volume claims",
                )
                .await
            }
            ObjectKind::PersistentVolume => {
                self.count_cluster::<PersistentVolume>("counting persistent volumes")
                    .await
            }
            ObjectKind::StorageClass => {
                self.count_cluster::<StorageClass>("counting storage classes")
                    .await
            }
        }
    }

    /// One request per namespace, one after another; any unknown count makes the total
    /// unknown.
    async fn count_namespaced<K>(
        &self,
        scope: &NamespaceScope,
        action: &'static str,
    ) -> Result<Option<u64>, ClusterError>
    where
        K: kube::Resource<Scope = NamespaceResourceScope> + Clone + DeserializeOwned + Debug,
        K::DynamicType: Default,
    {
        let mut total = 0_u64;
        for (_, api) in self.scoped_apis::<K>(scope) {
            let Some(count) = self.count_one(&api, action).await? else {
                return Ok(None);
            };
            total += count;
        }
        Ok(Some(total))
    }

    async fn count_cluster<K>(&self, action: &'static str) -> Result<Option<u64>, ClusterError>
    where
        K: kube::Resource + Clone + DeserializeOwned + Debug,
        K::DynamicType: Default,
    {
        let api = Api::<K>::all(self.client().clone());
        self.count_one(&api, action).await
    }

    async fn count_one<K>(
        &self,
        api: &Api<K>,
        action: &'static str,
    ) -> Result<Option<u64>, ClusterError>
    where
        K: kube::Resource + Clone + DeserializeOwned + Debug,
    {
        let list = self.run(action, api.list_metadata(&count_params())).await?;
        let has_continue = list
            .metadata
            .continue_
            .as_deref()
            .is_some_and(|token| !token.is_empty());
        Ok(count_of(
            list.items.len(),
            list.metadata.remaining_item_count,
            has_continue,
        ))
    }
}

/// `resource_version` stays `None` on purpose: `resourceVersion=0` is answered from the
/// watch cache, which ignores `limit` and omits `remainingItemCount`.
fn count_params() -> ListParams {
    ListParams::default().limit(1)
}

/// The total behind one limited page. A continue token without a remaining count (a server
/// that does not report it) leaves the total unknown.
fn count_of(items: usize, remaining: Option<i64>, has_continue: bool) -> Option<u64> {
    let items = u64::try_from(items).ok()?;
    match remaining {
        Some(remaining) => Some(items + u64::try_from(remaining).unwrap_or(0)),
        None if has_continue => None,
        None => Some(items),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_adds_remaining_items() {
        assert_eq!(count_of(1, Some(41), true), Some(42));
        let params = count_params();
        assert_eq!(params.limit, Some(1));
        assert_eq!(params.resource_version, None);
    }

    #[test]
    fn count_without_continue_is_item_count() {
        assert_eq!(count_of(1, None, false), Some(1));
        assert_eq!(count_of(0, None, false), Some(0));
    }

    #[test]
    fn count_with_continue_but_no_remaining_is_unknown() {
        assert_eq!(count_of(1, None, true), None);
    }
}
