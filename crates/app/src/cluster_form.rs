//! What the Clusters page of Settings shows and edits, without any view code: the grouped rows,
//! field validation, registry edits, and the Test connection future.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cluster::{ClusterConnection, ClusterError, Kubeconfig};
use gpui_kit::SharedString;

use crate::cluster_catalog::{PathStyle, same_path_text};
use crate::cluster_registry::{
    ClusterEntry, ClusterProfile, ClusterRef, ClusterRegistry, switcher_label,
};
use crate::environment::{Environment, guess_environment};
use crate::kubeconfig_import::is_app_owned;

const MAX_DISPLAY_NAME_CHARS: usize = 64;
const MAX_NAMESPACE_CHARS: usize = 63;

pub(crate) const TEST_CONNECTION_TIMEOUT: Duration = Duration::from_secs(15);

/// Where a listed cluster comes from, which decides whether it can be removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowOrigin {
    /// `KUBECONFIG` or `--kubeconfig`: the user's environment, so only Reset applies.
    Chain,
    /// A file the user added by path.
    Registry,
    /// A file k8sBoard wrote under `<config>/kubeconfigs/` when the user pasted a kubeconfig.
    AppOwned,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClusterRow {
    pub(crate) cluster: ClusterRef,
    pub(crate) profile: ClusterProfile,
    /// The switcher text: the display name, plus the file name when another file has the name.
    pub(crate) label: String,
    /// `{auth kind} · {source file name}`.
    pub(crate) meta: String,
    /// What the names alone suggest: the "Auto" choice of the Environment control.
    pub(crate) guessed: Environment,
    pub(crate) origin: RowOrigin,
}

pub(crate) struct ClusterGroup {
    pub(crate) title: &'static str,
    pub(crate) rows: Vec<ClusterRow>,
}

/// Production, Staging, then Development and Local together (W2).
const GROUP_TITLES: [&str; 3] = ["Production", "Staging", "Development · Local"];

fn group_index(environment: Environment) -> usize {
    match environment {
        Environment::Production => 0,
        Environment::Staging => 1,
        Environment::Development | Environment::Local => 2,
    }
}

/// The rows of `kubeconfigs` (in load order) grouped by environment. Within a group the cluster
/// entries of the registry come first, in registry order, then the rest in load order. Empty
/// groups are skipped.
pub(crate) fn cluster_groups(
    kubeconfigs: &[&Kubeconfig],
    registry: &ClusterRegistry,
    is_chain_source: impl Fn(&Path) -> bool,
    owned_dir: Option<&Path>,
) -> Vec<ClusterGroup> {
    let summaries: Vec<_> = kubeconfigs
        .iter()
        .flat_map(|kubeconfig| {
            kubeconfig
                .contexts()
                .iter()
                .map(move |summary| (*kubeconfig, summary))
        })
        .collect();
    let mut rows: Vec<ClusterRow> = summaries
        .iter()
        .map(|(kubeconfig, summary)| {
            let profile = registry.profile(summary);
            let is_duplicate_name = summaries
                .iter()
                .any(|(_, other)| other.name == summary.name && other.source != summary.source);
            let source = summary.source.as_path();
            let origin = if is_chain_source(source) {
                RowOrigin::Chain
            } else if owned_dir.is_some_and(|dir| is_app_owned(source, dir)) {
                RowOrigin::AppOwned
            } else {
                RowOrigin::Registry
            };
            let auth = kubeconfig.connection_info(summary).auth;
            ClusterRow {
                cluster: ClusterRef::of(summary),
                label: switcher_label(&profile, summary, is_duplicate_name),
                meta: format!("{auth} · {}", file_name_text(&source.to_string_lossy())),
                guessed: guess_environment(&summary.name, &summary.cluster),
                profile,
                origin,
            }
        })
        .collect();
    // A stable sort keeps load order among the rows without an entry.
    rows.sort_by_key(|row| {
        registry
            .clusters
            .iter()
            .position(|entry| entry.cluster == row.cluster)
            .unwrap_or(usize::MAX)
    });
    let mut groups: Vec<ClusterGroup> = GROUP_TITLES
        .iter()
        .map(|title| ClusterGroup {
            title,
            rows: Vec::new(),
        })
        .collect();
    for row in rows {
        groups[group_index(row.profile.environment)].rows.push(row);
    }
    groups.retain(|group| !group.rows.is_empty());
    groups
}

