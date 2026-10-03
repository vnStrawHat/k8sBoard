//! What the command palette lists and how a query ranks it. Everything here is pure over
//! borrowed, in-memory data: building the entries takes no session handle, no `cx`, and no
//! connection, so opening the palette or typing in it cannot start a list, a watch, or a request.
//! An entry holds only a kind, a namespace, a name, and a status label, never cell text, labels,
//! sections, or values, and a query is never stored or traced here.

use std::cmp::Reverse;

use cluster::{NamespaceScope, NamespaceSummary, NodeSummary, PodSummary};
use gpui_kit::{Action, SharedString};

use crate::app_shell::Screen;
use crate::cluster_registry::ClusterRef;
use crate::cluster_switcher::SwitchToCluster1;
use crate::cluster_switcher_rows::{SwitcherRow, SwitcherSection};
use crate::fuzzy_score::fuzzy_score;
use crate::keymap::{OpenKindPalette, OpenPalette, ShortcutGroup, shortcut_rows};
use crate::kind_row::KindRow;
use crate::navigation::{KindAvailability, kind_availability};
use crate::resource_actions::{
    KeyAvailability, ResourceAction, action_label, key_availability_of, subject_action,
};
use crate::resource_kind::ResourceKind;
use crate::settings_window::ImportKubeconfig;
use crate::status_tone::{StatusLabel, node_status_label, pod_status_label};
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::write_guard::ClusterGuard;

const ACTIONS_CAP: usize = 20;
const RESOURCES_CAP: usize = 50;
const GO_TO_CAP: usize = 30;

/// The reason of a dock command while the dock has no tab to act on.
const NO_DOCK_TABS_REASON: &str = "No dock tabs";

/// What the first character of the query selects (W9 note 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaletteMode {
    All,
    Kinds,
    Clusters,
    Namespaces,
    Actions,
}

pub(crate) struct PaletteQuery<'a> {
    pub(crate) mode: PaletteMode,
    pub(crate) text: &'a str,
}

/// The first character picks the mode (`:` `@` `#` `>`); the rest, trimmed, is the text. Leading
/// spaces are skipped before the prefix check.
pub(crate) fn parse_query(raw: &str) -> PaletteQuery<'_> {
    let raw = raw.trim_start();
    let (mode, text) = match raw.chars().next() {
        Some(':') => (PaletteMode::Kinds, &raw[1..]),
        Some('@') => (PaletteMode::Clusters, &raw[1..]),
        Some('#') => (PaletteMode::Namespaces, &raw[1..]),
        Some('>') => (PaletteMode::Actions, &raw[1..]),
        _ => (PaletteMode::All, raw),
    };
    PaletteQuery {
        mode,
        text: text.trim(),
    }
}

/// `text` splits on whitespace into tokens; every token must match some field, and the score is
/// the sum of each token's best field score. Two tokens may match the same field (`pay api` against
/// `payments-api`). An empty text scores every entry 0.
fn entry_score(text: &str, fields: &[&str]) -> Option<u32> {
    let mut total = 0_u32;
    for token in text.split_whitespace() {
        let best = fields
            .iter()
            .filter_map(|field| fuzzy_score(token, field))
            .max()?;
        total = total.saturating_add(best);
    }
    Some(total)
}

/// The groups in the order the palette draws them (W9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaletteGroup {
    Actions,
    Resources,
    GoTo,
}

impl PaletteGroup {
    pub(crate) const ALL: [Self; 3] = [Self::Actions, Self::Resources, Self::GoTo];

    pub(crate) fn heading(self) -> &'static str {
        match self {
            Self::Actions => "Actions",
            Self::Resources => "Resources",
            Self::GoTo => "Go to",
        }
    }

    /// The position in `ALL`.
    fn index(self) -> usize {
        match self {
            Self::Actions => 0,
            Self::Resources => 1,
            Self::GoTo => 2,
        }
    }

    fn cap(self) -> usize {
        match self {
            Self::Actions => ACTIONS_CAP,
            Self::Resources => RESOURCES_CAP,
            Self::GoTo => GO_TO_CAP,
        }
    }
}

