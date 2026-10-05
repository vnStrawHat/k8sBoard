//! What the Issues engine watches beyond the core lists, and how complete that coverage is. The
//! feeds run on the cluster runtime and reach the session through silent subscriptions; this
//! module holds their state. Event messages are arbitrary text, so nothing here logs them.

use std::collections::HashMap;
use std::ops::Range;
use std::time::Duration;

use cluster::{
    ClusterConnection, ClusterError, EventFilter, EventSummary, NamespaceScope, NodeReadiness,
    WatchUpdate,
};
use futures::stream::BoxStream;
use futures::{Stream, StreamExt as _};
use gpui_kit::{App, Context, Task};

use crate::cluster_metrics::FeedStatus;
use crate::cluster_runtime::{ClusterRuntime, WatchSubscription};
use crate::cluster_session::{
    AccessState, ClusterSession, LiveCluster, LiveList, scope_multiplicity,
};
use crate::issue::IssueObject;
use crate::kind_row::KindObject;
use crate::resource_kind::ResourceKind;
use crate::settings::AppSettings;

/// How long a scope change waits before the Warning events watch restarts. The API server keeps no
/// watch cache for events and cannot index them, so each start scans every event of the scope in
/// etcd; picking namespaces one after another must not run one scan per click.
const EVENTS_RESTART_DELAY: Duration = Duration::from_secs(1);

/// A source of problems, as the coverage names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IssueFeed {
    Pods,
    Nodes,
    Namespaces,
    WarningEvents,
    PodMetrics,
    NodeMetrics,
    VolumeUsage,
    /// A condition feed: Deployments, DaemonSets, Jobs, HPAs, PDBs, quotas, claims, or the TLS
    /// secrets (`Secrets`).
    Kind(ResourceKind),
}

impl IssueFeed {
    /// What the coverage note calls the feed. Deployments and DaemonSets are both `rollouts`.
    fn label(self) -> &'static str {
        match self {
            Self::Pods => "pods",
            Self::Nodes => "nodes",
            Self::Namespaces => "namespaces",
            Self::WarningEvents => "warning events",
            Self::PodMetrics => "pod metrics",
            Self::NodeMetrics => "node metrics",
            Self::VolumeUsage => "volume usage",
            Self::Kind(ResourceKind::Deployments | ResourceKind::DaemonSets) => "rollouts",
            Self::Kind(ResourceKind::Jobs) => "jobs",
            Self::Kind(ResourceKind::HorizontalPodAutoscalers) => "HPAs",
            Self::Kind(ResourceKind::PodDisruptionBudgets) => "PDBs",
            Self::Kind(ResourceKind::ResourceQuotas) => "quotas",
            Self::Kind(ResourceKind::PersistentVolumeClaims) => "volume claims",
            Self::Kind(ResourceKind::Secrets) => "certificates",
            Self::Kind(kind) => kind.label(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FeedState {
    Live,
    Loading,
    /// Watched by design on part of the cluster only, such as the kubelet node cap.
    Limited(String),
    /// Not watched, with the reason.
    Off(String),
}

/// The state of every feed a rule reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Coverage {
    pub(crate) feeds: Vec<(IssueFeed, FeedState)>,
}

impl Coverage {
    /// Whether a rule cannot run: a feed is loading or off. A Limited feed is a designed cap, not
    /// a gap.
    pub(crate) fn is_partial(&self) -> bool {
        self.feeds
            .iter()
            .any(|(_, state)| matches!(state, FeedState::Loading | FeedState::Off(_)))
    }

    /// Whether `feed` is listed and has stopped loading, whether it loaded or failed. A failed
    /// list is a gap the coverage names, not a reason to keep saying "checking".
    pub(crate) fn is_settled(&self, feed: IssueFeed) -> bool {
        self.feeds
            .iter()
            .any(|(listed, state)| *listed == feed && *state != FeedState::Loading)
    }

    /// The header and tooltip text, or `None` when every feed is live.
    pub(crate) fn note(&self) -> Option<String> {
        let mut sentences = Vec::new();
        // Two feeds can share a label (rollouts) and a reason; the note says it once.
        let mut off: Vec<String> = Vec::new();
        let mut loading: Vec<&str> = Vec::new();
        for (feed, state) in &self.feeds {
            match state {
                FeedState::Off(reason) => {
                    let text = format!("{} ({reason})", feed.label());
                    if !off.contains(&text) {
                        off.push(text);
                    }
                }
                FeedState::Loading if !loading.contains(&feed.label()) => {
                    loading.push(feed.label())
                }
                FeedState::Loading | FeedState::Live | FeedState::Limited(_) => {}
            }
        }
        if !off.is_empty() {
            sentences.push(format!("Not checked: {}.", off.join(", ")));
        }
        if !loading.is_empty() {
            sentences.push(format!("Loading: {}.", loading.join(", ")));
        }
        for (feed, state) in &self.feeds {
            if let FeedState::Limited(text) = state {
                sentences.push(format!("{}: {text}.", sentence_case(feed.label())));
            }
        }
        (!sentences.is_empty()).then(|| sentences.join(" "))
    }
}

fn sentence_case(text: &str) -> String {
    let mut characters = text.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(characters).collect()
    })
}

