use std::collections::BTreeSet;

use futures::Stream;
use k8s_openapi::api::core::v1::Namespace;
use kube::Api;

use crate::connection::{ClusterConnection, ClusterError};
use crate::resource_watch::{WatchUpdate, summary_watch};
use crate::workload::label_terms;

/// Which namespaces a query covers. Passed by value so futures and streams own it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamespaceScope {
    All,
    Named(String),
    /// Two or more namespaces, sorted and unique. Build it with `of_namespaces`.
    Several(Vec<String>),
}

impl NamespaceScope {
    /// Sorts and dedups: none is `All`, one is `Named`, more is `Several`.
    pub fn of_namespaces(names: impl IntoIterator<Item = String>) -> Self {
        let mut names: Vec<String> = names
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        match names.len() {
            0 => Self::All,
            1 => Self::Named(names.swap_remove(0)),
            _ => Self::Several(names),
        }
    }

    /// The picked namespaces in order; empty for `All`.
    pub fn namespaces(&self) -> &[String] {
        match self {
            Self::All => &[],
            Self::Named(namespace) => std::slice::from_ref(namespace),
            Self::Several(namespaces) => namespaces,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamespaceSummary {
    pub name: String,
    pub phase: NamespacePhase,
    /// `key=value` terms in key order.
    pub labels: Vec<String>,
    pub created_at: Option<jiff::Timestamp>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamespacePhase {
    Active,
    Terminating,
    Unknown,
}

impl ClusterConnection {
    /// Lists all namespaces, ordered by name.
    pub async fn list_namespaces(&self) -> Result<Vec<NamespaceSummary>, ClusterError> {
        let api = Api::<Namespace>::all(self.client().clone());
        let namespaces = self.list_all(api, "listing namespaces").await?;
        let mut summaries: Vec<_> = namespaces.iter().map(namespace_summary).collect();
        summaries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(summaries)
    }

    /// Watches all namespaces. Yields batched snapshots ordered by name.
    pub fn watch_namespaces(
        &self,
    ) -> impl Stream<Item = WatchUpdate<NamespaceSummary>> + Send + 'static {
        let api = Api::<Namespace>::all(self.client().clone());
        summary_watch(
            self,
            vec![(None, api)],
            "watching namespaces",
            namespace_summary,
        )
    }
}

pub(crate) fn namespace_summary(namespace: &Namespace) -> NamespaceSummary {
    let phase = match namespace
        .status
        .as_ref()
        .and_then(|status| status.phase.as_deref())
    {
        Some("Active") => NamespacePhase::Active,
        Some("Terminating") => NamespacePhase::Terminating,
        _ => NamespacePhase::Unknown,
    };
    NamespaceSummary {
        name: namespace.metadata.name.clone().unwrap_or_default(),
        labels: label_terms(&namespace.metadata),
        phase,
        created_at: namespace
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|time| time.0),
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::core::v1::NamespaceStatus;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};

    use super::*;

    fn namespace_with_phase(phase: Option<&str>) -> Namespace {
        Namespace {
            status: Some(NamespaceStatus {
                phase: phase.map(str::to_owned),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn of_namespaces_normalizes_to_all_named_or_several() {
        assert_eq!(NamespaceScope::of_namespaces([]), NamespaceScope::All);
        assert_eq!(
            NamespaceScope::of_namespaces(names(&["a", "a"])),
            NamespaceScope::Named("a".to_owned())
        );
        assert_eq!(
            NamespaceScope::of_namespaces(names(&["b", "a", "b"])),
            NamespaceScope::Several(names(&["a", "b"]))
        );
    }

    #[test]
    fn namespaces_lists_picked_names() {
        assert!(NamespaceScope::All.namespaces().is_empty());
        assert_eq!(
            NamespaceScope::Named("a".to_owned()).namespaces(),
            ["a".to_owned()]
        );
        assert_eq!(
            NamespaceScope::Several(names(&["a", "b"])).namespaces(),
            ["a".to_owned(), "b".to_owned()]
        );
    }

    #[test]
    fn namespace_phase_maps_active_terminating_and_unknown() {
        let phase = |text| namespace_summary(&namespace_with_phase(text)).phase;
        assert_eq!(phase(Some("Active")), NamespacePhase::Active);
        assert_eq!(phase(Some("Terminating")), NamespacePhase::Terminating);
        assert_eq!(phase(Some("Mystery")), NamespacePhase::Unknown);
        assert_eq!(phase(None), NamespacePhase::Unknown);
        assert_eq!(
            namespace_summary(&Namespace::default()).phase,
            NamespacePhase::Unknown
        );
    }

    #[test]
    fn namespace_summary_reads_labels_in_key_order() {
        let namespace = Namespace {
            metadata: ObjectMeta {
                labels: Some(
                    [
                        ("team".to_owned(), "a".to_owned()),
                        ("env".to_owned(), "dev".to_owned()),
                    ]
                    .into(),
                ),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(namespace_summary(&namespace).labels, ["env=dev", "team=a"]);
        assert!(namespace_summary(&Namespace::default()).labels.is_empty());
    }

    #[test]
    fn namespace_summary_reads_name_and_creation_time() {
        let created: jiff::Timestamp = "2024-05-01T10:00:00Z".parse().expect("valid timestamp");
        let namespace = Namespace {
            metadata: ObjectMeta {
                name: Some("kube-system".to_owned()),
                creation_timestamp: Some(Time(created)),
                ..Default::default()
            },
            ..Default::default()
        };
        let summary = namespace_summary(&namespace);
        assert_eq!(summary.name, "kube-system");
        assert_eq!(summary.created_at, Some(created));
    }
}
