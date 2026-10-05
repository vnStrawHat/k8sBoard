//! What the command palette lists and how a query ranks it. Everything here is pure over
//! borrowed, in-memory data: building the entries takes no session handle, no `cx`, and no
//! connection, so opening the palette or typing in it cannot start a list, a watch, or a request.
//! An entry holds only a kind, a namespace, a name, and a status label, never cell text, labels,
//! sections, or values, and a query is never stored or traced here.

use std::cmp::Reverse;
use std::ops::Range;

use cluster::{
    NamespaceScope, NamespaceSummary, NodeSummary, ObjectKind, PodSummary, ReplicaSetSummary,
};
use gpui_kit::{Action, SharedString};

use crate::app_shell::Screen;
use crate::cluster_registry::ClusterRef;
use crate::cluster_session::scope_includes;
use crate::cluster_switcher::SwitchToCluster1;
use crate::cluster_switcher_rows::{SwitcherRow, SwitcherSection};
use crate::fuzzy_score::{fuzzy_ranges, fuzzy_score};
use crate::issue_feeds::IssueFeeds;
use crate::keymap::{OpenKindPalette, OpenPalette, ShortcutGroup, shortcut_rows};
use crate::kind_row::{KindObject, KindRow};
use crate::navigation::{KindAvailability, kind_availability};
use crate::resource_actions::{
    KeyAvailability, ResourceAction, RowAction, action_label, is_planned, key_availability_of,
    needs_confirm, subject_action,
};
use crate::resource_kind::ResourceKind;
use crate::settings_window::ImportKubeconfig;
use crate::status_tone::{StatusLabel, node_status_label, pod_status_label};
use crate::table_selection::{ClusterObject, ResourceKey};
use crate::workload_actions::{
    NOT_LOADED_REASON, RevisionTarget, RollBackChoice, roll_back_choice, row_block, state_label,
};
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
    RowAction(RowAction),
    /// Roll back of the cursor Deployment to the revision the entry names, which the shell runs
    /// through its confirm dialog. It carries the revision, so it is not a plain row action.
    RollBack(ClusterObject, RevisionTarget),
    /// A row action on a search hit: the object becomes the cursor, then the action's key runs on
    /// it (`run_row_action_on`), so the gate and the confirm are the key's own.
    ObjectAction(ClusterObject, RowAction),
    Screen(Screen),
    /// An object of one viewed cluster, which the shell reveals in that cluster.
    Resource(ClusterObject),
    Namespace(NamespaceScope),
    /// A row of the cluster switcher (the cluster, its environment, health, and `Ctrl n` number)
    /// and the scope the switch carries; `None` keeps the target's own start scope (0026).
    Cluster(SwitcherRow, Option<NamespaceScope>),
}

