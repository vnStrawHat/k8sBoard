//! The kubelet feed of a live session (network and disk I/O counters, PVC usage): which nodes
//! it reads for the open drawer, what is in its history, and what state it is in. One
//! subscription lives for the whole session; the nodes it reads change through a `watch`
//! channel, so a click through drawers never restarts the poll.

use std::collections::{BTreeMap, BTreeSet};

use cluster::{
    ClusterError, KubeletTargets, NamespaceScope, NodeKubeletStats, NodeReadiness, NodeSummary,
    PodSummary, WatchUpdate,
};
use tokio::sync::watch;

use crate::cluster_metrics::FeedStatus;
use crate::cluster_runtime::WatchSubscription;
use crate::cluster_session::error_text;
use crate::kind_row::{PodOwner, owns_pod};
use crate::kubelet_history::KubeletHistory;
use crate::live_sections::claim_pods;
use crate::node_usage::takes_room;

/// At most this many Ready nodes are always polled for their summary; above it only the nodes
/// of the open drawer are, and never more than this many.
pub(crate) const SUMMARY_NODE_LIMIT: usize = 10;
/// The most nodes whose cAdvisor body is read (about 1 MB each per round).
pub(crate) const DISK_IO_NODE_LIMIT: usize = 3;

/// What the open drawer wants the kubelets for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum KubeletSubject {
    Pod {
        namespace: String,
        name: String,
    },
    Node(String),
    Workload(PodOwner),
    /// The pods that mount a PVC, for its Usage bars.
    Claim {
        namespace: String,
        claim: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct KubeletDemand {
    pub(crate) subject: Option<KubeletSubject>,
    /// A Monitor tab that shows Disk I/O is visible.
    pub(crate) wants_disk_io: bool,
}

/// A node and how many of the subject's pods run on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NodeShare {
    pub(crate) node: String,
    pub(crate) pods: usize,
}

/// Why a node's reads of the last round failed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct NodeErrors {
    pub(crate) summary: Option<String>,
    pub(crate) disk_io: Option<String>,
}

/// The feed of a live session. Dropping it stops the poll: the sender closes and the stream
/// ends.
pub(crate) struct KubeletFeed {
    pub(crate) history: KubeletHistory,
    pub(crate) status: FeedStatus,
    /// The nodes that failed in the last round.
    pub(crate) node_errors: BTreeMap<String, NodeErrors>,
    demand: KubeletDemand,
    targets: watch::Sender<KubeletTargets>,
    subscription: Option<WatchSubscription>,
}

impl KubeletFeed {
    /// Waits for the access review; nothing is read yet.
    pub(crate) fn new() -> Self {
        Self {
            history: KubeletHistory::default(),
            status: FeedStatus::Checking,
            node_errors: BTreeMap::new(),
            demand: KubeletDemand::default(),
            targets: watch::Sender::new(KubeletTargets::default()),
            subscription: None,
        }
    }

    pub(crate) fn demand(&self) -> &KubeletDemand {
        &self.demand
    }

    /// The nodes the poll reads now.
    pub(crate) fn targets(&self) -> watch::Ref<'_, KubeletTargets> {
        self.targets.borrow()
    }

    /// The review has not finished: a running poll continues, a stopped feed shows `Checking`.
    pub(crate) fn wait(&mut self) {
        if self.subscription.is_none() {
            self.status = FeedStatus::Checking;
        }
    }

    /// Denied: stops polling.
    pub(crate) fn turn_off(&mut self, reason: String) {
        self.subscription = None;
        self.set_status(FeedStatus::Unavailable(reason));
    }

    /// Allowed: refreshes the targets and subscribes with `subscribe` unless it already does.
    pub(crate) fn poll(
        &mut self,
        nodes: &[NodeSummary],
        pods: &[PodSummary],
        subscribe: impl FnOnce(watch::Receiver<KubeletTargets>) -> WatchSubscription,
    ) {
        self.refresh_targets(nodes, pods);
        if self.subscription.is_some() {
            return;
        }
        self.subscription = Some(subscribe(self.targets.subscribe()));
        self.status = FeedStatus::Waiting;
    }

    /// Stores the demand and moves the targets with it.
    pub(crate) fn set_demand(
        &mut self,
        demand: KubeletDemand,
        nodes: &[NodeSummary],
        pods: &[PodSummary],
    ) {
        if demand == self.demand {
            return;
        }
        self.demand = demand;
        self.refresh_targets(nodes, pods);
    }

    /// Sends the targets the demand, the nodes, and the pods give. Equal targets are not sent,
    /// so the poll is not woken for nothing.
    pub(crate) fn refresh_targets(&self, nodes: &[NodeSummary], pods: &[PodSummary]) {
        let next = kubelet_targets(&self.demand, nodes, pods);
        self.targets.send_if_modified(|current| {
            if *current == next {
                return false;
            }
            *current = next;
            true
        });
    }

    /// One round. `pods` and `scope` are the session's, for the host-network flags and for the
    /// namespaces the history keeps.
    pub(crate) fn receive(
        &mut self,
        update: WatchUpdate<NodeKubeletStats>,
        pods: &[PodSummary],
        scope: &NamespaceScope,
    ) {
        match update {
            WatchUpdate::Snapshot(round) => {
                self.history
                    .record(jiff::Timestamp::now(), &round, pods, scope);
                self.note_errors(&round);
                self.status = FeedStatus::Live;
            }
            WatchUpdate::Failed(error) => {
                let reason = kubelet_error_text(&error);
                let status = if self.history.tick_count() > 0 {
                    FeedStatus::Interrupted(reason)
                } else {
                    FeedStatus::Failed(reason)
                };
                self.set_status(status);
            }
        }
    }

    /// The stream ended, which a poll never does on its own.
    pub(crate) fn mark_stopped(&mut self) {
        self.subscription = None;
        self.set_status(FeedStatus::Failed(
            "kubelet polling stopped unexpectedly".to_owned(),
        ));
    }

    /// Remembers which nodes failed. A change is logged at debug level; the texts hold no
    /// secret and no metric value.
    fn note_errors(&mut self, round: &[NodeKubeletStats]) {
        let errors: BTreeMap<String, NodeErrors> = round
            .iter()
            .filter_map(|stats| {
                let errors = NodeErrors {
                    summary: stats.summary.as_ref().err().map(kubelet_error_text),
                    disk_io: stats
                        .disk_io
                        .as_ref()
                        .and_then(|read| read.as_ref().err())
                        .map(kubelet_error_text),
                };
                (errors != NodeErrors::default()).then(|| (stats.node.clone(), errors))
            })
            .collect();
        if errors != self.node_errors {
            for (node, error) in &errors {
                tracing::debug!(node, ?error, "kubelet reads failed");
            }
            self.node_errors = errors;
        }
    }

    /// Warns once when the feed moves into a problem state.
    fn set_status(&mut self, status: FeedStatus) {
        if status != self.status
            && let Some(reason) = status.reason()
        {
            tracing::warn!(reason, "kubelet feed has a problem");
        }
        self.status = status;
    }
}

