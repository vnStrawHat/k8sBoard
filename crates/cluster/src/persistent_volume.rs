use futures::Stream;
use k8s_openapi::api::core::v1::{
    NodeSelectorRequirement, NodeSelectorTerm, PersistentVolume, PersistentVolumeSpec,
};
use kube::Api;

use crate::connection::ClusterConnection;
use crate::event::optional_message;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::{label_terms, non_empty};

const STORAGE: &str = "storage";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersistentVolumeSummary {
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// `spec.capacity["storage"]` as written.
    pub capacity: Option<String>,
    /// API names (`ReadWriteOnce`).
    pub access_modes: Vec<String>,
    /// Defaults to `Retain`, the API default for manually created volumes.
    pub reclaim_policy: String,
    /// The API text (`Available`, `Bound`, `Released`, `Failed`); `Pending` while absent.
    pub phase: String,
    pub is_terminating: bool,
    pub claim: Option<ClaimRef>,
    pub storage_class: Option<String>,
    pub volume_mode: Option<String>,
    pub backend: VolumeBackend,
    /// One entry per required node selector term, in kubectl selector syntax.
    pub node_affinity: Vec<String>,
    pub mount_options: Vec<String>,
    /// `status.reason`.
    pub reason: Option<String>,
    /// `status.message`, cut to 1 KiB.
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimRef {
    pub namespace: String,
    pub name: String,
}

/// Where the data lives. CSI `volumeAttributes`, every secret reference, and flexVolume
/// `options` are never copied: they are free-form and may hold credentials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VolumeBackend {
    Csi {
        driver: String,
        volume_handle: String,
        fs_type: Option<String>,
    },
    Nfs {
        server: String,
        path: String,
    },
    HostPath {
        path: String,
    },
    Local {
        path: String,
    },
    /// An in-tree source by its field name (`awsElasticBlockStore`, `rbd`, ...), or `unknown`.
    Other {
        kind: &'static str,
    },
}

impl ClusterConnection {
    /// Watches persistent volumes. Yields batched snapshots ordered by name.
    pub fn watch_persistent_volumes(
        &self,
    ) -> impl Stream<Item = WatchUpdate<PersistentVolumeSummary>> + Send + 'static {
        let api = Api::<PersistentVolume>::all(self.client().clone());
        summary_watch(
            self,
            vec![(None, api)],
            "watching persistent volumes",
            persistent_volume_summary,
        )
    }
}

pub(crate) fn persistent_volume_summary(volume: &PersistentVolume) -> PersistentVolumeSummary {
    let spec = volume.spec.as_ref();
    let status = volume.status.as_ref();
    PersistentVolumeSummary {
        name: volume.metadata.name.clone().unwrap_or_default(),
        created_at: volume
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
        labels: label_terms(&volume.metadata),
        capacity: spec
            .and_then(|spec| spec.capacity.as_ref())
            .and_then(|capacity| capacity.get(STORAGE))
            .map(|quantity| quantity.0.clone()),
        access_modes: spec
            .and_then(|spec| spec.access_modes.clone())
            .unwrap_or_default(),
        reclaim_policy: non_empty(
            spec.and_then(|spec| spec.persistent_volume_reclaim_policy.as_deref()),
        )
        .unwrap_or_else(|| "Retain".to_owned()),
        phase: non_empty(status.and_then(|status| status.phase.as_deref()))
            .unwrap_or_else(|| "Pending".to_owned()),
        is_terminating: volume.metadata.deletion_timestamp.is_some(),
        claim: spec.and_then(claim_ref),
        storage_class: non_empty(spec.and_then(|spec| spec.storage_class_name.as_deref())),
        volume_mode: non_empty(spec.and_then(|spec| spec.volume_mode.as_deref())),
        backend: spec.map_or(VolumeBackend::Other { kind: "unknown" }, backend),
        node_affinity: spec
            .and_then(|spec| spec.node_affinity.as_ref())
            .and_then(|affinity| affinity.required.as_ref())
            .into_iter()
            .flat_map(|selector| &selector.node_selector_terms)
            .map(node_selector_term)
            .collect(),
        mount_options: spec
            .and_then(|spec| spec.mount_options.clone())
            .unwrap_or_default(),
        reason: non_empty(status.and_then(|status| status.reason.as_deref())),
        message: optional_message(status.and_then(|status| status.message.as_deref())),
    }
}

fn claim_ref(spec: &PersistentVolumeSpec) -> Option<ClaimRef> {
    let claim = spec.claim_ref.as_ref()?;
    Some(ClaimRef {
        namespace: non_empty(claim.namespace.as_deref())?,
        name: non_empty(claim.name.as_deref())?,
    })
}

