use std::collections::HashMap;
use std::error::Error;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use cluster::{
    AccessCheck, AccessDecision, AccessReport, BindingSummary, ChangeEventKind, ClusterConnection,
    ClusterError, ConfigMapValues, ContextSummary, CrdSummary, CustomObjectFields,
    DeploymentSummary, EndpointSliceSummary, EventFilter, EventSummary, HelmRevision,
    IngressSummary, InvolvedObject, JobSummary, Kubeconfig, KubeletTargets, LimitRangeSummary,
    LogRequest, LogSource, LogUpdate, MetricsError, MetricsSource, MetricsSourceError,
    NamespaceAccess, NamespaceScope, NamespaceSummary, NodeSummary, ObjectKind,
    PersistentVolumeSummary, PodSummary, ProxyChoice, ProxyUrlError, RbacSnapshot,
    ReplicaSetSummary, ResourceQuotaSummary, SecretSummary, ServerVersion, ServiceSummary,
    SourceCheck, StorageClassSummary, TrafficCounter, TrafficMetricSource, WatchUpdate,
};
use futures::StreamExt as _;
use gpui_kit::{App, Context, Task};
use tokio::sync::watch;

use crate::cluster_metrics::{
    ClusterMetrics, NodesGate, PodReview, PodReviewResult, PodsGate, SourceState, TrafficSources,
    nodes_gate, pods_gate,
};
use crate::cluster_registry::open_cluster;
use crate::cluster_runtime::{ClusterRuntime, WatchSubscription};
use crate::crd_rows::crd_row;
use crate::custom_kind::{CustomKind, CustomKindCache, custom_kinds};
use crate::event_rows::warnings_first;
use crate::issue_board::{ISSUE_TICK, IssueBoard, IssueChange, IssueInputs, RunReason};
use crate::issue_feeds::{FeedState, IssueFeeds, core_coverage};
use crate::kind_access::{KindAccess, KindAccessMap, lazy_checks};
use crate::kind_join::{JoinInputs, join_rows};
use crate::kind_row::{KindObject, KindRow};
use crate::kubelet_metrics::KubeletDemand;
use crate::last_log::{LastLog, LastLogKey, LastLogs, last_line};
use crate::live_sections::CanDoCell;
use crate::name_index::{NameIndex, NameListResult, name_index_plan};
use crate::related_objects::RelatedSubject;
use crate::resource_kind::ResourceKind;
use crate::settings::AppSettings;
use crate::table_selection::ResourceKey;
use crate::topology_feeds::{
    FeedStart, SubjectChange, TopologyFeed, TopologyFeeds, TopologySubject, feed_plan,
    subject_change,
};
use crate::watched_kinds::{WatchedKind, merge_watched};
use crate::write_guard::{ClusterGuard, WriteLock};

/// The next connection generation. One counter for the whole run, so a session that replaces another
/// for the same cluster never repeats a number.
fn next_generation() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// One connected kubeconfig context: the connection, its live lists, and the access report.
/// Dropping the entity cancels every task and watch it owns.
pub(crate) struct ClusterSession {
    inputs: ConnectInputs,
    /// The context this session connects to, so the write guard can name its cluster.
    summary: ContextSummary,
    /// Whether this session may offer changes. Starts from the cluster's profile and toggles for
    /// the session only; a reconnect keeps it.
    lock: WriteLock,
    /// Names this connection: a new session and every reconnect take a fresh one, so a dry-run or a
    /// confirm dialog can tell that the connection it was started on is gone.
    generation: u64,
    phase: SessionPhase,
    /// The kind screen being shown, kept across Connecting and retry so that `LiveCluster::start`
    /// can start its watch.
    explorer_kind: Option<ResourceKind>,
    /// The kind whose `update` and `delete` permissions the shown screen needs (Edit YAML, Delete).
    /// Kept across
    /// Connecting and retry like `explorer_kind`.
    access_kind: Option<ObjectKind>,
    /// Which events the Events screen asks the server for. Kept across Connecting and retry like
    /// `explorer_kind`; a new session starts at `All`.
    event_filter: EventFilter,
    /// Every custom kind definition seen so far; moves to the next session on a context switch.
    custom_kind_cache: CustomKindCache,
    /// The problems found in the live lists. It lives here, not in `LiveCluster`, so a retry and a
    /// scope change keep the first-seen times; a context switch makes a new session and clears it.
    issues: IssueBoard,
    /// The Issues screen is shown, so a time-only refresh repaints it (the ages move).
    is_issues_visible: bool,
    /// The Overview screen is shown, so its change feeds run. Kept across Connecting and retry like
    /// `explorer_kind`.
    is_overview_visible: bool,
    /// Overview shows its 24 h range, so its change feeds also watch the ReplicaSets. Kept like
    /// `is_overview_visible`.
    keeps_rollout_history: bool,
    /// What Topology draws while it is shown: its feeds run for this subject. Kept across Connecting
    /// and retry like `explorer_kind`.
    topology_subject: Option<TopologySubject>,
    /// The stored metrics source this session follows (spec 0048); the live state is rebuilt when it
    /// changes. Kept across Connecting and retry like `explorer_kind`.
    source_entry: Option<Result<MetricsSource, MetricsSourceError>>,
    _issue_tick: Task<()>,
}

/// What `connect` needs, kept so that `retry` can run it again.
struct ConnectInputs {
    kubeconfig: Arc<Kubeconfig>,
    context: String,
    requested_namespace: Option<NamespaceScope>,
}

pub(crate) enum SessionPhase {
    /// Open, server version, and the initial scope choice.
    Connecting {
        _task: Task<()>,
    },
    Live(Box<LiveCluster>),
    Failed {
        message: String,
    },
}

pub(crate) struct LiveCluster {
    pub(crate) server_version: ServerVersion,
    /// How long the version request took during the connect; the status bar and the switcher show it.
    pub(crate) api_latency: Duration,
    /// Decided before the session is live, so it is never unknown.
    pub(crate) scope: NamespaceScope,
    pub(crate) access: AccessState,
    /// The lazy `update` answers of the kinds a screen has shown in this scope (spec 0031).
    pub(crate) kind_access: KindAccessMap,
    /// The RBAC snapshot of the analysis tools, listed on first need.
    pub(crate) rbac: RbacState,
    /// The Can do chips of the account whose drawer is open (`live_sections.rs`).
    pub(crate) can_do: CanDoCell,
    pub(crate) namespaces: LiveList<NamespaceSummary>,
    pub(crate) pods: LiveList<PodSummary>,
    pub(crate) nodes: LiveList<NodeSummary>,
    /// Pod and node usage, polled while the metrics API is reachable and allowed.
    pub(crate) metrics: ClusterMetrics,
    /// Every custom resource definition, watched for the whole session once the access review does
    /// not deny it. `None` before that and when the review denies the list.
    pub(crate) crds: Option<CrdWatch>,
    /// Cluster-wide instance counts of the custom kinds.
    pub(crate) custom_counts: CustomCounts,
    /// The per-resource list review of each custom kind shown in this scope. Cleared on a scope
    /// change.
    pub(crate) custom_gates: HashMap<CustomKind, CustomGate>,
    /// The watch of the visible kind screen; `None` on Pods and Nodes.
    explorer: Option<KindList>,
    /// The open drawer's events; `None` while no drawer needs them.
    object_events: Option<ObjectEvents>,
    /// The objects related to the open drawer (a Deployment's ReplicaSets); `None` while no
    /// drawer needs them.
    related: Option<RelatedObjects>,
    /// The last log lines of crash-looping containers the open Pod drawer quotes.
    last_logs: LastLogs,
    /// The sidebar numbers of kinds without a running watch.
    kind_counts: KindCounts,
    /// The names of the palette-only kinds (spec 0056), listed while the user types in the palette.
    pub(crate) name_index: NameIndex,
    /// The watches only the Issues engine reads, running for the whole session.
    pub(crate) issue_feeds: IssueFeeds,
    /// The Overview's change feeds: `Some` exactly while Overview is visible (`Denied` included).
    pub(crate) change_events: Option<ChangeEvents>,
    /// Whether the change feeds watch the ReplicaSets too (the 24 h range).
    keeps_rollout_history: bool,
    /// The watches behind the Topology graph: `Some` exactly while Topology is shown.
    topology: Option<TopologyFeeds>,
    connection: ClusterConnection,
    subscriptions: Subscriptions,
}

/// The RBAC snapshot behind Who can and Check permissions: one immutable listing per session,
/// fetched when a tool first needs it and replaced by a refresh. It is no watch, so it is not
/// counted in `open_watch_count`.
pub(crate) enum RbacState {
    Idle,
    Loading {
        _task: Task<()>,
    },
    Ready {
        snapshot: Rc<RbacSnapshot>,
        listed_at: jiff::Timestamp,
    },
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RbacTrigger {
    /// A tool opened and needs a snapshot.
    Request,
    /// The Refresh or Retry button.
    Refresh,
}

impl RbacState {
    /// The snapshot is for the old scope's fallback namespaces, so the next tool lists again.
    fn reset_for_scope_change(&mut self) {
        *self = Self::Idle;
    }
}

/// A request lists from Idle or Failed; a refresh from Ready or Failed; neither interrupts a
/// listing in flight.
fn starts_fetch(state: &RbacState, trigger: RbacTrigger) -> bool {
    match (state, trigger) {
        (RbacState::Idle, RbacTrigger::Request) => true,
        (RbacState::Ready { .. }, RbacTrigger::Refresh) => true,
        (RbacState::Failed(_), _) => true,
        (RbacState::Loading { .. }, _)
        | (RbacState::Idle, RbacTrigger::Refresh)
        | (RbacState::Ready { .. }, RbacTrigger::Request) => false,
    }
}

/// Whether new snapshots reach a list. A paused list keeps its rows; only the newest snapshot
/// that arrived meanwhile is held.
pub(crate) enum StreamFlow<T> {
    Live,
    Paused { held: Option<Vec<T>> },
}

impl<T> StreamFlow<T> {
    /// A failure still reaches a paused list, so an interruption shows.
    fn receive(&mut self, list: &mut LiveList<T>, update: WatchUpdate<T>) {
        match (self, update) {
            (Self::Paused { held }, WatchUpdate::Snapshot(items)) => *held = Some(items),
            (_, update) => list.apply(update),
        }
    }

    /// Holds `list` still from now on. Returns whether it started: only a loaded list can be held,
    /// and a paused one already is.
    fn pause(&mut self, list: &LiveList<T>) -> bool {
        if list.ready_items().is_none() || matches!(self, Self::Paused { .. }) {
            return false;
        }
        *self = Self::Paused { held: None };
        true
    }

    /// Shows the held snapshot, if one arrived, and goes live.
    fn resume(&mut self, list: &mut LiveList<T>) {
        if let Self::Paused { held: Some(items) } = std::mem::replace(self, Self::Live) {
            list.apply(WatchUpdate::Snapshot(items));
        }
    }
}

/// What the header of a pausable screen shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FlowState {
    Live,
    Paused { has_held: bool },
}

/// The CRD watch. The list holds the definitions; the explorer rows of the CRDs screen arrive with
/// each snapshot, built on tokio. Dropping it stops the watch.
pub(crate) struct CrdWatch {
    pub(crate) list: LiveList<CrdSummary>,
    /// The Established CRDs with a served version as kinds, sorted by (group, label).
    pub(crate) kinds: Vec<CustomKind>,
    _subscription: WatchSubscription,
}

/// One CRD watch update. A snapshot carries the definitions and the CRDs table rows built from
/// them.
enum CrdUpdate {
    Snapshot {
        crds: Vec<CrdSummary>,
        rows: Vec<KindRow>,
    },
    Failed(ClusterError),
}

/// The visible explorer kind's list. Dropping it stops the watch.
pub(crate) struct KindList {
    pub(crate) kind: ResourceKind,
    pub(crate) list: LiveList<KindRow>,
    flow: StreamFlow<KindRow>,
    /// A second watch whose lists fill cells of the rows; `None` when the kind needs none or the
    /// access report denies it.
    companion: Option<Companion>,
    /// `None` for CRDs: the session's CRD watch feeds that list.
    subscription: Option<WatchSubscription>,
}

/// The watch of a second kind that the explorer rows join with (the endpoint slices of the
/// Services screen). Dropping it stops the watch.
struct Companion {
    lists: CompanionLists,
    _subscription: WatchSubscription,
}

/// The latest state of the companion watch, by what it lists.
pub(crate) enum CompanionLists {
    EndpointSlices(LiveList<EndpointSliceSummary>),
    PersistentVolumes(LiveList<PersistentVolumeSummary>),
    /// The Ingresses of the Secrets screen: the TLS users of a secret.
    Ingresses(LiveList<IngressSummary>),
    /// The TLS secrets of the Ingresses screen: the certificates its TLS column reads.
    TlsSecrets(LiveList<SecretSummary>),
    /// `cluster_role_bindings` is `None` for a kind that does not need it (Roles). The companion
    /// only starts when the access report allows every list its kind needs.
    Bindings {
        role_bindings: LiveList<BindingSummary>,
        cluster_role_bindings: Option<LiveList<BindingSummary>>,
    },
}

/// One companion watch update, typed on tokio so one subscription serves every companion.
enum CompanionUpdate {
    EndpointSlices(WatchUpdate<EndpointSliceSummary>),
    PersistentVolumes(WatchUpdate<PersistentVolumeSummary>),
    Ingresses(WatchUpdate<IngressSummary>),
    TlsSecrets(WatchUpdate<SecretSummary>),
    RoleBindings(WatchUpdate<BindingSummary>),
    ClusterRoleBindings(WatchUpdate<BindingSummary>),
}

/// Which companion an explorer kind starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CompanionKind {
    EndpointSlices,
    PersistentVolumes,
    Ingresses,
    TlsSecrets,
    Bindings { with_cluster_role_bindings: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CompanionPlan {
    None,
    Start(CompanionKind),
    /// The access report denies the companion list; the drawer shows the check as the reason.
    Denied(AccessCheck),
}

/// Which companion `kind` starts, or why not. A review that is still running or failed does not
/// block it: the watch then shows its own failure.
pub(crate) fn companion_plan(kind: ResourceKind, access: &AccessState) -> CompanionPlan {
    if let Some(plan) = bindings_plan(kind, access) {
        return plan;
    }
    let (companion, check) = match kind {
        ResourceKind::Services => (
            CompanionKind::EndpointSlices,
            AccessCheck::ListEndpointSlices,
        ),
        ResourceKind::StorageClasses => (
            CompanionKind::PersistentVolumes,
            AccessCheck::ListPersistentVolumes,
        ),
        ResourceKind::Secrets => (CompanionKind::Ingresses, AccessCheck::ListIngresses),
        ResourceKind::Ingresses => (CompanionKind::TlsSecrets, AccessCheck::ListSecrets),
        _ => return CompanionPlan::None,
    };
    match access {
        AccessState::Known(report) if !report.is_allowed(check) => CompanionPlan::Denied(check),
        AccessState::Known(_) | AccessState::Checking { .. } | AccessState::Unknown => {
            CompanionPlan::Start(companion)
        }
    }
}

/// The lists the Bindings companion of `kind` needs: Roles need the role bindings; ClusterRoles
/// and ServiceAccounts also the cluster role bindings. `None` for a kind without this companion.
fn bindings_checks(kind: ResourceKind) -> Option<&'static [AccessCheck]> {
    match kind {
        ResourceKind::Roles => Some(&[AccessCheck::ListRoleBindings]),
        ResourceKind::ClusterRoles | ResourceKind::ServiceAccounts => Some(&[
            AccessCheck::ListRoleBindings,
            AccessCheck::ListClusterRoleBindings,
        ]),
        _ => None,
    }
}

/// The needed binding lists the access report denies. A companion with a missing list could never
/// be ready (counts would be wrong), so it does not start while any is denied.
pub(crate) fn denied_binding_checks(kind: ResourceKind, access: &AccessState) -> Vec<AccessCheck> {
    let AccessState::Known(report) = access else {
        return Vec::new();
    };
    bindings_checks(kind)
        .into_iter()
        .flatten()
        .copied()
        .filter(|check| !report.is_allowed(*check))
        .collect()
}

fn bindings_plan(kind: ResourceKind, access: &AccessState) -> Option<CompanionPlan> {
    bindings_checks(kind)?;
    if let Some(check) = denied_binding_checks(kind, access).first() {
        return Some(CompanionPlan::Denied(*check));
    }
    Some(CompanionPlan::Start(CompanionKind::Bindings {
        with_cluster_role_bindings: kind != ResourceKind::Roles,
    }))
}

/// Where a drawer reads its companion lists from; see `LiveCluster::companion_source`.
pub(crate) enum CompanionSource<'a> {
    Explorer(&'a CompanionLists),
    /// Rebuilt from the Topology feeds on every read.
    Topology(CompanionLists),
}

impl CompanionSource<'_> {
    pub(crate) fn lists(&self) -> &CompanionLists {
        match self {
            Self::Explorer(lists) => lists,
            Self::Topology(lists) => lists,
        }
    }
}

impl CompanionLists {
    fn loading_for(kind: CompanionKind) -> Self {
        match kind {
            CompanionKind::EndpointSlices => Self::EndpointSlices(LiveList::Loading),
            CompanionKind::PersistentVolumes => Self::PersistentVolumes(LiveList::Loading),
            CompanionKind::Ingresses => Self::Ingresses(LiveList::Loading),
            CompanionKind::TlsSecrets => Self::TlsSecrets(LiveList::Loading),
            CompanionKind::Bindings {
                with_cluster_role_bindings,
            } => Self::Bindings {
                role_bindings: LiveList::Loading,
                cluster_role_bindings: with_cluster_role_bindings.then_some(LiveList::Loading),
            },
        }
    }

    /// An update of another variant is ignored: a stale one cannot reach a new companion.
    fn apply(&mut self, update: CompanionUpdate) {
        match (self, update) {
            (Self::EndpointSlices(list), CompanionUpdate::EndpointSlices(update)) => {
                list.apply(update);
            }
            (Self::PersistentVolumes(list), CompanionUpdate::PersistentVolumes(update)) => {
                list.apply(update);
            }
            (Self::Ingresses(list), CompanionUpdate::Ingresses(update)) => list.apply(update),
            (Self::TlsSecrets(list), CompanionUpdate::TlsSecrets(update)) => list.apply(update),
            (Self::Bindings { role_bindings, .. }, CompanionUpdate::RoleBindings(update)) => {
                role_bindings.apply(update);
            }
            (
                Self::Bindings {
                    cluster_role_bindings: Some(list),
                    ..
                },
                CompanionUpdate::ClusterRoleBindings(update),
            ) => list.apply(update),
            // A stale update of another companion's kind, or of a list that is not started.
            _ => {}
        }
    }

