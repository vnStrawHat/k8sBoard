//! What the Clusters page of Settings shows and edits, without any view code: the grouped rows,
//! field validation, registry edits, and the Test connection future.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cluster::{ClusterError, ConnectionInfo, Kubeconfig, ProxyChoice, ProxyUrl, ProxyUrlError};
use gpui_kit::SharedString;

use crate::cluster_catalog::{PathStyle, same_path_text};
use crate::cluster_registry::{
    ClusterEntry, ClusterProfile, ClusterProxy, ClusterRef, ClusterRegistry, open_cluster,
    switcher_label,
};
use crate::cluster_switcher_rows::{normalize_query, search_text};
use crate::environment::{ClusterColor, Environment, guess_environment};
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
    /// A file of a watched folder: the folder is the source of truth, so only stopping the watch or
    /// deleting the file takes the row away.
    Folder,
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
    /// For a row of a watched folder: where it comes from and what its file would read and run, so
    /// the user sees that before choosing it (`folder_trust_note`).
    pub(crate) trust_note: Option<String>,
}

#[derive(Clone)]
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
    folder_of: impl Fn(&Path) -> Option<PathBuf>,
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
            let folder = folder_of(source);
            let origin = if is_chain_source(source) {
                RowOrigin::Chain
            } else if owned_dir.is_some_and(|dir| is_app_owned(source, dir)) {
                RowOrigin::AppOwned
            } else if folder.is_some() {
                RowOrigin::Folder
            } else {
                RowOrigin::Registry
            };
            let info = kubeconfig.connection_info(summary);
            let trust_note = folder
                .as_ref()
                .filter(|_| origin == RowOrigin::Folder)
                .map(|folder| folder_trust_note(folder, &info));
            let auth = info.auth;
            ClusterRow {
                cluster: ClusterRef::of(summary),
                label: switcher_label(&profile, summary, is_duplicate_name),
                meta: format!("{auth} · {}", file_name_text(&source.to_string_lossy())),
                guessed: guess_environment(&summary.name, &summary.cluster),
                profile,
                origin,
                trust_note,
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
            || entry.confirm.is_some()
            || entry.default_namespace.is_some()
            || entry.allow_node_shell.is_some()
            || entry.debug_image.is_some()
            || entry.node_shell_namespace.is_some()
            || entry.color.is_some()
            || entry.proxy.is_some()
            || entry.metrics.is_some()
    });
}

/// Whether `row` matches the search text, by the switcher's rule: its name, context, environment
/// badge, and file name, with whitespace and case ignored.
pub(crate) fn cluster_matches(row: &ClusterRow, text: &str) -> bool {
    let haystack = search_text(
        &row.label,
        &row.cluster.context,
        row.profile.environment,
        &row.cluster.kubeconfig.to_string_lossy(),
    );
    haystack.contains(&normalize_query(text))
}

/// The groups of `groups` that keep at least one row matching `text`, with only those rows.
pub(crate) fn filter_groups(groups: &[ClusterGroup], text: &str) -> Vec<ClusterGroup> {
    groups
        .iter()
        .filter_map(|group| {
            let rows: Vec<ClusterRow> = group
                .rows
                .iter()
                .filter(|row| cluster_matches(row, text))
                .cloned()
                .collect();
            (!rows.is_empty()).then_some(ClusterGroup {
                title: group.title,
                rows,
            })
        })
        .collect()
}

/// What to store when the user picks `color`: nothing when it is the color of the cluster's
/// environment, so a cluster on that color keeps following its environment.
pub(crate) fn color_to_store(
    color: ClusterColor,
    environment: Environment,
) -> Option<ClusterColor> {
    (color != ClusterColor::of(environment)).then_some(color)
}