/// What confirming an entry does. Running it is `command_palette`'s job.
pub(crate) enum PaletteTarget {
    /// A 0028 action, dispatched on the shell.
    Command(Box<dyn Action>),
    /// A row action on the cursor row, dispatched as the action of its key.
    RowAction(ResourceAction),
    Screen(Screen),
    /// An object of one viewed cluster, which the shell reveals in that cluster.
    Resource(ClusterObject),
    Namespace(NamespaceScope),
    /// A row of the cluster switcher: the cluster, its environment, health, and `Ctrl n` number.
    Cluster(SwitcherRow),
}

// Cloned when an entry is confirmed, so the palette can close before the target runs.
impl Clone for PaletteTarget {
    fn clone(&self) -> Self {
        match self {
            Self::Command(action) => Self::Command(action.boxed_clone()),
            Self::RowAction(action) => Self::RowAction(*action),
            Self::Screen(screen) => Self::Screen(*screen),
            Self::Resource(key) => Self::Resource(key.clone()),
            Self::Namespace(scope) => Self::Namespace(scope.clone()),
            Self::Cluster(row) => Self::Cluster(row.clone()),
        }
    }
}

pub(crate) enum EntryState {
    Enabled,
    Disabled { reason: SharedString },
}

/// One row of the palette.
pub(crate) struct PaletteEntry {
    pub(crate) group: PaletteGroup,
    pub(crate) label: SharedString,
    pub(crate) detail: Option<SharedString>,
    /// Words the query matches besides the label and the detail: kind aliases, a cluster's context.
    pub(crate) keywords: Vec<SharedString>,
    pub(crate) status: Option<StatusLabel>,
    pub(crate) state: EntryState,
    /// The screen, namespace scope, or cluster that is open now.
    pub(crate) is_current: bool,
    pub(crate) target: PaletteTarget,
}

impl PaletteEntry {
    fn new(group: PaletteGroup, label: impl Into<SharedString>, target: PaletteTarget) -> Self {
        Self {
            group,
            label: label.into(),
            detail: None,
            keywords: Vec::new(),
            status: None,
            state: EntryState::Enabled,
            is_current: false,
            target,
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        matches!(self.state, EntryState::Enabled)
    }
}

/// The loaded lists of one viewed cluster the palette may read. All slices are what the shell
/// already holds.
pub(crate) struct PaletteSession<'a> {
    pub(crate) cluster: ClusterRef,
    /// The cluster's switcher text, set only while several clusters are viewed: it tells equal
    /// names apart and is matched by the query.
    pub(crate) label: Option<SharedString>,
    /// The cluster the namespace scope and the kind availability are read from.
    pub(crate) is_primary: bool,
    pub(crate) scope: &'a NamespaceScope,
    /// The guard of the cluster the palette acts on; its access report gates every row.
    pub(crate) guard: &'a ClusterGuard<'a>,
    pub(crate) namespaces: &'a [NamespaceSummary],
    pub(crate) pods: &'a [PodSummary],
    pub(crate) nodes: &'a [NodeSummary],
    /// The rows of the visible explorer kind; `None` on every other screen.
    pub(crate) kind_rows: Option<(ResourceKind, &'a [KindRow])>,
}

/// Everything the palette may show, borrowed from the shell. No session means no resources, no
/// namespaces, and no row actions.
pub(crate) struct PaletteInput<'a> {
    pub(crate) screen: Screen,
    pub(crate) cursor: Option<&'a ClusterObject>,
    pub(crate) has_dock_tabs: bool,
    /// Whether to build the resource entries at all: only a query in `All` mode with text lists
    /// them, and thousands of pods are not worth building for any other query.
    pub(crate) include_resources: bool,
    /// Every viewed cluster that is live, in display order.
    pub(crate) sessions: Vec<PaletteSession<'a>>,
    pub(crate) clusters: &'a [SwitcherSection],
}

impl<'a> PaletteInput<'a> {
    /// The session the scope and the kind availability come from: the primary one, else the first.
    fn scope_session(&self) -> Option<&PaletteSession<'a>> {
        self.sessions
            .iter()
            .find(|session| session.is_primary)
            .or_else(|| self.sessions.first())
    }
}