/// A watched list: `Ready` is live, a list that never loaded is off with the watch's error.
pub(crate) fn list_state<T>(list: &LiveList<T>) -> FeedState {
    match list {
        LiveList::Loading => FeedState::Loading,
        LiveList::Ready { .. } => FeedState::Live,
        LiveList::Failed { message } => FeedState::Off(message.clone()),
    }
}

/// A metrics poll: an interruption keeps older ticks, so it still counts as live.
pub(crate) fn metrics_state(status: &FeedStatus) -> FeedState {
    match status {
        FeedStatus::Live | FeedStatus::Interrupted(_) => FeedState::Live,
        FeedStatus::Checking | FeedStatus::Waiting => FeedState::Loading,
        FeedStatus::Unavailable(reason) | FeedStatus::Failed(reason) => {
            FeedState::Off(reason.clone())
        }
    }
}

/// The kubelet poll reads every Ready node up to `SUMMARY_NODE_LIMIT`; above it only the nodes of
/// the open drawer, so the volume rule sees part of the cluster. A feed that polls fewer nodes
/// than exist is limited from the start, `Waiting` included: with no node to poll no round ever
/// comes, and "loading" would never end.
pub(crate) fn volume_usage_state(
    status: &FeedStatus,
    polled_nodes: usize,
    ready_nodes: usize,
) -> FeedState {
    let is_polling = matches!(
        status,
        FeedStatus::Live | FeedStatus::Interrupted(_) | FeedStatus::Waiting
    );
    if is_polling && polled_nodes < ready_nodes {
        return FeedState::Limited(format!("{polled_nodes} of {ready_nodes} nodes polled"));
    }
    metrics_state(status)
}

/// The feeds of a live cluster, read at one moment.
pub(crate) fn core_coverage(live: &LiveCluster, feeds: &IssueFeeds) -> Coverage {
    let ready_nodes = live
        .nodes
        .items()
        .iter()
        .filter(|node| node.status.readiness == NodeReadiness::Ready)
        .count();
    let polled_nodes = live.metrics.kubelet.targets().summary_nodes.len();
    let mut coverage = Coverage {
        feeds: vec![
            (IssueFeed::Pods, list_state(&live.pods)),
            (IssueFeed::Nodes, list_state(&live.nodes)),
            (IssueFeed::Namespaces, list_state(&live.namespaces)),
            (IssueFeed::WarningEvents, feeds.events.state()),
            (
                IssueFeed::PodMetrics,
                metrics_state(&live.metrics.pods.status),
            ),
            (
                IssueFeed::NodeMetrics,
                metrics_state(&live.metrics.nodes.status),
            ),
            (
                IssueFeed::VolumeUsage,
                volume_usage_state(&live.metrics.kubelet.status, polled_nodes, ready_nodes),
            ),
        ],
    };
    coverage.feeds.extend(
        feeds
            .conditions
            .iter()
            .map(|feed| (IssueFeed::Kind(feed.kind), feed.state())),
    );
    coverage
}

/// The order of the Warning feed: events of one object are adjacent.
fn object_order(event: &EventSummary) -> (Option<&str>, &str, &str) {
    let object = &event.object;
    (
        object.namespace.as_deref(),
        object.kind.as_str(),
        object.name.as_str(),
    )
}

