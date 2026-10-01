use std::error::Error;
use std::sync::Arc;

use cluster::{
    AccessCheck, AccessReport, ClusterConnection, ClusterError, ContextSummary, Kubeconfig,
    NamespaceScope, NamespaceSummary, NodeSummary, PodSummary, ServerVersion, WatchUpdate,
};
use gpui_kit::{Context, Task};

use crate::cluster_runtime::{ClusterRuntime, WatchSubscription};

/// One connected kubeconfig context: the connection, its live lists, and the access report.
/// Dropping the entity cancels every task and watch it owns.
pub(crate) struct ClusterSession {
    inputs: ConnectInputs,
    user: Option<String>,
    phase: SessionPhase,
}

/// What `connect` needs, kept so that `retry` can run it again.
struct ConnectInputs {
    kubeconfig: Arc<Kubeconfig>,
    context: String,
    requested_namespace: Option<String>,
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
    connection: ClusterConnection,
    subscriptions: Subscriptions,
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

/// `all_namespaces_access` is `None` when `review_access(All)` failed or was not asked.
fn initial_scope(
    requested: Option<&str>,
    all_namespaces_access: Option<&AccessReport>,
    default_namespace: &str,
) -> NamespaceScope {
    if let Some(namespace) = requested {
        return NamespaceScope::Named(namespace.to_owned());
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
    requested_namespace: Option<String>,
) -> Result<Connected, ClusterError> {
    let connection = ClusterConnection::open(&kubeconfig, &context).await?;
    let server_version = connection.server_version().await?;

    let all_namespaces_review = match requested_namespace {
        Some(_) => None,
        None => Some(connection.review_access(NamespaceScope::All).await),
    };
    let scope = initial_scope(
        requested_namespace.as_deref(),
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
        requested_namespace: Option<String>,
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
            Ok(Ok(connected)) => SessionPhase::Live(Box::new(LiveCluster::start(connected, cx))),
            Ok(Err(error)) => SessionPhase::Failed {
                message: error_text(&error),
            },
            Err(_) => SessionPhase::Failed {
                message: "the connection task stopped unexpectedly".to_owned(),
            },
        };
        cx.notify();
    }

    /// Switches the pods watch to `scope` and reviews access again for it.
    pub(crate) fn set_scope(&mut self, scope: NamespaceScope, cx: &mut Context<Self>) {
        let Some(live) = self.live_mut() else {
            return;
        };
        if live.scope == scope {
            return;
        }
        let runtime = cx.global::<ClusterRuntime>().clone();
        live.pods = LiveList::Loading;
        live.subscriptions._pods = subscribe_pods(&runtime, &live.connection, scope.clone(), cx);
        live.access = review_access_again(&runtime, &live.connection, scope.clone(), cx);
        live.scope = scope;
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
        cx.notify();
    }
}

impl LiveCluster {
    /// The context namespace, or `default`.
    pub(crate) fn default_namespace(&self) -> &str {
        self.connection.default_namespace()
    }

    /// `all namespaces` or the namespace name, as used in headers and empty states.
    pub(crate) fn scope_label(&self) -> String {
        match &self.scope {
            NamespaceScope::All => "all namespaces".to_owned(),
            NamespaceScope::Named(namespace) => namespace.clone(),
        }
    }

    fn start(connected: Connected, cx: &mut Context<ClusterSession>) -> Self {
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
                    }
                },
                |session, _| {
                    if let Some(live) = session.live_mut() {
                        live.nodes.mark_stopped();
                    }
                },
            ),
        };
        Self {
            server_version,
            scope,
            access: AccessState::from_review(access),
            namespaces: LiveList::Loading,
            pods: LiveList::Loading,
            nodes: LiveList::Loading,
            connection,
            subscriptions,
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
            }
        },
        |session, _| {
            if let Some(live) = session.live_mut() {
                live.pods.mark_stopped();
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