/// The row actions in the order of the shortcut sheet.
const ROW_ACTIONS: [ResourceAction; 11] = [
    ResourceAction::ViewLogs,
    ResourceAction::ViewYaml,
    ResourceAction::CopyName,
    ResourceAction::OpenShell,
    ResourceAction::PortForward,
    ResourceAction::Cordon,
    ResourceAction::Drain,
    ResourceAction::EditYaml,
    ResourceAction::RestartRollout,
    ResourceAction::Scale,
    ResourceAction::Delete,
];

/// Everything the palette may show, in source order, from in-memory state only.
pub(crate) fn palette_entries(input: &PaletteInput<'_>) -> Vec<PaletteEntry> {
    let mut entries = Vec::new();
    entries.extend(row_action_entries(input));
    entries.extend(command_entries(input));
    if input.include_resources {
        entries.extend(input.sessions.iter().flat_map(resource_entries));
    }
    entries.extend(screen_entries(input));
    if let Some(session) = input.scope_session() {
        entries.extend(namespace_entries(session));
    }
    entries.extend(cluster_entries(input));
    entries
}

/// The row actions of the cursor row, gated by the cluster that row belongs to.
fn row_action_entries<'a>(input: &'a PaletteInput<'_>) -> impl Iterator<Item = PaletteEntry> + 'a {
    let cursor = input.cursor;
    let session = cursor.and_then(|cursor| {
        input
            .sessions
            .iter()
            .find(|session| session.cluster == cursor.cluster)
    });
    let pod = cursor
        .zip(session)
        .and_then(|(cursor, session)| session.pods.iter().find(|pod| cursor.key.is_pod(pod)));
    ROW_ACTIONS.into_iter().filter_map(move |action| {
        let subject = &cursor?.key;
        let session = session?;
        let state = match key_availability_of(action, subject, pod, session.guard) {
            KeyAvailability::NotOffered => return None,
            KeyAvailability::Run => EntryState::Enabled,
            KeyAvailability::Disabled { reason } => EntryState::Disabled { reason },
        };
        let mut entry = PaletteEntry::new(
            PaletteGroup::Actions,
            action_label(subject_action(action, subject)),
            PaletteTarget::RowAction(action),
        );
        entry.detail = Some(with_cluster(subject_text(subject), session).into());
        entry.state = state;
        Some(entry)
    })
}

/// `deployment/payments-api`: the kind word and the name of the cursor object.
fn subject_text(subject: &ResourceKey) -> String {
    match subject {
        ResourceKey::Pod { name, .. } => format!("pod/{name}"),
        ResourceKey::Node { name } => format!("node/{name}"),
        ResourceKey::Kind { kind, name, .. } => format!("{}/{name}", kind.singular()),
    }
}

/// The General and Dock rows of the shortcut sheet. Left out: the palette's own rows, the
/// Settings window's import (its handler is not on the shell, so a dispatch would do nothing), and
/// the 1–9 row (the clusters are listed under `@`).
fn command_entries(input: &PaletteInput<'_>) -> Vec<PaletteEntry> {
    shortcut_rows()
        .into_iter()
        .filter(|row| matches!(row.group, ShortcutGroup::General | ShortcutGroup::Dock))
        .filter(|row| {
            let action = row.action.as_any();
            !(action.is::<OpenPalette>()
                || action.is::<OpenKindPalette>()
                || action.is::<ImportKubeconfig>()
                || action.is::<SwitchToCluster1>())
        })
        .map(|row| {
            let is_dock = row.group == ShortcutGroup::Dock;
            let mut entry = PaletteEntry::new(
                PaletteGroup::Actions,
                row.label,
                PaletteTarget::Command(row.action),
            );
            if is_dock && !input.has_dock_tabs {
                entry.state = EntryState::Disabled {
                    reason: NO_DOCK_TABS_REASON.into(),
                };
            }
            entry
        })
        .collect()
}

/// `text`, and the cluster label after it while several clusters are viewed.
fn with_cluster(text: String, session: &PaletteSession<'_>) -> String {
    match &session.label {
        Some(label) => format!("{text} · {label}"),
        None => text,
    }
}