/// The Warning events of the scope, ordered by involved object so that the events of one pod are
/// one slice. The list holds at most `EVENT_LIMIT` of the newest events.
pub(crate) struct WarningEvents {
    list: LiveList<EventSummary>,
    by_object: HashMap<IssueObject, Range<usize>>,
}

/// A feed that has not delivered its first snapshot.
impl Default for WarningEvents {
    fn default() -> Self {
        Self {
            list: LiveList::Loading,
            by_object: HashMap::new(),
        }
    }
}

impl WarningEvents {
    /// Sorts by (namespace, kind, name), newest first, and rebuilds the index.
    pub(crate) fn apply(&mut self, update: WatchUpdate<EventSummary>) {
        let update = match update {
            WatchUpdate::Snapshot(mut events) => {
                events.sort_by(|left, right| {
                    object_order(left)
                        .cmp(&object_order(right))
                        .then_with(|| right.last_seen.cmp(&left.last_seen))
                });
                WatchUpdate::Snapshot(events)
            }
            failed @ WatchUpdate::Failed(_) => failed,
        };
        self.list.apply(update);
        self.reindex();
    }

    pub(crate) fn mark_stopped(&mut self) {
        self.list.mark_stopped();
    }

    fn reindex(&mut self) {
        self.by_object.clear();
        let events = self.list.items();
        let same_object = |left: &EventSummary, right: &EventSummary| left.object == right.object;
        let mut start = 0;
        while start < events.len() {
            let run = events[start..]
                .iter()
                .take_while(|event| same_object(event, &events[start]))
                .count();
            let object = &events[start].object;
            let key = IssueObject::new(&object.kind, object.namespace.as_deref(), &object.name);
            self.by_object.insert(key, start..start + run);
            start += run;
        }
    }

    pub(crate) fn state(&self) -> FeedState {
        list_state(&self.list)
    }

    /// The events about `object`, newest first. `None` unless the feed is ready, so a rule never
    /// mistakes "not loaded" for "no events".
    pub(crate) fn of(&self, object: &IssueObject) -> Option<&[EventSummary]> {
        let events = self.list.ready_items()?;
        Some(
            self.by_object
                .get(object)
                .map_or(&[], |range| &events[range.clone()]),
        )
    }

    /// Every event, grouped by object.
    pub(crate) fn recent(&self) -> impl Iterator<Item = &EventSummary> {
        self.list.items().iter()
    }
}

/// The kinds whose conditions the engine reads, always watched: Deployments, DaemonSets, Jobs,
/// HPAs, PDBs, quotas, claims, and the TLS secrets (the Secrets watch lists TLS secrets only).
const CONDITION_KINDS: [ResourceKind; 8] = [
    ResourceKind::Deployments,
    ResourceKind::DaemonSets,
    ResourceKind::Jobs,
    ResourceKind::HorizontalPodAutoscalers,
    ResourceKind::PodDisruptionBudgets,
    ResourceKind::ResourceQuotas,
    ResourceKind::PersistentVolumeClaims,
    ResourceKind::Secrets,
];

/// Whether the TLS Secrets list and watch run (`general.watch_tls_secrets`). It shows in API audit
/// logs, so the user can turn it off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CertificateWatch {
    Watch,
    Skip,
}

impl CertificateWatch {
    /// The saved choice; without settings (a view built in a test) the default is to watch.
    fn of(cx: &App) -> Self {
        match AppSettings::try_get(cx) {
            Some(settings) if !settings.general.watch_tls_secrets => Self::Skip,
            _ => Self::Watch,
        }
    }
}

/// What one condition feed does for a scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FeedPlan {
    /// Watch with `watch_scope`: the session scope up to two namespaces, else every namespace,
    /// with the rows outside the session scope dropped on tokio.
    Start {
        watch_scope: NamespaceScope,
    },
    /// The access review is still running.
    Wait,
    Off(String),
}

