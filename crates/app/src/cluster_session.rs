use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cluster::{
    AccessCheck, AccessReport, BindingSummary, ClusterConnection, ClusterError, ConfigMapValues,
    ContextSummary, EndpointSliceSummary, EventFilter, EventSummary, InvolvedObject, JobSummary,
    Kubeconfig, KubeletTargets, NamespaceAccess, NamespaceScope, NamespaceSummary, NodeSummary,
    PersistentVolumeSummary, PodSummary, ReplicaSetSummary, ResourceQuotaSummary, ServerVersion,
    WatchUpdate,
};
use futures::StreamExt as _;
use gpui_kit::{Context, Task};
use tokio::sync::watch;

use crate::cluster_metrics::{
    ClusterMetrics, NodesGate, PodReview, PodReviewResult, PodsGate, nodes_gate, pods_gate,
};
use crate::cluster_runtime::{ClusterRuntime, WatchSubscription};
use crate::event_rows::newest_first;
use crate::kind_join::{JoinInputs, join_rows};
use crate::kind_row::KindRow;
use crate::kubelet_metrics::KubeletDemand;
use crate::related_objects::RelatedSubject;
use crate::resource_kind::ResourceKind;

/// One connected kubeconfig context: the connection, its live lists, and the access report.
/// Dropping the entity cancels every task and watch it owns.
pub(crate) struct ClusterSession {
    inputs: ConnectInputs,
    user: Option<String>,
    phase: SessionPhase,
    /// The kind screen being shown, kept across Connecting and retry so that `LiveCluster::start`
    /// can start its watch.
    explorer_kind: Option<ResourceKind>,
    /// Which events the Events screen asks the server for. Kept across Connecting and retry like
    /// `explorer_kind`; a new session starts at `All`.
    event_filter: EventFilter,
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
    /// Decided before the session is live, so it is never unknown.
    pub(crate) scope: NamespaceScope,
    pub(crate) access: AccessState,
    pub(crate) namespaces: LiveList<NamespaceSummary>,
    pub(crate) pods: LiveList<PodSummary>,
    pub(crate) nodes: LiveList<NodeSummary>,
    /// Pod and node usage, polled while the metrics API is reachable and allowed.
    pub(crate) metrics: ClusterMetrics,
    /// The watch of the visible kind screen; `None` on Pods and Nodes.
    explorer: Option<KindList>,
    /// The open drawer's events; `None` while no drawer needs them.
    object_events: Option<ObjectEvents>,
    /// The objects related to the open drawer (a Deployment's ReplicaSets); `None` while no
    /// drawer needs them.
    related: Option<RelatedObjects>,
    /// The sidebar numbers of kinds without a running watch.
    kind_counts: KindCounts,
    connection: ClusterConnection,
    subscriptions: Subscriptions,
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

/// The visible explorer kind's list. Dropping it stops the watch.
pub(crate) struct KindList {
    pub(crate) kind: ResourceKind,
    pub(crate) list: LiveList<KindRow>,
    flow: StreamFlow<KindRow>,
    /// A second watch whose lists fill cells of the rows; `None` when the kind needs none or the
    /// access report denies it.
    companion: Option<Companion>,
    _subscription: WatchSubscription,
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
    RoleBindings(WatchUpdate<BindingSummary>),
    ClusterRoleBindings(WatchUpdate<BindingSummary>),
}

/// Which companion an explorer kind starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CompanionKind {
    EndpointSlices,
    PersistentVolumes,
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

impl CompanionLists {
    fn loading_for(kind: CompanionKind) -> Self {
        match kind {
            CompanionKind::EndpointSlices => Self::EndpointSlices(LiveList::Loading),
            CompanionKind::PersistentVolumes => Self::PersistentVolumes(LiveList::Loading),
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
            Self::PersistentVolumes(_) | Self::Bindings { .. } => None,
        }
    }

    /// The persistent volumes, when this companion lists them.
    pub(crate) fn persistent_volumes(&self) -> Option<&LiveList<PersistentVolumeSummary>> {
        match self {
            Self::PersistentVolumes(list) => Some(list),
            Self::EndpointSlices(_) | Self::Bindings { .. } => None,
        }
    }