    fn mark_stopped(&mut self) {
        match self {
            Self::EndpointSlices(list) => list.mark_stopped(),
            Self::PersistentVolumes(list) => list.mark_stopped(),
            Self::Ingresses(list) => list.mark_stopped(),
            Self::TlsSecrets(list) => list.mark_stopped(),
            Self::Bindings {
                role_bindings,
                cluster_role_bindings,
            } => {
                role_bindings.mark_stopped();
                if let Some(list) = cluster_role_bindings {
                    list.mark_stopped();
                }
            }
        }
    }

    /// The endpoint slices, when this companion lists them.
    pub(crate) fn endpoint_slices(&self) -> Option<&LiveList<EndpointSliceSummary>> {
        match self {
            Self::EndpointSlices(list) => Some(list),
            Self::PersistentVolumes(_)
            | Self::Ingresses(_)
            | Self::TlsSecrets(_)
            | Self::Bindings { .. } => None,
        }
    }

    /// The persistent volumes, when this companion lists them.
    pub(crate) fn persistent_volumes(&self) -> Option<&LiveList<PersistentVolumeSummary>> {
        match self {
            Self::PersistentVolumes(list) => Some(list),
            Self::EndpointSlices(_)
            | Self::Ingresses(_)
            | Self::TlsSecrets(_)
            | Self::Bindings { .. } => None,
        }
    }

    /// The ingresses, when this companion lists them.
    pub(crate) fn ingresses(&self) -> Option<&LiveList<IngressSummary>> {
        match self {
            Self::Ingresses(list) => Some(list),
            Self::EndpointSlices(_)
            | Self::PersistentVolumes(_)
            | Self::TlsSecrets(_)
            | Self::Bindings { .. } => None,
        }
    }

    /// The TLS secrets, when this companion lists them.
    pub(crate) fn tls_secrets(&self) -> Option<&LiveList<SecretSummary>> {
        match self {
            Self::TlsSecrets(list) => Some(list),
            Self::EndpointSlices(_)
            | Self::PersistentVolumes(_)
            | Self::Ingresses(_)
            | Self::Bindings { .. } => None,
        }
    }

    /// Whether the first snapshot has not arrived.
    #[cfg(feature = "screenshot")]
    fn is_loading(&self) -> bool {
        match self {
            Self::EndpointSlices(list) => list.is_loading(),
            Self::PersistentVolumes(list) => list.is_loading(),
            Self::Ingresses(list) => list.is_loading(),
            Self::TlsSecrets(list) => list.is_loading(),
            Self::Bindings {
                role_bindings,
                cluster_role_bindings,
            } => {
                role_bindings.is_loading()
                    || cluster_role_bindings
                        .as_ref()
                        .is_some_and(LiveList::is_loading)
            }
        }
    }

    /// The kinds the companion watches, with how many watches each takes (the same watches as
    /// `watches`).
    fn watched(&self, namespaces: usize) -> Vec<(&'static str, usize)> {
        match self {
            Self::EndpointSlices(_) => vec![("EndpointSlices", namespaces)],
            Self::PersistentVolumes(_) => vec![(ResourceKind::PersistentVolumes.label(), 1)],
            Self::Ingresses(_) => vec![(ResourceKind::Ingresses.label(), namespaces)],
            Self::TlsSecrets(_) => vec![(ResourceKind::Secrets.label(), namespaces)],
            Self::Bindings {
                cluster_role_bindings,
                ..
            } => vec![
                (ResourceKind::RoleBindings.label(), namespaces),
                (
                    ResourceKind::ClusterRoleBindings.label(),
                    usize::from(cluster_role_bindings.is_some()),
                ),
            ],
        }
    }

    /// How many watches the companion runs: one per namespace of the scope for a namespaced kind,
    /// one for a cluster-scoped one.
    fn watches(&self, namespaces: usize) -> usize {
        match self {
            Self::EndpointSlices(_) => namespaces,
            Self::PersistentVolumes(_) => 1,
            Self::Ingresses(_) | Self::TlsSecrets(_) => namespaces,
            Self::Bindings {
                cluster_role_bindings,
                ..
            } => namespaces + usize::from(cluster_role_bindings.is_some()),
        }
    }
}

/// The change feeds of Overview, one watch per kind and namespace. Dropping it stops both.
pub(crate) enum ChangeEvents {
    Live {
        rollouts: LiveList<EventSummary>,
        rescales: LiveList<EventSummary>,
        /// The ReplicaSets, while the 24 h range asks for rollouts older than the events.
        history: Option<Box<RolloutHistory>>,
        _subscriptions: [WatchSubscription; 2],
    },
    /// A known access report denies listing events, so nothing was started (no retry loop on a 403).
    Denied,
}

/// The ReplicaSets of the scope, watched while Overview shows its 24 h range: their creation times
/// are the rollouts that the events the API server keeps (about an hour) no longer hold. Dropping it
/// stops the watch.
pub(crate) struct RolloutHistory {
    pub(crate) replica_sets: LiveList<ReplicaSetSummary>,
    _subscription: WatchSubscription,
}

impl RolloutHistory {
    fn start(
        runtime: &ClusterRuntime,
        connection: &ClusterConnection,
        scope: &NamespaceScope,
        cx: &mut Context<ClusterSession>,
    ) -> Self {
        Self {
            replica_sets: LiveList::Loading,
            _subscription: runtime.subscribe(
                connection.watch_replica_sets(scope.clone()),
                cx,
                |session: &mut ClusterSession, update, _| {
                    if let Some(list) = session.history_list_mut() {
                        list.apply(update);
                    }
                },
                |session, _| {
                    if let Some(list) = session.history_list_mut() {
                        list.mark_stopped();
                    }
                },
            ),
        }
    }
}

/// The kinds of change events, in the order of `ChangeEvents::Live`'s subscriptions.
const CHANGE_EVENT_KINDS: [ChangeEventKind; 2] =
    [ChangeEventKind::Rollout, ChangeEventKind::Rescale];

/// A `Known` report that denies `ListEvents` for this scope. `Checking` and `Unknown` start the
/// watches: the server answers for itself.
fn is_change_feed_denied(access: &AccessState) -> bool {
    matches!(access, AccessState::Known(report) if !report.is_allowed(AccessCheck::ListEvents))
}

impl ChangeEvents {
    fn start(
        runtime: &ClusterRuntime,
        connection: &ClusterConnection,
        scope: &NamespaceScope,
        keeps_history: bool,
        cx: &mut Context<ClusterSession>,
    ) -> Self {
        let subscribe = |kind: ChangeEventKind, cx: &mut Context<ClusterSession>| {
            runtime.subscribe(
                connection.watch_change_events(scope.clone(), kind),
                cx,
                move |session: &mut ClusterSession, update, _| {
                    if let Some(list) = session.change_list_mut(kind) {
                        list.apply(update);
                    }
                },
                move |session, _| {
                    if let Some(list) = session.change_list_mut(kind) {
                        list.mark_stopped();
                    }
                },
            )
        };
        Self::Live {
            rollouts: LiveList::Loading,
            rescales: LiveList::Loading,
            history: keeps_history
                .then(|| Box::new(RolloutHistory::start(runtime, connection, scope, cx))),
            _subscriptions: CHANGE_EVENT_KINDS.map(|kind| subscribe(kind, cx)),
        }
    }
}

/// The events of the object whose drawer is open. Dropping it stops the watch.
pub(crate) struct ObjectEvents {
    pub(crate) subject: InvolvedObject,
    pub(crate) list: LiveList<EventSummary>,
    _subscription: WatchSubscription,
}

/// The objects related to the object whose drawer is open. Dropping it stops the watch.
struct RelatedObjects {
    subject: RelatedSubject,
    /// The lists of a Namespace drawer that were gated open when the watch started.
    gates: NamespaceListGates,
    list: RelatedList,
    _subscription: WatchSubscription,
}

/// The latest state of the related watch, by what it lists.
pub(crate) enum RelatedList {
    ReplicaSets(LiveList<ReplicaSetSummary>),
    Jobs(LiveList<JobSummary>),
    /// The value previews of one config map. They can be sensitive, so nothing here logs them.
    ConfigMapValues(LiveList<ConfigMapValues>),
    /// The FailedCreate events of a namespace.
    Events(LiveList<EventSummary>),
    /// The Quota section of a Namespace drawer: each half comes from its own watch.
    NamespaceLimits {
        quotas: LiveList<ResourceQuotaSummary>,
        limit_ranges: LiveList<LimitRangeSummary>,
    },
    /// Every revision of one Helm release, newest first. Labels and metadata only.
    HelmHistory(LiveList<HelmRevision>),
    /// The masked, flattened spec and status of one custom object (0 or 1 item).
    CustomFields(LiveList<CustomObjectFields>),
    /// The Services of one namespace, for a Pod drawer.
    Services(LiveList<ServiceSummary>),
    /// The Ingresses of one namespace, for a Service drawer.
    Ingresses(LiveList<IngressSummary>),
    /// Every StorageClass, for a claim drawer.
    StorageClasses(LiveList<StorageClassSummary>),
}

/// One related watch update, typed on tokio so one subscription serves every subject.
enum RelatedUpdate {
    ReplicaSets(WatchUpdate<ReplicaSetSummary>),
    Jobs(WatchUpdate<JobSummary>),
    ConfigMapValues(WatchUpdate<ConfigMapValues>),
    Events(WatchUpdate<EventSummary>),
    ResourceQuotas(WatchUpdate<ResourceQuotaSummary>),
    LimitRanges(WatchUpdate<LimitRangeSummary>),
    HelmHistory(WatchUpdate<HelmRevision>),
    CustomFields(WatchUpdate<CustomObjectFields>),
    Services(WatchUpdate<ServiceSummary>),
    Ingresses(WatchUpdate<IngressSummary>),
    StorageClasses(WatchUpdate<StorageClassSummary>),
}

impl RelatedList {
    /// A half of a Namespace drawer whose gate is closed never starts, so it begins `Failed` instead
    /// of `Loading`: the drawer prints the gate for it, and nothing waits on it.
    fn loading_for(subject: &RelatedSubject, gates: NamespaceListGates) -> Self {
        match subject {
            RelatedSubject::ReplicaSets { .. } => Self::ReplicaSets(LiveList::Loading),
            RelatedSubject::Jobs { .. } => Self::Jobs(LiveList::Loading),
            RelatedSubject::ConfigMapValues { .. } => Self::ConfigMapValues(LiveList::Loading),
            RelatedSubject::QuotaRejections { .. } => Self::Events(LiveList::Loading),
            RelatedSubject::NamespaceQuotas { .. } => Self::NamespaceLimits {
                quotas: loading_if(gates.quotas),
                limit_ranges: loading_if(gates.limit_ranges),
            },
            RelatedSubject::HelmHistory { .. } => Self::HelmHistory(LiveList::Loading),
            RelatedSubject::CustomFields { .. } => Self::CustomFields(LiveList::Loading),
            RelatedSubject::PodServices { .. } => Self::Services(LiveList::Loading),
            RelatedSubject::ServiceIngresses { .. } => Self::Ingresses(LiveList::Loading),
            RelatedSubject::ClaimClasses => Self::StorageClasses(LiveList::Loading),
        }
    }

    /// An update of the other variant is ignored: a stale one cannot reach a new subject.
    fn apply(&mut self, update: RelatedUpdate) {
        match (self, update) {
            (Self::ReplicaSets(list), RelatedUpdate::ReplicaSets(update)) => list.apply(update),
            (Self::Jobs(list), RelatedUpdate::Jobs(update)) => list.apply(update),
            (Self::ConfigMapValues(list), RelatedUpdate::ConfigMapValues(update)) => {
                list.apply(update);
            }
            (Self::Events(list), RelatedUpdate::Events(update)) => list.apply(update),
            (Self::NamespaceLimits { quotas, .. }, RelatedUpdate::ResourceQuotas(update)) => {
                quotas.apply(update);
            }
            (Self::NamespaceLimits { limit_ranges, .. }, RelatedUpdate::LimitRanges(update)) => {
                limit_ranges.apply(update);
            }
            (Self::HelmHistory(list), RelatedUpdate::HelmHistory(update)) => list.apply(update),
            (Self::CustomFields(list), RelatedUpdate::CustomFields(update)) => list.apply(update),
            (Self::Services(list), RelatedUpdate::Services(update)) => list.apply(update),
            (Self::Ingresses(list), RelatedUpdate::Ingresses(update)) => list.apply(update),
            (Self::StorageClasses(list), RelatedUpdate::StorageClasses(update)) => {
                list.apply(update);
            }
            // A stale update of another subject's kind.
            _ => {}
        }
    }

    fn mark_stopped(&mut self) {
        match self {
            Self::ReplicaSets(list) => list.mark_stopped(),
            Self::Jobs(list) => list.mark_stopped(),
            Self::ConfigMapValues(list) => list.mark_stopped(),
            Self::Events(list) => list.mark_stopped(),
            Self::NamespaceLimits {
                quotas,
                limit_ranges,
            } => {
                quotas.mark_stopped();
                limit_ranges.mark_stopped();
            }
            Self::HelmHistory(list) => list.mark_stopped(),
            Self::CustomFields(list) => list.mark_stopped(),
            Self::Services(list) => list.mark_stopped(),
            Self::Ingresses(list) => list.mark_stopped(),
            Self::StorageClasses(list) => list.mark_stopped(),
        }
    }

    /// The events of a quota's namespace, when this list holds them.
    pub(crate) fn events(&self) -> Option<&LiveList<EventSummary>> {
        match self {
            Self::Events(list) => Some(list),
            _ => None,
        }
    }

    /// The Services of a Pod drawer's namespace, when this list holds them.
    pub(crate) fn services(&self) -> Option<&LiveList<ServiceSummary>> {
        match self {
            Self::Services(list) => Some(list),
            _ => None,
        }
    }

    /// The Ingresses of a Service drawer's namespace, when this list holds them.
    pub(crate) fn ingresses(&self) -> Option<&LiveList<IngressSummary>> {
        match self {
            Self::Ingresses(list) => Some(list),
            _ => None,
        }
    }

    /// The StorageClasses of a claim drawer, when this list holds them.
    pub(crate) fn storage_classes(&self) -> Option<&LiveList<StorageClassSummary>> {
        match self {
            Self::StorageClasses(list) => Some(list),
            _ => None,
        }
    }

    /// The quotas and LimitRanges of a namespace, when this list holds them.
    pub(crate) fn namespace_limits(
        &self,
    ) -> Option<(
        &LiveList<ResourceQuotaSummary>,
        &LiveList<LimitRangeSummary>,
    )> {
        match self {
            Self::NamespaceLimits {
                quotas,
                limit_ranges,
            } => Some((quotas, limit_ranges)),
            _ => None,
        }
    }

    /// The revisions of a Helm release, when this list holds them.
    pub(crate) fn helm_history(&self) -> Option<&LiveList<HelmRevision>> {
        match self {
            Self::HelmHistory(list) => Some(list),
            _ => None,
        }
    }

    /// The fields of a custom object, when this list holds them.
    pub(crate) fn custom_fields(&self) -> Option<&LiveList<CustomObjectFields>> {
        match self {
            Self::CustomFields(list) => Some(list),
            _ => None,
        }
    }

    /// Whether the watch has not delivered its first snapshot.
    #[cfg(feature = "screenshot")]
    fn is_loading(&self) -> bool {
        match self {
            Self::ReplicaSets(list) => list.is_loading(),
            Self::Jobs(list) => list.is_loading(),
            Self::ConfigMapValues(list) => list.is_loading(),
            Self::Events(list) => list.is_loading(),
            Self::NamespaceLimits {
                quotas,
                limit_ranges,
            } => quotas.is_loading() || limit_ranges.is_loading(),
            Self::HelmHistory(list) => list.is_loading(),
            Self::CustomFields(list) => list.is_loading(),
            Self::Services(list) => list.is_loading(),
            Self::Ingresses(list) => list.is_loading(),
            Self::StorageClasses(list) => list.is_loading(),
        }
    }
}

/// Each list of the Namespace drawer's Quota section, gated by its own check. A check that is
/// `Known` and denied closes its gate; `Unknown` and allowed leave it open (the server answers). While
/// a review is `Checking`, the gates of the watch that `running` describes stay, so a re-review does
/// not restart a stream that is about to be denied again. Pure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NamespaceListGates {
    pub(crate) quotas: bool,
    pub(crate) limit_ranges: bool,
}

impl NamespaceListGates {
    /// Both lists open: the gates of every subject but a Namespace drawer's.
    const OPEN: Self = Self {
        quotas: true,
        limit_ranges: true,
    };

    fn is_closed(self) -> bool {
        !self.quotas && !self.limit_ranges
    }
}

pub(crate) fn namespace_list_gates(
    access: &AccessState,
    running: NamespaceListGates,
) -> NamespaceListGates {
    match access {
        AccessState::Checking { .. } => running,
        AccessState::Unknown => NamespaceListGates::OPEN,
        AccessState::Known(report) => NamespaceListGates {
            quotas: report.is_allowed(AccessCheck::ListResourceQuotas),
            limit_ranges: report.is_allowed(AccessCheck::ListLimitRanges),
        },
    }
}

fn loading_if<T>(is_open: bool) -> LiveList<T> {
    if is_open {
        LiveList::Loading
    } else {
        LiveList::Failed {
            message: "not permitted".to_owned(),
        }
    }
}

/// Whether the running related watch (its subject and gates) is the one wanted. No watch running and
/// none wanted is current too: `set_related_subject` runs on every render and must not notify then.
fn related_watch_is_current(
    running: Option<(&RelatedSubject, NamespaceListGates)>,
    wanted: Option<&RelatedSubject>,
    gates: NamespaceListGates,
) -> bool {
    match (running, wanted) {
        (None, None) => true,
        (Some((subject, running_gates)), Some(wanted)) => {
            subject == wanted && running_gates == gates
        }
        _ => false,
    }
}

/// Whether the related watch of `subject` is not started because the permissions deny it: the one
/// check of `denied_related_check`, or, for a Namespace drawer, both of its lists.
pub(crate) fn is_related_denied(subject: &RelatedSubject, access: &AccessState) -> bool {
    match subject {
        RelatedSubject::NamespaceQuotas { .. } => {
            namespace_list_gates(access, NamespaceListGates::OPEN).is_closed()
        }
        _ => denied_related_check(subject, access).is_some(),
    }
}