// Cloned when an entry is confirmed, so the palette can close before the target runs.
impl Clone for PaletteTarget {
    fn clone(&self) -> Self {
        match self {
            Self::Command(action) => Self::Command(action.boxed_clone()),
            Self::RowAction(action) => Self::RowAction(*action),
            Self::RollBack(object, revision) => Self::RollBack(object.clone(), revision.clone()),
            Self::ObjectAction(object, action) => Self::ObjectAction(object.clone(), *action),
            Self::Screen(screen) => Self::Screen(*screen),
            Self::Resource(key) => Self::Resource(key.clone()),
            Self::Namespace(scope) => Self::Namespace(scope.clone()),
            Self::Cluster(row, scope) => Self::Cluster(row.clone(), scope.clone()),
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
    /// Text shown where the detail would be when there is none; the query never matches it.
    pub(crate) note: Option<SharedString>,
    /// Words the query matches besides the label and the detail: kind aliases, a cluster's context.
    pub(crate) keywords: Vec<SharedString>,
    pub(crate) status: Option<StatusLabel>,
    pub(crate) state: EntryState,
    /// The screen, namespace scope, or cluster that is open now.
    pub(crate) is_current: bool,
    /// Enabled, and its action's gate is `Mutating`: the row shows `needs confirm`.
    pub(crate) needs_confirm: bool,
    /// The query score `palette_entries` already computed (the `All`-mode resource entries);
    /// `ranked` uses it instead of scoring again. `None`: not scored yet.
    pub(crate) score: Option<u32>,
    pub(crate) target: PaletteTarget,
}

impl PaletteEntry {
    fn new(group: PaletteGroup, label: impl Into<SharedString>, target: PaletteTarget) -> Self {
        Self {
            group,
            label: label.into(),
            detail: None,
            note: None,
            keywords: Vec::new(),
            status: None,
            state: EntryState::Enabled,
            is_current: false,
            needs_confirm: false,
            score: None,
            target,
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        matches!(self.state, EntryState::Enabled)
    }
}

/// The objects of one live condition feed (Deployments, DaemonSets, Jobs, HPAs, PDBs, quotas,
/// claims, TLS Secrets), which the Issues engine already holds.
pub(crate) struct FeedObjects<'a> {
    pub(crate) kind: ResourceKind,
    pub(crate) objects: &'a [KindObject],
}

/// The feeds a search may read: only those whose list is live and loaded.
pub(crate) fn live_feed_objects(feeds: &IssueFeeds) -> Vec<FeedObjects<'_>> {
    feeds
        .conditions
        .iter()
        .filter_map(|feed| {
            Some(FeedObjects {
                kind: feed.kind,
                objects: feed.live_objects()?,
            })
        })
        .collect()
}

/// The loaded lists of the open cluster the palette may read. All slices are what the shell
/// already holds.
pub(crate) struct PaletteSession<'a> {
    pub(crate) cluster: ClusterRef,
    pub(crate) scope: &'a NamespaceScope,
    /// The guard of the cluster the palette acts on; its access report gates every row.
    pub(crate) guard: &'a ClusterGuard<'a>,
    pub(crate) namespaces: &'a [NamespaceSummary],
    pub(crate) pods: &'a [PodSummary],
    pub(crate) nodes: &'a [NodeSummary],
    /// The rows of the visible explorer kind; `None` on every other screen.
    pub(crate) kind_rows: Option<(ResourceKind, &'a [KindRow])>,
    /// The ReplicaSets of the cursor Deployment, once its drawer has loaded them: the palette
    /// offers `Roll back to rev {n}` only from these, and starts no list for it.
    pub(crate) replica_sets: Option<&'a [ReplicaSetSummary]>,
    /// The condition feeds that are live and loaded, searched by name (spec 0056); none of them
    /// starts a request.
    pub(crate) feeds: &'a [FeedObjects<'a>],
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
    /// The query text the resource entries are scored against, once (`PaletteEntry::score`).
    pub(crate) query_text: &'a str,
    /// The query text when pairs apply (`lists_pairs`); `None` builds none.
    pub(crate) pair_text: Option<&'a str>,
    /// The open cluster, when it is live.
    pub(crate) session: Option<PaletteSession<'a>>,
    pub(crate) clusters: &'a [SwitcherSection],
}

/// The row actions in the order of the shortcut sheet.
const ROW_ACTIONS: [RowAction; 26] = [
    RowAction::ViewLogs,
    RowAction::ViewYaml,
    RowAction::CopyName,
    RowAction::OpenShell,
    RowAction::PortForward,
    RowAction::Attach,
    RowAction::Cordon,
    RowAction::Drain,
    RowAction::EditTaints,
    RowAction::EditLabels,
    RowAction::EditYaml,
    RowAction::EditValues,
    RowAction::RestartRollout,
    RowAction::RestartPod,
    RowAction::EvictPod,
    RowAction::Scale,
    RowAction::Delete,
    RowAction::PauseRollout,
    RowAction::RollBack,
    RowAction::SuspendCronJob,
    RowAction::TriggerCronJob,
    RowAction::RerunJob,
    RowAction::EditHpaRange,
    RowAction::ExpandClaim,
    RowAction::SetDefaultStorageClass,
    RowAction::RenewCertificate,
];

