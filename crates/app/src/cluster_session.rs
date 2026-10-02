use std::error::Error;
use std::sync::Arc;

use cluster::{
    AccessCheck, AccessReport, ClusterConnection, ClusterError, ContextSummary, EventFilter,
    EventSummary, InvolvedObject, Kubeconfig, KubeletTargets, NamespaceAccess, NamespaceScope,
    NamespaceSummary, NodeSummary, PodSummary, ServerVersion, WatchUpdate,
};
use futures::StreamExt as _;
use gpui_kit::{Context, Task};
use tokio::sync::watch;

use crate::cluster_metrics::{
    ClusterMetrics, NodesGate, PodReview, PodReviewResult, PodsGate, nodes_gate, pods_gate,
};
use crate::cluster_runtime::{ClusterRuntime, WatchSubscription};
use crate::event_rows::newest_first;
use crate::kind_row::KindRow;
use crate::kubelet_metrics::KubeletDemand;
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
    _subscription: WatchSubscription,
}

/// The events of the object whose drawer is open. Dropping it stops the watch.
pub(crate) struct ObjectEvents {
    pub(crate) subject: InvolvedObject,
    pub(crate) list: LiveList<EventSummary>,
    _subscription: WatchSubscription,
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
        // Namespaces are cluster-scoped, so their watch does not depend on the scope.
        if let Some(kind) = live
            .explorer
            .as_ref()
            .map(|explorer| explorer.kind)
            .filter(|kind| kind.is_namespaced())
        {
            live.explorer = None;
            live.explorer = Some(KindList::start(
                kind,
                &runtime,
                &live.connection,
                scope.clone(),
                event_filter,
                cx,
            ));
        }
        live.access = review_access_again(&runtime, &live.connection, scope.clone(), cx);
        let review = start_pod_review(&runtime, &live.connection, scope.clone(), cx);
        live.metrics.restart_pods(&scope, review);
        live.metrics.kubelet.history.retain_scope(&scope);
        live.scope = scope;
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

    /// The kind and item count of the loaded explorer list, for the sidebar.
    pub(crate) fn explorer_count(&self) -> Option<(ResourceKind, usize)> {
        let explorer = self.explorer.as_ref()?;
        Some((explorer.kind, explorer.list.ready_count()?))
    }

    /// Open watches: namespaces, pods, nodes, plus the explorer's and the drawer's events when
    /// they are open.
    pub(crate) fn watch_count(&self) -> usize {
        open_watch_count(
            self.explorer.as_ref().map(|explorer| &explorer.list),
            self.object_events.as_ref().map(|events| &events.list),
        )
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
        let explorer = explorer_kind.map(|kind| {
            KindList::start(kind, &runtime, &connection, scope.clone(), event_filter, cx)
        });
        let metrics =
            ClusterMetrics::new(start_pod_review(&runtime, &connection, scope.clone(), cx));
        Self {
            server_version,
            scope,
            access: AccessState::from_review(access),
            namespaces: LiveList::Loading,
            pods: LiveList::Loading,
            nodes: LiveList::Loading,
            metrics,
            explorer,
            object_events: None,
            connection,
            subscriptions,
        }
    }
}

/// Namespaces, pods and nodes are always watched; the explorer and the drawer's events add one
/// each.
fn open_watch_count(
    explorer: Option<&LiveList<KindRow>>,
    object_events: Option<&LiveList<EventSummary>>,
) -> usize {
    3 + usize::from(explorer.is_some()) + usize::from(object_events.is_some())
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
        cx: &mut Context<ClusterSession>,
    ) -> Self {
        Self {
            kind,
            list: LiveList::Loading,
            flow: StreamFlow::Live,
            _subscription: subscribe_explorer(runtime, connection, kind, scope, events, cx),
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
            }
        },
        |session, _| {
            if let Some(live) = session.live_mut() {
                live.pods.mark_stopped();
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