/// The check that denies the related watch of `subject`, when the access report is known and says
/// no. The watch is not started then, and the drawer names the check as the reason.
pub(crate) fn denied_related_check(
    subject: &RelatedSubject,
    access: &AccessState,
) -> Option<AccessCheck> {
    let check = match subject {
        RelatedSubject::QuotaRejections { .. } => AccessCheck::ListEvents,
        RelatedSubject::PodServices { .. } => AccessCheck::ListServices,
        RelatedSubject::ServiceIngresses { .. } => AccessCheck::ListIngresses,
        RelatedSubject::ClaimClasses => AccessCheck::ListStorageClasses,
        // Its two lists are gated each by its own check: see `namespace_list_gates`.
        RelatedSubject::NamespaceQuotas { .. } => return None,
        // The history reads the same Secrets the Releases kind lists, which its access check gates.
        RelatedSubject::ReplicaSets { .. }
        | RelatedSubject::Jobs { .. }
        | RelatedSubject::ConfigMapValues { .. }
        | RelatedSubject::HelmHistory { .. }
        // The explorer gate of the custom kind already allowed list and watch.
        | RelatedSubject::CustomFields { .. } => return None,
    };
    match access {
        AccessState::Known(report) if !report.is_allowed(check) => Some(check),
        AccessState::Known(_) | AccessState::Checking { .. } | AccessState::Unknown => None,
    }
}

/// Dropping a field stops that watch.
struct Subscriptions {
    _namespaces: WatchSubscription,
    _pods: WatchSubscription,
    _nodes: WatchSubscription,
}

pub(crate) enum AccessState {
    Checking {
        _task: Task<()>,
    },
    Known(AccessReport),
    /// The review failed; the reason is logged, and gated actions stay disabled.
    Unknown,
}

impl AccessState {
    fn from_review(review: Result<AccessReport, String>) -> Self {
        match review {
            Ok(report) => {
                // Both exec verbs decide the shell gate; the answers are what a live check records.
                tracing::debug!(
                    get_pods_exec = report.is_allowed(AccessCheck::GetPodExec),
                    create_pods_exec = report.is_allowed(AccessCheck::CreatePodExec),
                    "exec permissions of the session"
                );
                // The same for the two verbs of a port-forward (spec 0035).
                tracing::debug!(
                    get_pods_portforward = report.is_allowed(AccessCheck::GetPodPortForward),
                    create_pods_portforward = report.is_allowed(AccessCheck::CreatePodPortForward),
                    "port-forward permissions of the session"
                );
                // And the five checks of the debug container and the node shell (spec 0037).
                tracing::debug!(
                    create_pods = report.is_allowed(AccessCheck::CreatePods),
                    delete_pods = report.is_allowed(AccessCheck::DeletePods),
                    patch_ephemeralcontainers =
                        report.is_allowed(AccessCheck::PatchPodEphemeralContainers),
                    get_pods_attach = report.is_allowed(AccessCheck::GetPodAttach),
                    create_pods_attach = report.is_allowed(AccessCheck::CreatePodAttach),
                    "debug shell permissions of the session"
                );
                Self::Known(report)
            }
            Err(message) => {
                tracing::warn!(%message, "access review failed");
                Self::Unknown
            }
        }
    }
}

/// How long counted sidebar numbers stay fresh before a navigation counts again.
const KIND_COUNT_REFRESH: Duration = Duration::from_secs(30);
/// Count requests in flight at once: a run is about a dozen tiny lists.
const KIND_COUNT_CONCURRENCY: usize = 4;

/// One custom kind's result of a count run.
type CustomCount = (CustomKind, Result<Option<u64>, String>);

/// One kind's result of a count run: the number, `None` when the server reported no remaining
/// count, or the error text.
type KindCount = (ResourceKind, Result<Option<u64>, String>);

/// What asks for a count run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CountTrigger {
    /// An access review finished: counts once per scope, so a retried review does not recount.
    Review,
    /// The user went to another screen: counts again only when the numbers are 30 s old.
    Navigation,
}

/// The sidebar numbers of kinds whose watch is not running, from one-shot `limit=1` lists.
#[derive(Default)]
pub(crate) struct KindCounts {
    /// The scope the counts, or the run that is filling them, belong to.
    scope: Option<NamespaceScope>,
    counts: HashMap<ResourceKind, u64>,
    /// When the last run started, not finished: navigation re-runs are throttled from the start, so
    /// a slow run never lets a second one begin right behind it.
    refreshed_at: Option<Instant>,
    task: Option<Task<()>>,
}

impl KindCounts {
    /// Whether a run should start now. A run for the scope that is already counted or counting
    /// never starts from a review; from a navigation it starts only once the last one is stale.
    fn wants_run(&self, trigger: CountTrigger, scope: &NamespaceScope, now: Instant) -> bool {
        let is_counted = self.scope.as_ref() == Some(scope);
        match trigger {
            CountTrigger::Review => !is_counted,
            CountTrigger::Navigation => {
                is_counted
                    && self
                        .refreshed_at
                        .is_some_and(|at| now.duration_since(at) >= KIND_COUNT_REFRESH)
            }
        }
    }

    /// Whether a run has not delivered its numbers. Only the screenshot hook waits on it.
    #[cfg(feature = "screenshot")]
    pub(crate) fn is_running(&self) -> bool {
        self.task.is_some()
    }

    /// Every counted number, for the sidebar. Events are counted unfiltered, so they are left out
    /// while the Events screen shows warnings only: the number would not match its list.
    pub(crate) fn all(&self, event_filter: EventFilter) -> HashMap<ResourceKind, usize> {
        self.counts
            .iter()
            .filter(|(kind, _)| **kind != ResourceKind::Events || event_filter == EventFilter::All)
            .filter_map(|(kind, count)| Some((*kind, usize::try_from(*count).ok()?)))
            .collect()
    }
}

/// The kinds a run counts: those whose list the report allows. Without a known report nothing is
/// counted, so a denied kind never costs a request.
fn countable_kinds(access: &AccessState) -> Vec<ResourceKind> {
    let AccessState::Known(report) = access else {
        return Vec::new();
    };
    ResourceKind::ALL
        .into_iter()
        .filter(|kind| {
            kind.has_count()
                && kind
                    .access_check()
                    .is_some_and(|check| report.is_allowed(check))
        })
        .collect()
}

/// How long the cluster-wide instance counts stay fresh before opening the CRDs screen counts
/// again.
const CUSTOM_COUNT_REFRESH: Duration = Duration::from_secs(30);

/// The cluster-wide number of objects of each custom kind, from one `limit=1` list per CRD. They
/// do not depend on the namespace scope, so a scope change keeps them.
#[derive(Default)]
pub(crate) struct CustomCounts {
    pub(crate) counts: HashMap<CustomKind, u64>,
    /// When the last run started, so a slow run never lets a second begin right behind it.
    refreshed_at: Option<Instant>,
    task: Option<Task<()>>,
}

impl CustomCounts {
    /// Whether a run should start now: none ran yet, or the last one started 30 s ago.
    fn wants_run(&self, now: Instant) -> bool {
        self.refreshed_at
            .is_none_or(|at| now.duration_since(at) >= CUSTOM_COUNT_REFRESH)
    }
}

/// The counts after a run: a fresh number replaces the old one, a failed request keeps the previous
/// number (stale beats blank), and a server that reports no remaining count leaves the kind
/// without one. A kind the run did not ask about (now denied, or gone) is dropped.
fn merge_custom_counts(
    previous: &HashMap<CustomKind, u64>,
    results: Vec<CustomCount>,
) -> HashMap<CustomKind, u64> {
    let mut counts = HashMap::new();
    for (kind, count) in results {
        let kept = match count {
            Ok(count) => count,
            Err(_) => previous.get(&kind).copied(),
        };
        if let Some(count) = kept {
            counts.insert(kind, count);
        }
    }
    counts
}

/// The kinds a custom count run asks about: every served kind whose list review did not deny it,
/// so a denied kind never costs a request.
fn countable_custom_kinds(
    kinds: &[CustomKind],
    gates: &HashMap<CustomKind, CustomGate>,
) -> Vec<CustomKind> {
    kinds
        .iter()
        .copied()
        .filter(|kind| !matches!(gates.get(kind), Some(CustomGate::Denied { .. })))
        .collect()
}

/// The latest state of one watched kind.
pub(crate) enum LiveList<T> {
    Loading,
    /// `interruption` is set when the last watch update was a failure after data arrived.
    Ready {
        items: Vec<T>,
        interruption: Option<String>,
    },
    /// Failed before the first snapshot; the watch keeps retrying.
    Failed {
        message: String,
    },
}

impl<T> LiveList<T> {
    #[cfg_attr(
        feature = "hotpath-profiling",
        hotpath::measure(impl_type = "LiveList")
    )]
    pub(crate) fn apply(&mut self, update: WatchUpdate<T>) {
        match update {
            WatchUpdate::Snapshot(items) => {
                *self = Self::Ready {
                    items,
                    interruption: None,
                };
            }
            WatchUpdate::Failed(error) => self.fail(error_text(&error)),
        }
    }

    /// The watch stream ended, for whatever reason; the watches normally run forever.
    pub(crate) fn mark_stopped(&mut self) {
        self.fail("watch stopped unexpectedly".to_owned());
    }

    /// Stale data stays visible; only a list that never had data becomes `Failed`.
    fn fail(&mut self, message: String) {
        match self {
            Self::Ready { interruption, .. } => *interruption = Some(message),
            Self::Loading | Self::Failed { .. } => *self = Self::Failed { message },
        }
    }

    pub(crate) fn items(&self) -> &[T] {
        match self {
            Self::Ready { items, .. } => items,
            Self::Loading | Self::Failed { .. } => &[],
        }
    }

    /// The items once loaded, for rewriting cells in place; empty while loading or failed.
    pub(crate) fn items_mut(&mut self) -> &mut [T] {
        match self {
            Self::Ready { items, .. } => items,
            Self::Loading | Self::Failed { .. } => &mut [],
        }
    }

    pub(crate) fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }

    /// The items once the first snapshot has arrived; `None` while loading or failed.
    pub(crate) fn ready_items(&self) -> Option<&[T]> {
        match self {
            Self::Ready { items, .. } => Some(items),
            Self::Loading | Self::Failed { .. } => None,
        }
    }

    /// The number of items once the first snapshot has arrived.
    pub(crate) fn ready_count(&self) -> Option<usize> {
        match self {
            Self::Ready { items, .. } => Some(items.len()),
            Self::Loading | Self::Failed { .. } => None,
        }
    }

    /// Why the last update failed, while stale data is still shown.
    pub(crate) fn interruption(&self) -> Option<&str> {
        match self {
            Self::Ready { interruption, .. } => interruption.as_deref(),
            Self::Loading | Self::Failed { .. } => None,
        }
    }

    /// Why the list never loaded.
    pub(crate) fn failure(&self) -> Option<&str> {
        match self {
            Self::Failed { message } => Some(message),
            Self::Loading | Self::Ready { .. } => None,
        }
    }

    pub(crate) fn has_problem(&self) -> bool {
        matches!(
            self,
            Self::Failed { .. }
                | Self::Ready {
                    interruption: Some(_),
                    ..
                }
        )
    }
}

/// Whole milliseconds for display, at least 1 so a fast answer never reads `0 ms`.
pub(crate) fn latency_millis(latency: Duration) -> u64 {
    u64::try_from(latency.as_millis())
        .unwrap_or(u64::MAX)
        .max(1)
}