/// Everything the palette may show, in source order, from in-memory state only. `scan_loaded_rows`
/// states the cost of one rebuild.
#[cfg_attr(feature = "hotpath-profiling", hotpath::measure)]
pub(crate) fn palette_entries(input: &PaletteInput<'_>) -> Vec<PaletteEntry> {
    let scan = scan_loaded_rows(input);
    let mut entries = Vec::new();
    entries.extend(row_action_entries(input));
    entries.extend(pair_entries(input, &scan.pair_subjects));
    entries.extend(command_entries(input));
    entries.extend(scan.resources);
    entries.extend(screen_entries(input));
    if let Some(session) = &input.session {
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
            .session
            .as_ref()
            .filter(|session| session.cluster == cursor.cluster)
    });
    let pod = cursor
        .zip(session)
        .and_then(|(cursor, session)| session.pods.iter().find(|pod| cursor.key.is_pod(pod)));
    // The row of the cursor, when its kind list is the one loaded: its state flips a label and can
    // block an action (a paused Deployment does not restart).
    let object = cursor.zip(session).and_then(|(cursor, session)| {
        let (kind, rows) = session.kind_rows?;
        rows.iter()
            .find(|row| ResourceKey::of_row(kind, row) == cursor.key)
            .map(|row| &row.object)
    });
    ROW_ACTIONS.into_iter().filter_map(move |row| {
        let subject = &cursor?.key;
        let session = session?;
        let action = subject_action(row, subject)?;
        let state = match key_availability_of(row, subject, pod, session.guard) {
            KeyAvailability::NotOffered => return None,
            KeyAvailability::Run(_) => {
                let block = match object {
                    Some(object) => row_block(action, object, session.replica_sets),
                    // A cursor row the loaded list does not hold has no revisions loaded either.
                    None if row == RowAction::RollBack => Some(NOT_LOADED_REASON.into()),
                    None => None,
                };
                match block {
                    Some(reason) => EntryState::Disabled { reason },
                    None => EntryState::Enabled,
                }
            }
            KeyAvailability::Disabled { reason } => EntryState::Disabled { reason },
        };
        let mut label: SharedString = match object {
            Some(object) => state_label(action, action_label(action), object).into(),
            None => action_label(action).into(),
        };
        let mut target = PaletteTarget::RowAction(row);
        // Roll back names the revision it goes to, and runs it: only from revisions that are loaded.
        if let (Some(KindObject::Deployment(deployment)), RowAction::RollBack, EntryState::Enabled) =
            (object, row, &state)
            && let RollBackChoice::To(revision) =
                roll_back_choice(deployment, session.replica_sets)
        {
            label = format!("Roll back to rev {}", revision.revision).into();
            target = PaletteTarget::RollBack(cursor?.clone(), revision);
        }
        let mut entry = PaletteEntry::new(PaletteGroup::Actions, label, target);
        entry.detail = Some(subject_text(subject).into());
        entry.needs_confirm = matches!(state, EntryState::Enabled) && needs_confirm(action);
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

/// One loaded object the palette can search or act on, borrowed from the session lists.
#[derive(Clone, Copy)]
enum Subject<'a> {
    Pod(&'a PodSummary),
    Node(&'a NodeSummary),
    Row(ResourceKind, &'a KindRow),
    /// An object known by its name only: a condition feed object, no status, no action pairs.
    Named {
        kind: ResourceKind,
        namespace: Option<&'a str>,
        name: &'a str,
    },
}

impl<'a> Subject<'a> {
    fn name(self) -> &'a str {
        match self {
            Self::Pod(pod) => &pod.name,
            Self::Node(node) => &node.name,
            Self::Row(_, row) => &row.name,
            Self::Named { name, .. } => name,
        }
    }

    fn namespace(self) -> Option<&'a str> {
        match self {
            Self::Pod(pod) => Some(&pod.namespace),
            Self::Node(_) => None,
            Self::Row(_, row) => row.namespace.as_deref(),
            Self::Named { namespace, .. } => namespace,
        }
    }

    /// The kind words a query matches besides the name: `deployment`, `deployments`.
    fn words(self) -> [&'static str; 2] {
        match self {
            Self::Pod(_) => ["pod", "pods"],
            Self::Node(_) => ["node", "nodes"],
            Self::Row(kind, _) | Self::Named { kind, .. } => [kind.singular(), kind.plural()],
        }
    }

    /// `None` for a name-only object: its list is not the one on screen, so no status is known.
    fn status(self) -> Option<StatusLabel> {
        match self {
            Self::Pod(pod) => Some(pod_status_label(pod)),
            Self::Node(node) => Some(node_status_label(node.status)),
            Self::Row(_, row) => Some(row.status.clone()),
            Self::Named { .. } => None,
        }
    }

    fn key(self) -> ResourceKey {
        match self {
            Self::Pod(pod) => ResourceKey::of_pod(pod),
            Self::Node(node) => ResourceKey::of_node(node),
            Self::Row(kind, row) => ResourceKey::of_row(kind, row),
            Self::Named {
                kind,
                namespace,
                name,
            } => ResourceKey::Kind {
                kind,
                namespace: namespace.map(str::to_owned),
                name: name.to_owned(),
            },
        }
    }

    /// Whether `key` names this object, without building a key.
    fn is(self, key: &ResourceKey) -> bool {
        match self {
            Self::Pod(pod) => key.is_pod(pod),
            Self::Node(node) => key.is_node(node),
            Self::Row(kind, row) => key.is_row(kind, row),
            Self::Named {
                kind,
                namespace,
                name,
            } => {
                matches!(key, ResourceKey::Kind { kind: key_kind, namespace: key_namespace, name: key_name }
                if *key_kind == kind && key_namespace.as_deref() == namespace && key_name == name)
            }
        }
    }

    /// The text a query matches: the name, `namespace/name` (empty for a node), and the kind
    /// words. `buffer` holds the joined text, so scoring an object allocates nothing once it has
    /// grown.
    fn fields<'b>(self, buffer: &'b mut String) -> [&'b str; 4]
    where
        'a: 'b,
    {
        buffer.clear();
        if let Some(namespace) = self.namespace() {
            buffer.push_str(namespace);
            buffer.push('/');
            buffer.push_str(self.name());
        }
        let [singular, plural] = self.words();
        [self.name(), buffer.as_str(), singular, plural]
    }

    /// The best field score of each token, `None` for a token that matches no field. `scores` is
    /// reused across objects.
    fn token_scores(self, tokens: &[&str], buffer: &mut String, scores: &mut Vec<Option<u32>>) {
        let fields = self.fields(buffer);
        scores.clear();
        scores.extend(tokens.iter().map(|token| {
            fields
                .iter()
                .filter_map(|field| fuzzy_score(token, field))
                .max()
        }));
    }

    /// Whether each token matches one of this object's fields.
    fn token_hits(self, tokens: &[&str], buffer: &mut String) -> Vec<bool> {
        let fields = self.fields(buffer);
        tokens
            .iter()
            .map(|token| {
                fields
                    .iter()
                    .any(|field| fuzzy_score(token, field).is_some())
            })
            .collect()
    }
}

/// The sum of the scores of the tokens that matched; the scores of `eligible` tokens only when a
/// mask is given, and then `None` when none of those matched.
fn score_sum(scores: &[Option<u32>], eligible: Option<&[bool]>) -> Option<u32> {
    let mut sum = None;
    for (index, score) in scores.iter().enumerate() {
        let is_eligible = eligible.is_none_or(|mask| mask[index]);
        if let (Some(score), true) = (score, is_eligible) {
            sum = Some(sum.unwrap_or(0_u32).saturating_add(*score));
        }
    }
    sum
}

/// The actions a pair can carry, for `labelled_tokens`: the labels of `subject_action` that any
/// subject can offer, and the two that an object state flips (`state_label`).
const PAIR_LABEL_ACTIONS: [ResourceAction; 24] = [
    ResourceAction::ViewLogs,
    ResourceAction::OpenShell,
    ResourceAction::PortForward,
    ResourceAction::Attach,
    ResourceAction::OpenNodeShell,
    ResourceAction::DebugContainer,
    ResourceAction::Cordon,
    ResourceAction::Drain,
    ResourceAction::EditTaints,
    ResourceAction::EditLabels,
    ResourceAction::CopyName,
    ResourceAction::ViewYaml,
    ResourceAction::EditYaml(ObjectKind::Pod),
    ResourceAction::EditValues(ObjectKind::ConfigMap),
    ResourceAction::RestartRollout(ObjectKind::Deployment),
    ResourceAction::Scale(ObjectKind::Deployment),
    ResourceAction::PauseRollout,
    ResourceAction::SuspendCronJob,
    ResourceAction::TriggerCronJob,
    ResourceAction::RerunJob,
    ResourceAction::EditHpaRange,
    ResourceAction::ExpandClaim,
    ResourceAction::SetDefaultStorageClass,
    ResourceAction::RenewCertificate,
];
const STATE_LABELS: [&str; 2] = ["Resume rollout", "Resume"];

/// Which tokens an object may be picked for when it is a pair candidate. A token that matches the
/// label of some pairable action names the action, so an object that only matches that token
/// (every `logstash-*` pod for `logs`) would crowd out the object the other tokens name. The
/// other tokens are the eligible ones; when every token reads as an action label, all are.
fn object_tokens(tokens: &[&str]) -> Vec<bool> {
    let is_label = |token: &str| {
        PAIR_LABEL_ACTIONS
            .iter()
            .map(|action| action_label(*action))
            .chain(STATE_LABELS)
            .any(|label| fuzzy_score(token, label).is_some())
    };
    let on_label: Vec<bool> = tokens.iter().map(|token| is_label(token)).collect();
    if on_label.iter().all(|is_on_label| *is_on_label) {
        return vec![true; tokens.len()];
    }
    on_label
        .into_iter()
        .map(|is_on_label| !is_on_label)
        .collect()
}

/// The visible explorer kind first (it is what the user is looking at), then pods, then nodes, then
/// the objects of the live condition feeds that the scope includes. A feed of the visible kind is
/// left out once that list has rows: they hold the same objects, with their status.
fn subjects<'a>(session: &PaletteSession<'a>) -> impl Iterator<Item = Subject<'a>> + use<'a> {
    // The slices are copied out, so the iterator borrows the lists and not the session value.
    let (pods, nodes, scope, feeds) = (session.pods, session.nodes, session.scope, session.feeds);
    let shown_kind = session
        .kind_rows
        .filter(|(_, rows)| !rows.is_empty())
        .map(|(kind, _)| kind);
    let rows = session
        .kind_rows
        .into_iter()
        .flat_map(|(kind, rows)| rows.iter().map(move |row| Subject::Row(kind, row)));
    let fed = feeds
        .iter()
        .filter(move |feed| Some(feed.kind) != shown_kind)
        .flat_map(move |feed| {
            feed.objects.iter().filter_map(move |object| {
                let (namespace, name) = condition_identity(object)?;
                scope_includes(scope, namespace).then_some(Subject::Named {
                    kind: feed.kind,
                    namespace: Some(namespace),
                    name,
                })
            })
        });
    rows.chain(pods.iter().map(Subject::Pod))
        .chain(nodes.iter().map(Subject::Node))
        .chain(fed)
}

/// The namespace and name a condition feed summary carries.
fn condition_identity(object: &KindObject) -> Option<(&str, &str)> {
    let (namespace, name) = match object {
        KindObject::Deployment(item) => (&item.namespace, &item.name),
        KindObject::DaemonSet(item) => (&item.namespace, &item.name),
        KindObject::Job(item) => (&item.namespace, &item.name),
        KindObject::HorizontalPodAutoscaler(item) => (&item.namespace, &item.name),
        KindObject::PodDisruptionBudget(item) => (&item.namespace, &item.name),
        KindObject::ResourceQuota(item) => (&item.namespace, &item.name),
        KindObject::PersistentVolumeClaim(item) => (&item.namespace, &item.name),
        KindObject::Secret(item) => (&item.namespace, &item.name),
        _ => return None,
    };
    Some((namespace, name))
}

/// The Resources entry of an object: its name and status only, never cell text.
fn resource_entry(session: &PaletteSession<'_>, subject: Subject<'_>) -> PaletteEntry {
    let object = ClusterObject::new(session.cluster.clone(), subject.key());
    let mut entry = PaletteEntry::new(
        PaletteGroup::Resources,
        subject.name().to_owned(),
        PaletteTarget::Resource(object),
    );
    entry.detail = subject
        .namespace()
        .map(|namespace| format!("{namespace}/{}", subject.name()).into());
    entry.keywords = subject
        .words()
        .into_iter()
        .map(SharedString::from)
        .collect();
    entry.status = subject.status();
    entry
}

/// What the one scan of the loaded rows found.
struct Scan<'a> {
    /// The `All`-mode resource entries that matched every token, scored.
    resources: Vec<PaletteEntry>,
    /// The best objects for pairs (at most `RESOURCES_CAP`), matching at least one token.
    pair_subjects: Vec<Subject<'a>>,
}

