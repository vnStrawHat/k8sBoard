//! Why a Network or Disk I/O card may be empty or partial: the first rule that applies wins. Pure,
//! so tests need no session.

use std::collections::BTreeSet;

use cluster::{NodeReadiness, PodSummary};

use crate::cluster_metrics::FeedStatus;
use crate::drawer::MonitorScope;
use crate::kind_row::owns_pod;
use crate::kubelet_history::{DiskIoState, RateKind};
use crate::monitor_data::{MonitorInput, MonitorSubject};
use crate::node_usage::takes_room;

pub(crate) const COLLECTING: &str = "Collecting… rates need two samples";

/// What the card's subject is made of, as far as the kubelet is concerned.
struct Subject<'a> {
    /// A pod or container subject has one; a workload the pods it owns that still take room; a
    /// node none.
    pods: Vec<&'a PodSummary>,
    /// The nodes the subject runs on, sorted and distinct.
    nodes: Vec<&'a str>,
    /// The container a pod's disk series is narrowed to.
    container: Option<&'a str>,
    is_workload: bool,
}

impl<'a> Subject<'a> {
    fn of(input: &'a MonitorInput<'a>, scope: &'a MonitorScope) -> Self {
        let part = match scope {
            MonitorScope::Total => None,
            MonitorScope::Part(name) => Some(name.as_str()),
        };
        let (pods, container, is_workload) = match &input.subject {
            MonitorSubject::Pod(pod) => (vec![*pod], part, false),
            MonitorSubject::Container { pod, container } => (vec![*pod], Some(*container), false),
            MonitorSubject::Workload(owner) => {
                let pods = input
                    .pods
                    .iter()
                    .filter(|pod| owns_pod(owner, pod) && takes_room(pod))
                    .filter(|pod| part.is_none_or(|name| pod.name == name))
                    .collect();
                (pods, None, true)
            }
            MonitorSubject::Node(_) => (Vec::new(), None, false),
        };
        let nodes: BTreeSet<&str> = match &input.subject {
            MonitorSubject::Node(node) => BTreeSet::from([node.name.as_str()]),
            _ => pods
                .iter()
                .filter_map(|pod| pod.node_name.as_deref())
                .collect(),
        };
        Self {
            pods,
            nodes: nodes.into_iter().collect(),
            container,
            is_workload,
        }
    }
}

/// The notice of the `kind` card for the subject and scope of `input`, or `None` when the card
/// has nothing to explain.
pub(crate) fn kubelet_notice(
    input: &MonitorInput,
    scope: &MonitorScope,
    kind: RateKind,
) -> Option<String> {
    let feed = input.kubelet;
    let is_waiting = matches!(feed.status, FeedStatus::Checking | FeedStatus::Waiting);
    if is_waiting || feed.history.tick_count() < 2 {
        return Some(COLLECTING.to_owned());
    }
    let subject = Subject::of(input, scope);
    not_ready_notice(input, &subject)
        .or_else(|| node_error_notice(input, &subject, kind))
        .or_else(|| match kind {
            RateKind::Network => host_network_notice(input, &subject),
            RateKind::DiskIo => disk_notice(input, &subject),
        })
        .or_else(|| coverage_notice(input, &subject, kind))
        .or_else(|| match kind {
            RateKind::Network => {
                workload_host_network_notice(&subject).or_else(|| shared_network_notice(&subject))
            }
            RateKind::DiskIo => None,
        })
}

/// A pod's or node's own node that is not Ready answers nothing through the proxy.
fn not_ready_notice(input: &MonitorInput, subject: &Subject) -> Option<String> {
    if subject.is_workload {
        return None;
    }
    subject.nodes.iter().find_map(|name| {
        let node = input.nodes.iter().find(|node| node.name == *name)?;
        (node.status.readiness != NodeReadiness::Ready)
            .then(|| format!("Node {name} is not ready: no kubelet stats"))
    })
}