/// A `Display` text plus the first line of the cause, without anything secret: the
/// cluster crate keeps credentials out of its error text.
pub(crate) fn error_text(error: &(dyn Error + 'static)) -> String {
    let cause = error
        .source()
        .and_then(|source| source.to_string().lines().next().map(str::to_owned));
    match cause {
        Some(cause) if !cause.is_empty() => format!("{error}: {cause}"),
        _ => error.to_string(),
    }
}

/// The picked namespaces as shown in the title bar and in messages: `a, b`, or `a, b +N` when
/// there are more than two.
pub(crate) fn namespaces_label(names: &[String]) -> String {
    match names {
        [first, second, rest @ ..] if !rest.is_empty() => {
            format!("{first}, {second} +{}", rest.len())
        }
        _ => names.join(", "),
    }
}

/// `all_namespaces_access` is `None` when `review_access(All)` failed or was not asked.
fn initial_scope(
    requested: Option<&NamespaceScope>,
    all_namespaces_access: Option<&AccessReport>,
    default_namespace: &str,
) -> NamespaceScope {
    if let Some(scope) = requested {
        return scope.clone();
    }
    // Without a report the context namespace is the safest scope: it needs the fewest rights.
    match all_namespaces_access {
        Some(report) if report.is_allowed(AccessCheck::ListPods) => NamespaceScope::All,
        _ => NamespaceScope::Named(default_namespace.to_owned()),
    }
}

/// The outcome of `connect_cluster`, sent from tokio back to the session.
struct Connected {
    connection: ClusterConnection,
    server_version: ServerVersion,
    /// The round trip of the version request only, not the client setup.
    api_latency: Duration,
    scope: NamespaceScope,
    access: Result<AccessReport, String>,
}

async fn connect_cluster(
    kubeconfig: Arc<Kubeconfig>,
    context: String,
    proxy: Result<ProxyChoice, ProxyUrlError>,
    requested_namespace: Option<NamespaceScope>,
) -> Result<Connected, ClusterError> {
    let connection = open_cluster(&kubeconfig, &context, &proxy).await?;
    let version_started = Instant::now();
    let server_version = connection.server_version().await?;
    let api_latency = version_started.elapsed();

    let all_namespaces_review = match requested_namespace {
        Some(_) => None,
        None => Some(connection.review_access(NamespaceScope::All).await),
    };
    let scope = initial_scope(
        requested_namespace.as_ref(),
        all_namespaces_review
            .as_ref()
            .and_then(|review| review.as_ref().ok()),
        connection.default_namespace(),
    );
    let access = match all_namespaces_review {
        Some(Ok(report)) if scope == NamespaceScope::All => Ok(report),
        Some(Err(error)) => Err(error_text(&error)),
        Some(Ok(_)) | None => connection
            .review_access(scope.clone())
            .await
            .map_err(|error| error_text(&error)),
    };
    Ok(Connected {
        connection,
        server_version,
        api_latency,
        scope,
        access,
    })
}

impl ClusterSession {
    pub(crate) fn new(
        kubeconfig: Arc<Kubeconfig>,
        summary: &ContextSummary,
        requested_namespace: Option<NamespaceScope>,
        explorer_kind: Option<ResourceKind>,
        custom_kind_cache: CustomKindCache,
        cx: &mut Context<Self>,
    ) -> Self {
        let inputs = ConnectInputs {
            kubeconfig,
            context: summary.name.clone(),
            requested_namespace,
        };
        let phase = Self::begin_connect(&inputs, summary, cx);
        let profile = AppSettings::get(cx).registry.profile(summary);
        let lock = WriteLock::at_open(&profile, AppSettings::was_reset(cx));
        let source_entry = profile.metrics;
        Self {
            inputs,
            summary: summary.clone(),
            lock,
            generation: next_generation(),
            phase,
            explorer_kind,
            access_kind: None,
            event_filter: EventFilter::All,
            custom_kind_cache,
            issues: IssueBoard::default(),
            is_issues_visible: false,
            is_overview_visible: false,
            keeps_rollout_history: false,
            topology_subject: None,
            source_entry,
            _issue_tick: Self::start_issue_tick(cx),
        }
    }

    /// Looks every `ISSUE_TICK` whether the board needs a run. The task ends with the entity.
    fn start_issue_tick(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(ISSUE_TICK).await;
                if this
                    .update(cx, |session, cx| session.refresh_issues(cx))
                    .is_err()
                {
                    return;
                }
            }
        })
    }

    /// The problems found in the live lists, for the sidebar, the title bar, and the Issues screen.
    pub(crate) fn issues(&self) -> &IssueBoard {
        &self.issues
    }

    /// Whether the Issues screen is shown; set from `AppShell::show_screen`.
    pub(crate) fn set_issues_visible(&mut self, is_visible: bool) {
        self.is_issues_visible = is_visible;
    }

    /// Starts the change feeds while Overview is shown and stops them when it is left. A session
    /// that is not live only remembers the choice; `LiveCluster::start` reads it.
    pub(crate) fn set_overview_visible(&mut self, is_visible: bool, cx: &mut Context<Self>) {
        self.is_overview_visible = is_visible;
        let runtime = cx.global::<ClusterRuntime>().clone();
        let Some(live) = self.live_mut() else {
            return;
        };
        if is_visible == live.change_events.is_some() {
            return;
        }
        // A visible Overview starts the feeds; leaving drops both subscriptions.
        live.change_events = None;
        if is_visible {
            live.start_change_events(&runtime, cx);
        }
        cx.notify();
    }

    /// Starts or stops the ReplicaSet watch behind the 24 h range of Recent changes. A session that
    /// is not live only remembers the choice; `LiveCluster::start` reads it.
    pub(crate) fn set_rollout_history(&mut self, keeps: bool, cx: &mut Context<Self>) {
        self.keeps_rollout_history = keeps;
        let runtime = cx.global::<ClusterRuntime>().clone();
        let Some(live) = self.live_mut() else {
            return;
        };
        if live.keeps_rollout_history == keeps {
            return;
        }
        live.keeps_rollout_history = keeps;
        if let Some(ChangeEvents::Live { history, .. }) = live.change_events.as_mut() {
            *history = keeps.then(|| {
                Box::new(RolloutHistory::start(
                    &runtime,
                    &live.connection,
                    &live.scope,
                    cx,
                ))
            });
        }
        cx.notify();
    }

    /// The subject the Topology feeds were last pointed at.
    pub(crate) fn topology_subject(&self) -> Option<&TopologySubject> {
        self.topology_subject.as_ref()
    }

    /// Points the Topology feeds at `subject`, or drops them with `None`. The same subject is a
    /// no-op; another namespace restarts every feed; other kind chips start and stop only their
    /// own. A session that is not live only remembers the choice; `LiveCluster::start` reads it.
    pub(crate) fn set_topology_subject(
        &mut self,
        subject: Option<TopologySubject>,
        cx: &mut Context<Self>,
    ) {
        self.topology_subject = subject.clone();
        let runtime = cx.global::<ClusterRuntime>().clone();
        let Some(live) = self.live_mut() else {
            return;
        };
        let running = live.topology.as_ref().map(|feeds| &feeds.subject);
        match subject_change(running, subject.as_ref()) {
            SubjectChange::Keep => return,
            SubjectChange::Stop => live.topology = None,
            SubjectChange::Restart => {
                // The old feeds drop first, so two namespaces never overlap.
                live.topology = None;
                live.topology = subject.map(|subject| live.start_topology(subject, &runtime, cx));
            }
            SubjectChange::Adjust { stop, start } => {
                if let Some(feeds) = live.topology.as_mut() {
                    feeds.remove(&stop);
                    if let Some(subject) = subject {
                        feeds.subject = subject;
                    }
                }
                let namespace = live
                    .topology
                    .as_ref()
                    .map(|feeds| feeds.subject.namespace.clone())
                    .unwrap_or_default();
                for kind in start {
                    let feed = live.topology_feed(kind, &namespace, &runtime, cx);
                    if let Some(feeds) = live.topology.as_mut() {
                        feeds.feeds.push(feed);
                    }
                }
            }
        }
        cx.notify();
    }

    /// Whether a change feed that runs has not delivered its first snapshot; a screenshot of
    /// Overview waits on it.
    #[cfg(feature = "screenshot")]
    pub(crate) fn is_change_feed_pending(&self) -> bool {
        let Some(live) = self.live() else {
            return false;
        };
        matches!(
            &live.change_events,
            Some(ChangeEvents::Live { rollouts, rescales, history, .. })
                if rollouts.is_loading()
                    || rescales.is_loading()
                    || history.as_ref().is_some_and(|history| history.replica_sets.is_loading())
        )
    }

    /// Whether a Topology feed that runs has not delivered its first snapshot; a screenshot of
    /// Topology waits on it.
    #[cfg(feature = "screenshot")]
    pub(crate) fn is_topology_pending(&self) -> bool {
        self.live()
            .and_then(|live| live.topology.as_ref())
            .is_some_and(TopologyFeeds::has_pending)
    }

    /// Whether the board has not run on loaded lists yet, or a feed it reads still loads. A launch
    /// that opens an issue and the screenshot hook wait on it.
    pub(crate) fn is_issues_pending(&self) -> bool {
        self.issues.summary().is_none()
            || self
                .issues
                .coverage()
                .feeds
                .iter()
                .any(|(_, state)| *state == FeedState::Loading)
    }

    /// Runs the board when something changed or the clock moved enough. It repaints only when the
    /// issues or the coverage changed, or, while the Issues screen is shown, on a time refresh.
    fn refresh_issues(&mut self, cx: &mut Context<Self>) {
        let now = jiff::Timestamp::now();
        let Some(reason) = self.issues.run_due(now) else {
            return;
        };
        let SessionPhase::Live(live) = &self.phase else {
            return;
        };
        let feeds = &live.issue_feeds;
        let events = &feeds.events;
        // Only a feed that has loaded is read; the coverage names the others.
        let objects: Vec<(ResourceKind, &[KindObject])> = feeds
            .conditions
            .iter()
            .filter(|feed| feed.state() == FeedState::Live)
            .filter_map(|feed| Some((feed.kind, feed.list.ready_items()?)))
            .collect();
        let inputs = IssueInputs {
            pods: live.pods.ready_items(),
            nodes: live.nodes.ready_items(),
            scope: &live.scope,
            namespaces: live.namespaces.ready_items(),
            events: (events.state() == FeedState::Live).then_some(events),
            objects: &objects,
            pod_usage: Some(&live.metrics.pods.history),
            node_usage: Some(&live.metrics.nodes.history),
            kubelet: Some(&live.metrics.kubelet.history),
            is_job_feed_live: objects.iter().any(|(kind, _)| *kind == ResourceKind::Jobs),
            now,
        };
        let change = self.issues.refresh(&inputs, core_coverage(live, feeds));
        if refresh_repaints(change, reason, self.is_issues_visible) {
            cx.notify();
        }
    }

    /// A Warning events update: it only marks the board, which repaints when it finds a change.
    pub(crate) fn apply_warning_events(&mut self, update: WatchUpdate<EventSummary>) {
        self.issues.mark_dirty();
        if let Some(live) = self.live_mut() {
            live.issue_feeds.events.apply(update);
        }
    }

    pub(crate) fn stop_warning_events(&mut self) {
        self.issues.mark_dirty();
        if let Some(live) = self.live_mut() {
            live.issue_feeds.events.mark_stopped();
        }
    }

    /// A condition feed update: it only marks the board, which repaints when it finds a change.
    pub(crate) fn apply_condition_update(
        &mut self,
        kind: ResourceKind,
        update: WatchUpdate<KindObject>,
    ) {
        self.issues.mark_dirty();
        if let Some(feed) = self
            .live_mut()
            .and_then(|live| live.issue_feeds.condition_mut(kind))
        {
            feed.apply(update);
        }
    }

    pub(crate) fn stop_condition(&mut self, kind: ResourceKind) {
        self.issues.mark_dirty();
        if let Some(feed) = self
            .live_mut()
            .and_then(|live| live.issue_feeds.condition_mut(kind))
        {
            feed.mark_stopped();
        }
    }

    /// The scope has settled: starts the Warning events watch for it.
    pub(crate) fn start_issue_events(&mut self, cx: &mut Context<Self>) {
        let runtime = cx.global::<ClusterRuntime>().clone();
        let Some(live) = self.live_mut() else {
            return;
        };
        let (connection, scope) = (live.connection.clone(), live.scope.clone());
        live.issue_feeds
            .finish_restart(&runtime, &connection, scope, cx);
    }

    /// Hands the custom kind definitions to the next session, so a context switch reuses them.
    pub(crate) fn take_custom_kind_cache(&mut self) -> CustomKindCache {
        std::mem::take(&mut self.custom_kind_cache)
    }

    pub(crate) fn context(&self) -> &str {
        &self.inputs.context
    }

    /// The kubeconfig user entry name, not a credential.
    pub(crate) fn user(&self) -> Option<&str> {
        self.summary.user.as_deref()
    }

    /// What the write gate and the confirm step read for this session's own cluster. `None` until
    /// the session is live, because the permission report belongs to the live cluster.
    pub(crate) fn guard(&self, cx: &App) -> Option<ClusterGuard<'_>> {
        let live = self.live()?;
        let profile = AppSettings::get(cx).registry.profile(&self.summary);
        Some(ClusterGuard::new(
            &live.access,
            &live.kind_access,
            self.lock,
            profile,
            self.summary.clone(),
            self.generation,
        ))
    }

    /// The connection generation: a reconnect takes a new one.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// The lock of this session, also while it is not live.
    pub(crate) fn lock(&self) -> WriteLock {
        self.lock
    }

    /// Locks or unlocks this session only; nothing is written to the settings.
    pub(crate) fn set_lock(&mut self, lock: WriteLock, cx: &mut Context<Self>) {
        self.lock = lock;
        cx.notify();
    }

    pub(crate) fn phase(&self) -> &SessionPhase {
        &self.phase
    }

    pub(crate) fn live(&self) -> Option<&LiveCluster> {
        match &self.phase {
            SessionPhase::Live(live) => Some(live),
            SessionPhase::Connecting { .. } | SessionPhase::Failed { .. } => None,
        }
    }

    fn explorer_mut(&mut self, kind: ResourceKind) -> Option<&mut KindList> {
        self.live_mut()?
            .explorer
            .as_mut()
            .filter(|explorer| explorer.kind == kind)
    }

    /// The list of one change feed, while it runs.
    fn change_list_mut(&mut self, kind: ChangeEventKind) -> Option<&mut LiveList<EventSummary>> {
        match self.live_mut()?.change_events.as_mut()? {
            ChangeEvents::Live {
                rollouts, rescales, ..
            } => Some(match kind {
                ChangeEventKind::Rollout => rollouts,
                ChangeEventKind::Rescale => rescales,
            }),
            ChangeEvents::Denied => None,
        }
    }

    /// The ReplicaSets of the 24 h range, while that watch runs.
    fn history_list_mut(&mut self) -> Option<&mut LiveList<ReplicaSetSummary>> {
        match self.live_mut()?.change_events.as_mut()? {
            ChangeEvents::Live {
                history: Some(history),
                ..
            } => Some(&mut history.replica_sets),
            ChangeEvents::Live { history: None, .. } | ChangeEvents::Denied => None,
        }
    }

    /// The Topology feed of `kind`, while it exists.
    fn topology_feed_mut(&mut self, kind: ResourceKind) -> Option<&mut TopologyFeed> {
        self.live_mut()?.topology.as_mut()?.feed_mut(kind)
    }

    fn object_events_mut(&mut self, subject: &InvolvedObject) -> Option<&mut ObjectEvents> {
        self.live_mut()?
            .object_events
            .as_mut()
            .filter(|events| events.subject == *subject)
    }

    fn related_mut(&mut self, subject: &RelatedSubject) -> Option<&mut RelatedObjects> {
        self.live_mut()?
            .related
            .as_mut()
            .filter(|related| related.subject == *subject)
    }

    fn live_mut(&mut self) -> Option<&mut LiveCluster> {
        match &mut self.phase {
            SessionPhase::Live(live) => Some(live),
            SessionPhase::Connecting { .. } | SessionPhase::Failed { .. } => None,
        }
    }

    /// Connects again with the same inputs, after a failure.
    pub(crate) fn retry(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.phase, SessionPhase::Failed { .. }) {
            return;
        }
        self.generation = next_generation();
        self.phase = Self::begin_connect(&self.inputs, &self.summary, cx);
        cx.notify();
    }

    /// The proxy is read here, on the main thread, each time a connect starts: a Retry picks up a
    /// proxy changed in Settings since the failure.
    fn begin_connect(
        inputs: &ConnectInputs,
        summary: &ContextSummary,
        cx: &mut Context<Self>,
    ) -> SessionPhase {
        let runtime = cx.global::<ClusterRuntime>().clone();
        let proxy = AppSettings::get(cx).registry.profile(summary).proxy;
        let connecting = runtime.spawn(connect_cluster(
            Arc::clone(&inputs.kubeconfig),
            inputs.context.clone(),
            proxy,
            inputs.requested_namespace.clone(),
        ));
        let task = cx.spawn(async move |this, cx| {
            let result = connecting.await;
            let _ = this.update(cx, |session, cx| session.finish_connect(result, cx));
        });
        SessionPhase::Connecting { _task: task }
    }

    /// The `Ready` source state of the live session being replaced, when the stored entry is still
    /// the one it checked: a new connection keeps it, so a 7d or 30d Monitor survives the swap
    /// instead of dropping to the sampler for the length of a check.
    fn kept_ready_source(&self) -> Option<SourceState> {
        let (source, check) = self.live()?.metrics.source.ready()?;
        let is_same = self
            .source_entry
            .as_ref()
            .is_some_and(|entry| entry.as_ref().ok() == Some(source));
        is_same.then(|| SourceState::Ready {
            source: source.clone(),
            check: *check,
        })
    }

    fn finish_connect(
        &mut self,
        result: Result<Result<Connected, ClusterError>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        let kept = self.kept_ready_source();
        self.phase = match result {
            Ok(Ok(connected)) => SessionPhase::Live(Box::new(LiveCluster::start(
                connected,
                self.explorer_kind,
                self.event_filter,
                self.is_overview_visible,
                self.keeps_rollout_history,
                self.topology_subject.clone(),
                cx,
            ))),
            Ok(Err(error)) => SessionPhase::Failed {
                message: error_text(&error),
            },
            Err(_) => SessionPhase::Failed {
                message: "the connection task stopped unexpectedly".to_owned(),
            },
        };
        self.review_kind_access(cx);
        self.refresh_kind_counts(CountTrigger::Review, cx);
        self.update_metrics_feeds(cx);
        match kept {
            Some(ready) => {
                if let Some(live) = self.live_mut() {
                    live.metrics.source = ready;
                }
            }
            None => self.restart_source_check(cx),
        }
        cx.notify();
    }

    /// Asks which kind the shown screen acts on; `None` when it acts on none. The `update` and
    /// `delete` review runs once per kind and scope (spec 0031 decision 24, 0033 decision 25) and is
    /// kept across a reconnect, which asks again.
    pub(crate) fn request_kind_access(&mut self, kind: Option<ObjectKind>, cx: &mut Context<Self>) {
        self.access_kind = kind;
        self.review_kind_access(cx);
    }

    fn review_kind_access(&mut self, cx: &mut Context<Self>) {
        let Some(kind) = self.access_kind else {
            return;
        };
        let runtime = cx.global::<ClusterRuntime>().clone();
        let Some(live) = self.live_mut() else {
            return;
        };
        if live.kind_access.get(kind).is_some() {
            return;
        }
        let connection = live.connection.clone();
        let scope = live.scope.clone();
        let reviewing = runtime.spawn(async move {
            connection
                .review_access_for(&lazy_checks(kind), scope)
                .await
        });
        let task = cx.spawn(async move |this, cx| {
            let result = reviewing.await;
            let _ = this.update(cx, |session, cx| {
                session.finish_kind_access(kind, result, cx)
            });
        });
        live.kind_access
            .set(kind, KindAccess::Checking { _task: task });
    }

    fn finish_kind_access(
        &mut self,
        kind: ObjectKind,
        result: Result<Result<AccessReport, ClusterError>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        let Some(live) = self.live_mut() else {
            return;
        };
        let access = match result {
            Ok(Ok(report)) => KindAccess::Known(report),
            Ok(Err(error)) => {
                tracing::warn!(message = %error_text(&error), "update access review failed");
                KindAccess::Unknown
            }
            Err(_) => KindAccess::Unknown,
        };
        // The answer is what a live check records; it names a kind and a bool, nothing more.
        tracing::debug!(
            kind = kind.name(),
            is_delete_allowed = matches!(&access, KindAccess::Known(report) if report.is_allowed(AccessCheck::Delete(kind))),
            // False for a kind whose values are not edited: the check was not asked (spec 0047).
            is_patch_allowed = matches!(&access, KindAccess::Known(report) if report.is_allowed(AccessCheck::Patch(kind))),
            // False for a kind with no `New` button: the check was not asked (spec 0042).
            is_create_allowed = matches!(&access, KindAccess::Known(report) if report.is_allowed(AccessCheck::Create(kind))),
            "delete, patch, and create permission of the kind"
        );
        live.kind_access.set(kind, access);
        cx.notify();
    }

    /// Switches the pods watch to `scope` and reviews access again for it.
    pub(crate) fn set_scope(&mut self, scope: NamespaceScope, cx: &mut Context<Self>) {
        let event_filter = self.event_filter;
        self.issues.mark_dirty();
        let Some(live) = self.live_mut() else {
            return;
        };
        if live.scope == scope {
            return;
        }
        let runtime = cx.global::<ClusterRuntime>().clone();
        live.pods = LiveList::Loading;
        live.subscriptions._pods = subscribe_pods(&runtime, &live.connection, scope.clone(), cx);
        // Before the explorer restarts, so a companion plan never reads the report of the old
        // scope.
        live.access = review_access_again(&runtime, &live.connection, scope.clone(), cx);
        // A review of a namespaced kind is per scope; a cluster-scoped one is not, and its running
        // explorer is not restarted, so its gate must survive or the list would stay Loading.
        live.custom_gates
            .retain(|kind, _| keeps_gate_on_scope_change(*kind));
        if let Some(kind) = live
            .explorer
            .as_ref()
            .map(|explorer| explorer.kind)
            .filter(|kind| restarts_on_scope_change(*kind))
        {
            live.start_explorer(kind, scope.clone(), event_filter, &runtime, cx);
        }
        let review = start_pod_review(&runtime, &live.connection, scope.clone(), cx);
        live.metrics.restart_pods(&scope, review);
        live.metrics.kubelet.history.retain_scope(&scope);
        // The events watch scans etcd, so it waits for the scope to settle.
        live.issue_feeds.restart_events(cx);
        live.scope = scope;
        // The review of the new scope is running, so the condition feeds wait for it.
        live.issue_feeds.restart_conditions(
            &runtime,
            &live.connection,
            &live.scope,
            &live.access,
            cx,
        );
        // The feeds are for the old namespaces; the new ones start again while Overview is shown.
        if live.change_events.take().is_some() {
            live.start_change_events(&runtime, cx);
        }
        // Topology draws one namespace of the scope; one that left it has nothing to draw.
        let leaves_scope = live
            .topology
            .as_ref()
            .is_some_and(|feeds| !scope_includes(&live.scope, &feeds.subject.namespace));
        if leaves_scope {
            live.topology = None;
        }
        // The numbers are for the old scope; the review for the new one counts again.
        live.kind_counts = KindCounts::default();
        // The names are for the old scope; dropping the index aborts a run that is still listing.
        live.name_index = NameIndex::default();
        // The fallback namespaces and so the coverage may differ in the new scope.
        live.rbac.reset_for_scope_change();
        // The answers are for the old scope; the screen asks again for the new one.
        live.kind_access.clear();
        live.refresh_kubelet_targets();
        if leaves_scope {
            self.topology_subject = None;
        }
        self.review_kind_access(cx);
        cx.notify();
    }

    /// Lists the RBAC objects once for a tool that needs them (no-op while Ready or Loading).
    pub(crate) fn request_rbac(&mut self, cx: &mut Context<Self>) {
        self.fetch_rbac(RbacTrigger::Request, cx);
    }

    /// Lists again for the Refresh and Retry buttons (no-op while Loading or Idle).
    pub(crate) fn refresh_rbac(&mut self, cx: &mut Context<Self>) {
        self.fetch_rbac(RbacTrigger::Refresh, cx);
    }

    fn fetch_rbac(&mut self, trigger: RbacTrigger, cx: &mut Context<Self>) {
        let runtime = cx.global::<ClusterRuntime>().clone();
        let Some(live) = self.live_mut() else {
            return;
        };
        if !starts_fetch(&live.rbac, trigger) {
            return;
        }
        // Roles and RoleBindings fall back to these when a cluster-wide list is forbidden.
        let fallback: Vec<String> = match live.namespaces.ready_items() {
            Some(namespaces) => namespaces.iter().map(|item| item.name.clone()).collect(),
            None => live.scope.namespaces().to_vec(),
        };
        let connection = live.connection.clone();
        let fetching = runtime.spawn(async move { connection.read_rbac(&fallback).await });
        let task = cx.spawn(async move |this, cx| {
            let result = fetching.await;
            let _ = this.update(cx, |session, cx| session.finish_rbac(result, cx));
        });
        live.rbac = RbacState::Loading { _task: task };
        cx.notify();
    }

    fn finish_rbac(
        &mut self,
        result: Result<Result<RbacSnapshot, ClusterError>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        let Some(live) = self.live_mut() else {
            return;
        };
        live.rbac = match result {
            Ok(Ok(snapshot)) => RbacState::Ready {
                snapshot: Rc::new(snapshot),
                listed_at: jiff::Timestamp::now(),
            },
            Ok(Err(error)) => RbacState::Failed(error_text(&error)),
            Err(_) => RbacState::Failed("the RBAC listing stopped unexpectedly".to_owned()),
        };
        cx.notify();
    }

    /// Starts, replaces, or stops the watch of the visible kind screen. The same kind again is a
    /// no-op, so there is no re-list. Before the session is live only the choice is stored.
    pub(crate) fn set_explorer_kind(&mut self, kind: Option<ResourceKind>, cx: &mut Context<Self>) {
        self.explorer_kind = kind;
        let event_filter = self.event_filter;
        let runtime = cx.global::<ClusterRuntime>().clone();
        let Some(live) = self.live_mut() else {
            return;
        };
        if live.explorer.as_ref().map(|explorer| explorer.kind) == kind {
            return;
        }
        match kind {
            Some(kind) => {
                let scope = live.scope.clone();
                live.start_explorer(kind, scope, event_filter, &runtime, cx);
            }
            None => live.explorer = None,
        }
        live.seed_crd_explorer();
        cx.notify();
    }

    /// Holds the explorer list still, or shows what arrived meanwhile. Only a loaded list can be
    /// paused. Every restart of the list (scope, kind, Warnings only) starts it live again.
    pub(crate) fn set_explorer_paused(&mut self, is_paused: bool, cx: &mut Context<Self>) {
        let Some(live) = self.live_mut() else {
            return;
        };
        let Some(explorer) = live.explorer.as_mut() else {
            return;
        };
        if is_paused {
            if !explorer.flow.pause(&explorer.list) {
                return;
            }
        } else {
            explorer.flow.resume(&mut explorer.list);
            // The held snapshot was built without the joined cells.
            live.join_explorer();
        }
        cx.notify();
    }

    pub(crate) fn event_filter(&self) -> EventFilter {
        self.event_filter
    }

    /// Switches the Events screen between all events and warnings only. The server filters, so the
    /// list reloads; other screens only remember the choice.
    pub(crate) fn set_event_filter(&mut self, filter: EventFilter, cx: &mut Context<Self>) {
        if self.event_filter == filter {
            return;
        }
        self.event_filter = filter;
        let runtime = cx.global::<ClusterRuntime>().clone();
        if let Some(live) = self.live_mut()
            && live.explorer.as_ref().map(|explorer| explorer.kind) == Some(ResourceKind::Events)
        {
            // The old subscription drops first, so two explorer watches never overlap.
            live.explorer = None;
            live.explorer = Some(KindList::start(
                ResourceKind::Events,
                &runtime,
                &live.connection,
                live.scope.clone(),
                filter,
                &live.access,
                cx,
            ));
        }
        cx.notify();
    }

    /// Starts, replaces, or stops the object events watch. The same subject again is a no-op, so
    /// there is no re-list. A session that is not live ignores it: a new session has no selection.
    pub(crate) fn set_event_subject(
        &mut self,
        subject: Option<InvolvedObject>,
        cx: &mut Context<Self>,
    ) {
        let runtime = cx.global::<ClusterRuntime>().clone();
        let Some(live) = self.live_mut() else {
            return;
        };
        if live.event_subject() == subject.as_ref() {
            return;
        }
        // The old subscription drops first, so two object events watches never overlap.
        live.object_events = None;
        live.object_events =
            subject.map(|subject| ObjectEvents::start(subject, &runtime, &live.connection, cx));
        cx.notify();
    }

    /// Starts, replaces, or stops the related objects watch. The same subject again is a no-op,
    /// so there is no re-list. A session that is not live ignores it: a new session has no
    /// selection.
    pub(crate) fn set_related_subject(
        &mut self,
        subject: Option<RelatedSubject>,
        cx: &mut Context<Self>,
    ) {
        let runtime = cx.global::<ClusterRuntime>().clone();
        let Some(live) = self.live_mut() else {
            return;
        };
        // A Namespace drawer restarts when the permissions close or open one of its lists.
        let gates = match &subject {
            Some(RelatedSubject::NamespaceQuotas { .. }) => live.namespace_gates(),
            _ => NamespaceListGates::OPEN,
        };
        let running = live
            .related
            .as_ref()
            .map(|related| (&related.subject, related.gates));
        if related_watch_is_current(running, subject.as_ref(), gates) {
            return;
        }
        // The old subscription drops first, so two related watches never overlap.
        live.related = None;
        live.related = subject
            .map(|subject| RelatedObjects::start(subject, gates, &runtime, &live.connection, cx));
        cx.notify();
    }

    /// Asks for the previous log line of `key` (one request, then cached for its restart count), or
    /// stops asking with `None`. The stream runs on tokio; the line arrives through `subscribe`.
    pub(crate) fn set_last_log(&mut self, key: Option<LastLogKey>, cx: &mut Context<Self>) {
        let runtime = cx.global::<ClusterRuntime>().clone();
        let Some(live) = self.live_mut() else {
            return;
        };
        let Some(key) = key else {
            live.last_logs.stop();
            return;
        };
        if live.last_logs.is_running(&key) {
            return;
        }
        live.last_logs.stop();
        if live.last_logs.has(&key) {
            return;
        }
        let request = LogRequest {
            namespace: key.namespace.clone(),
            pod: key.pod.clone(),
            container: key.container.clone(),
            source: LogSource::Previous,
            tail_lines: 1,
            since_seconds: None,
        };
        let updates = live.connection.pod_logs(request);
        let applied = key.clone();
        let closed = key.clone();
        let subscription = runtime.subscribe(
            updates,
            cx,
            move |session: &mut ClusterSession, update, _| {
                let Some(live) = session.live_mut() else {
                    return;
                };
                match update {
                    LogUpdate::Started => {}
                    LogUpdate::Lines(lines) => {
                        live.last_logs
                            .set(&applied, LastLog::Line(last_line(&lines)));
                    }
                    LogUpdate::Failed(_) => live.last_logs.set(&applied, LastLog::Unavailable),
                }
            },
            move |session, _| {
                let Some(live) = session.live_mut() else {
                    return;
                };
                // A stream that ended with no line means the container logged nothing.
                if live.last_logs.get(&closed) == Some(&LastLog::Loading) {
                    live.last_logs.set(&closed, LastLog::Line(None));
                }
            },
        );
        live.last_logs.start(key, subscription);
        cx.notify();
    }

    /// The kubelet demand of the open drawer. It runs from `render`, so it never notifies: the
    /// targets move through a watch channel and the poll's own updates notify.
    pub(crate) fn set_kubelet_demand(&mut self, demand: KubeletDemand) {
        let Some(live) = self.live_mut() else {
            return;
        };
        live.metrics
            .kubelet
            .set_demand(demand, live.nodes.items(), live.pods.items());
    }

    /// Counts the kinds whose watch is not running, for the sidebar, with one tiny list per kind
    /// and namespace on the cluster runtime. A run for the same scope replaces the last one's
    /// task, and its numbers arrive in one update.
    pub(crate) fn refresh_kind_counts(&mut self, trigger: CountTrigger, cx: &mut Context<Self>) {
        let runtime = cx.global::<ClusterRuntime>().clone();
        let Some(live) = self.live_mut() else {
            return;
        };
        let kinds = countable_kinds(&live.access);
        let now = Instant::now();
        if kinds.is_empty() || !live.kind_counts.wants_run(trigger, &live.scope, now) {
            return;
        }
        let connection = live.connection.clone();
        let scope = live.scope.clone();
        let counting = {
            let scope = scope.clone();
            runtime.spawn(async move {
                futures::stream::iter(kinds)
                    .map(|kind| {
                        let connection = connection.clone();
                        let scope = scope.clone();
                        async move {
                            let Some(object) = kind.builtin_object() else {
                                return (kind, Ok(None));
                            };
                            let count = connection.count_objects(object, &scope).await;
                            (kind, count.map_err(|error| error_text(&error)))
                        }
                    })
                    .buffer_unordered(KIND_COUNT_CONCURRENCY)
                    .collect::<Vec<_>>()
                    .await
            })
        };
        let task = cx.spawn(async move |this, cx| {
            let result = counting.await;
            let _ = this.update(cx, |session, cx| {
                session.finish_kind_counts(result, cx);
            });
        });
        // The old numbers stay on screen until the new ones arrive, unless the scope changed.
        live.kind_counts.scope = Some(scope);
        live.kind_counts.refreshed_at = Some(now);
        live.kind_counts.task = Some(task);
    }

    /// Lists the names of the index kinds for the palette's name search, one metadata-only list
    /// per kind and namespace on the cluster runtime. It starts only when `NameIndex::wants_run`
    /// says so, and not while the access review is still running: the report decides which kinds
    /// are asked for. The names arrive in one update.
    pub(crate) fn request_name_index(&mut self, cx: &mut Context<Self>) {
        let runtime = cx.global::<ClusterRuntime>().clone();
        let Some(live) = self.live_mut() else {
            return;
        };
        if matches!(live.access, AccessState::Checking { .. })
            || !live.name_index.wants_run(&live.scope, Instant::now())
        {
            return;
        }
        let plan = name_index_plan(&live.access);
        let kinds: Vec<ResourceKind> = plan
            .iter()
            .filter(|(_, denied)| denied.is_none())
            .map(|(kind, _)| *kind)
            .collect();
        let connection = live.connection.clone();
        let scope = live.scope.clone();
        let listing = {
            let scope = scope.clone();
            runtime.spawn(async move {
                futures::stream::iter(kinds)
                    .map(|kind| {
                        let connection = connection.clone();
                        let scope = scope.clone();
                        async move {
                            let Some(object) = kind.builtin_object() else {
                                return (kind, Err("not a built-in kind".to_owned()));
                            };
                            let names = connection.list_object_names(object, &scope).await;
                            (kind, names.map_err(|error| error_text(&error)))
                        }
                    })
                    .buffer_unordered(KIND_COUNT_CONCURRENCY)
                    .collect::<Vec<NameListResult>>()
                    .await
            })
        };
        let run_scope = scope.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = listing.await;
            let _ = this.update(cx, |session, cx| {
                session.finish_name_index(&run_scope, result, cx);
            });
        });
        live.name_index.begin(scope, &plan, task);
    }

    fn finish_name_index(
        &mut self,
        scope: &NamespaceScope,
        result: Result<Vec<NameListResult>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        let Some(live) = self.live_mut() else {
            return;
        };
        // A run that stopped answers nothing, so every list still loading fails.
        let results = result.unwrap_or_default();
        for (kind, names) in &results {
            if let Err(message) = names {
                tracing::warn!(kind = kind.label(), %message, "listing object names failed");
            }
        }
        // A scope change drops the index with its run; the scope check is the second gate.
        if live.name_index.finish(scope, results, Instant::now()) {
            cx.notify();
        }
    }

    /// Counts the instances of every served custom kind whose list is not denied, cluster-wide,
    /// for the CRDs Instances column and the sidebar. One tiny list per CRD, four at a time, on
    /// the cluster runtime; the numbers arrive in one update.
    pub(crate) fn refresh_custom_counts(&mut self, cx: &mut Context<Self>) {
        let runtime = cx.global::<ClusterRuntime>().clone();
        let Some(live) = self.live_mut() else {
            return;
        };
        let now = Instant::now();
        let kinds = countable_custom_kinds(live.crd_kinds(), &live.custom_gates);
        // The CRD list has not loaded yet: nothing to count, and the next update asks again.
        if kinds.is_empty() || !live.custom_counts.wants_run(now) {
            return;
        }
        let connection = live.connection.clone();
        let counting = runtime.spawn(async move {
            futures::stream::iter(kinds)
                .map(|kind| {
                    let connection = connection.clone();
                    async move {
                        let count = connection.count_custom_objects(kind.resource()).await;
                        (kind, count.map_err(|error| error_text(&error)))
                    }
                })
                .buffer_unordered(KIND_COUNT_CONCURRENCY)
                .collect::<Vec<_>>()
                .await
        });
        let task = cx.spawn(async move |this, cx| {
            let result = counting.await;
            let _ = this.update(cx, |session, cx| session.finish_custom_counts(result, cx));
        });
        live.custom_counts.refreshed_at = Some(now);
        live.custom_counts.task = Some(task);
    }

    fn finish_custom_counts(
        &mut self,
        result: Result<Vec<CustomCount>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        let Some(live) = self.live_mut() else {
            return;
        };
        live.custom_counts.task = None;
        // A task that stopped leaves the old numbers; the next visit after 30 s counts again.
        let Ok(results) = result else {
            return;
        };
        for (kind, count) in &results {
            if let Err(message) = count {
                tracing::warn!(crd = kind.crd_name(), %message, "counting instances failed");
            }
        }
        live.custom_counts.counts = merge_custom_counts(&live.custom_counts.counts, results);
        live.join_explorer();
        cx.notify();
    }

    fn finish_kind_counts(
        &mut self,
        result: Result<Vec<KindCount>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        // A scope change drops the run with its `KindCounts`, so a result that arrives is current.
        let Some(live) = self.live_mut() else {
            return;
        };
        live.kind_counts.task = None;
        // A task that stopped leaves the old numbers; the next navigation after 30 s counts again.
        let Ok(results) = result else {
            return;
        };
        let mut counts = HashMap::new();
        for (kind, count) in results {
            match count {
                Ok(Some(count)) => {
                    counts.insert(kind, count);
                }
                // A server that reports no remaining count leaves the kind without a number.
                Ok(None) => {}
                Err(message) => {
                    tracing::warn!(kind = kind.label(), %message, "counting objects failed");
                }
            }
        }
        live.kind_counts.counts = counts;
        cx.notify();
    }

    fn finish_access_review(
        &mut self,
        result: Result<Result<AccessReport, ClusterError>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        let Some(live) = self.live_mut() else {
            return;
        };
        let review = match result {
            Ok(Ok(report)) => Ok(report),
            Ok(Err(error)) => Err(error_text(&error)),
            Err(_) => Err("the access review task stopped unexpectedly".to_owned()),
        };
        let runtime = cx.global::<ClusterRuntime>().clone();
        live.access = AccessState::from_review(review);
        live.drop_denied_companion();
        live.plan_change_events(&runtime, cx);
        live.replan_topology(&runtime, cx);
        // The review decides which condition feeds may start.
        live.issue_feeds.restart_conditions(
            &runtime,
            &live.connection,
            &live.scope,
            &live.access,
            cx,
        );
        live.start_crd_watch(&runtime, cx);
        // A denial that arrives after the CRDs screen opened fails its list.
        if live.crds.is_none() {
            live.seed_crd_explorer();
        }
        self.refresh_kind_counts(CountTrigger::Review, cx);
        self.update_metrics_feeds(cx);
        cx.notify();
    }

    /// Applies a CRD update and rebuilds the custom kinds, reusing every cached definition.
    fn apply_crd_update(&mut self, update: CrdUpdate, cx: &mut Context<Self>) {
        let kinds = match &update {
            CrdUpdate::Snapshot { crds, .. } => {
                Some(custom_kinds(crds, &mut self.custom_kind_cache))
            }
            CrdUpdate::Failed(_) => None,
        };
        let Some(live) = self.live_mut() else {
            return;
        };
        live.apply_crd_update(update, kinds);
        // The CRDs screen was waiting for the list to know what to count.
        if live
            .explorer
            .as_ref()
            .is_some_and(|explorer| explorer.kind == ResourceKind::Crds)
        {
            self.refresh_custom_counts(cx);
        }
    }

    /// The review of a custom kind finished. Allowed, or a review that failed (nothing cached, so
    /// the next visit asks again), starts the list; a denial caches and fails it with the reason.
    fn finish_custom_gate(
        &mut self,
        custom: CustomKind,
        result: Result<Result<AccessDecision, ClusterError>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        let event_filter = self.event_filter;
        let runtime = cx.global::<ClusterRuntime>().clone();
        let Some(live) = self.live_mut() else {
            return;
        };
        let kind = ResourceKind::Custom(custom);
        let outcome = gate_outcome(
            result.as_ref().ok().map(Result::as_ref),
            custom,
            &live.scope,
        );
        match &outcome {
            GateOutcome::Allowed => {
                live.custom_gates.insert(custom, CustomGate::Allowed);
            }
            GateOutcome::Denied(reason) => {
                let gate = CustomGate::Denied {
                    reason: reason.clone(),
                };
                live.custom_gates.insert(custom, gate);
            }
            GateOutcome::Unknown => {
                live.custom_gates.remove(&custom);
            }
        }
        let shown = live.explorer.as_ref().map(|explorer| explorer.kind);
        match explorer_action(shown, kind, outcome) {
            ExplorerAction::Leave => {}
            ExplorerAction::Fail(message) => {
                live.explorer = Some(KindList::unsubscribed(kind, LiveList::Failed { message }));
            }
            ExplorerAction::Start => {
                let scope = live.scope.clone();
                live.start_explorer(kind, scope, event_filter, &runtime, cx);
            }
        }
        cx.notify();
    }

    /// The `pod-monitor-source-fixture` screen sends no request to a source: the session forgets
    /// the stored entry, so no check runs.
    #[cfg(feature = "screenshot")]
    pub(crate) fn forget_metrics_source_for_fixture(&mut self) {
        self.source_entry = None;
    }

    /// Follows the stored metrics source of the cluster. The same entry is a no-op; another one
    /// rebuilds the state at once (a running check is dropped), or waits for the session to be live.
    pub(crate) fn set_metrics_source(
        &mut self,
        entry: Option<Result<MetricsSource, MetricsSourceError>>,
        cx: &mut Context<Self>,
    ) {
        if self.source_entry == entry {
            return;
        }
        self.source_entry = entry;
        self.restart_source_check(cx);
    }

    /// Builds the `SourceState` from the stored entry: one `check_metrics_source` for a valid one,
    /// no request for `Invalid` or `None`. A live session only.
    fn restart_source_check(&mut self, cx: &mut Context<Self>) {
        let runtime = cx.global::<ClusterRuntime>().clone();
        let entry = self.source_entry.clone();
        let Some(live) = self.live_mut() else {
            return;
        };
        // The list belongs to the source it was read from.
        live.metrics.traffic_sources = TrafficSources::NotLoaded;
        live.metrics.source = match entry {
            None => SourceState::None,
            Some(Err(_)) => SourceState::Invalid,
            Some(Ok(source)) => {
                let connection = live.connection.clone();
                let checked = source.clone();
                let checking =
                    runtime.spawn(async move { connection.check_metrics_source(&checked).await });
                let task = cx.spawn(async move |this, cx| {
                    let result = checking.await;
                    let _ = this.update(cx, |session, cx| session.finish_source_check(result, cx));
                });
                SourceState::Checking {
                    source,
                    _task: task,
                }
            }
        };
        cx.notify();
    }

    /// Reads which traffic metrics the Ready source holds (spec 0049): one `metric_names` request,
    /// started by the first Traffic use. A list already loaded stays; a failed one is read again.
    pub(crate) fn load_traffic_sources(&mut self, cx: &mut Context<Self>) {
        let runtime = cx.global::<ClusterRuntime>().clone();
        let Some(live) = self.live_mut() else {
            return;
        };
        match &live.metrics.traffic_sources {
            TrafficSources::Loading { .. } | TrafficSources::Loaded(Ok(_)) => return,
            TrafficSources::NotLoaded | TrafficSources::Loaded(Err(_)) => {}
        }
        let Some((source, _)) = live.metrics.source.ready() else {
            return;
        };
        let source = source.clone();
        let connection = live.connection.clone();
        let loading = runtime.spawn(async move {
            connection
                .metric_names(&source)
                .await
                .map(|names| TrafficMetricSource::detect(&names))
        });
        let task = cx.spawn(async move |this, cx| {
            let result = loading.await;
            let _ = this.update(cx, |session, cx| session.finish_traffic_sources(result, cx));
        });
        live.metrics.traffic_sources = TrafficSources::Loading { _task: task };
        cx.notify();
    }

    fn finish_traffic_sources(
        &mut self,
        result: Result<Result<Vec<TrafficMetricSource>, MetricsError>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        let Some(live) = self.live_mut() else {
            return;
        };
        if !matches!(live.metrics.traffic_sources, TrafficSources::Loading { .. }) {
            return;
        }
        live.metrics.traffic_sources = TrafficSources::Loaded(result.unwrap_or_else(|_| {
            Err(MetricsError::Unexpected(
                "the metric list task stopped unexpectedly".to_owned(),
            ))
        }));
        cx.notify();
    }

    fn finish_source_check(
        &mut self,
        result: Result<Result<SourceCheck, MetricsError>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        let Some(live) = self.live_mut() else {
            return;
        };
        let SourceState::Checking { source, .. } = &live.metrics.source else {
            return;
        };
        let source = source.clone();
        live.metrics.source = match result {
            Ok(Ok(check)) => SourceState::Ready { source, check },
            Ok(Err(error)) => SourceState::Failed { source, error },
            Err(_) => SourceState::Failed {
                source,
                error: MetricsError::Unexpected("the check task stopped unexpectedly".to_owned()),
            },
        };
        cx.notify();
    }

    fn finish_pod_review(
        &mut self,
        result: Result<Result<Vec<NamespaceAccess>, ClusterError>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        let Some(live) = self.live_mut() else {
            return;
        };
        let review: PodReviewResult = match result {
            Ok(Ok(entries)) => Ok(entries),
            Ok(Err(error)) => Err(error_text(&error)),
            Err(_) => Err("the metrics access review task stopped unexpectedly".to_owned()),
        };
        live.metrics.finish_pod_review(review);
        self.update_metrics_feeds(cx);
        cx.notify();
    }

    /// Starts or stops each metrics poll from its access gate. Safe to call at any time: a feed
    /// that already polls is left running.
    fn update_metrics_feeds(&mut self, cx: &mut Context<Self>) {
        // The coverage reads the feed states, which this may change.
        self.issues.mark_dirty();
        let runtime = cx.global::<ClusterRuntime>().clone();
        let Some(live) = self.live_mut() else {
            return;
        };
        match pods_gate(live.metrics.pod_review(), &live.scope) {
            PodsGate::Wait => live.metrics.pods.wait(),
            PodsGate::Off(reason) => live.metrics.pods.turn_off(reason),
            PodsGate::Poll { scope, note } => {
                let connection = &live.connection;
                live.metrics.pods.poll(note, || {
                    subscribe_pod_metrics(&runtime, connection, scope, cx)
                });
            }
        }
        match nodes_gate(&live.access, AccessCheck::ListNodeMetrics) {
            NodesGate::Wait => live.metrics.nodes.wait(),
            NodesGate::Off(reason) => live.metrics.nodes.turn_off(reason),
            NodesGate::Poll => {
                let connection = &live.connection;
                live.metrics
                    .nodes
                    .poll(None, || subscribe_node_metrics(&runtime, connection, cx));
            }
        }
        match nodes_gate(&live.access, AccessCheck::GetNodeProxy) {
            NodesGate::Wait => live.metrics.kubelet.wait(),
            NodesGate::Off(reason) => live.metrics.kubelet.turn_off(reason),
            NodesGate::Poll => {
                let connection = &live.connection;
                live.metrics
                    .kubelet
                    .poll(live.nodes.items(), live.pods.items(), |targets| {
                        subscribe_kubelet_stats(&runtime, connection, targets, cx)
                    });
            }
        }
        cx.notify();
    }
}