/// Moves `from` to the place of `to` inside `group` (display order). Every row of the group gets an
/// entry, then the group's entries move to the end of `registry.clusters` in the new order; the
/// other groups keep their relative order. Nothing happens when `from == to` or either is outside
/// the group.
pub(crate) fn move_cluster(
    registry: &mut ClusterRegistry,
    group: &ClusterGroup,
    from: &ClusterRef,
    to: &ClusterRef,
) {
    let mut order: Vec<ClusterRef> = group.rows.iter().map(|row| row.cluster.clone()).collect();
    let position = |cluster: &ClusterRef| order.iter().position(|other| other == cluster);
    let (Some(from_index), Some(to_index)) = (position(from), position(to)) else {
        return;
    };
    if from_index == to_index {
        return;
    }
    let moved = order.remove(from_index);
    order.insert(to_index, moved);
    for cluster in &order {
        registry.entry_mut(cluster);
    }
    let (mut moving, mut kept): (Vec<_>, Vec<_>) = std::mem::take(&mut registry.clusters)
        .into_iter()
        .partition(|entry| order.contains(&entry.cluster));
    moving.sort_by_key(|entry| order.iter().position(|cluster| *cluster == entry.cluster));
    kept.append(&mut moving);
    registry.clusters = kept;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MoveStep {
    Up,
    Down,
}

/// Moves `cluster` one place up or down inside `group`; nothing happens at the ends.
pub(crate) fn step_cluster(
    registry: &mut ClusterRegistry,
    group: &ClusterGroup,
    cluster: &ClusterRef,
    step: MoveStep,
) {
    let Some(index) = group.rows.iter().position(|row| row.cluster == *cluster) else {
        return;
    };
    let target = match step {
        MoveStep::Up => index.checked_sub(1),
        MoveStep::Down => Some(index + 1),
    };
    let Some(target) = target.and_then(|target| group.rows.get(target)) else {
        return;
    };
    move_cluster(registry, group, cluster, &target.cluster);
}

/// What the Proxy control of the Connection section offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProxyMode {
    /// The kubeconfig's own `proxy-url`, else none.
    FromKubeconfig,
    Direct,
    Custom,
}

/// The mode the control shows: a stored URL is Custom, and so is a pick whose URL is not
/// committed yet.
pub(crate) fn proxy_mode(stored: Option<&ClusterProxy>, is_custom_picked: bool) -> ProxyMode {
    match stored {
        Some(ClusterProxy::Url(_)) => ProxyMode::Custom,
        _ if is_custom_picked => ProxyMode::Custom,
        Some(ClusterProxy::Direct) => ProxyMode::Direct,
        None => ProxyMode::FromKubeconfig,
    }
}

/// `kubeconfig_proxy` is the kubeconfig's `proxy-url` as `scheme://host[:port]`, never with userinfo.
pub(crate) fn proxy_mode_label(mode: ProxyMode, kubeconfig_proxy: Option<&str>) -> String {
    match mode {
        ProxyMode::FromKubeconfig => {
            format!("From kubeconfig ({})", kubeconfig_proxy.unwrap_or("none"))
        }
        ProxyMode::Direct => "None (direct)".to_owned(),
        ProxyMode::Custom => "Custom URL".to_owned(),
    }
}

/// What the URL input starts with: the stored URL as `scheme://host:port`, or nothing when the
/// stored text does not parse, so that raw text (which a hand edit could fill with a password)
/// is never shown.
pub(crate) fn proxy_input_prefill(stored: Option<&ClusterProxy>) -> String {
    match stored {
        Some(ClusterProxy::Url(text)) => ProxyUrl::parse(text)
            .map(|url| url.display())
            .unwrap_or_default(),
        Some(ClusterProxy::Direct) | None => String::new(),
    }
}

/// Whether the typed text differs from what is stored, so that the `Not applied` note shows. A
/// stored URL that does not parse counts as nothing stored.
pub(crate) fn is_proxy_pending(stored: Option<&ClusterProxy>, typed: &str) -> bool {
    let shown = |text: &str| ProxyUrl::parse(text).map(|url| url.display()).ok();
    let applied = match stored {
        Some(ClusterProxy::Url(text)) => shown(text),
        Some(ClusterProxy::Direct) | None => None,
    };
    applied.is_none() || applied != shown(typed)
}