/// Scans the loaded rows once for the resource entries and the pair objects.
///
/// Per keystroke this is tokens x rows x 4 `fuzzy_score` calls (the name, `namespace/name`, and the
/// two kind words), byte compares with no allocation: about 60,000 for 5,000 rows and 3 tokens. It
/// runs on every query change; a shell notify with an unchanged query is throttled to one per
/// `SHELL_REFRESH_INTERVAL` (`command_palette.rs`). The `palette ranked` trace measures it
/// against the 4 ms budget. Pair building afterwards is bounded by `RESOURCES_CAP` objects x
/// `ROW_ACTIONS`. Only a row that matched every token builds a resource entry.
///
/// ponytail: top-50 objects by one linear scan; an index per token if traces exceed the budget.
fn scan_loaded_rows<'a>(input: &PaletteInput<'a>) -> Scan<'a> {
    let mut scan = Scan {
        resources: Vec::new(),
        pair_subjects: Vec::new(),
    };
    let Some(session) = &input.session else {
        return scan;
    };
    let wants_pairs = input.pair_text.is_some();
    if !input.include_resources && !wants_pairs {
        return scan;
    }
    let tokens: Vec<&str> = input.query_text.split_whitespace().collect();
    // Once per scan, not per row.
    let eligible = if wants_pairs {
        object_tokens(&tokens)
    } else {
        Vec::new()
    };
    let mut hits: Vec<(u32, Subject<'_>)> = Vec::new();
    let mut buffer = String::new();
    let mut scores = Vec::with_capacity(tokens.len());
    for subject in subjects(session) {
        subject.token_scores(&tokens, &mut buffer, &mut scores);
        // Name-only objects have no row to act on, so they never carry pairs.
        if wants_pairs
            && !matches!(subject, Subject::Named { .. })
            && let Some(score) = score_sum(&scores, Some(&eligible))
            && !input
                .cursor
                .is_some_and(|cursor| cursor.cluster == session.cluster && subject.is(&cursor.key))
        {
            hits.push((score, subject));
        }
        if input.include_resources && scores.iter().all(Option::is_some) {
            let mut entry = resource_entry(session, subject);
            entry.score = Some(score_sum(&scores, None).unwrap_or(0));
            scan.resources.push(entry);
        }
    }
    // Stable, so equal scores keep the source order.
    hits.sort_by_key(|(score, _)| Reverse(*score));
    hits.truncate(RESOURCES_CAP);
    scan.pair_subjects = hits.into_iter().map(|(_, subject)| subject).collect();
    scan
}

