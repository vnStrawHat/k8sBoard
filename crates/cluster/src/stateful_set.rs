use futures::Stream;
use k8s_openapi::api::apps::v1::{StatefulSet, StatefulSetSpec};
use k8s_openapi::api::core::v1::PersistentVolumeClaim;

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::pod_status::non_negative;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::{
    TemplateContainer, label_terms, non_empty, optional_count, selector_terms, template_containers,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatefulSetSummary {
    pub namespace: String,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// `spec.replicas`, defaulting to 1 as the API server does.
    pub desired: u32,
    pub ready: u32,
    pub current: u32,
    pub updated: u32,
    /// `spec.serviceName`; empty is `None`.
    pub service_name: Option<String>,
    /// `spec.updateStrategy.type`, or empty when absent.
    pub update_strategy: String,
    pub pod_management_policy: String,
    pub selector: Vec<String>,
    pub containers: Vec<TemplateContainer>,
    pub claim_templates: Vec<ClaimTemplate>,
    /// `persistentVolumeClaimRetentionPolicy` as `whenDeleted Retain · whenScaled Delete`;
    /// `None` when unset.
    pub claim_retention: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimTemplate {
    pub name: String,
    /// `spec.resources.requests["storage"]` as quantity text.
    pub storage: Option<String>,
    pub storage_class: Option<String>,
    pub access_modes: Vec<String>,
}

impl ClusterConnection {
    /// Watches stateful sets in `scope`. Yields batched snapshots ordered by (namespace, name).
    pub fn watch_stateful_sets(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<StatefulSetSummary>> + Send + 'static {
        summary_watch(
            self,
            self.scoped_apis(&scope),
            "watching stateful sets",
            stateful_set_summary,
        )
    }
}

pub(crate) fn stateful_set_summary(stateful_set: &StatefulSet) -> StatefulSetSummary {
    let spec = stateful_set.spec.as_ref();
    let status = stateful_set.status.as_ref();
    StatefulSetSummary {
        namespace: stateful_set.metadata.namespace.clone().unwrap_or_default(),
        name: stateful_set.metadata.name.clone().unwrap_or_default(),
        created_at: stateful_set
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
        labels: label_terms(&stateful_set.metadata),
        desired: spec.and_then(|spec| spec.replicas).map_or(1, non_negative),
        ready: optional_count(status.and_then(|status| status.ready_replicas)),
        current: optional_count(status.and_then(|status| status.current_replicas)),
        updated: optional_count(status.and_then(|status| status.updated_replicas)),
        service_name: non_empty(spec.and_then(|spec| spec.service_name.as_deref())),
        update_strategy: spec
            .and_then(|spec| spec.update_strategy.as_ref())
            .and_then(|strategy| strategy.type_.clone())
            .unwrap_or_default(),
        pod_management_policy: spec
            .and_then(|spec| spec.pod_management_policy.clone())
            .unwrap_or_default(),
        selector: spec.map_or_else(Vec::new, |spec| selector_terms(&spec.selector)),
        containers: spec.map_or_else(Vec::new, |spec| template_containers(&spec.template)),
        claim_templates: spec
            .into_iter()
            .flat_map(|spec| spec.volume_claim_templates.iter().flatten())
            .map(claim_template)
            .collect(),
        claim_retention: spec.and_then(claim_retention),
    }
}

/// An unset field reads `Retain`, the API default.
fn claim_retention(spec: &StatefulSetSpec) -> Option<String> {
    let policy = spec.persistent_volume_claim_retention_policy.as_ref()?;
    let behavior =
        |value: &Option<String>| non_empty(value.as_deref()).unwrap_or_else(|| "Retain".to_owned());
    Some(format!(
        "whenDeleted {} · whenScaled {}",
        behavior(&policy.when_deleted),
        behavior(&policy.when_scaled)
    ))
}

fn claim_template(claim: &PersistentVolumeClaim) -> ClaimTemplate {
    let spec = claim.spec.as_ref();
    ClaimTemplate {
        name: claim.metadata.name.clone().unwrap_or_default(),
        storage: spec
            .and_then(|spec| spec.resources.as_ref())
            .and_then(|resources| resources.requests.as_ref())
            .and_then(|requests| requests.get("storage"))
            .map(|quantity| quantity.0.clone()),
        storage_class: non_empty(spec.and_then(|spec| spec.storage_class_name.as_deref())),
        access_modes: spec
            .into_iter()
            .flat_map(|spec| spec.access_modes.iter().flatten())
            .cloned()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::api::apps::v1::{
        StatefulSetPersistentVolumeClaimRetentionPolicy, StatefulSetStatus,
        StatefulSetUpdateStrategy,
    };
    use k8s_openapi::api::core::v1::{PersistentVolumeClaimSpec, VolumeResourceRequirements};
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    use super::*;

    fn claim(name: &str, storage: Option<&str>, class: &str) -> PersistentVolumeClaim {
        PersistentVolumeClaim {
            metadata: ObjectMeta {
                name: Some(name.to_owned()),
                ..Default::default()
            },
            spec: Some(PersistentVolumeClaimSpec {
                access_modes: Some(vec!["ReadWriteOnce".to_owned()]),
                storage_class_name: Some(class.to_owned()),
                resources: storage.map(|storage| VolumeResourceRequirements {
                    requests: Some(BTreeMap::from([(
                        "storage".to_owned(),
                        Quantity(storage.to_owned()),
                    )])),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn stateful_set_summary_reads_service_and_claim_templates() {
        let stateful_set = StatefulSet {
            spec: Some(StatefulSetSpec {
                replicas: Some(3),
                service_name: Some("db-headless".to_owned()),
                pod_management_policy: Some("OrderedReady".to_owned()),
                update_strategy: Some(StatefulSetUpdateStrategy {
                    type_: Some("RollingUpdate".to_owned()),
                    ..Default::default()
                }),
                volume_claim_templates: Some(vec![
                    claim("data", Some("10Gi"), "fast"),
                    claim("scratch", None, ""),
                ]),
                ..Default::default()
            }),
            status: Some(StatefulSetStatus {
                ready_replicas: Some(2),
                current_replicas: Some(3),
                updated_replicas: Some(1),
                replicas: 3,
                ..Default::default()
            }),
            ..Default::default()
        };
        let summary = stateful_set_summary(&stateful_set);
        assert_eq!(
            (
                summary.desired,
                summary.ready,
                summary.current,
                summary.updated
            ),
            (3, 2, 3, 1)
        );
        assert_eq!(summary.service_name.as_deref(), Some("db-headless"));
        assert_eq!(summary.update_strategy, "RollingUpdate");
        assert_eq!(summary.pod_management_policy, "OrderedReady");
        assert_eq!(
            summary.claim_templates,
            [
                ClaimTemplate {
                    name: "data".to_owned(),
                    storage: Some("10Gi".to_owned()),
                    storage_class: Some("fast".to_owned()),
                    access_modes: vec!["ReadWriteOnce".to_owned()],
                },
                ClaimTemplate {
                    name: "scratch".to_owned(),
                    storage: None,
                    storage_class: None,
                    access_modes: vec!["ReadWriteOnce".to_owned()],
                },
            ]
        );
    }

    #[test]
    fn stateful_set_empty_service_name_is_none() {
        let stateful_set = StatefulSet {
            spec: Some(StatefulSetSpec {
                service_name: Some(String::new()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let summary = stateful_set_summary(&stateful_set);
        assert_eq!(summary.service_name, None);
        assert_eq!(summary.desired, 1);
    }

    #[test]
    fn claim_retention_text() {
        let with_policy = |when_deleted: Option<&str>, when_scaled: Option<&str>| StatefulSet {
            spec: Some(StatefulSetSpec {
                persistent_volume_claim_retention_policy: Some(
                    StatefulSetPersistentVolumeClaimRetentionPolicy {
                        when_deleted: when_deleted.map(str::to_owned),
                        when_scaled: when_scaled.map(str::to_owned),
                    },
                ),
                ..Default::default()
            }),
            ..Default::default()
        };
        let retention =
            |stateful_set: &StatefulSet| stateful_set_summary(stateful_set).claim_retention;
        assert_eq!(
            retention(&with_policy(Some("Retain"), Some("Delete"))).as_deref(),
            Some("whenDeleted Retain \u{b7} whenScaled Delete")
        );
        assert_eq!(
            retention(&with_policy(None, Some("Delete"))).as_deref(),
            Some("whenDeleted Retain \u{b7} whenScaled Delete")
        );
        assert_eq!(retention(&StatefulSet::default()), None);
    }
}