/// The visible explorer kind, pods, and nodes of one cluster: names and status only.
fn resource_entries<'a>(
    session: &'a PaletteSession<'_>,
) -> impl Iterator<Item = PaletteEntry> + 'a {
    let object = |key| PaletteTarget::Resource(ClusterObject::new(session.cluster.clone(), key));
    let pods = session.pods.iter().map(move |pod| {
        let mut entry = PaletteEntry::new(
            PaletteGroup::Resources,
            pod.name.clone(),
            object(ResourceKey::of_pod(pod)),
        );
        entry.detail =
            Some(with_cluster(format!("{}/{}", pod.namespace, pod.name), session).into());
        entry.keywords = vec!["pod".into(), "pods".into()];
        entry.status = Some(pod_status_label(pod));
        entry
    });
    let nodes = session.nodes.iter().map(move |node| {
        let mut entry = PaletteEntry::new(
            PaletteGroup::Resources,
            node.name.clone(),
            object(ResourceKey::of_node(node)),
        );
        entry.detail = session.label.clone();
        entry.keywords = vec!["node".into(), "nodes".into()];
        entry.status = Some(node_status_label(node.status));
        entry
    });
    let rows = session
        .kind_rows
        .into_iter()
        .flat_map(|(kind, rows)| rows.iter().map(move |row| (kind, row)))
        .map(move |(kind, row)| {
            let mut entry = PaletteEntry::new(
                PaletteGroup::Resources,
                row.name.clone(),
                object(ResourceKey::of_row(kind, row)),
            );
            entry.detail = match &row.namespace {
                Some(namespace) => {
                    Some(with_cluster(format!("{namespace}/{}", row.name), session).into())
                }
                None => session.label.clone(),
            };
            entry.keywords = vec![kind.singular().into(), kind.plural().into()];
            entry.status = Some(row.status.clone());
            entry
        });
    // The visible kind first: it is what the user is looking at.
    rows.chain(pods).chain(nodes)
}

/// Pods, Nodes, and every explorer kind, with the words `:` matches (plural, singular, short names).
fn screen_entries(input: &PaletteInput<'_>) -> Vec<PaletteEntry> {
    let screens = [
        (Screen::Pods, "Pods", &["pod", "pods", "po"][..]),
        (Screen::Nodes, "Nodes", &["node", "nodes", "no"][..]),
    ];
    let fixed = screens.into_iter().map(|(screen, label, words)| {
        let mut entry = PaletteEntry::new(PaletteGroup::GoTo, label, PaletteTarget::Screen(screen));
        entry.keywords = words.iter().map(|word| (*word).into()).collect();
        entry.is_current = screen == input.screen;
        entry
    });
    let kinds = ResourceKind::ALL.into_iter().map(|kind| {
        let screen = Screen::Kind(kind);
        let mut entry = PaletteEntry::new(
            PaletteGroup::GoTo,
            kind.label(),
            PaletteTarget::Screen(screen),
        );
        entry.keywords = [kind.plural(), kind.singular()]
            .into_iter()
            .chain(kind.short_names().iter().copied())
            .map(SharedString::from)
            .collect();
        entry.is_current = screen == input.screen;
        if let Some(session) = input.scope_session()
            && let KindAvailability::Denied { reason } =
                kind_availability(kind, session.guard.access, session.scope)
        {
            entry.state = EntryState::Disabled { reason };
        }
        entry
    });
    fixed.chain(kinds).collect()
}

fn namespace_entries(session: &PaletteSession<'_>) -> Vec<PaletteEntry> {
    let all = std::iter::once((NamespaceScope::All, "All namespaces".to_owned()));
    let named = session.namespaces.iter().map(|namespace| {
        (
            NamespaceScope::Named(namespace.name.clone()),
            namespace.name.clone(),
        )
    });
    all.chain(named)
        .map(|(scope, label)| {
            let mut entry = PaletteEntry::new(
                PaletteGroup::GoTo,
                label,
                PaletteTarget::Namespace(scope.clone()),
            );
            entry.is_current = scope == *session.scope;
            entry
        })
        .collect()
}

fn cluster_entries(input: &PaletteInput<'_>) -> Vec<PaletteEntry> {
    input
        .clusters
        .iter()
        .flat_map(|section| &section.rows)
        .map(|row| {
            let mut entry = PaletteEntry::new(
                PaletteGroup::GoTo,
                row.label.clone(),
                PaletteTarget::Cluster(row.clone()),
            );
            // The switcher's own search text: the context, the environment badge, the file name.
            entry.keywords = row.search_text.lines().map(SharedString::from).collect();
            entry.is_current = row.is_active;
            entry
        })
        .collect()
}