/// The row actions a pair may carry: Delete acts on the ticked set (decision 26), Roll back needs
/// the revisions only the cursor Deployment's drawer loads, and Restart pod and Evict (spec 0040)
/// are cursor entries only, since `rest pay` would otherwise list a refused Restart pod for every
/// bare pod.
fn is_pairable(row: RowAction) -> bool {
    !matches!(
        row,
        RowAction::Delete | RowAction::RollBack | RowAction::RestartPod | RowAction::EvictPod
    )
}

/// Pairs apply to `All` or `Actions` mode with two or more tokens: one names the action, another
/// the object (decision 24).
pub(crate) fn lists_pairs(query: &PaletteQuery<'_>) -> bool {
    matches!(query.mode, PaletteMode::All | PaletteMode::Actions)
        && query.text.split_whitespace().nth(1).is_some()
}

/// The action × object entries of the best objects (`> rest pay` → `Restart rollout ·
/// deployment/payments-api`). Pure over the input, like every source here.
fn pair_entries(input: &PaletteInput<'_>, subjects: &[Subject<'_>]) -> Vec<PaletteEntry> {
    let (Some(text), Some(session)) = (input.pair_text, &input.session) else {
        return Vec::new();
    };
    let tokens: Vec<&str> = text.split_whitespace().collect();
    subjects
        .iter()
        .flat_map(|subject| subject_pairs(session, *subject, &tokens))
        .collect()
}

