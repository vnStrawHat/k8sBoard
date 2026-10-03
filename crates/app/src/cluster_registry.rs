//! The cluster registry: per-context overrides, the extra kubeconfig files, and the last-used //! cluster. Paths and names only, never credentials. Also picks the cluster to open at start.

use std::collections::HashMap;
use std::path::PathBuf;

use cluster::{ContextSummary, DEFAULT_DEBUG_IMAGE, NamespaceScope};
use serde::{Deserialize, Serialize};

use crate::environment::{Environment, guess_environment};
use crate::write_guard::ConfirmMode;

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
    /// The lock a session starts in (spec 0030); `None` is on for Production.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) read_only: Option<bool>,
    /// How changes are confirmed; `None` follows the environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) confirm: Option<ConfirmMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) default_namespace: Option<String>,
    /// Whether the node shell is offered (spec 0037); `None` follows the environment: on for
    /// Development and Local, on for Staging only when the environment is set here, off otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) allow_node_shell: Option<bool>,
    /// The image of debug containers and node shell pods; `None` is the pinned busybox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) debug_image: Option<String>,
    /// Where the node shell pod is created; `None` is `kube-system`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) node_shell_namespace: Option<String>,
}

/// The namespace of a node shell pod unless the entry names another: usually exempt from Pod
/// Security enforcement, and where admins expect system pods.
pub(crate) const DEFAULT_NODE_SHELL_NAMESPACE: &str = "kube-system";

/// What the views show for a context: the entry's overrides over the defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClusterProfile {
    pub(crate) display_name: String,
    pub(crate) environment: Environment,
    pub(crate) default_namespace: Option<String>,
    /// The stored switch, else on for Production: the lock a session opens in.
    pub(crate) read_only: bool,
    /// The stored mode, else the environment's default.
    pub(crate) confirm: ConfirmMode,
    /// The stored switch, else the environment's default: off for Production and for an
    /// environment that was only guessed to be Staging.
    pub(crate) allow_node_shell: bool,
    pub(crate) debug_image: String,
    pub(crate) node_shell_namespace: String,
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
                confirm: None,
                default_namespace: None,
                allow_node_shell: None,
                debug_image: None,
                node_shell_namespace: None,
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
        let read_only = entry
            .and_then(|entry| entry.read_only)
            .unwrap_or(environment == Environment::Production);
        let confirm = entry
            .and_then(|entry| entry.confirm)
            .unwrap_or_else(|| ConfirmMode::for_environment(environment));
        // A guess must never switch a privileged pod on: Staging is also what an unknown name
        // falls back to, so it counts only when the entry says it.
        let allow_node_shell =
            entry
                .and_then(|entry| entry.allow_node_shell)
                .unwrap_or(match environment {
                    Environment::Development | Environment::Local => true,
                    Environment::Staging => entry.is_some_and(|entry| entry.environment.is_some()),
                    Environment::Production => false,
                });
        let stored_text = |text: Option<&String>| {
            text.map(|text| text.trim())
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        ClusterProfile {
            display_name,
            environment,
            default_namespace: entry.and_then(|entry| entry.default_namespace.clone()),
            read_only,
            confirm,
            allow_node_shell,
            debug_image: stored_text(entry.and_then(|entry| entry.debug_image.as_ref()))
                .unwrap_or_else(|| DEFAULT_DEBUG_IMAGE.to_owned()),
            node_shell_namespace: stored_text(
                entry.and_then(|entry| entry.node_shell_namespace.as_ref()),
            )
            .unwrap_or_else(|| DEFAULT_NODE_SHELL_NAMESPACE.to_owned()),
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

/// The namespace scope each cluster had when the user left it, for this app run only.
pub(crate) type ScopeMemory = HashMap<ClusterRef, NamespaceScope>;

pub(crate) fn remember_scope(memory: &mut ScopeMemory, cluster: ClusterRef, scope: NamespaceScope) {
    memory.insert(cluster, scope);
}

/// Where a session for `target` starts: the scope remembered for it, else the saved default
/// namespace, else `None` for the session's own default.
pub(crate) fn start_scope(
    memory: &ScopeMemory,
    target: &ClusterRef,
    profile: &ClusterProfile,
) -> Option<NamespaceScope> {
    memory.get(target).cloned().or_else(|| {
        let name = profile.default_namespace.as_ref()?;
        Some(NamespaceScope::of_namespaces([name.clone()]))
    })
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