    /// Whether the first snapshot has not arrived.
    #[cfg(feature = "screenshot")]
    fn is_loading(&self) -> bool {
        match self {
            Self::EndpointSlices(list) => list.is_loading(),
            Self::PersistentVolumes(list) => list.is_loading(),
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

    /// How many watches the companion runs: one per namespace of the scope for a namespaced kind,
    /// one for a cluster-scoped one.
    fn watches(&self, namespaces: usize) -> usize {
        match self {
            Self::EndpointSlices(_) => namespaces,
            Self::PersistentVolumes(_) => 1,
            Self::Bindings {
                cluster_role_bindings,
                ..
            } => namespaces + usize::from(cluster_role_bindings.is_some()),
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
    ResourceQuotas(LiveList<ResourceQuotaSummary>),
}

/// One related watch update, typed on tokio so one subscription serves every subject.
enum RelatedUpdate {
    ReplicaSets(WatchUpdate<ReplicaSetSummary>),
    Jobs(WatchUpdate<JobSummary>),
    ConfigMapValues(WatchUpdate<ConfigMapValues>),
    Events(WatchUpdate<EventSummary>),
    ResourceQuotas(WatchUpdate<ResourceQuotaSummary>),
}

impl RelatedList {
    fn loading_for(subject: &RelatedSubject) -> Self {
        match subject {
            RelatedSubject::ReplicaSets { .. } => Self::ReplicaSets(LiveList::Loading),
            RelatedSubject::Jobs { .. } => Self::Jobs(LiveList::Loading),
            RelatedSubject::ConfigMapValues { .. } => Self::ConfigMapValues(LiveList::Loading),
            RelatedSubject::QuotaRejections { .. } => Self::Events(LiveList::Loading),
            RelatedSubject::NamespaceQuotas { .. } => Self::ResourceQuotas(LiveList::Loading),
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
            (Self::ResourceQuotas(list), RelatedUpdate::ResourceQuotas(update)) => {
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
            Self::ResourceQuotas(list) => list.mark_stopped(),
        }
    }

    /// The events of a quota's namespace, when this list holds them.
    pub(crate) fn events(&self) -> Option<&LiveList<EventSummary>> {
        match self {
            Self::Events(list) => Some(list),
            Self::ReplicaSets(_)
            | Self::Jobs(_)
            | Self::ConfigMapValues(_)
            | Self::ResourceQuotas(_) => None,
        }
    }

    /// The quotas of a namespace, when this list holds them.
    pub(crate) fn resource_quotas(&self) -> Option<&LiveList<ResourceQuotaSummary>> {
        match self {
            Self::ResourceQuotas(list) => Some(list),
            Self::ReplicaSets(_) | Self::Jobs(_) | Self::ConfigMapValues(_) | Self::Events(_) => {
                None
            }
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
            Self::ResourceQuotas(list) => list.is_loading(),
        }
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
        RelatedSubject::NamespaceQuotas { .. } => AccessCheck::ListResourceQuotas,
        RelatedSubject::ReplicaSets { .. }
        | RelatedSubject::Jobs { .. }
        | RelatedSubject::ConfigMapValues { .. } => return None,
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
            Ok(report) => Self::Known(report),
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
        .filter(|kind| report.is_allowed(kind.access_check()))
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
    fn mark_stopped(&mut self) {
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
    scope: NamespaceScope,
    access: Result<AccessReport, String>,
}

async fn connect_cluster(
    kubeconfig: Arc<Kubeconfig>,
    context: String,
    requested_namespace: Option<NamespaceScope>,
) -> Result<Connected, ClusterError> {
    let connection = ClusterConnection::open(&kubeconfig, &context).await?;
    let server_version = connection.server_version().await?;

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
        cx: &mut Context<Self>,
    ) -> Self {
        let inputs = ConnectInputs {
            kubeconfig,
            context: summary.name.clone(),
            requested_namespace,
        };
        let phase = Self::begin_connect(&inputs, cx);
        Self {
            inputs,
            user: summary.user.clone(),
            phase,
            explorer_kind,
            event_filter: EventFilter::All,
        }
    }

    pub(crate) fn context(&self) -> &str {
        &self.inputs.context
    }

    /// The kubeconfig user entry name, not a credential.
    pub(crate) fn user(&self) -> Option<&str> {
        self.user.as_deref()
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
        self.phase = Self::begin_connect(&self.inputs, cx);
        cx.notify();
    }

    fn begin_connect(inputs: &ConnectInputs, cx: &mut Context<Self>) -> SessionPhase {
        let runtime = cx.global::<ClusterRuntime>().clone();
        let connecting = runtime.spawn(connect_cluster(
            Arc::clone(&inputs.kubeconfig),
            inputs.context.clone(),
            inputs.requested_namespace.clone(),
        ));
        let task = cx.spawn(async move |this, cx| {
            let result = connecting.await;
            let _ = this.update(cx, |session, cx| session.finish_connect(result, cx));
        });
        SessionPhase::Connecting { _task: task }
    }

    fn finish_connect(
        &mut self,
        result: Result<Result<Connected, ClusterError>, tokio::task::JoinError>,
        cx: &mut Context<Self>,
    ) {
        self.phase = match result {
            Ok(Ok(connected)) => SessionPhase::Live(Box::new(LiveCluster::start(
                connected,
                self.explorer_kind,
                self.event_filter,
                cx,
            ))),
            Ok(Err(error)) => SessionPhase::Failed {
                message: error_text(&error),
            },
            Err(_) => SessionPhase::Failed {
                message: "the connection task stopped unexpectedly".to_owned(),
            },
        };
        self.refresh_kind_counts(CountTrigger::Review, cx);
        self.update_metrics_feeds(cx);
        cx.notify();
    }

    /// Switches the pods watch to `scope` and reviews access again for it.
    pub(crate) fn set_scope(&mut self, scope: NamespaceScope, cx: &mut Context<Self>) {
        let event_filter = self.event_filter;
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
        if let Some(kind) = live
            .explorer
            .as_ref()
            .map(|explorer| explorer.kind)
            .filter(|kind| restarts_on_scope_change(*kind))
        {
            live.explorer = None;
            live.explorer = Some(KindList::start(
                kind,
                &runtime,
                &live.connection,
                scope.clone(),
                event_filter,
                &live.access,
                cx,
            ));
        }
        let review = start_pod_review(&runtime, &live.connection, scope.clone(), cx);
        live.metrics.restart_pods(&scope, review);
        live.metrics.kubelet.history.retain_scope(&scope);
        live.scope = scope;
        // The numbers are for the old scope; the review for the new one counts again.
        live.kind_counts = KindCounts::default();
        live.refresh_kubelet_targets();
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
        // The old subscription drops first, so two explorer watches never overlap.
        live.explorer = None;
        live.explorer = kind.map(|kind| {
            KindList::start(
                kind,
                &runtime,
                &live.connection,
                live.scope.clone(),
                event_filter,
                &live.access,
                cx,
            )
        });
        cx.notify();
    }

    /// Holds the explorer list still, or shows what arrived meanwhile. Only a loaded list can be
    /// paused. Every restart of the list (scope, kind, Warnings only) starts it live again.
    pub(crate) fn set_explorer_paused(&mut self, is_paused: bool, cx: &mut Context<Self>) {
        let Some(explorer) = self.live_mut().and_then(|live| live.explorer.as_mut()) else {
            return;
        };
        if is_paused {
            if !explorer.flow.pause(&explorer.list) {
                return;
            }
        } else {
            explorer.flow.resume(&mut explorer.list);
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
        if live.related_subject() == subject.as_ref() {
            return;
        }
        // The old subscription drops first, so two related watches never overlap.
        live.related = None;
        live.related =
            subject.map(|subject| RelatedObjects::start(subject, &runtime, &live.connection, cx));
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
                            let count = connection.count_objects(kind.object(), &scope).await;
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
        live.access = AccessState::from_review(review);
        live.drop_denied_companion();
        self.refresh_kind_counts(CountTrigger::Review, cx);
        self.update_metrics_feeds(cx);
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

    /// Open watches: namespaces, pods, nodes, the explorer's and its companion, and the drawer's
    /// events and related objects when they are open.
    pub(crate) fn watch_count(&self) -> usize {
        let namespaces = scope_multiplicity(&self.scope);
        open_watch_count(OpenWatches {
            namespaces,
            explorer: self
                .explorer
                .as_ref()
                .map_or(0, |explorer| explorer_watches(explorer.kind, namespaces)),
            companion: self
                .companion()
                .map_or(0, |lists| lists.watches(namespaces)),
            object_events: self.object_events.is_some(),
            related: self.related.is_some(),
        })
    }

    /// The companion lists of the explorer, when it runs a companion watch.
    pub(crate) fn companion(&self) -> Option<&CompanionLists> {
        let companion = self.explorer.as_ref()?.companion.as_ref()?;
        Some(&companion.lists)
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

    /// The related list of `subject`, or `None` while another subject (or none) is watched.
    pub(crate) fn related_of(&self, subject: &RelatedSubject) -> Option<&RelatedList> {
        self.related
            .as_ref()
            .filter(|related| related.subject == *subject)
            .map(|related| &related.list)
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
        any_list_has_problem(
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

    /// The explorer list of `kind`, or `None` while another kind (or no kind) is shown.
    pub(crate) fn kind_list(&self, kind: ResourceKind) -> Option<&KindList> {
        self.explorer
            .as_ref()
            .filter(|explorer| explorer.kind == kind)
    }

    fn start(
        connected: Connected,
        explorer_kind: Option<ResourceKind>,
        event_filter: EventFilter,
        cx: &mut Context<ClusterSession>,
    ) -> Self {
        let runtime = cx.global::<ClusterRuntime>().clone();
        let Connected {
            connection,
            server_version,
            scope,
            access,
        } = connected;
        let subscriptions = Subscriptions {
            _namespaces: runtime.subscribe(
                connection.watch_namespaces(),
                cx,
                |session: &mut ClusterSession, update, _| {
                    if let Some(live) = session.live_mut() {
                        live.namespaces.apply(update);
                    }
                },
                |session, _| {
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
                    if let Some(live) = session.live_mut() {
                        live.nodes.apply(update);
                        live.refresh_kubelet_targets();
                    }
                },
                |session, _| {
                    if let Some(live) = session.live_mut() {
                        live.nodes.mark_stopped();
                    }
                },
            ),
        };
        let access = AccessState::from_review(access);
        let explorer = explorer_kind.map(|kind| {
            KindList::start(
                kind,
                &runtime,
                &connection,
                scope.clone(),
                event_filter,
                &access,
                cx,
            )
        });
        let metrics =
            ClusterMetrics::new(start_pod_review(&runtime, &connection, scope.clone(), cx));
        Self {
            server_version,
            scope,
            access,
            namespaces: LiveList::Loading,
            pods: LiveList::Loading,
            nodes: LiveList::Loading,
            metrics,
            explorer,
            object_events: None,
            related: None,
            kind_counts: KindCounts::default(),
            connection,
            subscriptions,
        }
    }
}

/// Which watches are open, as `open_watch_count` counts them.
struct OpenWatches {
    /// How many namespaces the scope names; `All` is 1. The pods watch runs once per namespace.
    namespaces: usize,
    /// The explorer's watches: 0 without one, 1 for a cluster-scoped kind, else one per namespace.
    explorer: usize,
    /// The explorer companion's watches: one per namespace for a namespaced companion, 0 without
    /// one.
    companion: usize,
    object_events: bool,
    related: bool,
}

/// The namespaces list and the nodes are always watched, pods once per namespace of the scope,
/// then the explorer's watches and its companion's, and one each for the drawer's events and
/// related objects. The total stays within `3N + 4` for N picked namespaces.
fn open_watch_count(watches: OpenWatches) -> usize {
    2 + watches.namespaces
        + watches.explorer
        + watches.companion
        + usize::from(watches.object_events)
        + usize::from(watches.related)
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
fn scope_multiplicity(scope: &NamespaceScope) -> usize {
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
            _subscription: subscribe_explorer(runtime, connection, kind, scope, events, cx),
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

fn subscribe_explorer(
    runtime: &ClusterRuntime,
    connection: &ClusterConnection,
    kind: ResourceKind,
    scope: NamespaceScope,
    events: EventFilter,
    cx: &mut Context<ClusterSession>,
) -> WatchSubscription {
    // The kind guards are defense in depth: dropping the subscription already cancels it.
    runtime.subscribe(
        kind.watch_rows(connection, scope, events),
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
    )
}

impl ObjectEvents {
    fn start(
        subject: InvolvedObject,
        runtime: &ClusterRuntime,
        connection: &ClusterConnection,
        cx: &mut Context<ClusterSession>,
    ) -> Self {
        let updates = connection.watch_object_events(&subject).map(newest_first);
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

impl RelatedObjects {
    fn start(
        subject: RelatedSubject,
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
            RelatedSubject::NamespaceQuotas { namespace } => connection
                .watch_resource_quotas(NamespaceScope::Named(namespace.clone()))
                .map(RelatedUpdate::ResourceQuotas)
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
            list: RelatedList::loading_for(&subject),
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
            if let Some(live) = session.live_mut() {
                live.pods.apply(update);
                live.refresh_kubelet_targets();
                live.join_explorer();
            }
        },
        |session, _| {
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
            if let Some(live) = session.live_mut() {
                live.metrics.pods.receive(update, live.pods.items());
            }
        },
        |session, _| {
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
            if let Some(live) = session.live_mut() {
                live.metrics.nodes.receive(update);
            }
        },
        |session, _| {
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