fn backend(spec: &PersistentVolumeSpec) -> VolumeBackend {
    if let Some(csi) = &spec.csi {
        return VolumeBackend::Csi {
            driver: csi.driver.clone(),
            volume_handle: csi.volume_handle.clone(),
            fs_type: non_empty(csi.fs_type.as_deref()),
        };
    }
    if let Some(nfs) = &spec.nfs {
        return VolumeBackend::Nfs {
            server: nfs.server.clone(),
            path: nfs.path.clone(),
        };
    }
    if let Some(host_path) = &spec.host_path {
        return VolumeBackend::HostPath {
            path: host_path.path.clone(),
        };
    }
    if let Some(local) = &spec.local {
        return VolumeBackend::Local {
            path: local.path.clone(),
        };
    }
    VolumeBackend::Other {
        kind: in_tree_source(spec),
    }
}

/// The API field name of the first in-tree source that is set.
fn in_tree_source(spec: &PersistentVolumeSpec) -> &'static str {
    let sources = [
        (
            "awsElasticBlockStore",
            spec.aws_elastic_block_store.is_some(),
        ),
        ("azureDisk", spec.azure_disk.is_some()),
        ("azureFile", spec.azure_file.is_some()),
        ("cephfs", spec.cephfs.is_some()),
        ("cinder", spec.cinder.is_some()),
        ("fc", spec.fc.is_some()),
        ("flexVolume", spec.flex_volume.is_some()),
        ("flocker", spec.flocker.is_some()),
        ("gcePersistentDisk", spec.gce_persistent_disk.is_some()),
        ("glusterfs", spec.glusterfs.is_some()),
        ("iscsi", spec.iscsi.is_some()),
        (
            "photonPersistentDisk",
            spec.photon_persistent_disk.is_some(),
        ),
        ("portworxVolume", spec.portworx_volume.is_some()),
        ("quobyte", spec.quobyte.is_some()),
        ("rbd", spec.rbd.is_some()),
        ("scaleIO", spec.scale_io.is_some()),
        ("storageos", spec.storageos.is_some()),
        ("vsphereVolume", spec.vsphere_volume.is_some()),
    ];
    sources
        .into_iter()
        .find_map(|(kind, is_set)| is_set.then_some(kind))
        .unwrap_or("unknown")
}

/// `matchExpressions` then `matchFields`, joined with `, `.
fn node_selector_term(term: &NodeSelectorTerm) -> String {
    term.match_expressions
        .iter()
        .flatten()
        .chain(term.match_fields.iter().flatten())
        .map(requirement_text)
        .collect::<Vec<_>>()
        .join(", ")
}