impl LiveCluster {
    /// The byte counters of this cluster's API client.
    pub(crate) fn traffic(&self) -> TrafficCounter {
        self.connection.traffic()
    }

    /// The ReplicaSets behind the 24 h range of Recent changes, while that watch runs.
    pub(crate) fn rollout_history(&self) -> Option<&LiveList<ReplicaSetSummary>> {
        match self.change_events.as_ref()? {
            ChangeEvents::Live {
                history: Some(history),
                ..
            } => Some(&history.replica_sets),
            ChangeEvents::Live { history: None, .. } | ChangeEvents::Denied => None,
        }
    }

    /// Whether the ReplicaSets of the 24 h range have not delivered their first snapshot.
    pub(crate) fn is_rollout_history_loading(&self) -> bool {
        self.rollout_history().is_some_and(LiveList::is_loading)
    }

    /// Starts the change feeds unless a known review denies listing events.
    fn start_change_events(&mut self, runtime: &ClusterRuntime, cx: &mut Context<ClusterSession>) {
        self.change_events = Some(if is_change_feed_denied(&self.access) {
            ChangeEvents::Denied
        } else {
            ChangeEvents::start(
                runtime,
                &self.connection,
                &self.scope,
                self.keeps_rollout_history,
                cx,
            )
        });
    }

