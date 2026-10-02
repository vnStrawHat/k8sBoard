//! The cluster registry: per-context overrides, the extra kubeconfig files, and the last-used //! cluster. Paths and names only, never credentials. Also picks the cluster to open at start.

use std::path::PathBuf;

use cluster::ContextSummary;
use serde::{Deserialize, Serialize};

use crate::environment::{Environment, guess_environment};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ClusterRegistry {
    /// Files added by the user; each loads standalone, apart from the launch chain.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) kubeconfigs: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) clusters: Vec<ClusterEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_used: Option<ClusterRef>,
}

/// A cluster is its context name in the file that defined it: the same name appears in many files.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct ClusterRef {
    pub(crate) kubeconfig: PathBuf,
    pub(crate) context: String,
}

/// Overrides only: a context without an entry gets the defaults.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ClusterEntry {
    #[serde(flatten)]
    pub(crate) cluster: ClusterRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) display_name: Option<String>,
    /// `None` is guessed from the names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) environment: Option<Environment>,
    /// Stored now, read by the write guard of 0030.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) read_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) default_namespace: Option<String>,
}

/// What the views show for a context: the entry's overrides over the defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClusterProfile {
    pub(crate) display_name: String,
    pub(crate) environment: Environment,
    pub(crate) default_namespace: Option<String>,
}

impl ClusterRef {
    pub(crate) fn of(summary: &ContextSummary) -> Self {
        Self {
            kubeconfig: summary.source.clone(),
            context: summary.name.clone(),
        }
    }

    pub(crate) fn is_of(&self, summary: &ContextSummary) -> bool {
        self.kubeconfig == summary.source && self.context == summary.name
    }
}

impl ClusterRegistry {
    /// The entry for `summary`, without cloning its path into a key.
    fn entry_of(&self, summary: &ContextSummary) -> Option<&ClusterEntry> {
        self.clusters
            .iter()
            .find(|entry| entry.cluster.is_of(summary))
    }

    /// The entry for `cluster`; a missing one is appended with no overrides, so the first
    /// edit of a context registers it (nothing registers on browsing).
    pub(crate) fn entry_mut(&mut self, cluster: &ClusterRef) -> &mut ClusterEntry {
        let position = self
            .clusters
            .iter()
            .position(|entry| entry.cluster == *cluster);
        let index = position.unwrap_or_else(|| {
            self.clusters.push(ClusterEntry {
                cluster: cluster.clone(),
                display_name: None,
                environment: None,
                read_only: None,
                default_namespace: None,
            });
            self.clusters.len() - 1
        });
        &mut self.clusters[index]
    }

    pub(crate) fn profile(&self, summary: &ContextSummary) -> ClusterProfile {
        let entry = self.entry_of(summary);
        let display_name = entry
            .and_then(|entry| entry.display_name.as_deref())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&summary.name)
            .to_owned();
        let environment = entry
            .and_then(|entry| entry.environment)
            .unwrap_or_else(|| guess_environment(&summary.name, &summary.cluster));
        ClusterProfile {
            display_name,
            environment,
            default_namespace: entry.and_then(|entry| entry.default_namespace.clone()),
        }
    }
}

/// The switcher text: the display name, plus ` · {file name}` when another loaded kubeconfig
/// has a context with the same name.
pub(crate) fn switcher_label(
    profile: &ClusterProfile,
    summary: &ContextSummary,
    is_duplicate_name: bool,
) -> String {
    let file_name = summary.source.file_name().and_then(|name| name.to_str());
    match file_name {
        Some(file_name) if is_duplicate_name => format!("{} · {file_name}", profile.display_name),
        _ => profile.display_name.clone(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StartChoice {
    Cluster(ClusterRef),
    RequestedMissing,
    CurrentContext,
}

/// `contexts` are in load order (the launch chain first). A requested name takes its first
/// match; else an exact `last_used` that is still loaded; else the current-context.
pub(crate) fn start_choice(
    requested: Option<&str>,
    last_used: Option<&ClusterRef>,
    contexts: &[&ContextSummary],
) -> StartChoice {
    if let Some(name) = requested {
        return match contexts.iter().find(|summary| summary.name == name) {
            Some(summary) => StartChoice::Cluster(ClusterRef::of(summary)),
            None => StartChoice::RequestedMissing,
        };
    }
    let last_used =
        last_used.filter(|cluster| contexts.iter().any(|summary| cluster.is_of(summary)));
    match last_used {
        Some(cluster) => StartChoice::Cluster(cluster.clone()),
        None => StartChoice::CurrentContext,
    }
}

/// The saved `last_used` that may pick the start cluster. With an explicit `--kubeconfig`, only
/// one from the named files counts, so a registry file never overrides the user's choice.
pub(crate) fn launch_last_used<'a>(
    last_used: Option<&'a ClusterRef>,
    explicit_files: Option<&[PathBuf]>,
) -> Option<&'a ClusterRef> {
    last_used
        .filter(|cluster| explicit_files.is_none_or(|files| files.contains(&cluster.kubeconfig)))
}

#[cfg(test)]
#[path = "cluster_registry_tests.rs"]
mod cluster_registry_tests;