/// What committing the typed text means: `Ok(None)` for blank text (nothing to store), the URL to
/// store, or the message under the field. The URL is stored as `scheme://host:port`.
pub(crate) fn validate_proxy_url(text: &str) -> Result<Option<ClusterProxy>, FieldError> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    match ProxyUrl::parse(text) {
        Ok(url) => Ok(Some(ClusterProxy::Url(url.display()))),
        Err(error) => Err(FieldError(proxy_error_text(error).into())),
    }
}

fn proxy_error_text(error: ProxyUrlError) -> &'static str {
    match error {
        ProxyUrlError::Scheme => "Use http:// or socks5://.",
        ProxyUrlError::Credentials => {
            "Leave out the user name and password: k8sBoard does not store proxy credentials. Put them in the kubeconfig proxy-url instead."
        }
        ProxyUrlError::Host => "Add the proxy host.",
        ProxyUrlError::Port => "Use a port from 1 to 65535.",
        ProxyUrlError::Path => "Remove the path; a proxy URL is scheme://host:port.",
        ProxyUrlError::TooLong | ProxyUrlError::Malformed => {
            "Enter a URL such as http://proxy.example:3128."
        }
    }
}

/// Adds `folder` to the watched folders unless the registry has it already (`same_path_text`).
/// Returns whether it was added.
pub(crate) fn add_watched_folder(registry: &mut ClusterRegistry, folder: PathBuf) -> bool {
    let folder = std::path::absolute(&folder).unwrap_or(folder);
    let text = folder.to_string_lossy();
    let is_known = registry
        .kubeconfig_folders
        .iter()
        .any(|known| same_path_text(&known.to_string_lossy(), &text, PathStyle::HOST));
    if !is_known {
        registry.kubeconfig_folders.push(folder);
    }
    !is_known
}

/// Stops watching `folder`. Only the registry changes: nothing in the folder is touched.
pub(crate) fn stop_watching_folder(registry: &mut ClusterRegistry, folder: &Path) {
    let folder = std::path::absolute(folder).unwrap_or_else(|_| folder.to_path_buf());
    let text = folder.to_string_lossy();
    registry
        .kubeconfig_folders
        .retain(|known| !same_path_text(&known.to_string_lossy(), &text, PathStyle::HOST));
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
        RowOrigin::Chain | RowOrigin::Registry | RowOrigin::Folder => {
            "The file itself is not changed."
        }
    };
    let body = format!(
        "{} {noun} from this file {verb} the list: {}. {fate}",
        labels.len(),
        labels.join(", ")
    );
    (title, body)
}

/// What a row of a watched folder would do when chosen: its file is not the user's own, so the
/// files it reads for a credential and the command it runs are named up front (a dropped file
/// can aim `tokenFile` anywhere and name any plugin).
pub(crate) fn folder_trust_note(folder: &Path, info: &ConnectionInfo) -> String {
    let mut note = format!("From watched folder {}", folder.display());
    let mut parts = Vec::new();
    if !info.credential_files.is_empty() {
        parts.push(format!("reads {}", info.credential_files.join(", ")));
    }
    if let Some(command) = &info.exec_command {
        parts.push(format!("runs {command}"));
    }
    if parts.is_empty() {
        parts.push("reads no credential file, runs no command".to_owned());
    }
    note.push_str(": ");
    note.push_str(&parts.join(" / "));
    note
}

/// Why Remove from k8sBoard is off for `row`, or `None` when it applies. `folder` is the watched folder
/// the row's file belongs to.
pub(crate) fn remove_block_reason(row: &ClusterRow, folder: Option<&Path>) -> Option<String> {
    match row.origin {
        RowOrigin::Chain => {
            Some("Comes from KUBECONFIG or ~/.kube/config; edit that instead.".to_owned())
        }
        RowOrigin::Folder => Some(format!(
            "Comes from the watched folder {}; stop watching it or delete the file.",
            folder.map_or_else(String::new, |folder| folder.display().to_string())
        )),
        RowOrigin::Registry | RowOrigin::AppOwned => None,
    }
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
        registry.last_used_stamp = None;
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
    proxy: Result<ProxyChoice, ProxyUrlError>,
    timeout: Duration,
) -> TestState {
    let attempt = async {
        let connection = open_cluster(&kubeconfig, &context, &proxy).await?;
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
