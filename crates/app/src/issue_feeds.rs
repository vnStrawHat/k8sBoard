//! What the Issues engine watches beyond the core lists, and how complete that coverage is. The
//! feeds run on the cluster runtime and reach the session through silent subscriptions; this
//! module holds their state. Event messages are arbitrary text, so nothing here logs them.

use std::collections::HashMap;
use std::ops::Range;
use std::time::Duration;

use cluster::{EventFilter, EventSummary, NamespaceScope, NodeReadiness, WatchUpdate};
use futures::StreamExt as _;
use gpui_kit::{Context, Task};

use crate::cluster_metrics::FeedStatus;
use crate::cluster_runtime::{ClusterRuntime, WatchSubscription};
use crate::cluster_session::{ClusterSession, LiveCluster, LiveList};
use crate::issue::IssueObject;

/// How long a scope change waits before the Warning events watch restarts. The API server keeps no
/// watch cache for events and cannot index them, so each start scans every event of the scope in
/// etcd; picking namespaces one after another must not run one scan per click.
const EVENTS_RESTART_DELAY: Duration = Duration::from_secs(1);

/// A source of problems, as the coverage names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum IssueFeed {
    Pods,
    Nodes,
    WarningEvents,
    PodMetrics,
    NodeMetrics,
    VolumeUsage,
}

impl IssueFeed {
    fn label(self) -> &'static str {
        match self {
            Self::Pods => "pods",
            Self::Nodes => "nodes",
            Self::WarningEvents => "warning events",
            Self::PodMetrics => "pod metrics",
            Self::NodeMetrics => "node metrics",
            Self::VolumeUsage => "volume usage",
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
        let off: Vec<String> = self
            .feeds
            .iter()
            .filter_map(|(feed, state)| match state {
                FeedState::Off(reason) => Some(format!("{} ({reason})", feed.label())),
                _ => None,
            })
            .collect();
        if !off.is_empty() {
            sentences.push(format!("Not checked: {}.", off.join(", ")));
        }
        let loading: Vec<&str> = self
            .feeds
            .iter()
            .filter(|(_, state)| *state == FeedState::Loading)
            .map(|(feed, _)| feed.label())
            .collect();
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
pub(crate) fn core_coverage(live: &LiveCluster, events: &WarningEvents) -> Coverage {
    let ready_nodes = live
        .nodes
        .items()
        .iter()
        .filter(|node| node.status.readiness == NodeReadiness::Ready)
        .count();
    let polled_nodes = live.metrics.kubelet.targets().summary_nodes.len();
    Coverage {
        feeds: vec![
            (IssueFeed::Pods, list_state(&live.pods)),
            (IssueFeed::Nodes, list_state(&live.nodes)),
            (IssueFeed::WarningEvents, events.state()),
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
    }
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

/// The watches the issues need beyond the core lists; in `LiveCluster`. Dropping it stops them.
pub(crate) struct IssueFeeds {
    pub(crate) events: WarningEvents,
    /// `None` while a restart waits for its delay.
    events_watch: Option<WatchSubscription>,
    /// The delayed restart; replacing or dropping it cancels the wait.
    events_restart: Option<Task<()>>,
}

impl IssueFeeds {
    /// Starts the Warning events watch at once.
    pub(crate) fn start(
        runtime: &ClusterRuntime,
        live_connection: &cluster::ClusterConnection,
        scope: NamespaceScope,
        cx: &mut Context<ClusterSession>,
    ) -> Self {
        Self {
            events: WarningEvents::default(),
            events_watch: Some(watch_warning_events(runtime, live_connection, scope, cx)),
            events_restart: None,
        }
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
        connection: &cluster::ClusterConnection,
        scope: NamespaceScope,
        cx: &mut Context<ClusterSession>,
    ) {
        self.events_restart = None;
        self.events_watch = Some(watch_warning_events(runtime, connection, scope, cx));
    }
}

fn watch_warning_events(
    runtime: &ClusterRuntime,
    connection: &cluster::ClusterConnection,
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