/// Which condition feeds start for `scope`, and with what watch scope. Up to two namespaces each
/// feed watches them; above that one cluster-wide watch per kind costs less than a watch per
/// namespace (`4N + 12` watches instead of `12N + 4`). A review that is still running waits; a
/// failed one starts the feed, which shows its own error. With at most two namespaces a known
/// denial turns the feed off at once, so a 403 is not retried for the whole session.
pub(crate) fn condition_plan(
    scope: &NamespaceScope,
    access: &AccessState,
    certificates: CertificateWatch,
) -> Vec<(ResourceKind, FeedPlan)> {
    let is_narrow = scope.namespaces().len() <= 2;
    let watch_scope = if is_narrow {
        scope.clone()
    } else {
        NamespaceScope::All
    };
    CONDITION_KINDS
        .into_iter()
        .map(|kind| {
            if kind == ResourceKind::Secrets && certificates == CertificateWatch::Skip {
                return (kind, FeedPlan::Off("off in Settings".to_owned()));
            }
            let plan = match access {
                AccessState::Checking { .. } => FeedPlan::Wait,
                AccessState::Known(report) if is_narrow => {
                    match kind
                        .access_check()
                        .filter(|check| !report.is_allowed(*check))
                    {
                        Some(check) => FeedPlan::Off(format!("not permitted: {check}")),
                        None => FeedPlan::Start {
                            watch_scope: watch_scope.clone(),
                        },
                    }
                }
                AccessState::Known(_) | AccessState::Unknown => FeedPlan::Start {
                    watch_scope: watch_scope.clone(),
                },
            };
            (kind, plan)
        })
        .collect()
}

/// One condition feed: the compact summaries of a kind, as `KindObject`s the WHY rules read.
pub(crate) struct ConditionFeed {
    pub(crate) kind: ResourceKind,
    pub(crate) list: LiveList<KindObject>,
    /// Why the feed does not run; set by the plan or by a 403 on a cluster-wide watch.
    off: Option<String>,
    /// The scope of the running watch; `None` while waiting or off.
    watch_scope: Option<NamespaceScope>,
    subscription: Option<WatchSubscription>,
}

impl ConditionFeed {
    fn idle(kind: ResourceKind, off: Option<String>) -> Self {
        Self {
            kind,
            list: LiveList::Loading,
            off,
            watch_scope: None,
            subscription: None,
        }
    }

    pub(crate) fn state(&self) -> FeedState {
        match &self.off {
            Some(reason) => FeedState::Off(reason.clone()),
            None => list_state(&self.list),
        }
    }

    /// The objects once the feed has loaded and runs; `None` while it is loading, waiting for its
    /// access review, failed, or off, so a reader never mistakes "not loaded" for "none".
    pub(crate) fn live_objects(&self) -> Option<&[KindObject]> {
        if self.off.is_some() {
            return None;
        }
        self.list.ready_items()
    }

    /// The watches this feed runs now.
    fn watches(&self) -> usize {
        match (&self.subscription, &self.watch_scope) {
            (Some(_), Some(scope)) => scope_multiplicity(scope),
            _ => 0,
        }
    }

    /// A 403 on a cluster-wide watch means no list right; the watch stops instead of retrying.
    pub(crate) fn apply(&mut self, update: WatchUpdate<KindObject>) {
        let is_forbidden = matches!(&update, WatchUpdate::Failed(ClusterError::Forbidden { .. }));
        if is_forbidden && self.watch_scope == Some(NamespaceScope::All) {
            self.off = Some("not permitted cluster-wide".to_owned());
            self.subscription = None;
            return;
        }
        self.list.apply(update);
    }

    pub(crate) fn mark_stopped(&mut self) {
        self.list.mark_stopped();
    }
}

/// The watches the issues need beyond the core lists; in `LiveCluster`. Dropping it stops them.
pub(crate) struct IssueFeeds {
    pub(crate) events: WarningEvents,
    /// `None` while a restart waits for its delay.
    events_watch: Option<WatchSubscription>,
    /// The delayed restart; replacing or dropping it cancels the wait.
    events_restart: Option<Task<()>>,
    /// One per `CONDITION_KINDS`, in that order.
    pub(crate) conditions: Vec<ConditionFeed>,
}

