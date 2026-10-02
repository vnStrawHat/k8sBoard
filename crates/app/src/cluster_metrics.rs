//! The two metrics feeds of a live session (pod usage and node usage): what each may poll
//! given the access reviews, where its history lives, and what state it is in. Feeds are
//! polls, so they are not watches and never set the status bar's problem flag.

use cluster::{
    AccessCheck, AccessDecision, ClusterError, NamespaceAccess, NamespaceScope, NodeMetrics,
    PodMetrics, PodSummary, WatchUpdate,
};
use gpui_kit::Task;

use crate::cluster_runtime::WatchSubscription;
use crate::cluster_session::{AccessState, error_text};
use crate::metrics_history::{NodeUsageHistory, PodUsageHistory};

/// The pod metrics access review: one entry per namespace of the scope.
pub(crate) type PodReviewResult = Result<Vec<NamespaceAccess>, String>;

/// Both metrics feeds of a live session. Dropping it stops the review and both polls.
pub(crate) struct ClusterMetrics {
    pub(crate) pods: MetricsFeed<PodUsageHistory>,
    pub(crate) nodes: MetricsFeed<NodeUsageHistory>,
    pod_review: PodReview,
}

pub(crate) enum PodReview {
    Running { _task: Task<()> },
    Done(PodReviewResult),
}

impl ClusterMetrics {
    /// Both feeds wait for their access reviews.
    pub(crate) fn new(pod_review: PodReview) -> Self {
        Self {
            pods: MetricsFeed::new("pod metrics"),
            nodes: MetricsFeed::new("node metrics"),
            pod_review,
        }
    }

    /// The review of the pods in the current scope, once it has finished.
    pub(crate) fn pod_review(&self) -> Option<&PodReviewResult> {
        match &self.pod_review {
            PodReview::Running { .. } => None,
            PodReview::Done(result) => Some(result),
        }
    }

    pub(crate) fn finish_pod_review(&mut self, result: PodReviewResult) {
        self.pod_review = PodReview::Done(result);
    }

    /// A new pods scope: the old poll stops, other namespaces are forgotten, and the access
    /// of the new scope is reviewed. The nodes feed does not depend on the scope.
    pub(crate) fn restart_pods(&mut self, scope: &NamespaceScope, review: PodReview) {
        self.pods.stop_polling();
        self.pods.note = None;
        self.pods.history.retain_scope(scope);
        self.pods.status = FeedStatus::Checking;
        self.pod_review = review;
    }
}

/// One metrics feed: the history it fills and where it stands.
pub(crate) struct MetricsFeed<H> {
    pub(crate) history: H,
    pub(crate) status: FeedStatus,
    /// Namespaces left out for lack of access, such as `no access in web`; the pods feed only.
    pub(crate) note: Option<String>,
    /// What the logs call this feed.
    name: &'static str,
    /// `Some` exactly while polling.
    subscription: Option<WatchSubscription>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FeedStatus {
    /// An access review still runs.
    Checking,
    /// Polling, no sample yet.
    Waiting,
    Live,
    /// The last poll failed; older ticks stay.
    Interrupted(String),
    /// Polling, no sample ever; retrying with backoff.
    Failed(String),
    /// Denied: never polled.
    Unavailable(String),
}

impl FeedStatus {
    fn reason(&self) -> Option<&str> {
        match self {
            Self::Interrupted(reason) | Self::Failed(reason) | Self::Unavailable(reason) => {
                Some(reason)
            }
            Self::Checking | Self::Waiting | Self::Live => None,
        }
    }
}

impl<H: Default> MetricsFeed<H> {
    fn new(name: &'static str) -> Self {
        Self {
            history: H::default(),
            status: FeedStatus::Checking,
            note: None,
            name,
            subscription: None,
        }
    }
}

impl<H> MetricsFeed<H> {
    /// The review has not finished: a running poll continues, a stopped feed shows `Checking`.
    pub(crate) fn wait(&mut self) {
        if self.subscription.is_none() {
            self.status = FeedStatus::Checking;
        }
    }

    /// Denied: stops polling. Logged once per transition, never per update.
    pub(crate) fn turn_off(&mut self, reason: String) {
        self.subscription = None;
        self.note = None;
        self.set_status(FeedStatus::Unavailable(reason));
    }

    /// Allowed: starts polling with `subscribe` unless it already does. `note` names the
    /// namespaces left out.
    pub(crate) fn poll(
        &mut self,
        note: Option<String>,
        subscribe: impl FnOnce() -> WatchSubscription,
    ) {
        self.note = note;
        if self.subscription.is_some() {
            return;
        }
        self.subscription = Some(subscribe());
        self.status = FeedStatus::Waiting;
    }

    fn stop_polling(&mut self) {
        self.subscription = None;
    }

    /// The stream ended, which a poll never does on its own.
    pub(crate) fn mark_stopped(&mut self) {
        self.subscription = None;
        self.set_status(FeedStatus::Failed(
            "metrics polling stopped unexpectedly".to_owned(),
        ));
    }

    /// A failed poll keeps the older ticks, and polling continues with backoff.
    fn fail(&mut self, error: &ClusterError, ticks: u64) {
        let reason = poll_error_text(error);
        self.set_status(if ticks > 0 {
            FeedStatus::Interrupted(reason)
        } else {
            FeedStatus::Failed(reason)
        });
    }