    /// A review that finished moves a running feed to `Denied` and a denied one to running.
    fn plan_change_events(&mut self, runtime: &ClusterRuntime, cx: &mut Context<ClusterSession>) {
        let is_denied = is_change_feed_denied(&self.access);
        let needs_new_plan = match &self.change_events {
            Some(ChangeEvents::Live { .. }) => is_denied,
            Some(ChangeEvents::Denied) => !is_denied,
            None => false,
        };
        if needs_new_plan {
            self.change_events = None;
            self.start_change_events(runtime, cx);
        }
    }

    /// Starts the feeds of `subject`, each unless a known review denies its list.
    fn start_topology(
        &self,
        subject: TopologySubject,
        runtime: &ClusterRuntime,
        cx: &mut Context<ClusterSession>,
    ) -> TopologyFeeds {
        let feeds = subject
            .wanted_kinds()
            .into_iter()
            .map(|kind| self.topology_feed(kind, &subject.namespace, runtime, cx))
            .collect();
        TopologyFeeds { subject, feeds }
    }

    /// The feed of `kind` in `namespace`: a watch, or off when a known review denies the list.
    fn topology_feed(
        &self,
        kind: ResourceKind,
        namespace: &str,
        runtime: &ClusterRuntime,
        cx: &mut Context<ClusterSession>,
    ) -> TopologyFeed {
        if let FeedStart::Off(reason) = feed_plan(kind, &self.access) {
            return TopologyFeed::off(kind, reason);
        }
        let scope = NamespaceScope::Named(namespace.to_owned());
        let Some(updates) = kind.watch_rows(&self.connection, scope, EventFilter::default()) else {
            return TopologyFeed::off(kind, "not available".to_owned());
        };
        let subscription = runtime.subscribe(
            updates,
            cx,
            move |session: &mut ClusterSession, update, _| {
                if let Some(feed) = session.topology_feed_mut(kind) {
                    feed.list.apply(update);
                }
            },
            move |session, _| {
                if let Some(feed) = session.topology_feed_mut(kind) {
                    feed.list.mark_stopped();
                }
            },
        );
        TopologyFeed::started(kind, subscription)
    }

    /// A review that finished turns an Off feed on when it allows the list and a running feed off
    /// when it denies it, so a 403 never retries.
    fn replan_topology(&mut self, runtime: &ClusterRuntime, cx: &mut Context<ClusterSession>) {
        let Some(feeds) = self.topology.as_ref() else {
            return;
        };
        let namespace = feeds.subject.namespace.clone();
        let changed: Vec<ResourceKind> = feeds
            .feeds
            .iter()
            .filter(|feed| {
                feed.is_open() != matches!(feed_plan(feed.kind, &self.access), FeedStart::Start)
            })
            .map(|feed| feed.kind)
            .collect();
        for kind in changed {
            let feed = self.topology_feed(kind, &namespace, runtime, cx);
            if let Some(slot) = self
                .topology
                .as_mut()
                .and_then(|feeds| feeds.feed_mut(kind))
            {
                *slot = feed;
            }
        }
    }

    /// Moves the kubelet targets with the nodes and pods lists.
    fn refresh_kubelet_targets(&self) {
        self.metrics
            .kubelet
            .refresh_targets(self.nodes.items(), self.pods.items());
    }

    /// The context namespace, or `default`.
    pub(crate) fn default_namespace(&self) -> &str {
        self.connection.default_namespace()
    }

    /// The connection that log streams open on.
    pub(crate) fn connection(&self) -> &ClusterConnection {
        &self.connection
    }

    /// `all namespaces` or the namespace name, as used in headers and empty states.
    pub(crate) fn scope_label(&self) -> String {
        match &self.scope {
            NamespaceScope::All => "all namespaces".to_owned(),
            NamespaceScope::Named(namespace) => namespace.clone(),
            NamespaceScope::Several(names) => namespaces_label(names),
        }
    }

    /// The counted sidebar numbers of kinds without a running watch.
    pub(crate) fn kind_counts(&self) -> &KindCounts {
        &self.kind_counts
    }

    /// The kind and item count of the loaded explorer list, for the sidebar.
    pub(crate) fn explorer_count(&self) -> Option<(ResourceKind, usize)> {
        let explorer = self.explorer.as_ref()?;
        Some((explorer.kind, explorer.list.ready_count()?))
    }

    /// The served custom kinds of the CRD list; empty until it has loaded.
    pub(crate) fn crd_kinds(&self) -> &[CustomKind] {
        self.crds.as_ref().map_or(&[], |crds| &crds.kinds)
    }

    /// Open watches: namespaces, pods, nodes, the Warning events of the Issues engine, the
    /// explorer's and its companion, the drawer's events and related objects when they are open,
    /// and the change-event watches while Overview is visible.
    pub(crate) fn watch_count(&self) -> usize {
        let namespaces = scope_multiplicity(&self.scope);
        open_watch_count(OpenWatches {
            namespaces,
            crds: self.crds.is_some(),
            explorer: self
                .explorer
                .as_ref()
                .filter(|explorer| explorer.is_watching())
                .map_or(0, |explorer| explorer_watches(explorer.kind, namespaces)),
            companion: self
                .companion()
                .map_or(0, |lists| lists.watches(namespaces)),
            object_events: self.object_events.is_some(),
            related: self.related.is_some(),
            issue_feeds: self.issue_feeds.watch_count(namespaces),
            change_events: match self.change_events {
                Some(ChangeEvents::Live { ref history, .. }) => {
                    (CHANGE_EVENT_KINDS.len() + usize::from(history.is_some())) * namespaces
                }
                Some(ChangeEvents::Denied) | None => 0,
            },
            topology: self.topology.as_ref().map_or(0, TopologyFeeds::open_count),
        })
    }

    /// The kinds behind `watch_count`, by name: the same watches, listed. The totals match:
    /// `watched_kinds` counts sum to `watch_count`.
    pub(crate) fn watched_kinds(&self) -> Vec<WatchedKind> {
        let namespaces = scope_multiplicity(&self.scope);
        let explorer = self
            .explorer
            .as_ref()
            .filter(|explorer| explorer.is_watching())
            .map(|explorer| {
                (
                    explorer.kind.label(),
                    explorer_watches(explorer.kind, namespaces),
                )
            });
        let companion = self
            .companion()
            .into_iter()
            .flat_map(|lists| lists.watched(namespaces));
        let (change_events, history) = match self.change_events {
            Some(ChangeEvents::Live { ref history, .. }) => {
                (namespaces, namespaces * usize::from(history.is_some()))
            }
            Some(ChangeEvents::Denied) | None => (0, 0),
        };
        let topology = self.topology.iter().flat_map(|topology| {
            topology
                .feeds
                .iter()
                .filter(|feed| feed.is_open())
                .map(|feed| (feed.kind.label(), 1))
        });
        merge_watched(
            [
                ("Namespaces", 1),
                ("Nodes", 1),
                ("Pods", namespaces),
                (ResourceKind::Crds.label(), usize::from(self.crds.is_some())),
                ("Object events", usize::from(self.object_events.is_some())),
                ("Related objects", usize::from(self.related.is_some())),
                ("Rollout events", change_events),
                ("Rescale events", change_events),
                ("Rollout history", history),
            ]
            .into_iter()
            .chain(explorer)
            .chain(companion)
            .chain(self.issue_feeds.watched(namespaces))
            .chain(topology),
        )
    }

    /// The companion lists of the explorer, when it runs a companion watch.
    pub(crate) fn companion(&self) -> Option<&CompanionLists> {
        let companion = self.explorer.as_ref()?.companion.as_ref()?;
        Some(&companion.lists)
    }