/// The pairs of one object: every shipped row action it offers whose label takes one token while
/// the object takes another. The state follows the gate of the action's key, read now and again
/// when the pair runs.
fn subject_pairs(
    session: &PaletteSession<'_>,
    subject: Subject<'_>,
    tokens: &[&str],
) -> Vec<PaletteEntry> {
    let key = subject.key();
    let object = ClusterObject::new(session.cluster.clone(), key.clone());
    let pod = match subject {
        Subject::Pod(pod) => Some(pod),
        Subject::Node(_) | Subject::Row(..) | Subject::Named { .. } => None,
    };
    // The loaded row: its state flips a label and can block an action (a paused Deployment).
    let loaded = match subject {
        Subject::Row(_, row) => Some(&row.object),
        Subject::Pod(_) | Subject::Node(_) | Subject::Named { .. } => None,
    };
    let on_object = subject.token_hits(tokens, &mut String::new());
    let namespace = subject
        .namespace()
        .filter(|_| !matches!(session.scope, NamespaceScope::Named(_)));
    ROW_ACTIONS
        .into_iter()
        .filter(|row| is_pairable(*row))
        .filter_map(|row| {
            let action = subject_action(row, &key)?;
            // An action that has not shipped can never run.
            if is_planned(action) {
                return None;
            }
            let label = match loaded {
                Some(object) => state_label(action, action_label(action), object),
                None => action_label(action),
            };
            if !is_pair(tokens, label, &on_object) {
                return None;
            }
            let (state, can_run) = match key_availability_of(row, &key, pod, session.guard) {
                KeyAvailability::NotOffered => return None,
                KeyAvailability::Disabled { reason } => (EntryState::Disabled { reason }, None),
                KeyAvailability::Run(action) => {
                    match loaded.and_then(|object| row_block(action, object, None)) {
                        Some(reason) => (EntryState::Disabled { reason }, None),
                        None => (EntryState::Enabled, Some(action)),
                    }
                }
            };
            let detail = match namespace {
                Some(namespace) => format!("{} · {namespace}", subject_text(&key)),
                None => subject_text(&key),
            };
            let mut entry = PaletteEntry::new(
                PaletteGroup::Actions,
                label,
                PaletteTarget::ObjectAction(object.clone(), row),
            );
            entry.detail = Some(detail.into());
            entry.state = state;
            entry.needs_confirm = can_run.is_some_and(needs_confirm);
            Some(entry)
        })
        .collect()
}