impl IssueFeeds {
    /// Starts the Warning events watch at once. The condition feeds wait for `restart_conditions`.
    pub(crate) fn start(
        runtime: &ClusterRuntime,
        live_connection: &ClusterConnection,
        scope: NamespaceScope,
        cx: &mut Context<ClusterSession>,
    ) -> Self {
        Self {
            events: WarningEvents::default(),
            events_watch: Some(watch_warning_events(runtime, live_connection, scope, cx)),
            events_restart: None,
            conditions: Vec::new(),
        }
    }

    /// Plans the condition feeds again for `scope` and `access`: every old watch stops, and the
    /// new plan starts, waits, or turns off each kind. Runs when the access review finishes and
    /// on a scope change, whose review is still running (so the feeds wait).
    pub(crate) fn restart_conditions(
        &mut self,
        runtime: &ClusterRuntime,
        connection: &ClusterConnection,
        scope: &NamespaceScope,
        access: &AccessState,
        cx: &mut Context<ClusterSession>,
    ) {
        self.conditions = condition_plan(scope, access, CertificateWatch::of(cx))
            .into_iter()
            .map(|(kind, plan)| match plan {
                FeedPlan::Wait => ConditionFeed::idle(kind, None),
                FeedPlan::Off(reason) => ConditionFeed::idle(kind, Some(reason)),
                FeedPlan::Start { watch_scope } => {
                    let subscription =
                        watch_condition(runtime, connection, kind, &watch_scope, scope, cx);
                    ConditionFeed {
                        kind,
                        list: LiveList::Loading,
                        off: None,
                        watch_scope: Some(watch_scope),
                        subscription: Some(subscription),
                    }
                }
            })
            .collect();
    }

    /// The watches the engine runs: the Warning events and the condition feeds, each over the
    /// namespaces of its scope (`namespaces` for the events).
    pub(crate) fn watch_count(&self, namespaces: usize) -> usize {
        self.watched(namespaces)
            .iter()
            .map(|(_, count)| count)
            .sum()
    }

    /// Each running watch of the engine with its kind name, so the status bar lists what the
    /// count counts.
    pub(crate) fn watched(&self, namespaces: usize) -> Vec<(&'static str, usize)> {
        let events = if self.is_watching_events() {
            namespaces
        } else {
            0
        };
        let conditions = self
            .conditions
            .iter()
            .map(|feed| (feed.kind.label(), feed.watches()));
        std::iter::once(("Events (Warning)", events))
            .chain(conditions)
            .collect()
    }

    /// The condition feed of `kind`, read only.
    pub(crate) fn condition(&self, kind: ResourceKind) -> Option<&ConditionFeed> {
        self.conditions.iter().find(|feed| feed.kind == kind)
    }

    /// The Deployments of their condition feed once it has loaded; `None` while it is loading or off.
    pub(crate) fn deployments(&self) -> Option<&[KindObject]> {
        self.condition(ResourceKind::Deployments)
            .and_then(|feed| feed.list.ready_items())
    }

    /// The condition feed of `kind`.
    pub(crate) fn condition_mut(&mut self, kind: ResourceKind) -> Option<&mut ConditionFeed> {
        self.conditions.iter_mut().find(|feed| feed.kind == kind)
    }

    /// Whether the Warning events watch runs; false while a restart waits for its delay.
    pub(crate) fn is_watching_events(&self) -> bool {
        self.events_watch.is_some()
    }

    /// Replaces the Warning events watch after a scope change, once the scope has settled for
    /// `EVENTS_RESTART_DELAY`. The old watch stops now, and the feed reads as loading.
    pub(crate) fn restart_events(&mut self, cx: &mut Context<ClusterSession>) {
        self.events = WarningEvents::default();
        self.events_watch = None;
        self.events_restart = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(EVENTS_RESTART_DELAY).await;
            let _ = this.update(cx, |session, cx| session.start_issue_events(cx));
        }));
    }

    /// Ends the wait and starts the watch for `scope`.
    pub(crate) fn finish_restart(
        &mut self,
        runtime: &ClusterRuntime,
        connection: &ClusterConnection,
        scope: NamespaceScope,
        cx: &mut Context<ClusterSession>,
    ) {
        self.events_restart = None;
        self.events_watch = Some(watch_warning_events(runtime, connection, scope, cx));
    }
}