    /// The companion lists a drawer of `kind` reads, wherever it is open: the explorer's companion
    /// on its kind screen, else the lists rebuilt from the Topology feeds (no explorer runs over
    /// the graph, so a drawer would wait on a companion that never starts). A list Topology does
    /// not watch reads as failed with that reason, never as loading. `None` for a kind without a
    /// companion, while the access report denies it (the drawer then names the check), and when
    /// neither the explorer nor Topology runs.
    pub(crate) fn companion_source(&self, kind: ResourceKind) -> Option<CompanionSource<'_>> {
        let CompanionPlan::Start(companion) = companion_plan(kind, &self.access) else {
            return None;
        };
        let is_explorer_kind = self
            .explorer
            .as_ref()
            .is_some_and(|explorer| explorer.kind == kind);
        if is_explorer_kind && let Some(lists) = self.companion() {
            return Some(CompanionSource::Explorer(lists));
        }
        let topology = self.topology.as_ref()?;
        Some(CompanionSource::Topology(
            topology.companion_lists(companion),
        ))
    }

    /// Rewrites the joined cells of the explorer rows from the current pods and companion lists.
    /// Runs after every update of the explorer, the pods, or the companion; the explorer and the
    /// companion are never paused, so frozen rows need no join.
    fn join_explorer(&mut self) {
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        let inputs = JoinInputs {
            pods: &self.pods,
            companion: explorer
                .companion
                .as_ref()
                .map(|companion| &companion.lists),
            kubelet: Some(&self.metrics.kubelet.history),
            scope: &self.scope,
            custom_counts: Some(&self.custom_counts.counts),
        };
        join_rows(explorer.kind, explorer.list.items_mut(), &inputs);
    }

    /// Stops the companion watch once the access report denies its list, so a 403 does not retry
    /// forever; the drawer then shows the check as the reason.
    fn drop_denied_companion(&mut self) {
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        let is_denied = matches!(
            companion_plan(explorer.kind, &self.access),
            CompanionPlan::Denied(_)
        );
        if is_denied && explorer.companion.take().is_some() {
            self.join_explorer();
        }
    }

    /// Applies a companion update for the explorer of `kind`; a stale one for another kind is
    /// dropped.
    fn apply_companion_update(&mut self, kind: ResourceKind, update: CompanionUpdate) {
        let Some(companion) = self
            .explorer
            .as_mut()
            .filter(|explorer| explorer.kind == kind)
            .and_then(|explorer| explorer.companion.as_mut())
        else {
            return;
        };
        companion.lists.apply(update);
        self.join_explorer();
    }

    /// Marks the companion of the explorer of `kind` as stopped after its stream ended.
    fn stop_companion(&mut self, kind: ResourceKind) {
        let Some(companion) = self
            .explorer
            .as_mut()
            .filter(|explorer| explorer.kind == kind)
            .and_then(|explorer| explorer.companion.as_mut())
        else {
            return;
        };
        companion.lists.mark_stopped();
        self.join_explorer();
    }

    /// The subject of the running related watch.
    pub(crate) fn related_subject(&self) -> Option<&RelatedSubject> {
        self.related.as_ref().map(|related| &related.subject)
    }

    /// The gates of the Namespace drawer's lists now: the permissions decide, and while they are
    /// being reviewed the running watch keeps its own.
    pub(crate) fn namespace_gates(&self) -> NamespaceListGates {
        let running = self
            .related
            .as_ref()
            .map_or(NamespaceListGates::OPEN, |related| related.gates);
        namespace_list_gates(&self.access, running)
    }

    /// The last line the previous container of a crash-looping pod logged, once it has arrived.
    pub(crate) fn last_log_of(&self, key: &LastLogKey) -> Option<&str> {
        match self.last_logs.get(key)? {
            LastLog::Line(Some(line)) => Some(line),
            LastLog::Line(None) | LastLog::Loading | LastLog::Unavailable => None,
        }
    }

    /// The related list of `subject`, or `None` while another subject (or none) is watched.
    pub(crate) fn related_of(&self, subject: &RelatedSubject) -> Option<&RelatedList> {
        self.related
            .as_ref()
            .filter(|related| related.subject == *subject)
            .map(|related| &related.list)
    }

    /// Whether an instance count run has not delivered its numbers. Only the screenshot hook
    /// waits on it.
    #[cfg(feature = "screenshot")]
    pub(crate) fn is_counting_instances(&self) -> bool {
        self.custom_counts.task.is_some()
    }

    /// Whether the related watch runs and has not delivered its first snapshot. Only the
    /// screenshot hook waits on it.
    #[cfg(feature = "screenshot")]
    pub(crate) fn is_related_loading(&self) -> bool {
        self.related
            .as_ref()
            .is_some_and(|related| related.list.is_loading())
    }

    /// Whether what the explorer rows join with has not delivered a first snapshot: the pods for
    /// Services, ConfigMaps, and Namespaces, and the companion watch. Only the screenshot hook
    /// waits on it.
    #[cfg(feature = "screenshot")]
    pub(crate) fn is_join_loading(&self) -> bool {
        let Some(explorer) = self.explorer.as_ref() else {
            return false;
        };
        let joins_pods = matches!(
            explorer.kind,
            ResourceKind::Services
                | ResourceKind::ConfigMaps
                | ResourceKind::Namespaces
                | ResourceKind::NetworkPolicies
                | ResourceKind::ServiceAccounts
        );
        (joins_pods && self.pods.is_loading())
            || self.companion().is_some_and(CompanionLists::is_loading)
    }

    /// The subject of the running object events watch.
    pub(crate) fn event_subject(&self) -> Option<&InvolvedObject> {
        self.object_events.as_ref().map(|events| &events.subject)
    }

    /// The events list of `subject`, or `None` while another subject (or none) is watched.
    pub(crate) fn events_of(&self, subject: &InvolvedObject) -> Option<&LiveList<EventSummary>> {
        self.object_events
            .as_ref()
            .filter(|events| events.subject == *subject)
            .map(|events| &events.list)
    }

    /// Whether the object events watch runs and has not delivered its first snapshot. Only the
    /// screenshot hook waits on it.
    #[cfg(feature = "screenshot")]
    pub(crate) fn is_object_events_loading(&self) -> bool {
        self.object_events
            .as_ref()
            .is_some_and(|events| events.list.is_loading())
    }

    /// Whether any watch has failed or is interrupted, for the status bar.
    pub(crate) fn has_problem(&self) -> bool {
        self.crds
            .as_ref()
            .is_some_and(|crds| crds.list.has_problem())
            || any_list_has_problem(
                &self.namespaces,
                &self.pods,
                &self.nodes,
                self.explorer.as_ref().map(|explorer| &explorer.list),
            )
    }

    /// Whether the explorer list is paused; `None` without an explorer.
    pub(crate) fn explorer_flow(&self) -> Option<FlowState> {
        Some(match &self.explorer.as_ref()?.flow {
            StreamFlow::Live => FlowState::Live,
            StreamFlow::Paused { held } => FlowState::Paused {
                has_held: held.is_some(),
            },
        })
    }

    /// The Topology feeds, while Topology is shown.
    pub(crate) fn topology(&self) -> Option<&TopologyFeeds> {
        self.topology.as_ref()
    }

    /// The kind row of `key`: the explorer list first, then the Topology feeds. Only the drawer, the
    /// kind menu, and the YAML lookups read it, so over Topology they find their row.
    pub(crate) fn row_of(&self, key: &ResourceKey) -> Option<&KindRow> {
        let ResourceKey::Kind { kind, .. } = key else {
            return None;
        };
        row_in(key, *kind, self.kind_list(*kind), self.topology())
    }

    /// The HPAs this session already holds (the Issues feed that watches them), empty while that
    /// list is loading, denied, or off: the Scale warning uses them and starts no list for it.
    pub(crate) fn loaded_hpas(&self) -> &[KindObject] {
        self.issue_feeds
            .conditions
            .iter()
            .find(|feed| feed.kind == ResourceKind::HorizontalPodAutoscalers)
            .map_or(&[][..], |feed| feed.list.items())
    }

    /// The ResourceQuotas the Issues feed already holds, empty while that list is loading, denied, or
    /// off: the Scale warning uses them and starts no list for it.
    pub(crate) fn loaded_quotas(&self) -> &[KindObject] {
        self.issue_feeds
            .conditions
            .iter()
            .find(|feed| feed.kind == ResourceKind::ResourceQuotas)
            .map_or(&[][..], |feed| feed.list.items())
    }

    /// The PodDisruptionBudgets this session already holds (the Issues feed that watches them),
    /// empty while that list is loading, denied, or off: the Scale warning uses them and starts no
    /// list for it.
    pub(crate) fn loaded_pdbs(&self) -> &[KindObject] {
        self.issue_feeds
            .conditions
            .iter()
            .find(|feed| feed.kind == ResourceKind::PodDisruptionBudgets)
            .map_or(&[][..], |feed| feed.list.items())
    }

    /// The storage classes the explorer holds, empty unless it shows the StorageClasses screen and
    /// its first snapshot arrived: an Expand checks the claim's class against them when they are
    /// there and starts no list for the hint (spec 0032b decision 10).
    pub(crate) fn loaded_storage_classes(&self) -> Vec<&StorageClassSummary> {
        self.kind_list(ResourceKind::StorageClasses)
            .and_then(|explorer| explorer.list.ready_items())
            .into_iter()
            .flatten()
            .filter_map(|row| match &row.object {
                KindObject::StorageClass(class) => Some(class),
                _ => None,
            })
            .collect()
    }

    /// The explorer list of `kind`, or `None` while another kind (or no kind) is shown.
    pub(crate) fn kind_list(&self, kind: ResourceKind) -> Option<&KindList> {
        self.explorer
            .as_ref()
            .filter(|explorer| explorer.kind == kind)
    }

    /// The listed workload `key` names: the explorer or Topology row first, else the condition feed
    /// of its kind (Deployments and DaemonSets are always watched). `None` for a StatefulSet while
    /// its screen is not the one shown.
    pub(crate) fn workload_object(&self, key: &ResourceKey) -> Option<&KindObject> {
        if let Some(row) = self.row_of(key) {
            return Some(&row.object);
        }
        let ResourceKey::Kind {
            kind,
            namespace: Some(namespace),
            name,
        } = key
        else {
            return None;
        };
        let feed = self.issue_feeds.condition(*kind)?.list.ready_items()?;
        feed.iter().find(|object| match object {
            KindObject::Deployment(deployment) => {
                deployment.namespace == *namespace && deployment.name == *name
            }
            KindObject::DaemonSet(set) => set.namespace == *namespace && set.name == *name,
            _ => false,
        })
    }

    /// The Deployment as this session lists it: the Deployments screen when it is shown, else the
    /// Deployments condition feed. `None` while neither has the Deployment.
    pub(crate) fn deployment_of(&self, namespace: &str, name: &str) -> Option<&DeploymentSummary> {
        let on_screen = self
            .kind_list(ResourceKind::Deployments)
            .and_then(|kind_list| kind_list.list.ready_items())
            .into_iter()
            .flatten()
            .map(|row| &row.object);
        let in_feed = self
            .issue_feeds
            .condition(ResourceKind::Deployments)
            .and_then(|feed| feed.list.ready_items())
            .into_iter()
            .flatten();
        on_screen.chain(in_feed).find_map(|object| match object {
            KindObject::Deployment(deployment)
                if deployment.namespace == namespace && deployment.name == name =>
            {
                Some(deployment)
            }
            _ => None,
        })
    }

    /// The selector of a Deployment as this session lists it, `key=value` terms joined by `,` (the
    /// syntax of a label selector query).
    pub(crate) fn deployment_selector(&self, namespace: &str, name: &str) -> Option<String> {
        Some(self.deployment_of(namespace, name)?.selector.join(","))
    }

    fn start(
        connected: Connected,
        explorer_kind: Option<ResourceKind>,
        event_filter: EventFilter,
        is_overview_visible: bool,
        keeps_rollout_history: bool,
        topology_subject: Option<TopologySubject>,
        cx: &mut Context<ClusterSession>,
    ) -> Self {
        let runtime = cx.global::<ClusterRuntime>().clone();
        let Connected {
            connection,
            server_version,
            api_latency,
            scope,
            access,
        } = connected;
        let subscriptions = Subscriptions {
            _namespaces: runtime.subscribe(
                connection.watch_namespaces(),
                cx,
                |session: &mut ClusterSession, update, _| {
                    session.issues.mark_dirty();
                    if let Some(live) = session.live_mut() {
                        live.namespaces.apply(update);
                    }
                },
                |session, _| {
                    session.issues.mark_dirty();
                    if let Some(live) = session.live_mut() {
                        live.namespaces.mark_stopped();
                    }
                },
            ),
            _pods: subscribe_pods(&runtime, &connection, scope.clone(), cx),
            _nodes: runtime.subscribe(
                connection.watch_nodes(),
                cx,
                |session: &mut ClusterSession, update, _| {
                    session.issues.mark_dirty();
                    if let Some(live) = session.live_mut() {
                        live.nodes.apply(update);
                        live.refresh_kubelet_targets();
                    }
                },
                |session, _| {
                    session.issues.mark_dirty();
                    if let Some(live) = session.live_mut() {
                        live.nodes.mark_stopped();
                    }
                },
            ),
        };
        let access = AccessState::from_review(access);
        let metrics =
            ClusterMetrics::new(start_pod_review(&runtime, &connection, scope.clone(), cx));
        let explorer_scope = scope.clone();
        let issue_feeds = IssueFeeds::start(&runtime, &connection, scope.clone(), cx);
        let mut live = Self {
            server_version,
            api_latency,
            scope,
            access,
            kind_access: KindAccessMap::new(),
            rbac: RbacState::Idle,
            can_do: CanDoCell::default(),
            namespaces: LiveList::Loading,
            pods: LiveList::Loading,
            nodes: LiveList::Loading,
            metrics,
            crds: None,
            custom_counts: CustomCounts::default(),
            custom_gates: HashMap::new(),
            explorer: None,
            object_events: None,
            related: None,
            last_logs: LastLogs::default(),
            kind_counts: KindCounts::default(),
            name_index: NameIndex::default(),
            issue_feeds,
            change_events: None,
            keeps_rollout_history,
            topology: None,
            connection,
            subscriptions,
        };
        if let Some(kind) = explorer_kind {
            live.start_explorer(kind, explorer_scope, event_filter, &runtime, cx);
        }
        live.start_crd_watch(&runtime, cx);
        live.seed_crd_explorer();
        live.issue_feeds.restart_conditions(
            &runtime,
            &live.connection,
            &live.scope,
            &live.access,
            cx,
        );
        if is_overview_visible {
            live.start_change_events(&runtime, cx);
        }
        live.topology = topology_subject.map(|subject| live.start_topology(subject, &runtime, cx));
        live
    }

    /// Starts the explorer of `kind` for `scope`, dropping the old one first so two explorer
    /// watches never overlap. A custom kind first needs its per-resource list review: until it
    /// is allowed (or failed, which is not cached) the list has no watch of its own.
    fn start_explorer(
        &mut self,
        kind: ResourceKind,
        scope: NamespaceScope,
        event_filter: EventFilter,
        runtime: &ClusterRuntime,
        cx: &mut Context<ClusterSession>,
    ) {
        self.explorer = None;
        let started = |live: &Self, cx: &mut Context<ClusterSession>| {
            KindList::start(
                kind,
                runtime,
                &live.connection,
                scope.clone(),
                event_filter,
                &live.access,
                cx,
            )
        };
        let Some(custom) = kind.custom() else {
            self.explorer = Some(started(self, cx));
            return;
        };
        let explorer = match self.custom_gates.get(&custom) {
            Some(CustomGate::Allowed) => started(self, cx),
            Some(CustomGate::Denied { reason }) => KindList::unsubscribed(
                kind,
                LiveList::Failed {
                    message: reason.clone(),
                },
            ),
            Some(CustomGate::Checking { .. }) => KindList::unsubscribed(kind, LiveList::Loading),
            None => {
                let gate = review_custom_gate(runtime, &self.connection, custom, scope.clone(), cx);
                self.custom_gates.insert(custom, gate);
                KindList::unsubscribed(kind, LiveList::Loading)
            }
        };
        self.explorer = Some(explorer);
    }

    /// Whether the access report is known and denies listing CRDs.
    pub(crate) fn is_crds_denied(&self) -> bool {
        crds_denied(&self.access)
    }

    /// Starts the CRD watch unless it runs or the access report denies the list: a denied watch
    /// would retry a 403 for the whole session. A running or failed review does not block it.
    fn start_crd_watch(&mut self, runtime: &ClusterRuntime, cx: &mut Context<ClusterSession>) {
        if self.crds.is_some() || crds_denied(&self.access) {
            return;
        }
        self.crds = Some(CrdWatch::start(runtime, &self.connection, cx));
    }

    /// Fills the CRDs explorer list from the CRD watch, for a CRDs screen opened after the
    /// snapshot arrived. The rows are cheap clones of the definitions.
    fn seed_crd_explorer(&mut self) {
        let Some(explorer) = self
            .explorer
            .as_mut()
            .filter(|explorer| explorer.kind == ResourceKind::Crds)
        else {
            return;
        };
        explorer.list = crd_explorer_list(self.crds.as_ref().map(|crds| &crds.list), &self.access);
        self.join_explorer();
    }

    fn apply_crd_update(&mut self, update: CrdUpdate, kinds: Option<Vec<CustomKind>>) {
        let Some(crds) = self.crds.as_mut() else {
            return;
        };
        if let Some(kinds) = kinds {
            crds.kinds = kinds;
        }
        let explorer = self
            .explorer
            .as_mut()
            .filter(|explorer| explorer.kind == ResourceKind::Crds);
        feed_crds(&mut crds.list, explorer, update);
        // The new rows start without their Instances cells.
        self.join_explorer();
    }

    fn stop_crd_watch(&mut self) {
        if let Some(crds) = self.crds.as_mut() {
            crds.list.mark_stopped();
        }
        if let Some(explorer) = self
            .explorer
            .as_mut()
            .filter(|explorer| explorer.kind == ResourceKind::Crds)
        {
            explorer.list.mark_stopped();
        }
    }
}

/// The row of the Kind `key`: in the explorer list when it shows that kind, else in a Topology feed.
fn row_in<'a>(
    key: &ResourceKey,
    kind: ResourceKind,
    explorer: Option<&'a KindList>,
    topology: Option<&'a TopologyFeeds>,
) -> Option<&'a KindRow> {
    let in_explorer = explorer.and_then(|explorer| {
        explorer
            .list
            .items()
            .iter()
            .find(|row| key.is_row(kind, row))
    });
    in_explorer.or_else(|| topology?.row(kind, |row| key.is_row(kind, row)))
}

/// Whether `namespace` is one of the scope's namespaces; `All` includes every one.
pub(crate) fn scope_includes(scope: &NamespaceScope, namespace: &str) -> bool {
    let picked = scope.namespaces();
    picked.is_empty() || picked.iter().any(|name| name == namespace)
}

/// The CRDs table list for the state of the CRD watch.
fn crd_explorer_list(
    crds: Option<&LiveList<CrdSummary>>,
    access: &AccessState,
) -> LiveList<KindRow> {
    match crds {
        Some(LiveList::Ready {
            items,
            interruption,
        }) => LiveList::Ready {
            items: items.iter().map(crd_row).collect(),
            interruption: interruption.clone(),
        },
        Some(LiveList::Failed { message }) => LiveList::Failed {
            message: message.clone(),
        },
        // Without a watch the list is denied, or the review has not finished.
        None if crds_denied(access) => LiveList::Failed {
            message: crds_denied_reason(),
        },
        Some(LiveList::Loading) | None => LiveList::Loading,
    }
}

/// Why the CRDs list cannot load, worded like the sidebar lock of `kind_availability`.
fn crds_denied_reason() -> String {
    format!(
        "Not permitted: {}",
        AccessCheck::ListCustomResourceDefinitions
    )
}

/// Applies one CRD update to the definitions and, when the CRDs screen is shown, to its table.
fn feed_crds(crds: &mut LiveList<CrdSummary>, explorer: Option<&mut KindList>, update: CrdUpdate) {
    match update {
        CrdUpdate::Snapshot { crds: items, rows } => {
            crds.apply(WatchUpdate::Snapshot(items));
            if let Some(explorer) = explorer {
                explorer
                    .flow
                    .receive(&mut explorer.list, WatchUpdate::Snapshot(rows));
            }
        }
        CrdUpdate::Failed(error) => {
            let message = error_text(&error);
            crds.fail(message.clone());
            if let Some(explorer) = explorer {
                explorer.list.fail(message);
            }
        }
    }
}

/// The per-resource list review of a custom kind, which decides whether its list may start
/// (decision 21): a denied watch would retry a 403 for the whole session.
pub(crate) enum CustomGate {
    Checking { _task: Task<()> },
    Allowed,
    Denied { reason: String },
}

/// What a finished custom review says about the kind.
#[derive(Debug, PartialEq, Eq)]
enum GateOutcome {
    Allowed,
    /// Cached, and the list fails with this reason.
    Denied(String),
    /// The review failed or never finished: nothing is cached, and the list starts anyway so the
    /// watch shows its own error.
    Unknown,
}

fn gate_outcome(
    result: Option<Result<&AccessDecision, &ClusterError>>,
    custom: CustomKind,
    scope: &NamespaceScope,
) -> GateOutcome {
    match result {
        Some(Ok(AccessDecision::Allowed)) => GateOutcome::Allowed,
        Some(Ok(AccessDecision::Denied { .. })) => {
            GateOutcome::Denied(custom_denied_reason(custom, scope))
        }
        Some(Err(_)) | None => GateOutcome::Unknown,
    }
}

/// What a finished review does to the shown explorer.
#[derive(Debug, PartialEq, Eq)]
enum ExplorerAction {
    /// The review is for a kind that is not shown: only its gate changes.
    Leave,
    /// Replace the list with a failed one that gives the reason.
    Fail(String),
    /// Start the list. A review that failed starts it too, so the watch shows its own error.
    Start,
}

/// What the explorer of `shown` does when the review of `reviewed` finishes with `outcome`.
fn explorer_action(
    shown: Option<ResourceKind>,
    reviewed: ResourceKind,
    outcome: GateOutcome,
) -> ExplorerAction {
    if shown != Some(reviewed) {
        return ExplorerAction::Leave;
    }
    match outcome {
        GateOutcome::Denied(reason) => ExplorerAction::Fail(reason),
        GateOutcome::Allowed | GateOutcome::Unknown => ExplorerAction::Start,
    }
}

/// Whether a kind's review survives a scope change. A namespaced kind is reviewed per scope. A
/// cluster-scoped kind is not, and its running explorer is not restarted, so dropping its gate
/// would leave a list that is waiting for a review nobody runs.
fn keeps_gate_on_scope_change(kind: CustomKind) -> bool {
    !kind.spec().is_namespaced
}

/// Why a custom kind cannot be listed, like `kind_availability` words the built-in kinds.
pub(crate) fn custom_denied_reason(custom: CustomKind, scope: &NamespaceScope) -> String {
    let resource = custom.resource();
    let target = format!("list {}.{}", resource.plural, resource.group);
    match scope {
        NamespaceScope::All if custom.spec().is_namespaced => {
            format!("Not permitted: {target} in all namespaces")
        }
        NamespaceScope::Several(names) if custom.spec().is_namespaced => {
            format!("Not permitted: {target} in {}", namespaces_label(names))
        }
        NamespaceScope::All | NamespaceScope::Named(_) | NamespaceScope::Several(_) => {
            format!("Not permitted: {target}")
        }
    }
}

