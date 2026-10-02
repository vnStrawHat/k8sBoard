use futures::Stream;
use k8s_openapi::api::core::v1::PersistentVolumeClaim;

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::{WorkloadCondition, condition, label_terms, non_empty};

const STORAGE: &str = "storage";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersistentVolumeClaimSummary {
    pub namespace: String,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// The API text (`Bound`, `Pending`, `Lost`); `Pending` while the status is absent.
    pub phase: String,
    pub is_terminating: bool,
    /// `spec.volumeName`: the bound PV.
    pub volume: Option<String>,
    /// `status.capacity["storage"]` as written.
    pub capacity: Option<String>,
    /// `spec.resources.requests["storage"]` as written.
    pub requested: Option<String>,
    /// API names (`ReadWriteOnce`): the status modes, else the spec modes.
    pub access_modes: Vec<String>,
    pub storage_class: Option<String>,
    pub volume_mode: Option<String>,
    pub conditions: Vec<WorkloadCondition>,
}

impl ClusterConnection {
    /// Watches persistent volume claims in `scope`. Yields batched snapshots ordered by
    /// (namespace, name).
    pub fn watch_persistent_volume_claims(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<PersistentVolumeClaimSummary>> + Send + 'static {
        summary_watch(
            self,
            self.scoped_apis(&scope),
            "watching persistent volume claims",
            persistent_volume_claim_summary,
        )
    }
}

pub(crate) fn persistent_volume_claim_summary(
    claim: &PersistentVolumeClaim,
) -> PersistentVolumeClaimSummary {
    let spec = claim.spec.as_ref();
    let status = claim.status.as_ref();
    let status_modes = status
        .and_then(|status| status.access_modes.clone())
        .filter(|modes| !modes.is_empty());
    PersistentVolumeClaimSummary {
        namespace: claim.metadata.namespace.clone().unwrap_or_default(),
        name: claim.metadata.name.clone().unwrap_or_default(),
        created_at: claim
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
        labels: label_terms(&claim.metadata),
        phase: non_empty(status.and_then(|status| status.phase.as_deref()))
            .unwrap_or_else(|| "Pending".to_owned()),
        is_terminating: claim.metadata.deletion_timestamp.is_some(),
        volume: non_empty(spec.and_then(|spec| spec.volume_name.as_deref())),
        capacity: status
            .and_then(|status| status.capacity.as_ref())
            .and_then(|capacity| capacity.get(STORAGE))
            .map(|quantity| quantity.0.clone()),
        requested: spec
            .and_then(|spec| spec.resources.as_ref())
            .and_then(|resources| resources.requests.as_ref())
            .and_then(|requests| requests.get(STORAGE))
            .map(|quantity| quantity.0.clone()),
        access_modes: status_modes
            .or_else(|| spec.and_then(|spec| spec.access_modes.clone()))
            .unwrap_or_default(),
        storage_class: non_empty(spec.and_then(|spec| spec.storage_class_name.as_deref())),
        volume_mode: non_empty(spec.and_then(|spec| spec.volume_mode.as_deref())),
        conditions: status
            .into_iter()
            .flat_map(|status| status.conditions.iter().flatten())
            .map(|item| {
                condition(
                    &item.type_,
                    &item.status,
                    item.reason.as_deref(),
                    item.message.as_deref(),
                )
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::api::core::v1::{
        PersistentVolumeClaimCondition, PersistentVolumeClaimSpec, PersistentVolumeClaimStatus,
        VolumeResourceRequirements,
    };
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};

    use super::*;

    fn storage(text: &str) -> BTreeMap<String, Quantity> {
        BTreeMap::from([(STORAGE.to_owned(), Quantity(text.to_owned()))])
    }

    #[test]
    fn pvc_summary_reads_status_and_spec() {
        let summary = persistent_volume_claim_summary(&PersistentVolumeClaim {
            metadata: ObjectMeta {
                name: Some("data".to_owned()),
                namespace: Some("shop".to_owned()),
                ..Default::default()
            },
            spec: Some(PersistentVolumeClaimSpec {
                volume_name: Some("pv-1".to_owned()),
                storage_class_name: Some("gp3".to_owned()),
                volume_mode: Some("Filesystem".to_owned()),
                resources: Some(VolumeResourceRequirements {
                    requests: Some(storage("10Gi")),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            status: Some(PersistentVolumeClaimStatus {
                phase: Some("Bound".to_owned()),
                capacity: Some(storage("12Gi")),
                conditions: Some(vec![PersistentVolumeClaimCondition {
                    type_: "Resizing".to_owned(),
                    status: "True".to_owned(),
                    message: Some("working".to_owned()),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
        });
        assert_eq!(summary.namespace, "shop");
        assert_eq!(summary.name, "data");
        assert_eq!(summary.phase, "Bound");
        assert_eq!(summary.volume.as_deref(), Some("pv-1"));
        assert_eq!(summary.capacity.as_deref(), Some("12Gi"));
        assert_eq!(summary.requested.as_deref(), Some("10Gi"));
        assert_eq!(summary.storage_class.as_deref(), Some("gp3"));
        assert_eq!(summary.volume_mode.as_deref(), Some("Filesystem"));
        let [item] = summary.conditions.as_slice() else {
            panic!("one condition");
        };
        assert_eq!(item.name, "Resizing");
        assert!(item.is_true);
        assert_eq!(item.message.as_deref(), Some("working"));
    }

    #[test]
    fn pvc_phase_defaults_to_pending() {
        let summary = persistent_volume_claim_summary(&PersistentVolumeClaim::default());
        assert_eq!(summary.phase, "Pending");
        assert_eq!(summary.volume, None);
        assert_eq!(summary.capacity, None);
        assert!(summary.access_modes.is_empty());
    }

    #[test]
    fn pvc_access_modes_prefer_status() {
        let modes = |names: &[&str]| Some(names.iter().map(|&name| name.to_owned()).collect());
        let claim = |status: Option<Vec<String>>| PersistentVolumeClaim {
            spec: Some(PersistentVolumeClaimSpec {
                access_modes: modes(&["ReadWriteMany"]),
                ..Default::default()
            }),
            status: Some(PersistentVolumeClaimStatus {
                access_modes: status,
                ..Default::default()
            }),
            ..Default::default()
        };
        let bound = persistent_volume_claim_summary(&claim(modes(&["ReadWriteOnce"])));
        assert_eq!(bound.access_modes, ["ReadWriteOnce"]);
        let pending = persistent_volume_claim_summary(&claim(None));
        assert_eq!(pending.access_modes, ["ReadWriteMany"]);
    }

    #[test]
    fn pvc_terminating_from_deletion_timestamp() {
        let mut claim = PersistentVolumeClaim::default();
        assert!(!persistent_volume_claim_summary(&claim).is_terminating);
        claim.metadata.deletion_timestamp = Some(Time(jiff::Timestamp::UNIX_EPOCH));
        assert!(persistent_volume_claim_summary(&claim).is_terminating);
    }
}