/// The entries a query keeps, best first, and how many a cap cut.
pub(crate) struct Ranked {
    pub(crate) entries: Vec<PaletteEntry>,
    pub(crate) more: usize,
}

/// Whether the mode (and an empty text) lists the entry at all. No prefix searches everything, but
/// an empty text shows only the actions: listing every pod unasked is noise.
fn is_listed(entry: &PaletteEntry, query: &PaletteQuery<'_>) -> bool {
    match query.mode {
        PaletteMode::All => entry.group == PaletteGroup::Actions || !query.text.is_empty(),
        PaletteMode::Kinds => matches!(entry.target, PaletteTarget::Screen(_)),
        PaletteMode::Clusters => matches!(entry.target, PaletteTarget::Cluster(_)),
        PaletteMode::Namespaces => matches!(entry.target, PaletteTarget::Namespace(_)),
        PaletteMode::Actions => entry.group == PaletteGroup::Actions,
    }
}

fn score_of(entry: &PaletteEntry, text: &str) -> Option<u32> {
    if text.is_empty() {
        return Some(0);
    }
    let mut fields: Vec<&str> = vec![&entry.label];
    fields.extend(entry.detail.as_deref());
    fields.extend(entry.keywords.iter().map(SharedString::as_ref));
    entry_score(text, &fields)
}

/// Filters by mode, scores, orders each group by score (equal scores keep source order), and cuts
/// each group to its cap.
///
/// An exact alias such as `po` or `deploy` outranks every other match through the scorer's exact
/// bonus, so `:po` always lists Pods first.
pub(crate) fn ranked(entries: Vec<PaletteEntry>, query: &PaletteQuery<'_>) -> Ranked {
    let mut scored: Vec<(u32, PaletteEntry)> = entries
        .into_iter()
        .filter(|entry| is_listed(entry, query))
        .filter_map(|entry| Some((score_of(&entry, query.text)?, entry)))
        .collect();
    // Stable, so equal scores keep the source order.
    scored.sort_by_key(|(score, _)| Reverse(*score));
    let mut buckets: [Vec<PaletteEntry>; 3] = Default::default();
    for (_, entry) in scored {
        buckets[entry.group.index()].push(entry);
    }
    let mut ranked = Ranked {
        entries: Vec::new(),
        more: 0,
    };
    for (group, mut bucket) in PaletteGroup::ALL.into_iter().zip(buckets) {
        ranked.more += bucket.len().saturating_sub(group.cap());
        bucket.truncate(group.cap());
        ranked.entries.extend(bucket);
    }
    ranked
}

/// Whether `query` lists resources at all: only text in `All` mode does (decision 6), so no other
/// query needs the resource entries built.
pub(crate) fn lists_resources(query: &PaletteQuery<'_>) -> bool {
    query.mode == PaletteMode::All && !query.text.is_empty()
}

/// What the list says when nothing matches: the group hint of decision 9, or why there is none.
pub(crate) fn empty_text(mode: PaletteMode, has_session: bool, screen: Screen) -> String {
    if !has_session && matches!(mode, PaletteMode::All | PaletteMode::Namespaces) {
        return "No matches. Cluster not connected.".to_owned();
    }
    match mode {
        PaletteMode::All => format!("No matches. {}", resources_hint(screen)),
        PaletteMode::Kinds => "No matching kind.".to_owned(),
        PaletteMode::Clusters => "No matching cluster.".to_owned(),
        PaletteMode::Namespaces => "No matching namespace.".to_owned(),
        PaletteMode::Actions => "No matching action.".to_owned(),
    }
}

/// Which lists the Resources group searched (decision 9).
fn resources_hint(screen: Screen) -> String {
    let visible = match screen {
        Screen::Kind(kind) => format!(", {}", kind.label()),
        _ => String::new(),
    };
    format!("Searched: Pods, Nodes{visible}. Type :kind to open another kind.")
}

#[cfg(test)]
#[path = "palette_search_tests.rs"]
mod palette_search_tests;