/// Starts the watch of one condition kind. Wider than the session scope, it drops the rows
/// outside it on tokio, before the snapshot reaches the main thread.
fn watch_condition(
    runtime: &ClusterRuntime,
    connection: &ClusterConnection,
    kind: ResourceKind,
    watch_scope: &NamespaceScope,
    session_scope: &NamespaceScope,
    cx: &mut Context<ClusterSession>,
) -> WatchSubscription {
    let keep = (watch_scope != session_scope).then(|| session_scope.namespaces().to_vec());
    let updates = condition_updates(connection, kind, watch_scope.clone(), keep);
    runtime.subscribe_silent(
        updates,
        cx,
        move |session: &mut ClusterSession, update, _| session.apply_condition_update(kind, update),
        move |session, _| session.stop_condition(kind),
    )
}

/// The summaries of `kind` as objects; `keep` limits them to those namespaces.
fn condition_updates(
    connection: &ClusterConnection,
    kind: ResourceKind,
    scope: NamespaceScope,
    keep: Option<Vec<String>>,
) -> BoxStream<'static, WatchUpdate<KindObject>> {
    match kind {
        ResourceKind::Deployments => objects(
            connection.watch_deployments(scope),
            keep,
            |item| &item.namespace,
            KindObject::Deployment,
        ),
        ResourceKind::DaemonSets => objects(
            connection.watch_daemon_sets(scope),
            keep,
            |item| &item.namespace,
            KindObject::DaemonSet,
        ),
        ResourceKind::Jobs => objects(
            connection.watch_jobs(scope),
            keep,
            |item| &item.namespace,
            KindObject::Job,
        ),
        ResourceKind::HorizontalPodAutoscalers => objects(
            connection.watch_horizontal_pod_autoscalers(scope),
            keep,
            |item| &item.namespace,
            KindObject::HorizontalPodAutoscaler,
        ),
        ResourceKind::PodDisruptionBudgets => objects(
            connection.watch_pod_disruption_budgets(scope),
            keep,
            |item| &item.namespace,
            KindObject::PodDisruptionBudget,
        ),
        ResourceKind::ResourceQuotas => objects(
            connection.watch_resource_quotas(scope),
            keep,
            |item| &item.namespace,
            KindObject::ResourceQuota,
        ),
        ResourceKind::PersistentVolumeClaims => objects(
            connection.watch_persistent_volume_claims(scope),
            keep,
            |item| &item.namespace,
            KindObject::PersistentVolumeClaim,
        ),
        ResourceKind::Secrets => objects(
            connection.watch_tls_secrets(scope),
            keep,
            |item| &item.namespace,
            KindObject::Secret,
        ),
        // Not a condition kind: an empty stream reads as a stopped feed.
        _ => futures::stream::empty().boxed(),
    }
}

/// Wraps each summary of a watch as an object, dropping those outside `keep`.
fn objects<S: Send + 'static>(
    updates: impl Stream<Item = WatchUpdate<S>> + Send + 'static,
    keep: Option<Vec<String>>,
    namespace_of: fn(&S) -> &str,
    wrap: fn(S) -> KindObject,
) -> BoxStream<'static, WatchUpdate<KindObject>> {
    updates
        .map(move |update| match update {
            WatchUpdate::Snapshot(items) => WatchUpdate::Snapshot(
                items
                    .into_iter()
                    .filter(|item| {
                        keep.as_ref()
                            .is_none_or(|names| names.iter().any(|name| name == namespace_of(item)))
                    })
                    .map(wrap)
                    .collect(),
            ),
            WatchUpdate::Failed(error) => WatchUpdate::Failed(error),
        })
        .boxed()
}

fn watch_warning_events(
    runtime: &ClusterRuntime,
    connection: &ClusterConnection,
    scope: NamespaceScope,
    cx: &mut Context<ClusterSession>,
) -> WatchSubscription {
    runtime.subscribe_silent(
        connection
            .watch_events(scope, EventFilter::WarningsOnly)
            .boxed(),
        cx,
        |session: &mut ClusterSession, update, _| session.apply_warning_events(update),
        |session, _| session.stop_warning_events(),
    )
}

#[cfg(test)]
#[path = "issue_feeds_tests.rs"]
mod issue_feeds_tests;
