use futures::Stream;
use k8s_openapi::api::storage::v1::StorageClass;
use kube::Api;

use crate::connection::ClusterConnection;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::label_terms;

const DEFAULT_ANNOTATIONS: [&str; 2] = [
    "storageclass.kubernetes.io/is-default-class",
    "storageclass.beta.kubernetes.io/is-default-class",
];
/// Parameter-name fragments that mark a plaintext credential (after normalizing the key).
const SECRET_FRAGMENTS: [&str; 8] = [
    "password",
    "passwd",
    "token",
    "credential",
    "secretkey",
    "accesskey",
    "userkey",
    "privatekey",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageClassSummary {
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    pub provisioner: String,
    /// Defaults to `Delete`.
    pub reclaim_policy: String,
    /// Defaults to `Immediate`.
    pub binding_mode: String,
    pub allows_expansion: bool,
    pub is_default: bool,
    /// In key order; secret-like values are dropped before the summary is built.
    pub parameters: Vec<StorageParameter>,
    pub mount_options: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageParameter {
    pub key: String,
    /// `None` means the value is hidden.
    pub value: Option<String>,
}

impl ClusterConnection {
    /// Watches storage classes. Yields batched snapshots ordered by name.
    pub fn watch_storage_classes(
        &self,
    ) -> impl Stream<Item = WatchUpdate<StorageClassSummary>> + Send + 'static {
        let api = Api::<StorageClass>::all(self.client().clone());
        summary_watch(
            self,
            vec![(None, api)],
            "watching storage classes",
            storage_class_summary,
        )
    }
}

pub(crate) fn storage_class_summary(class: &StorageClass) -> StorageClassSummary {
    StorageClassSummary {
        name: class.metadata.name.clone().unwrap_or_default(),
        created_at: class
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
        labels: label_terms(&class.metadata),
        provisioner: class.provisioner.clone(),
        reclaim_policy: class
            .reclaim_policy
            .clone()
            .filter(|policy| !policy.is_empty())
            .unwrap_or_else(|| "Delete".to_owned()),
        binding_mode: class
            .volume_binding_mode
            .clone()
            .filter(|mode| !mode.is_empty())
            .unwrap_or_else(|| "Immediate".to_owned()),
        allows_expansion: class.allow_volume_expansion.unwrap_or(false),
        is_default: is_default_class(class),
        parameters: class
            .parameters
            .iter()
            .flatten()
            .map(|(key, value)| StorageParameter {
                key: key.clone(),
                value: (!is_secret_parameter(key)).then(|| value.clone()),
            })
            .collect(),
        mount_options: class.mount_options.clone().unwrap_or_default(),
    }
}

/// The only annotations ever read: the default class exists only as this flag, and it holds
/// `true` or `false`, never a secret.
fn is_default_class(class: &StorageClass) -> bool {
    let Some(annotations) = &class.metadata.annotations else {
        return false;
    };
    DEFAULT_ANNOTATIONS.iter().any(|key| {
        annotations
            .get(*key)
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    })
}

/// Whether a StorageClass parameter value looks like a credential. A key-name heuristic:
/// `*-secret-name` and `*-secret-namespace` are references and stay visible.
pub(crate) fn is_secret_parameter(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|ch| !matches!(ch, '-' | '_' | '.'))
        .flat_map(char::to_lowercase)
        .collect();
    if normalized.ends_with("secretname") || normalized.ends_with("secretnamespace") {
        return false;
    }
    SECRET_FRAGMENTS
        .iter()
        .any(|fragment| normalized.contains(fragment))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn class_with_parameters(pairs: &[(&str, &str)]) -> StorageClassSummary {
        storage_class_summary(&StorageClass {
            parameters: Some(
                pairs
                    .iter()
                    .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                    .collect(),
            ),
            ..Default::default()
        })
    }

    fn class_with_annotations(pairs: &[(&str, &str)]) -> StorageClassSummary {
        let mut class = StorageClass::default();
        class.metadata.annotations = Some(
            pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect::<BTreeMap<_, _>>(),
        );
        storage_class_summary(&class)
    }

    #[test]
    fn storage_class_defaults_delete_and_immediate() {
        let summary = storage_class_summary(&StorageClass {
            provisioner: "ebs.csi.aws.com".to_owned(),
            ..Default::default()
        });
        assert_eq!(summary.provisioner, "ebs.csi.aws.com");
        assert_eq!(summary.reclaim_policy, "Delete");
        assert_eq!(summary.binding_mode, "Immediate");
        assert!(!summary.allows_expansion);
        assert!(!summary.is_default);
    }

    #[test]
    fn default_class_from_either_annotation() {
        assert!(
            class_with_annotations(&[("storageclass.kubernetes.io/is-default-class", "true")])
                .is_default
        );
        assert!(
            class_with_annotations(&[("storageclass.beta.kubernetes.io/is-default-class", "TRUE")])
                .is_default
        );
        assert!(
            !class_with_annotations(&[("storageclass.kubernetes.io/is-default-class", "false")])
                .is_default
        );
    }

    #[test]
    fn other_annotations_are_ignored() {
        let summary = class_with_annotations(&[
            (
                "kubectl.kubernetes.io/last-applied-configuration",
                "distinctive-manifest",
            ),
            ("owner", "distinctive-owner"),
        ]);
        assert!(!summary.is_default);
        let text = format!("{summary:?}");
        assert!(!text.contains("distinctive"), "{text}");
    }

    #[test]
    fn secret_like_parameters_are_hidden() {
        let summary = class_with_parameters(&[
            ("restuserkey", "distinctive-1"),
            ("adminPassword", "distinctive-2"),
            ("access-key", "distinctive-3"),
            ("token", "distinctive-4"),
        ]);
        assert!(summary.parameters.iter().all(|item| item.value.is_none()));
        assert_eq!(summary.parameters.len(), 4);
        let text = format!("{summary:?}");
        assert!(!text.contains("distinctive"), "{text}");
    }

    #[test]
    fn harmless_parameters_are_shown() {
        let summary = class_with_parameters(&[
            ("kmsKeyId", "alias/ebs"),
            ("type", "gp3"),
            ("csi.storage.k8s.io/provisioner-secret-name", "creds"),
            (
                "csi.storage.k8s.io/node-stage-secret-namespace",
                "kube-system",
            ),
        ]);
        assert!(summary.parameters.iter().all(|item| item.value.is_some()));
    }

    #[test]
    fn parameters_in_key_order() {
        let summary = class_with_parameters(&[("zone", "a"), ("fsType", "ext4"), ("type", "gp3")]);
        let keys: Vec<_> = summary
            .parameters
            .iter()
            .map(|item| item.key.as_str())
            .collect();
        assert_eq!(keys, ["fsType", "type", "zone"]);
    }
}