    /// Warns once when the feed moves into a problem state; the reason holds no secret and no
    /// metric value.
    fn set_status(&mut self, status: FeedStatus) {
        if status != self.status
            && let Some(reason) = status.reason()
        {
            tracing::warn!(feed = self.name, reason, "metrics feed has a problem");
        }
        self.status = status;
    }
}

impl MetricsFeed<PodUsageHistory> {
    /// `pods` is the live pods list, for the controllers and OOM kills of the history.
    pub(crate) fn receive(&mut self, update: WatchUpdate<PodMetrics>, pods: &[PodSummary]) {
        match update {
            WatchUpdate::Snapshot(items) => {
                self.history.record(jiff::Timestamp::now(), &items, pods);
                self.status = FeedStatus::Live;
            }
            WatchUpdate::Failed(error) => self.fail(&error, self.history.tick_count()),
        }
    }
}

impl MetricsFeed<NodeUsageHistory> {
    pub(crate) fn receive(&mut self, update: WatchUpdate<NodeMetrics>) {
        match update {
            WatchUpdate::Snapshot(items) => {
                self.history.record(jiff::Timestamp::now(), &items);
                self.status = FeedStatus::Live;
            }
            WatchUpdate::Failed(error) => self.fail(&error, self.history.tick_count()),
        }
    }
}

/// What the pods feed may do.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PodsGate {
    Wait,
    /// Poll `scope`; `note` names the namespaces left out for lack of access.
    Poll {
        scope: NamespaceScope,
        note: Option<String>,
    },
    Off(String),
}

/// What the nodes feed may do.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum NodesGate {
    Wait,
    Poll,
    Off(String),
}

const PODS_CHECK: AccessCheck = AccessCheck::ListPodMetrics;

/// Polls the namespaces of `scope` the review allows. A failed review polls everything, like
/// an access report that is `Unknown`: the server answers for itself.
pub(crate) fn pods_gate(review: Option<&PodReviewResult>, scope: &NamespaceScope) -> PodsGate {
    let Some(review) = review else {
        return PodsGate::Wait;
    };
    let Ok(entries) = review else {
        return poll_all(scope);
    };
    let denied: Vec<&NamespaceAccess> = entries
        .iter()
        .filter(|entry| matches!(entry.decision, AccessDecision::Denied { .. }))
        .collect();
    if denied.is_empty() {
        return poll_all(scope);
    }
    let allowed: Vec<String> = entries
        .iter()
        .filter(|entry| entry.decision == AccessDecision::Allowed)
        .filter_map(|entry| entry.namespace.clone())
        .collect();
    let names = denied_names(&denied);
    if !allowed.is_empty() {
        return PodsGate::Poll {
            scope: NamespaceScope::of_namespaces(allowed),
            note: Some(format!("no access in {names}")),
        };
    }
    let mut reason = format!("not allowed to {PODS_CHECK} in {names}");
    let first_reason = denied.iter().find_map(|entry| match &entry.decision {
        AccessDecision::Denied { reason } => reason.as_deref(),
        AccessDecision::Allowed => None,
    });
    if let Some(first_reason) = first_reason {
        reason.push_str(": ");
        reason.push_str(first_reason);
    }
    PodsGate::Off(reason)
}

fn poll_all(scope: &NamespaceScope) -> PodsGate {
    PodsGate::Poll {
        scope: scope.clone(),
        note: None,
    }
}

/// `a, b`, or `all namespaces` for the cluster-wide review.
fn denied_names(denied: &[&NamespaceAccess]) -> String {
    let names: Vec<&str> = denied
        .iter()
        .filter_map(|entry| entry.namespace.as_deref())
        .collect();
    if names.is_empty() {
        return "all namespaces".to_owned();
    }
    names.join(", ")
}

/// The nodes feed follows the session's own access report for `check`.
pub(crate) fn nodes_gate(access: &AccessState, check: AccessCheck) -> NodesGate {
    let report = match access {
        AccessState::Checking { .. } => return NodesGate::Wait,
        AccessState::Unknown => return NodesGate::Poll,
        AccessState::Known(report) => report,
    };
    let denial = report
        .reviews
        .iter()
        .find(|review| review.check == check)
        .and_then(|review| match &review.decision {
            AccessDecision::Denied { reason } => Some(reason),
            AccessDecision::Allowed => None,
        });
    let Some(reason) = denial else {
        return NodesGate::Poll;
    };
    let mut text = format!("not allowed to {check}");
    if let Some(reason) = reason {
        text.push_str(": ");
        text.push_str(reason);
    }
    NodesGate::Off(text)
}

/// The text of a failed poll: a missing or broken metrics-server is named as such.
pub(crate) fn poll_error_text(error: &ClusterError) -> String {
    match error {
        ClusterError::Api { code: 404, .. } => {
            "metrics-server is not installed: the cluster does not serve metrics.k8s.io".to_owned()
        }
        ClusterError::Api { code: 503, .. } => {
            format!("metrics.k8s.io does not answer: {}", error_text(error))
        }
        other => error_text(other),
    }
}

/// Whether the screenshot hook may capture: the feed is unavailable, failed, or interrupted (it
/// will not get better soon), or it has `min_ticks` ticks.
#[cfg(any(feature = "screenshot", test))]
pub(crate) fn is_metrics_settled(status: &FeedStatus, ticks: u64, min_ticks: u64) -> bool {
    matches!(
        status,
        FeedStatus::Unavailable(_) | FeedStatus::Failed(_) | FeedStatus::Interrupted(_)
    ) || ticks >= min_ticks
}

#[cfg(test)]
#[path = "cluster_metrics_tests.rs"]
mod cluster_metrics_tests;