/// Reviews listing `custom` on tokio. The returned gate owns the task, so clearing the gates (a
/// scope change) or dropping the session aborts the review.
fn review_custom_gate(
    runtime: &ClusterRuntime,
    connection: &ClusterConnection,
    custom: CustomKind,
    scope: NamespaceScope,
    cx: &mut Context<ClusterSession>,
) -> CustomGate {
    let connection = connection.clone();
    let resource = custom.resource().clone();
    let reviewing =
        runtime.spawn(async move { connection.review_custom_access(&resource, &scope).await });
    let task = cx.spawn(async move |this, cx| {
        let result = reviewing.await;
        let _ = this.update(cx, |session, cx| {
            session.finish_custom_gate(custom, result, cx)
        });
    });
    CustomGate::Checking { _task: task }
}

/// Whether the report is known and denies listing CRDs.
fn crds_denied(access: &AccessState) -> bool {
    matches!(
        access,
        AccessState::Known(report)
            if !report.is_allowed(AccessCheck::ListCustomResourceDefinitions)
    )
}

impl CrdWatch {
    fn start(
        runtime: &ClusterRuntime,
        connection: &ClusterConnection,
        cx: &mut Context<ClusterSession>,
    ) -> Self {
        // The rows are built here, on tokio, so the main thread only swaps vectors.
        let updates = connection.watch_crds().map(|update| match update {
            WatchUpdate::Snapshot(crds) => {
                let rows = crds.iter().map(crd_row).collect();
                CrdUpdate::Snapshot { crds, rows }
            }
            WatchUpdate::Failed(error) => CrdUpdate::Failed(error),
        });
        let subscription = runtime.subscribe(
            updates,
            cx,
            |session: &mut ClusterSession, update, cx| session.apply_crd_update(update, cx),
            |session, _| {
                if let Some(live) = session.live_mut() {
                    live.stop_crd_watch();
                }
            },
        );
        Self {
            list: LiveList::Loading,
            kinds: Vec::new(),
            _subscription: subscription,
        }
    }
}

/// Which watches are open, as `open_watch_count` counts them.
struct OpenWatches {
    /// The always-on CRD watch runs.
    crds: bool,
    /// How many namespaces the scope names; `All` is 1. The pods watch runs once per namespace.
    namespaces: usize,
    /// The explorer's watches: 0 without one, 1 for a cluster-scoped kind, else one per namespace.
    explorer: usize,
    /// The explorer companion's watches: one per namespace for a namespaced companion, 0 without
    /// one.
    companion: usize,
    object_events: bool,
    related: bool,
    /// The Issues engine's watches: one Warning events watch per namespace; 0 while a scope
    /// change waits to restart it.
    issue_feeds: usize,
    /// Overview's change feeds: two watches per namespace while they run.
    change_events: usize,
    /// Topology's feeds: one watch each, all named, while Topology is shown; Off feeds count 0.
    topology: usize,
}

/// The namespaces list and the nodes are always watched, pods once per namespace of the scope,
/// then the explorer's watches and its companion's, one each for the drawer's events and
/// related objects, and the Warning events of the Issues engine, one per namespace, plus two
/// change-event watches per namespace while Overview is visible. The total stays within
/// `6N + 5` for N picked namespaces.
fn open_watch_count(watches: OpenWatches) -> usize {
    2 + watches.namespaces
        + usize::from(watches.crds)
        + watches.explorer
        + watches.companion
        + usize::from(watches.object_events)
        + usize::from(watches.related)
        + watches.issue_feeds
        + watches.change_events
        + watches.topology
}

/// Whether a refresh that ran for `reason` repaints the app. A new, gone, or changed issue always
/// does; a text that only aged, and the clock alone, matter only while the Issues screen shows
/// them.
fn refresh_repaints(change: IssueChange, reason: RunReason, is_issues_visible: bool) -> bool {
    match change {
        IssueChange::Shape => true,
        IssueChange::TextOnly => is_issues_visible,
        IssueChange::Unchanged => reason == RunReason::TimeRefresh && is_issues_visible,
    }
}

/// A cluster-scoped explorer (Namespaces, PVs) does not depend on the scope, so a scope change
/// leaves its watch running.
fn restarts_on_scope_change(kind: ResourceKind) -> bool {
    kind.is_namespaced()
}

/// PVC rows carry the Used cell of the kubelet history, which changes every round.
fn rejoins_after_kubelet_round(kind: ResourceKind) -> bool {
    kind == ResourceKind::PersistentVolumeClaims
}

/// Namespaces are cluster-scoped, so their explorer is one watch whatever the scope; every
/// other kind runs one watch per namespace.
fn explorer_watches(kind: ResourceKind, namespaces: usize) -> usize {
    if kind.is_namespaced() { namespaces } else { 1 }
}

/// The number of namespaces a scope watches per namespaced kind; `All` is one watch.
pub(crate) fn scope_multiplicity(scope: &NamespaceScope) -> usize {
    match scope {
        NamespaceScope::All | NamespaceScope::Named(_) => 1,
        NamespaceScope::Several(names) => names.len(),
    }
}

/// The explorer list counts like the three always-on lists: its failure is a live-update problem.
fn any_list_has_problem(
    namespaces: &LiveList<NamespaceSummary>,
    pods: &LiveList<PodSummary>,
    nodes: &LiveList<NodeSummary>,
    explorer: Option<&LiveList<KindRow>>,
) -> bool {
    namespaces.has_problem()
        || pods.has_problem()
        || nodes.has_problem()
        || explorer.is_some_and(LiveList::has_problem)
}

impl KindList {
    /// Whether the list runs a watch of its own.
    fn is_watching(&self) -> bool {
        self.subscription.is_some()
    }

    /// A list that has no watch of its own: CRDs, and a custom kind before its review allows it.
    fn unsubscribed(kind: ResourceKind, list: LiveList<KindRow>) -> Self {
        Self {
            kind,
            list,
            flow: StreamFlow::Live,
            companion: None,
            subscription: None,
        }
    }

    fn start(
        kind: ResourceKind,
        runtime: &ClusterRuntime,
        connection: &ClusterConnection,
        scope: NamespaceScope,
        events: EventFilter,
        access: &AccessState,
        cx: &mut Context<ClusterSession>,
    ) -> Self {
        let companion = match companion_plan(kind, access) {
            CompanionPlan::Start(companion) => Some(Companion::start(
                companion,
                kind,
                runtime,
                connection,
                scope.clone(),
                cx,
            )),
            CompanionPlan::None | CompanionPlan::Denied(_) => None,
        };
        Self {
            kind,
            list: LiveList::Loading,
            flow: StreamFlow::Live,
            companion,
            subscription: subscribe_explorer(runtime, connection, kind, scope, events, cx),
        }
    }
}

impl Companion {
    /// Starts the watch for the explorer of `explorer`; its updates are ignored once another kind
    /// is shown.
    fn start(
        kind: CompanionKind,
        explorer: ResourceKind,
        runtime: &ClusterRuntime,
        connection: &ClusterConnection,
        scope: NamespaceScope,
        cx: &mut Context<ClusterSession>,
    ) -> Self {
        // Boxed because each kind has its own stream type.
        let updates = match kind {
            CompanionKind::EndpointSlices => connection
                .watch_endpoint_slices(scope)
                .map(CompanionUpdate::EndpointSlices)
                .boxed(),
            CompanionKind::PersistentVolumes => connection
                .watch_persistent_volumes()
                .map(CompanionUpdate::PersistentVolumes)
                .boxed(),
            CompanionKind::Ingresses => connection
                .watch_ingresses(scope)
                .map(CompanionUpdate::Ingresses)
                .boxed(),
            CompanionKind::TlsSecrets => connection
                .watch_tls_secrets(scope)
                .map(CompanionUpdate::TlsSecrets)
                .boxed(),
            CompanionKind::Bindings {
                with_cluster_role_bindings,
            } => {
                let role_bindings = connection
                    .watch_role_bindings(scope)
                    .map(CompanionUpdate::RoleBindings);
                let cluster_role_bindings = with_cluster_role_bindings.then(|| {
                    connection
                        .watch_cluster_role_bindings()
                        .map(CompanionUpdate::ClusterRoleBindings)
                });
                // One subscription serves both lists; an absent one is an empty stream.
                futures::stream::select(
                    role_bindings,
                    futures::stream::iter(cluster_role_bindings).flatten(),
                )
                .boxed()
            }
        };
        let subscription = runtime.subscribe(
            updates,
            cx,
            move |session: &mut ClusterSession, update, _| {
                if let Some(live) = session.live_mut() {
                    live.apply_companion_update(explorer, update);
                }
            },
            move |session, _| {
                if let Some(live) = session.live_mut() {
                    live.stop_companion(explorer);
                }
            },
        );
        Self {
            lists: CompanionLists::loading_for(kind),
            _subscription: subscription,
        }
    }
}

/// `None` for a kind without a watch of its own (CRDs).
fn subscribe_explorer(
    runtime: &ClusterRuntime,
    connection: &ClusterConnection,
    kind: ResourceKind,
    scope: NamespaceScope,
    events: EventFilter,
    cx: &mut Context<ClusterSession>,
) -> Option<WatchSubscription> {
    // The kind guards are defense in depth: dropping the subscription already cancels it.
    let updates = kind.watch_rows(connection, scope, events)?;
    Some(runtime.subscribe(
        updates,
        cx,
        move |session: &mut ClusterSession, update, _| {
            if let Some(explorer) = session.explorer_mut(kind) {
                explorer.flow.receive(&mut explorer.list, update);
            }
            if let Some(live) = session.live_mut() {
                live.join_explorer();
            }
        },
        move |session, _| {
            if let Some(explorer) = session.explorer_mut(kind) {
                explorer.list.mark_stopped();
            }
        },
    ))
}

impl ObjectEvents {
    fn start(
        subject: InvolvedObject,
        runtime: &ClusterRuntime,
        connection: &ClusterConnection,
        cx: &mut Context<ClusterSession>,
    ) -> Self {
        let updates = connection.watch_object_events(&subject).map(warnings_first);
        let applied = subject.clone();
        let closed = subject.clone();
        let subscription = runtime.subscribe(
            updates,
            cx,
            move |session: &mut ClusterSession, update, _| {
                if let Some(events) = session.object_events_mut(&applied) {
                    events.list.apply(update);
                }
            },
            move |session, _| {
                if let Some(events) = session.object_events_mut(&closed) {
                    events.list.mark_stopped();
                }
            },
        );
        Self {
            subject,
            list: LiveList::Loading,
            _subscription: subscription,
        }
    }
}

/// The watches of a Namespace drawer's Quota section: the quotas and the LimitRanges, each only when
/// its gate is open, so a denied list never starts and never loops on a 403.
fn namespace_updates(
    connection: &ClusterConnection,
    namespace: &str,
    gates: NamespaceListGates,
) -> futures::stream::BoxStream<'static, RelatedUpdate> {
    let scope = || NamespaceScope::Named(namespace.to_owned());
    let quotas = gates.quotas.then(|| {
        connection
            .watch_resource_quotas(scope())
            .map(RelatedUpdate::ResourceQuotas)
            .boxed()
    });
    let limit_ranges = gates.limit_ranges.then(|| {
        connection
            .watch_limit_ranges(scope())
            .map(RelatedUpdate::LimitRanges)
            .boxed()
    });
    futures::stream::select_all(quotas.into_iter().chain(limit_ranges)).boxed()
}

impl RelatedObjects {
    fn start(
        subject: RelatedSubject,
        gates: NamespaceListGates,
        runtime: &ClusterRuntime,
        connection: &ClusterConnection,
        cx: &mut Context<ClusterSession>,
    ) -> Self {
        let updates = match &subject {
            RelatedSubject::ReplicaSets {
                namespace,
                selector,
                ..
            } => connection
                .watch_selected_replica_sets(namespace, selector)
                .map(RelatedUpdate::ReplicaSets)
                .boxed(),
            RelatedSubject::Jobs { namespace, .. } => connection
                .watch_namespace_jobs(namespace)
                .map(RelatedUpdate::Jobs)
                .boxed(),
            RelatedSubject::ConfigMapValues { namespace, name } => connection
                .watch_config_map_values(namespace, name)
                .map(RelatedUpdate::ConfigMapValues)
                .boxed(),
            RelatedSubject::QuotaRejections { namespace, .. } => connection
                .watch_failed_creates(namespace)
                .map(RelatedUpdate::Events)
                .boxed(),
            RelatedSubject::NamespaceQuotas { namespace } => {
                namespace_updates(connection, namespace, gates)
            }
            RelatedSubject::HelmHistory { namespace, release } => connection
                .watch_helm_history(namespace, release)
                .map(RelatedUpdate::HelmHistory)
                .boxed(),
            RelatedSubject::CustomFields {
                kind,
                namespace,
                name,
            } => match ResourceKind::Custom(*kind).object_ref(namespace.clone(), name.clone()) {
                Some(object) => connection
                    .watch_custom_object_fields(&object)
                    .map(RelatedUpdate::CustomFields)
                    .boxed(),
                // A row always fits its kind's scope; an end of stream reads as a failed list.
                None => futures::stream::empty().boxed(),
            },
            RelatedSubject::PodServices { namespace } => connection
                .watch_services(NamespaceScope::Named(namespace.clone()))
                .map(RelatedUpdate::Services)
                .boxed(),
            RelatedSubject::ServiceIngresses { namespace } => connection
                .watch_ingresses(NamespaceScope::Named(namespace.clone()))
                .map(RelatedUpdate::Ingresses)
                .boxed(),
            RelatedSubject::ClaimClasses => connection
                .watch_storage_classes()
                .map(RelatedUpdate::StorageClasses)
                .boxed(),
        };
        let applied = subject.clone();
        let closed = subject.clone();
        let subscription = runtime.subscribe(
            updates,
            cx,
            move |session: &mut ClusterSession, update, _| {
                if let Some(related) = session.related_mut(&applied) {
                    related.list.apply(update);
                }
            },
            move |session, _| {
                if let Some(related) = session.related_mut(&closed) {
                    related.list.mark_stopped();
                }
            },
        );
        Self {
            list: RelatedList::loading_for(&subject, gates),
            gates,
            subject,
            _subscription: subscription,
        }
    }
}

fn subscribe_pods(
    runtime: &ClusterRuntime,
    connection: &ClusterConnection,
    scope: NamespaceScope,
    cx: &mut Context<ClusterSession>,
) -> WatchSubscription {
    runtime.subscribe(
        connection.watch_pods(scope),
        cx,
        |session: &mut ClusterSession, update, _| {
            session.issues.mark_dirty();
            if let Some(live) = session.live_mut() {
                live.pods.apply(update);
                live.refresh_kubelet_targets();
                live.join_explorer();
            }
        },
        |session, _| {
            session.issues.mark_dirty();
            if let Some(live) = session.live_mut() {
                live.pods.mark_stopped();
                live.join_explorer();
            }
        },
    )
}

/// Reviews pod metrics access per namespace of `scope` on tokio. The returned state owns the
/// task, so replacing it (a scope change) or the session aborts the old review.
fn start_pod_review(
    runtime: &ClusterRuntime,
    connection: &ClusterConnection,
    scope: NamespaceScope,
    cx: &mut Context<ClusterSession>,
) -> PodReview {
    let connection = connection.clone();
    let reviewing = runtime.spawn(async move {
        connection
            .review_namespaces(AccessCheck::ListPodMetrics, &scope)
            .await
    });
    let task = cx.spawn(async move |this, cx| {
        let result = reviewing.await;
        let _ = this.update(cx, |session, cx| session.finish_pod_review(result, cx));
    });
    PodReview::Running { _task: task }
}

fn subscribe_pod_metrics(
    runtime: &ClusterRuntime,
    connection: &ClusterConnection,
    scope: NamespaceScope,
    cx: &mut Context<ClusterSession>,
) -> WatchSubscription {
    runtime.subscribe(
        connection.poll_pod_metrics(scope),
        cx,
        |session: &mut ClusterSession, update, _| {
            session.issues.mark_dirty();
            if let Some(live) = session.live_mut() {
                live.metrics.pods.receive(update, live.pods.items());
            }
        },
        |session, _| {
            session.issues.mark_dirty();
            if let Some(live) = session.live_mut() {
                live.metrics.pods.mark_stopped();
            }
        },
    )
}

fn subscribe_kubelet_stats(
    runtime: &ClusterRuntime,
    connection: &ClusterConnection,
    targets: watch::Receiver<KubeletTargets>,
    cx: &mut Context<ClusterSession>,
) -> WatchSubscription {
    runtime.subscribe(
        connection.poll_kubelet_stats(targets),
        cx,
        |session: &mut ClusterSession, update, _| {
            session.issues.mark_dirty();
            if let Some(live) = session.live_mut() {
                live.metrics
                    .kubelet
                    .receive(update, live.pods.items(), &live.scope);
                if live
                    .explorer
                    .as_ref()
                    .is_some_and(|explorer| rejoins_after_kubelet_round(explorer.kind))
                {
                    live.join_explorer();
                }
            }
        },
        |session, _| {
            session.issues.mark_dirty();
            if let Some(live) = session.live_mut() {
                live.metrics.kubelet.mark_stopped();
            }
        },
    )
}

fn subscribe_node_metrics(
    runtime: &ClusterRuntime,
    connection: &ClusterConnection,
    cx: &mut Context<ClusterSession>,
) -> WatchSubscription {
    runtime.subscribe(
        connection.poll_node_metrics(),
        cx,
        |session: &mut ClusterSession, update, _| {
            session.issues.mark_dirty();
            if let Some(live) = session.live_mut() {
                live.metrics.nodes.receive(update);
            }
        },
        |session, _| {
            session.issues.mark_dirty();
            if let Some(live) = session.live_mut() {
                live.metrics.nodes.mark_stopped();
            }
        },
    )
}

/// The review runs on tokio; the returned state owns the task, so a newer review (or the
/// session) dropping it aborts the old one.
fn review_access_again(
    runtime: &ClusterRuntime,
    connection: &ClusterConnection,
    scope: NamespaceScope,
    cx: &mut Context<ClusterSession>,
) -> AccessState {
    let connection = connection.clone();
    let reviewing = runtime.spawn(async move { connection.review_access(scope).await });
    let task = cx.spawn(async move |this, cx| {
        let result = reviewing.await;
        let _ = this.update(cx, |session, cx| session.finish_access_review(result, cx));
    });
    AccessState::Checking { _task: task }
}

#[cfg(test)]
#[path = "cluster_session_tests.rs"]
mod cluster_session_tests;