fn requirement_text(requirement: &NodeSelectorRequirement) -> String {
    let key = &requirement.key;
    let values = requirement.values.as_deref().unwrap_or_default();
    match requirement.operator.as_str() {
        "In" => format!("{key} in ({})", values.join(",")),
        "NotIn" => format!("{key} notin ({})", values.join(",")),
        "Exists" => key.clone(),
        "DoesNotExist" => format!("!{key}"),
        "Gt" => format!("{key} > {}", values.join(",")),
        "Lt" => format!("{key} < {}", values.join(",")),
        other => format!("{key} {other} ({})", values.join(",")),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::api::core::v1::{
        CSIPersistentVolumeSource, HostPathVolumeSource, NFSVolumeSource, NodeSelector,
        ObjectReference, PersistentVolumeStatus, RBDPersistentVolumeSource, SecretReference,
        VolumeNodeAffinity,
    };
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

    use super::*;

    fn volume(spec: PersistentVolumeSpec) -> PersistentVolumeSummary {
        persistent_volume_summary(&PersistentVolume {
            spec: Some(spec),
            ..Default::default()
        })
    }

    fn requirement(key: &str, operator: &str, values: &[&str]) -> NodeSelectorRequirement {
        NodeSelectorRequirement {
            key: key.to_owned(),
            operator: operator.to_owned(),
            values: Some(values.iter().map(|&value| value.to_owned()).collect()),
        }
    }

    #[test]
    fn pv_summary_reads_claim_class_and_reclaim() {
        let summary = persistent_volume_summary(&PersistentVolume {
            spec: Some(PersistentVolumeSpec {
                capacity: Some(BTreeMap::from([(
                    STORAGE.to_owned(),
                    Quantity("20Gi".to_owned()),
                )])),
                access_modes: Some(vec!["ReadWriteOnce".to_owned()]),
                persistent_volume_reclaim_policy: Some("Delete".to_owned()),
                storage_class_name: Some("gp3".to_owned()),
                volume_mode: Some("Filesystem".to_owned()),
                claim_ref: Some(ObjectReference {
                    namespace: Some("shop".to_owned()),
                    name: Some("data".to_owned()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            status: Some(PersistentVolumeStatus {
                phase: Some("Bound".to_owned()),
                reason: Some("Why".to_owned()),
                message: Some("Because".to_owned()),
                ..Default::default()
            }),
            ..Default::default()
        });
        assert_eq!(summary.capacity.as_deref(), Some("20Gi"));
        assert_eq!(summary.access_modes, ["ReadWriteOnce"]);
        assert_eq!(summary.reclaim_policy, "Delete");
        assert_eq!(summary.phase, "Bound");
        assert_eq!(summary.storage_class.as_deref(), Some("gp3"));
        assert_eq!(summary.volume_mode.as_deref(), Some("Filesystem"));
        assert_eq!(
            summary.claim,
            Some(ClaimRef {
                namespace: "shop".to_owned(),
                name: "data".to_owned(),
            })
        );
        assert_eq!(summary.reason.as_deref(), Some("Why"));
        assert_eq!(summary.message.as_deref(), Some("Because"));
    }

    #[test]
    fn pv_reclaim_defaults_to_retain() {
        let summary = persistent_volume_summary(&PersistentVolume::default());
        assert_eq!(summary.reclaim_policy, "Retain");
        assert_eq!(summary.phase, "Pending");
        assert_eq!(summary.claim, None);
        assert_eq!(summary.backend, VolumeBackend::Other { kind: "unknown" });
    }

    #[test]
    fn csi_backend_keeps_driver_handle_fs_type() {
        let summary = volume(PersistentVolumeSpec {
            csi: Some(CSIPersistentVolumeSource {
                driver: "ebs.csi.aws.com".to_owned(),
                volume_handle: "vol-0abc".to_owned(),
                fs_type: Some("ext4".to_owned()),
                ..Default::default()
            }),
            ..Default::default()
        });
        assert_eq!(
            summary.backend,
            VolumeBackend::Csi {
                driver: "ebs.csi.aws.com".to_owned(),
                volume_handle: "vol-0abc".to_owned(),
                fs_type: Some("ext4".to_owned()),
            }
        );
    }

    #[test]
    fn csi_attributes_and_secret_refs_are_not_copied() {
        let secret = || SecretReference {
            name: Some("distinctive-secret-name".to_owned()),
            namespace: Some("kube-system".to_owned()),
        };
        let summary = volume(PersistentVolumeSpec {
            csi: Some(CSIPersistentVolumeSource {
                driver: "d".to_owned(),
                volume_handle: "h".to_owned(),
                volume_attributes: Some(BTreeMap::from([(
                    "key".to_owned(),
                    "distinctive-attribute-value".to_owned(),
                )])),
                controller_publish_secret_ref: Some(secret()),
                node_stage_secret_ref: Some(secret()),
                ..Default::default()
            }),
            ..Default::default()
        });
        let text = format!("{summary:?}");
        assert!(!text.contains("distinctive-attribute-value"), "{text}");
        assert!(!text.contains("distinctive-secret-name"), "{text}");
    }

    #[test]
    fn nfs_and_host_path_backends() {
        let nfs = volume(PersistentVolumeSpec {
            nfs: Some(NFSVolumeSource {
                server: "10.0.0.5".to_owned(),
                path: "/exports/data".to_owned(),
                read_only: None,
            }),
            ..Default::default()
        });
        assert_eq!(
            nfs.backend,
            VolumeBackend::Nfs {
                server: "10.0.0.5".to_owned(),
                path: "/exports/data".to_owned(),
            }
        );
        let host_path = volume(PersistentVolumeSpec {
            host_path: Some(HostPathVolumeSource {
                path: "/mnt/disk".to_owned(),
                type_: None,
            }),
            ..Default::default()
        });
        assert_eq!(
            host_path.backend,
            VolumeBackend::HostPath {
                path: "/mnt/disk".to_owned()
            }
        );
    }

    #[test]
    fn in_tree_source_reports_field_name() {
        let summary = volume(PersistentVolumeSpec {
            rbd: Some(RBDPersistentVolumeSource {
                image: "img".to_owned(),
                monitors: Vec::new(),
                ..Default::default()
            }),
            ..Default::default()
        });
        assert_eq!(summary.backend, VolumeBackend::Other { kind: "rbd" });
    }

    #[test]
    fn node_affinity_terms_in_selector_syntax() {
        let summary = volume(PersistentVolumeSpec {
            node_affinity: Some(VolumeNodeAffinity {
                required: Some(NodeSelector {
                    node_selector_terms: vec![
                        NodeSelectorTerm {
                            match_expressions: Some(vec![
                                requirement("topology.kubernetes.io/zone", "In", &["a", "b"]),
                                requirement("disk", "Exists", &[]),
                            ]),
                            match_fields: Some(vec![requirement("metadata.name", "In", &["n1"])]),
                        },
                        NodeSelectorTerm {
                            match_expressions: Some(vec![requirement("gpu", "DoesNotExist", &[])]),
                            match_fields: None,
                        },
                    ],
                }),
            }),
            ..Default::default()
        });
        assert_eq!(
            summary.node_affinity,
            [
                "topology.kubernetes.io/zone in (a,b), disk, metadata.name in (n1)",
                "!gpu",
            ]
        );
    }

    #[test]
    fn pv_keeps_mount_options() {
        let summary = volume(PersistentVolumeSpec {
            mount_options: Some(vec!["nfsvers=4.1".to_owned(), "hard".to_owned()]),
            ..Default::default()
        });
        assert_eq!(summary.mount_options, ["nfsvers=4.1", "hard"]);
    }
}