/// Decision 24: every token matches the label or the object, some token matches the label, and a
/// different one matches the object. `on_object` says which tokens the object matches.
fn is_pair(tokens: &[&str], label: &str, on_object: &[bool]) -> bool {
    let on_label: Vec<bool> = tokens
        .iter()
        .map(|token| fuzzy_score(token, label).is_some())
        .collect();
    let covers_every_token = on_label
        .iter()
        .zip(on_object)
        .all(|(label, object)| *label || *object);
    let splits = on_label.iter().enumerate().any(|(index, is_on_label)| {
        *is_on_label
            && on_object
                .iter()
                .enumerate()
                .any(|(other, is_on_object)| *is_on_object && other != index)
    });
    covers_every_token && splits
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
        if let Some(session) = &input.session
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

/// The cluster rows. A row of another cluster carries the live scope when it is one named
/// namespace (decision 31): `All` and several namespaces carry nothing, so the target starts in its
/// own remembered scope, and a switch to the active cluster does nothing anyway.
fn cluster_entries(input: &PaletteInput<'_>) -> Vec<PaletteEntry> {
    let carried = input
        .session
        .as_ref()
        .and_then(|session| match session.scope {
            NamespaceScope::Named(name) => Some(name),
            NamespaceScope::All | NamespaceScope::Several(_) => None,
        });
    input
        .clusters
        .iter()
        .flat_map(|section| &section.rows)
        .map(|row| {
            let namespace = carried.filter(|_| !row.is_active);
            let scope = namespace.map(|name| NamespaceScope::Named(name.clone()));
            let mut entry = PaletteEntry::new(
                PaletteGroup::GoTo,
                row.label.clone(),
                PaletteTarget::Cluster(row.clone(), scope),
            );
            entry.note = namespace.map(|name| format!("same namespace {name}").into());
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
        PaletteMode::Clusters => matches!(entry.target, PaletteTarget::Cluster(..)),
        PaletteMode::Namespaces => matches!(entry.target, PaletteTarget::Namespace(_)),
        PaletteMode::Actions => entry.group == PaletteGroup::Actions,
    }
}

/// The text a query matches: the label, the detail when there is one, then the keywords.
fn score_fields(entry: &PaletteEntry) -> Vec<&str> {
    let mut fields: Vec<&str> = vec![&entry.label];
    fields.extend(entry.detail.as_deref());
    fields.extend(entry.keywords.iter().map(SharedString::as_ref));
    fields
}

fn score_of(entry: &PaletteEntry, text: &str) -> Option<u32> {
    if text.is_empty() {
        return Some(0);
    }
    entry_score(text, &score_fields(entry))
}

/// The byte ranges to underline in an entry's label and detail.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EntryRanges {
    pub(crate) label: Vec<Range<usize>>,
    pub(crate) detail: Vec<Range<usize>>,
}

/// Each token underlines only the field that gave its best score, in the order `score_fields`
/// lists them (the first field wins a tie). A token whose best field is a keyword underlines
/// nothing: keywords are not drawn. Called for the shown rows only, never while ranking.
pub(crate) fn entry_match_ranges(entry: &PaletteEntry, query_text: &str) -> EntryRanges {
    let fields = score_fields(entry);
    let has_detail = entry.detail.is_some();
    let mut ranges = EntryRanges::default();
    for token in query_text.split_whitespace() {
        let mut best: Option<(usize, u32)> = None;
        for (index, field) in fields.iter().enumerate() {
            let Some(score) = fuzzy_score(token, field) else {
                continue;
            };
            if best.is_none_or(|(_, best_score)| score > best_score) {
                best = Some((index, score));
            }
        }
        let Some((index, _)) = best else {
            continue;
        };
        let target = match index {
            0 => &mut ranges.label,
            1 if has_detail => &mut ranges.detail,
            _ => continue,
        };
        if let Some(found) = fuzzy_ranges(token, fields[index]) {
            target.extend(found);
        }
    }
    merge_overlapping(&mut ranges.label);
    merge_overlapping(&mut ranges.detail);
    ranges
}

/// Sorts the ranges and joins those that overlap or touch, which two tokens on one field can make.
fn merge_overlapping(ranges: &mut Vec<Range<usize>>) {
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges.drain(..) {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    *ranges = merged;
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
        .filter_map(|entry| Some((entry.score.or_else(|| score_of(&entry, query.text))?, entry)))
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
/// `searched_feeds` are the condition feeds that were live (`live_feed_objects`).
pub(crate) fn empty_text(
    mode: PaletteMode,
    has_session: bool,
    screen: Screen,
    searched_feeds: &[ResourceKind],
) -> String {
    if !has_session && matches!(mode, PaletteMode::All | PaletteMode::Namespaces) {
        return "No matches. Cluster not connected.".to_owned();
    }
    match mode {
        PaletteMode::All => format!("No matches. {}", resources_hint(screen, searched_feeds)),
        PaletteMode::Kinds => "No matching kind.".to_owned(),
        PaletteMode::Clusters => "No matching cluster.".to_owned(),
        PaletteMode::Namespaces => "No matching namespace.".to_owned(),
        PaletteMode::Actions => "No matching action.".to_owned(),
    }
}

/// Which lists the Resources group searched (decision 9): Pods, Nodes, the visible kind, then the
/// live condition feeds. A feed that is loading or off is not named, for it was not searched.
fn resources_hint(screen: Screen, searched_feeds: &[ResourceKind]) -> String {
    let visible = screen.kind();
    let mut searched = vec!["Pods", "Nodes"];
    searched.extend(visible.map(ResourceKind::label));
    searched.extend(
        searched_feeds
            .iter()
            .filter(|kind| Some(**kind) != visible)
            .map(|kind| kind.label()),
    );
    format!(
        "Searched: {}. Type :kind for other kinds.",
        searched.join(", ")
    )
}

#[cfg(test)]
#[path = "palette_search_tests.rs"]
mod palette_search_tests;