/// A summary error stops the Network card; a cAdvisor error stops the Disk I/O card.
fn node_error_notice(input: &MonitorInput, subject: &Subject, kind: RateKind) -> Option<String> {
    subject.nodes.iter().find_map(|name| {
        let errors = input.kubelet.node_errors.get(*name)?;
        let error = match kind {
            RateKind::Network => errors.summary.as_ref(),
            RateKind::DiskIo => errors.disk_io.as_ref(),
        }?;
        Some(format!("Kubelet on {name}: {error}"))
    })
}

/// A pod's or container's own host-network flag: its counters are the node's.
fn host_network_notice(input: &MonitorInput, subject: &Subject) -> Option<String> {
    if subject.is_workload || matches!(input.subject, MonitorSubject::Node(_)) {
        return None;
    }
    let is_host_network = subject.pods.first().is_some_and(|pod| pod.host_network);
    is_host_network
        .then(|| "Host network: traffic is the node's (see the node's Monitor tab)".to_owned())
}

fn disk_notice(input: &MonitorInput, subject: &Subject) -> Option<String> {
    let history = &input.kubelet.history;
    match &input.subject {
        MonitorSubject::Node(node) => match history.disk_io_state(&node.name) {
            // Not read yet: the card says it is still collecting once it knows it has no rate.
            DiskIoState::NotSampled | DiskIoState::Sampled { has_root: true } => None,
            DiskIoState::Sampled { has_root: false } => {
                Some("The kubelet reports no node-level disk I/O".to_owned())
            }
        },
        MonitorSubject::Pod(_) | MonitorSubject::Container { .. } => {
            let pod = subject.pods.first()?;
            let state = history.disk_io_state(pod.node_name.as_deref()?);
            match state {
                DiskIoState::NotSampled => None,
                DiskIoState::Sampled { .. }
                    if !history.has_disk_series(&pod.namespace, &pod.name, subject.container) =>
                {
                    let what = if subject.container.is_some() {
                        "container"
                    } else {
                        "pod"
                    };
                    Some(format!("The kubelet reports no disk I/O for this {what}"))
                }
                DiskIoState::Sampled { .. } => None,
            }
        }
        MonitorSubject::Workload(_) => {
            if subject.pods.is_empty() {
                return None;
            }
            let sampled: Vec<&&PodSummary> = subject
                .pods
                .iter()
                .filter(|pod| {
                    let node = pod.node_name.as_deref().unwrap_or_default();
                    matches!(history.disk_io_state(node), DiskIoState::Sampled { .. })
                })
                .collect();
            if sampled.is_empty() {
                return None;
            }
            let has_none = sampled
                .iter()
                .all(|pod| !history.has_disk_series(&pod.namespace, &pod.name, None));
            has_none.then(|| "The kubelet reports no disk I/O for these pods".to_owned())
        }
    }
}

/// A workload with pods on nodes the feed does not read.
fn coverage_notice(input: &MonitorInput, subject: &Subject, kind: RateKind) -> Option<String> {
    if !subject.is_workload {
        return None;
    }
    let targets = input.kubelet.targets();
    let polled = match kind {
        RateKind::Network => &targets.summary_nodes,
        RateKind::DiskIo => &targets.disk_io_nodes,
    };
    let covered = subject
        .nodes
        .iter()
        .filter(|name| polled.iter().any(|node| node == **name))
        .count();
    (covered < subject.nodes.len())
        .then(|| format!("Covers pods on {covered} of {} nodes", subject.nodes.len()))
}

/// Owned pods that share the node's network add nothing to the sum. A `Part(p)` scope on one of
/// them reads `1 host-network pod not counted`.
fn workload_host_network_notice(subject: &Subject) -> Option<String> {
    if !subject.is_workload {
        return None;
    }
    match subject.pods.iter().filter(|pod| pod.host_network).count() {
        0 => None,
        1 => Some("1 host-network pod not counted".to_owned()),
        count => Some(format!("{count} host-network pods not counted")),
    }
}

/// A container's network is its pod's.
fn shared_network_notice(subject: &Subject) -> Option<String> {
    subject
        .container
        .map(|_| "Pod network, shared by all containers".to_owned())
}
