//! Kubelet stats read through the API server node proxy (`stats/summary` for network and
//! PVC usage, `metrics/cadvisor` for disk I/O). Only the two fixed paths of `KubeletPath`
//! are ever requested. Success bodies are never traced, only counts and sizes.

use std::future::Future;
use std::pin::pin;
use std::time::Duration;

use futures::io::AsyncBufRead;
use futures::stream::{self, Stream, StreamExt};
use futures::{AsyncBufReadExt, AsyncReadExt};
use serde::Deserialize;
use tokio::sync::watch;
use tokio::time::Instant;

use crate::cadvisor_text::{DiskIoBuilder, DiskIoSample};
use crate::connection::{ClusterConnection, ClusterError, REQUEST_TIMEOUT};
use crate::quantity::ByteAmount;
use crate::resource_metrics::next_delay;
use crate::resource_watch::WatchUpdate;

const NODES_URL: &str = "/api/v1/nodes";
/// Nodes fetched at once in one round.
const KUBELET_CONCURRENCY: usize = 4;
/// An early round never starts sooner than this after the previous round ended.
const MIN_ROUND_GAP: Duration = Duration::from_secs(3);
const CADVISOR_BYTE_LIMIT: u64 = 64 * 1024 * 1024;

const BODY_UNREADABLE: &str = "cAdvisor body could not be read";
const BODY_TOO_LARGE: &str = "cAdvisor metrics exceed 64 MiB";

/// Interface names that are not the node's own uplink (decision 22).
const VIRTUAL_INTERFACE_PREFIXES: [&str; 12] = [
    "veth", "cali", "cni", "flannel", "cilium", "lxc", "docker", "tunl", "vxlan", "kube-", "weave",
    "br-",
];

/// The kubelet paths that may be proxied. A path is never built from anything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KubeletPath {
    StatsSummary,
    MetricsCadvisor,
}

impl KubeletPath {
    fn subresource(self) -> &'static str {
        match self {
            Self::StatsSummary => "proxy/stats/summary",
            Self::MetricsCadvisor => "proxy/metrics/cadvisor",
        }
    }

    fn action(self) -> &'static str {
        match self {
            Self::StatsSummary => "reading kubelet stats",
            Self::MetricsCadvisor => "reading kubelet cAdvisor metrics",
        }
    }
}

/// DNS-1123 subdomain: 1-253 chars; dot-separated labels of 1-63 chars from `[a-z0-9-]`,
/// each starting and ending alphanumeric. Load-bearing: kube does not percent-encode the
/// node name, so `a/../x` or `a#b` would otherwise change the proxied path.
fn is_node_name(name: &str) -> bool {
    (1..=253).contains(&name.len()) && name.split('.').all(is_dns_label)
}

fn is_dns_label(label: &str) -> bool {
    (1..=63).contains(&label.len())
        && label
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !label.starts_with('-')
        && !label.ends_with('-')
}

impl ClusterConnection {
    /// `GET` of one kubelet path as text. `client.request` is not used: on a decode error
    /// it traces the whole body.
    async fn kubelet_text(&self, node: &str, path: KubeletPath) -> Result<String, ClusterError> {
        self.check_node_name(node, path)?;
        let request = kube::core::Request::new(NODES_URL)
            .get_subresource(path.subresource(), node)
            .map_err(|_| self.invalid_node_name(path))?;
        self.run(path.action(), self.client().request_text(request))
            .await
    }

    /// `GET` of one kubelet path as a streamed body; callers pin it.
    async fn kubelet_lines(
        &self,
        node: &str,
        path: KubeletPath,
    ) -> Result<impl AsyncBufRead + use<>, ClusterError> {
        self.check_node_name(node, path)?;
        let request = kube::core::Request::new(NODES_URL)
            .get_subresource(path.subresource(), node)
            .map_err(|_| self.invalid_node_name(path))?;
        self.run(path.action(), self.client().request_stream(request))
            .await
    }

