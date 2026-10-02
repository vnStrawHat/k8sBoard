use futures::Stream;
use k8s_openapi::api::core::v1::ServiceAccount;

use crate::connection::ClusterConnection;
use crate::namespace::NamespaceScope;
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::label_terms;

/// The only annotations this module reads: cloud identities are identifiers (a role ARN, a
/// service account e-mail, a client id), never credentials. Every other annotation is ignored.
const CLOUD_IDENTITY_KEYS: [(&str, CloudProvider); 3] = [
    ("eks.amazonaws.com/role-arn", CloudProvider::Aws),
    ("iam.gke.io/gcp-service-account", CloudProvider::Gcp),
    ("azure.workload.identity/client-id", CloudProvider::Azure),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceAccountSummary {
    pub namespace: String,
    pub name: String,
    pub created_at: Option<jiff::Timestamp>,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    /// Names of `secrets[]` only; no Secret is ever requested.
    pub secrets: Vec<String>,
    /// Names of `imagePullSecrets[]` only.
    pub image_pull_secrets: Vec<String>,
    /// `None` means the field is unset (the token is mounted).
    pub automount_token: Option<bool>,
    pub cloud_identities: Vec<CloudIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloudIdentity {
    pub provider: CloudProvider,
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudProvider {
    Aws,
    Gcp,
    Azure,
}

impl CloudProvider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Aws => "IAM role",
            Self::Gcp => "GCP service account",
            Self::Azure => "Azure client ID",
        }
    }
}

impl ClusterConnection {
    /// Watches service accounts in `scope`. Yields batched snapshots ordered by
    /// (namespace, name).
    pub fn watch_service_accounts(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<ServiceAccountSummary>> + Send + 'static {
        summary_watch(
            self,
            self.scoped_apis(&scope),
            "watching service accounts",
            service_account_summary,
        )
    }
}

pub(crate) fn service_account_summary(account: &ServiceAccount) -> ServiceAccountSummary {
    ServiceAccountSummary {
        namespace: account.metadata.namespace.clone().unwrap_or_default(),
        name: account.metadata.name.clone().unwrap_or_default(),
        created_at: account
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
        labels: label_terms(&account.metadata),
        secrets: account
            .secrets
            .iter()
            .flatten()
            .filter_map(|reference| reference.name.clone())
            .filter(|name| !name.is_empty())
            .collect(),
        image_pull_secrets: account
            .image_pull_secrets
            .iter()
            .flatten()
            .map(|reference| reference.name.clone())
            .filter(|name| !name.is_empty())
            .collect(),
        automount_token: account.automount_service_account_token,
        cloud_identities: cloud_identities(account),
    }
}

fn cloud_identities(account: &ServiceAccount) -> Vec<CloudIdentity> {
    let Some(annotations) = &account.metadata.annotations else {
        return Vec::new();
    };
    CLOUD_IDENTITY_KEYS
        .iter()
        .filter_map(|(key, provider)| {
            let value = annotations.get(*key).filter(|value| !value.is_empty())?;
            Some(CloudIdentity {
                provider: *provider,
                value: value.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::api::core::v1::{LocalObjectReference, ObjectReference};

    use super::*;

    fn account_with_annotations(pairs: &[(&str, &str)]) -> ServiceAccount {
        let mut account = ServiceAccount::default();
        account.metadata.annotations = Some(
            pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect::<BTreeMap<_, _>>(),
        );
        account
    }

    #[test]
    fn service_account_summary_keeps_names_only() {
        let mut account = account_with_annotations(&[
            ("owner", "distinctive-owner"),
            (
                "eks.amazonaws.com/role-arn",
                "arn:aws:iam::123456789012:role/api",
            ),
        ]);
        account.secrets = Some(vec![ObjectReference {
            name: Some("api-token".to_owned()),
            uid: Some("distinctive-uid".to_owned()),
            resource_version: Some("distinctive-rv".to_owned()),
            namespace: Some("distinctive-ns".to_owned()),
            ..Default::default()
        }]);
        account.image_pull_secrets = Some(vec![LocalObjectReference {
            name: "registry".to_owned(),
        }]);
        let summary = service_account_summary(&account);
        assert_eq!(summary.secrets, ["api-token"]);
        assert_eq!(summary.image_pull_secrets, ["registry"]);
        let text = format!("{summary:?}");
        assert!(
            text.contains("arn:aws:iam::123456789012:role/api"),
            "{text}"
        );
        for hidden in [
            "distinctive-uid",
            "distinctive-rv",
            "distinctive-ns",
            "distinctive-owner",
        ] {
            assert!(!text.contains(hidden), "{text}");
        }
    }

    #[test]
    fn cloud_identity_reads_only_allowlisted_keys() {
        let summary = service_account_summary(&account_with_annotations(&[
            ("azure.workload.identity/client-id", "client-1"),
            ("iam.gke.io/gcp-service-account", "sa@project.iam"),
            ("eks.amazonaws.com/role-arn", "arn:aws:iam::1:role/x"),
        ]));
        let found: Vec<_> = summary
            .cloud_identities
            .iter()
            .map(|identity| (identity.provider, identity.value.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                (CloudProvider::Aws, "arn:aws:iam::1:role/x"),
                (CloudProvider::Gcp, "sa@project.iam"),
                (CloudProvider::Azure, "client-1"),
            ]
        );
        let empty = service_account_summary(&account_with_annotations(&[(
            "eks.amazonaws.com/role-arn",
            "",
        )]));
        assert!(empty.cloud_identities.is_empty());
    }

    #[test]
    fn other_annotations_are_ignored() {
        let summary = service_account_summary(&account_with_annotations(&[
            ("kubernetes.io/enforce-mountable-secrets", "distinctive-1"),
            ("eks.amazonaws.com/role-arn-x", "distinctive-2"),
            (
                "kubectl.kubernetes.io/last-applied-configuration",
                "distinctive-3",
            ),
        ]));
        assert!(summary.cloud_identities.is_empty());
        assert!(!format!("{summary:?}").contains("distinctive"));
    }

    #[test]
    fn automount_token_is_optional() {
        assert_eq!(
            service_account_summary(&ServiceAccount::default()).automount_token,
            None
        );
        let account = ServiceAccount {
            automount_service_account_token: Some(false),
            ..Default::default()
        };
        assert_eq!(
            service_account_summary(&account).automount_token,
            Some(false)
        );
    }

    #[test]
    fn empty_secret_names_are_dropped() {
        let account = ServiceAccount {
            secrets: Some(vec![
                ObjectReference::default(),
                ObjectReference {
                    name: Some(String::new()),
                    ..Default::default()
                },
            ]),
            image_pull_secrets: Some(vec![LocalObjectReference::default()]),
            ..Default::default()
        };
        let summary = service_account_summary(&account);
        assert!(summary.secrets.is_empty());
        assert!(summary.image_pull_secrets.is_empty());
    }

    #[test]
    fn provider_labels() {
        assert_eq!(CloudProvider::Aws.label(), "IAM role");
        assert_eq!(CloudProvider::Gcp.label(), "GCP service account");
        assert_eq!(CloudProvider::Azure.label(), "Azure client ID");
    }
}