/// The text after the last `/` or backslash. A string split, not `Path::file_name`: a Windows
/// path read on another host has no backslash separator for `Path`.
pub(crate) fn file_name_text(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// `1 cluster · 1 kubeconfig file`, `10 clusters · 2 kubeconfig files`.
pub(crate) fn count_text(clusters: usize, files: usize) -> String {
    let plural = |count: usize| if count == 1 { "" } else { "s" };
    format!(
        "{clusters} cluster{} · {files} kubeconfig file{}",
        plural(clusters),
        plural(files)
    )
}

/// A message shown under a field in the danger color; the value is not saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FieldError(pub(crate) SharedString);

/// `Ok(None)` means empty: the context name is shown. The text is trimmed.
pub(crate) fn validate_display_name(
    text: &str,
    cluster: &ClusterRef,
    rows: &[ClusterRow],
) -> Result<Option<String>, FieldError> {
    let name = text.trim();
    if name.is_empty() {
        return Ok(None);
    }
    if name.chars().count() > MAX_DISPLAY_NAME_CHARS {
        return Err(FieldError("Use at most 64 characters.".into()));
    }
    if name.chars().any(char::is_control) {
        return Err(FieldError("Remove line breaks and tabs.".into()));
    }
    let lowered = name.to_lowercase();
    let is_taken = rows
        .iter()
        .any(|row| row.cluster != *cluster && row.label.to_lowercase() == lowered);
    if is_taken {
        return Err(FieldError(
            format!("Another cluster is already shown as '{name}'.").into(),
        ));
    }
    Ok(Some(name.to_owned()))
}

/// A DNS-1123 label, or empty (`Ok(None)`: no default namespace). Hand-written: not worth a
/// regex for one rule.
pub(crate) fn validate_namespace(text: &str) -> Result<Option<String>, FieldError> {
    let name = text.trim();
    if name.is_empty() {
        return Ok(None);
    }
    let is_alphanumeric = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    let is_label = name.len() <= MAX_NAMESPACE_CHARS
        && name.chars().all(|c| is_alphanumeric(c) || c == '-')
        && name.starts_with(is_alphanumeric)
        && name.ends_with(is_alphanumeric);
    if !is_label {
        return Err(FieldError(
            "Use 1–63 lowercase letters, digits, or '-', starting and ending with a letter or digit."
                .into(),
        ));
    }
    Ok(Some(name.to_owned()))
}

/// Applies `edit` to the entry of `cluster`, which a missing one creates. An entry left with no
/// override is dropped again, so editing a field back to its default leaves no trace.
pub(crate) fn edit_entry(
    registry: &mut ClusterRegistry,
    cluster: &ClusterRef,
    edit: impl FnOnce(&mut ClusterEntry),
) {
    edit(registry.entry_mut(cluster));
    registry.clusters.retain(|entry| {
        entry.cluster != *cluster
            || entry.display_name.is_some()
            || entry.environment.is_some()
            || entry.read_only.is_some()
            || entry.default_namespace.is_some()
    });
}

/// The cluster the page shows: `selected` while a row still has it, else `preferred`, else the
/// first row.
pub(crate) fn resolve_selection(
    selected: Option<&ClusterRef>,
    preferred: Option<&ClusterRef>,
    groups: &[ClusterGroup],
) -> Option<ClusterRef> {
    let rows = || groups.iter().flat_map(|group| &group.rows);
    [selected, preferred]
        .into_iter()
        .flatten()
        .find(|cluster| rows().any(|row| row.cluster == **cluster))
        .cloned()
        .or_else(|| rows().next().map(|row| row.cluster.clone()))
}

/// The title and body of the dialog that confirms removing the kubeconfig file `path`.
pub(crate) fn remove_dialog_text(
    path: &Path,
    rows: &[ClusterRow],
    origin: RowOrigin,
) -> (String, String) {
    let title = format!(
        "Remove {} from k8sBoard?",
        file_name_text(&path.to_string_lossy())
    );
    let labels: Vec<&str> = rows
        .iter()
        .filter(|row| row.cluster.kubeconfig == path)
        .map(|row| row.label.as_str())
        .collect();
    let (noun, verb) = if labels.len() == 1 {
        ("cluster", "leaves")
    } else {
        ("clusters", "leave")
    };
    let fate = match origin {
        RowOrigin::AppOwned => "k8sBoard created this file when you pasted it; it will be deleted.",
        RowOrigin::Chain | RowOrigin::Registry => "The file itself is not changed.",
    };
    let body = format!(
        "{} {noun} from this file {verb} the list: {}. {fate}",
        labels.len(),
        labels.join(", ")
    );
    (title, body)
}

/// Drops the entry of `cluster`: name, environment, lock, and namespace go back to the defaults.
pub(crate) fn reset_entry(registry: &mut ClusterRegistry, cluster: &ClusterRef) {
    registry.clusters.retain(|entry| entry.cluster != *cluster);
}

/// Drops the path, every entry with that `kubeconfig`, and a matching `last_used`.
pub(crate) fn remove_kubeconfig(registry: &mut ClusterRegistry, path: &Path) {
    let text = path.to_string_lossy();
    let is_path = |other: &Path| same_path_text(&other.to_string_lossy(), &text, PathStyle::HOST);
    registry.kubeconfigs.retain(|file| !is_path(file));
    registry
        .clusters
        .retain(|entry| !is_path(&entry.cluster.kubeconfig));
    if registry
        .last_used
        .as_ref()
        .is_some_and(|cluster| is_path(&cluster.kubeconfig))
    {
        registry.last_used = None;
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum TestState {
    #[default]
    Idle,
    Running,
    Connected {
        version: String,
        latency_ms: u64,
    },
    Failed(SharedString),
}

/// Runs on the tokio runtime. `timeout` covers opening the client and reading the version; the
/// latency covers the version request only. A failure is the top-level error text, never its
/// source chain.
pub(crate) async fn test_connection(
    kubeconfig: Arc<Kubeconfig>,
    context: String,
    timeout: Duration,
) -> TestState {
    let attempt = async {
        let connection = ClusterConnection::open(&kubeconfig, &context).await?;
        let started = Instant::now();
        let version = connection.server_version().await?;
        Ok::<_, ClusterError>((version, started.elapsed()))
    };
    match tokio::time::timeout(timeout, attempt).await {
        Ok(Ok((version, elapsed))) => TestState::Connected {
            version: version.git_version,
            latency_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        },
        Ok(Err(error)) => TestState::Failed(error.to_string().into()),
        Err(_elapsed) => {
            let error = ClusterError::TimedOut {
                context,
                action: "testing the connection",
            };
            TestState::Failed(error.to_string().into())
        }
    }
}

#[cfg(test)]
#[path = "cluster_form_tests.rs"]
mod cluster_form_tests;