    fn check_node_name(&self, node: &str, path: KubeletPath) -> Result<(), ClusterError> {
        if is_node_name(node) {
            Ok(())
        } else {
            Err(self.invalid_node_name(path))
        }
    }

    // Fixed text: the name is untrusted input and is not echoed.
    fn invalid_node_name(&self, path: KubeletPath) -> ClusterError {
        ClusterError::UnexpectedResponse {
            context: self.context().to_owned(),
            action: path.action(),
            source: "invalid node name".into(),
        }
    }

    /// Polls the newest `targets` value every `METRICS_INTERVAL`: the round reads the
    /// targets when it starts, and a change that adds a node starts an early round (never
    /// sooner than 3 s after the previous one). Empty targets idle without requests, and a
    /// closed sender ends the stream. Snapshot items are ordered by node. The poll fails
    /// (`WatchUpdate::Failed`) only when every node's summary failed.
    pub fn poll_kubelet_stats(
        &self,
        targets: watch::Receiver<KubeletTargets>,
    ) -> impl Stream<Item = WatchUpdate<NodeKubeletStats>> + Send + 'static {
        let connection = self.clone();
        poll_targets(targets, move |targets| {
            kubelet_round(connection.clone(), targets)
        })
    }
}

/// The nodes one round reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KubeletTargets {
    pub summary_nodes: Vec<String>,
    /// Nodes that also get a cAdvisor read; only those in `summary_nodes` are used.
    pub disk_io_nodes: Vec<String>,
}