/// The nodes running the subject's pods that still take room, most pods first, then by name. A
/// Node subject is itself.
pub(crate) fn subject_nodes(subject: &KubeletSubject, pods: &[PodSummary]) -> Vec<NodeShare> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    match subject {
        KubeletSubject::Node(name) => {
            let running = pods
                .iter()
                .filter(|pod| pod.node_name.as_deref() == Some(name.as_str()) && takes_room(pod))
                .count();
            return vec![NodeShare {
                node: name.clone(),
                pods: running,
            }];
        }
        KubeletSubject::Pod { namespace, name } => {
            let subject_pods = pods
                .iter()
                .filter(|pod| pod.namespace == *namespace && pod.name == *name);
            count_nodes(&mut counts, subject_pods);
        }
        KubeletSubject::Workload(owner) => {
            count_nodes(&mut counts, pods.iter().filter(|pod| owns_pod(owner, pod)));
        }
        KubeletSubject::Claim { namespace, claim } => {
            let mounting = claim_pods(namespace, claim, pods)
                .into_iter()
                .map(|(pod, _)| pod);
            count_nodes(&mut counts, mounting);
        }
    }
    let mut shares: Vec<NodeShare> = counts
        .into_iter()
        .map(|(node, pods)| NodeShare {
            node: node.to_owned(),
            pods,
        })
        .collect();
    // The map is ordered by name, and the sort is stable.
    shares.sort_by_key(|share| std::cmp::Reverse(share.pods));
    shares
}

fn count_nodes<'a>(
    counts: &mut BTreeMap<&'a str, usize>,
    pods: impl Iterator<Item = &'a PodSummary>,
) {
    for pod in pods.filter(|pod| takes_room(pod)) {
        if let Some(node) = pod.node_name.as_deref() {
            *counts.entry(node).or_default() += 1;
        }
    }
}

/// The nodes to read. Not-Ready nodes are never targets: the proxy would answer 503 or time out.
/// Both lists are sorted, so equal demand gives equal targets.
pub(crate) fn kubelet_targets(
    demand: &KubeletDemand,
    nodes: &[NodeSummary],
    pods: &[PodSummary],
) -> KubeletTargets {
    let ready: BTreeSet<&str> = nodes
        .iter()
        .filter(|node| node.status.readiness == NodeReadiness::Ready)
        .map(|node| node.name.as_str())
        .collect();
    let shares = demand
        .subject
        .as_ref()
        .map(|subject| subject_nodes(subject, pods))
        .unwrap_or_default();
    let demanded: Vec<&str> = shares
        .iter()
        .filter(|share| ready.contains(share.node.as_str()))
        .map(|share| share.node.as_str())
        .collect();
    let summary: Vec<&str> = if ready.len() <= SUMMARY_NODE_LIMIT {
        ready.iter().copied().collect()
    } else {
        demanded.iter().copied().take(SUMMARY_NODE_LIMIT).collect()
    };
    let disk_io: Vec<&str> = if demand.wants_disk_io {
        demanded.iter().copied().take(DISK_IO_NODE_LIMIT).collect()
    } else {
        Vec::new()
    };
    KubeletTargets {
        summary_nodes: sorted(summary),
        disk_io_nodes: sorted(disk_io),
    }
}

fn sorted(mut nodes: Vec<&str>) -> Vec<String> {
    nodes.sort_unstable();
    nodes.into_iter().map(str::to_owned).collect()
}

/// A kubelet that cannot be reached through the proxy is named as such.
pub(crate) fn kubelet_error_text(error: &ClusterError) -> String {
    match error {
        ClusterError::Api { code: 503, .. } => {
            "the kubelet does not answer through the API server proxy".to_owned()
        }
        ClusterError::Api { code: 404, .. } => {
            "the kubelet does not serve this endpoint".to_owned()
        }
        other => error_text(other),
    }
}

#[cfg(test)]
#[path = "kubelet_metrics_tests.rs"]
mod kubelet_metrics_tests;