#[derive(Debug)]
pub struct NodeKubeletStats {
    pub node: String,
    pub summary: Result<KubeletSummary, ClusterError>,
    /// `None` when the node is not a disk I/O target this round.
    pub disk_io: Option<Result<DiskIoSample, ClusterError>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KubeletSummary {
    pub network: Option<NetworkCounters>,
    pub pods: Vec<PodKubeletStats>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PodKubeletStats {
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub network: Option<NetworkCounters>,
    /// Only volumes backed by a PersistentVolumeClaim.
    pub volumes: Vec<PvcUsage>,
}

/// Cumulative network bytes at the kubelet's sample time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkCounters {
    pub sampled_at: Option<jiff::Timestamp>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PvcUsage {
    pub namespace: String,
    pub claim: String,
    pub sampled_at: Option<jiff::Timestamp>,
    pub used: Option<ByteAmount>,
    pub capacity: Option<ByteAmount>,
    pub available: Option<ByteAmount>,
    pub inodes_used: Option<u64>,
    pub inodes: Option<u64>,
}

// Serde skips unknown fields, so only the fields listed here are allocated. Lists are
// `Option` because the kubelet sends `null` for an empty one.

#[derive(Deserialize)]
struct SummaryDoc {
    node: Option<NodeDoc>,
    pods: Option<Vec<PodDoc>>,
}

#[derive(Deserialize)]
struct NodeDoc {
    network: Option<NetworkDoc>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PodDoc {
    pod_ref: Option<PodRefDoc>,
    network: Option<NetworkDoc>,
    volume: Option<Vec<VolumeDoc>>,
}

#[derive(Deserialize)]
struct PodRefDoc {
    name: Option<String>,
    namespace: Option<String>,
    uid: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NetworkDoc {
    time: Option<String>,
    rx_bytes: Option<u64>,
    tx_bytes: Option<u64>,
    interfaces: Option<Vec<InterfaceDoc>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InterfaceDoc {
    name: Option<String>,
    rx_bytes: Option<u64>,
    tx_bytes: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VolumeDoc {
    time: Option<String>,
    pvc_ref: Option<PvcRefDoc>,
    used_bytes: Option<u64>,
    capacity_bytes: Option<u64>,
    available_bytes: Option<u64>,
    inodes_used: Option<u64>,
    inodes: Option<u64>,
}

#[derive(Deserialize)]
struct PvcRefDoc {
    name: Option<String>,
    namespace: Option<String>,
}

/// Which interfaces a sum over `interfaces` keeps.
#[derive(Clone, Copy)]
enum InterfaceRule {
    /// Everything but loopback.
    Pod,
    /// Also drops the virtual interfaces of the container network.
    Node,
}

impl InterfaceRule {
    fn keeps(self, name: &str) -> bool {
        if name == "lo" {
            return false;
        }
        match self {
            Self::Pod => true,
            Self::Node => !VIRTUAL_INTERFACE_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix)),
        }
    }
}

fn kubelet_summary(doc: SummaryDoc) -> KubeletSummary {
    KubeletSummary {
        network: doc
            .node
            .and_then(|node| node.network)
            .and_then(|network| network_counters(&network, InterfaceRule::Node)),
        pods: doc
            .pods
            .unwrap_or_default()
            .into_iter()
            .filter_map(pod_stats)
            .collect(),
    }
}

/// `None` unless `podRef` has a namespace, a name, and a uid.
fn pod_stats(doc: PodDoc) -> Option<PodKubeletStats> {
    let PodRefDoc {
        name: Some(name),
        namespace: Some(namespace),
        uid: Some(uid),
    } = doc.pod_ref?
    else {
        return None;
    };
    Some(PodKubeletStats {
        network: doc
            .network
            .and_then(|network| network_counters(&network, InterfaceRule::Pod)),
        volumes: doc
            .volume
            .unwrap_or_default()
            .into_iter()
            .filter_map(pvc_usage)
            .collect(),
        namespace,
        name,
        uid,
    })
}

/// `None` for a volume without a claim reference.
fn pvc_usage(doc: VolumeDoc) -> Option<PvcUsage> {
    let PvcRefDoc {
        name: Some(claim),
        namespace: Some(namespace),
    } = doc.pvc_ref?
    else {
        return None;
    };
    Some(PvcUsage {
        namespace,
        claim,
        sampled_at: sample_time(doc.time.as_deref()),
        used: doc.used_bytes.map(ByteAmount::from_bytes),
        capacity: doc.capacity_bytes.map(ByteAmount::from_bytes),
        available: doc.available_bytes.map(ByteAmount::from_bytes),
        inodes_used: doc.inodes_used,
        inodes: doc.inodes,
    })
}

/// The top-level counters when both are present, else the sum over the interfaces the
/// `rule` keeps; `None` without any.
fn network_counters(doc: &NetworkDoc, rule: InterfaceRule) -> Option<NetworkCounters> {
    let sampled_at = sample_time(doc.time.as_deref());
    if let (Some(rx_bytes), Some(tx_bytes)) = (doc.rx_bytes, doc.tx_bytes) {
        return Some(NetworkCounters {
            sampled_at,
            rx_bytes,
            tx_bytes,
        });
    }
    let mut counted = doc
        .interfaces
        .iter()
        .flatten()
        .filter(|interface| {
            interface
                .name
                .as_deref()
                .is_some_and(|name| rule.keeps(name))
        })
        .filter_map(|interface| Some((interface.rx_bytes?, interface.tx_bytes?)))
        .peekable();
    counted.peek()?;
    let (rx_bytes, tx_bytes) = counted.fold((0u64, 0u64), |(rx, tx), (more_rx, more_tx)| {
        (rx.saturating_add(more_rx), tx.saturating_add(more_tx))
    });
    Some(NetworkCounters {
        sampled_at,
        rx_bytes,
        tx_bytes,
    })
}

/// A bad time only drops the time; the counters stay usable.
fn sample_time(text: Option<&str>) -> Option<jiff::Timestamp> {
    text?.parse().ok()
}

struct PollState<Fetch> {
    targets: watch::Receiver<KubeletTargets>,
    fetch: Fetch,
    failures: u32,
    /// The targets of the previous round.
    last: KubeletTargets,
    /// When the previous round ended; `None` before the first round.
    last_round_end: Option<Instant>,
}

/// The testable core of `poll_kubelet_stats`: `fetch` runs one round for the targets read at
/// its start. Spawns nothing.
fn poll_targets<Fetch, Fut>(
    targets: watch::Receiver<KubeletTargets>,
    fetch: Fetch,
) -> impl Stream<Item = WatchUpdate<NodeKubeletStats>> + Send + 'static
where
    Fetch: FnMut(KubeletTargets) -> Fut + Send + 'static,
    Fut: Future<Output = Result<Vec<NodeKubeletStats>, ClusterError>> + Send + 'static,
{
    let state = PollState {
        targets,
        fetch,
        failures: 0,
        last: KubeletTargets::default(),
        last_round_end: None,
    };
    stream::unfold(state, next_round)
}

/// `None` ends the stream: the sender is gone.
async fn next_round<Fetch, Fut>(
    mut state: PollState<Fetch>,
) -> Option<(WatchUpdate<NodeKubeletStats>, PollState<Fetch>)>
where
    Fetch: FnMut(KubeletTargets) -> Fut,
    Fut: Future<Output = Result<Vec<NodeKubeletStats>, ClusterError>>,
{
    if let Some(ended) = state.last_round_end
        && !wait_for_round(&mut state, ended).await
    {
        return None;
    }
    let current = loop {
        let current = state.targets.borrow_and_update().clone();
        if !current.summary_nodes.is_empty() {
            break current;
        }
        state.targets.changed().await.ok()?;
    };
    state.last = current.clone();
    let update = match (state.fetch)(current).await {
        Ok(items) => {
            state.failures = 0;
            WatchUpdate::Snapshot(items)
        }
        Err(error) => {
            state.failures = state.failures.saturating_add(1);
            WatchUpdate::Failed(error)
        }
    };
    state.last_round_end = Some(Instant::now());
    Some((update, state))
}

/// Waits until the next round is due: the regular delay, or earlier when targets that add a
/// node arrive. Returns `false` when the sender is gone.
async fn wait_for_round<Fetch>(state: &mut PollState<Fetch>, ended: Instant) -> bool {
    let mut due = ended + next_delay(state.failures);
    loop {
        tokio::select! {
            () = tokio::time::sleep_until(due) => return true,
            changed = state.targets.changed() => {
                if changed.is_err() {
                    return false;
                }
                let adds_node = adds_node(&state.last, &state.targets.borrow_and_update());
                if adds_node {
                    due = due.min(ended + MIN_ROUND_GAP);
                }
            }
        }
    }
}

/// A shrink keeps waiting: the next round reads it.
fn adds_node(last: &KubeletTargets, current: &KubeletTargets) -> bool {
    let is_new =
        |nodes: &[String], known: &[String]| nodes.iter().any(|node| !known.contains(node));
    is_new(&current.summary_nodes, &last.summary_nodes)
        || is_new(&current.disk_io_nodes, &last.disk_io_nodes)
}

/// One request set per distinct summary node, in name order; `true` also reads cAdvisor. A disk
/// node outside the summary nodes is ignored.
fn round_jobs(mut targets: KubeletTargets) -> Vec<(String, bool)> {
    targets.summary_nodes.sort();
    targets.summary_nodes.dedup();
    targets
        .summary_nodes
        .into_iter()
        .map(|node| {
            let has_disk_io = targets.disk_io_nodes.contains(&node);
            (node, has_disk_io)
        })
        .collect()
}

async fn kubelet_round(
    connection: ClusterConnection,
    targets: KubeletTargets,
) -> Result<Vec<NodeKubeletStats>, ClusterError> {
    let stats = stream::iter(round_jobs(targets))
        .map(|(node, has_disk_io)| {
            let connection = connection.clone();
            async move { node_stats(&connection, &node, has_disk_io).await }
        })
        .buffer_unordered(KUBELET_CONCURRENCY)
        .collect()
        .await;
    round_result(stats)
}

/// Sorts by node. A round fails only when every node's summary failed, with the first
/// node's error, so one NotReady node cannot stall the others.
fn round_result(mut stats: Vec<NodeKubeletStats>) -> Result<Vec<NodeKubeletStats>, ClusterError> {
    stats.sort_by(|left, right| left.node.cmp(&right.node));
    if stats.iter().any(|node| node.summary.is_ok()) {
        return Ok(stats);
    }
    match stats.into_iter().next() {
        Some(NodeKubeletStats {
            summary: Err(error),
            ..
        }) => Err(error),
        _ => Ok(Vec::new()),
    }
}

async fn node_stats(
    connection: &ClusterConnection,
    node: &str,
    has_disk_io: bool,
) -> NodeKubeletStats {
    let disk_io = async {
        if has_disk_io {
            Some(read_disk_io(connection, node).await)
        } else {
            None
        }
    };
    let (summary, disk_io) = futures::join!(read_summary(connection, node), disk_io);
    NodeKubeletStats {
        node: node.to_owned(),
        summary,
        disk_io,
    }
}

async fn read_summary(
    connection: &ClusterConnection,
    node: &str,
) -> Result<KubeletSummary, ClusterError> {
    let path = KubeletPath::StatsSummary;
    let text = connection.kubelet_text(node, path).await?;
    let bytes = text.len();
    // Serde text can quote the body, so it is replaced by a fixed one.
    let doc = serde_json::from_str::<SummaryDoc>(&text).map_err(|_| {
        ClusterError::UnexpectedResponse {
            context: connection.context().to_owned(),
            action: path.action(),
            source: "kubelet summary could not be decoded".into(),
        }
    })?;
    drop(text);
    let summary = kubelet_summary(doc);
    let pvcs: usize = summary.pods.iter().map(|pod| pod.volumes.len()).sum();
    tracing::debug!(
        node,
        pods = summary.pods.len(),
        pvcs,
        bytes,
        "read kubelet summary"
    );
    Ok(summary)
}

async fn read_disk_io(
    connection: &ClusterConnection,
    node: &str,
) -> Result<DiskIoSample, ClusterError> {
    let path = KubeletPath::MetricsCadvisor;
    // One deadline for the head and the body, so a slow kubelet costs at most one timeout.
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    let reader = connection.kubelet_lines(node, path).await?;
    let read = parse_disk_io(reader, CADVISOR_BYTE_LIMIT);
    let (sample, bytes) = match tokio::time::timeout_at(deadline, read).await {
        Ok(Ok(parsed)) => parsed,
        Ok(Err(source)) => {
            return Err(ClusterError::UnexpectedResponse {
                context: connection.context().to_owned(),
                action: path.action(),
                source: source.into(),
            });
        }
        Err(_elapsed) => {
            return Err(ClusterError::TimedOut {
                context: connection.context().to_owned(),
                action: path.action(),
            });
        }
    };
    tracing::debug!(
        node,
        containers = sample.containers.len(),
        bytes,
        "read cAdvisor metrics"
    );
    Ok(sample)
}

/// Reads at most `limit` bytes of `reader` line by line. The error is a fixed text: io and
/// UTF-8 errors are not kept.
async fn parse_disk_io(
    reader: impl AsyncBufRead,
    limit: u64,
) -> Result<(DiskIoSample, u64), &'static str> {
    let reader = pin!(reader);
    // One byte over the limit tells "too large" from "exactly the limit"; the cap also
    // bounds a single line without a newline.
    let mut lines = reader.take(limit.saturating_add(1)).lines();
    let mut builder = DiskIoBuilder::default();
    let mut bytes = 0u64;
    while let Some(line) = lines.next().await {
        let line = line.map_err(|_| BODY_UNREADABLE)?;
        bytes = bytes.saturating_add(line.len() as u64 + 1);
        if bytes > limit {
            return Err(BODY_TOO_LARGE);
        }
        builder.push_line(&line);
    }
    Ok((builder.finish(), bytes))
}

#[cfg(test)]
#[path = "kubelet_stats_tests.rs"]
mod kubelet_stats_tests;
