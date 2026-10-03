//! What the switcher knows about the clusters it is not connected to: a cache of one `GET
//! /version` per cluster, and the stream that fills it. The active cluster's health comes from
//! its session, never from here.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cluster::{AuthKind, ClusterConnection, ClusterError, Kubeconfig};
use futures::{Stream, StreamExt as _};

use crate::cluster_registry::ClusterRef;
use crate::cluster_session::error_text;

/// How long an answer stays valid; the switcher probes a cluster again after it.
pub(crate) const PROBE_TTL: Duration = Duration::from_secs(60);
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// Bounds the connections one open of the switcher makes at the same time.
pub(crate) const PROBE_CONCURRENCY: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProbeResult {
    Reachable {
        latency: Duration,
    },
    /// The reason is credential-free (`error_text`) and shown as a tooltip only.
    Unreachable {
        reason: String,
    },
}

struct ProbeEntry {
    result: ProbeResult,
    at: Instant,
}

/// The line a row shows under its label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowHealth {
    Live(Duration),
    Connecting,
    Interrupted,
    Reachable(Duration),
    Unreachable,
    Checking,
    NotChecked,
}

/// What `HealthBoard::due` needs to know about a row.
pub(crate) struct ProbeCandidate {
    pub(crate) cluster: ClusterRef,
    pub(crate) auth: AuthKind,
    pub(crate) is_active: bool,
}

/// One cluster to probe: the loaded kubeconfig and the context inside it.
pub(crate) struct ProbeTarget {
    pub(crate) cluster: ClusterRef,
    pub(crate) kubeconfig: Arc<Kubeconfig>,
    pub(crate) context: String,
}

#[derive(Default)]
pub(crate) struct HealthBoard {
    entries: HashMap<ClusterRef, ProbeEntry>,
    running: HashSet<ClusterRef>,
}

/// Exec plugins and auth providers may open a browser login or an MFA prompt, so they are probed
/// only when the user asks.
pub(crate) fn is_probed_automatically(auth: &AuthKind) -> bool {
    match auth {
        AuthKind::Exec { .. } | AuthKind::AuthProvider { .. } => false,
        AuthKind::Token
        | AuthKind::TokenFile
        | AuthKind::ClientCertificate
        | AuthKind::Basic
        | AuthKind::None => true,
    }
}

impl HealthBoard {
    /// The clusters to probe when the switcher opens: in-process auth, not the active cluster,
    /// not already probing, and no answer younger than `PROBE_TTL`.
    pub(crate) fn due(&self, rows: &[ProbeCandidate], now: Instant) -> Vec<ClusterRef> {
        rows.iter()
            .filter(|row| {
                !row.is_active
                    && is_probed_automatically(&row.auth)
                    && !self.is_running(&row.cluster)
                    && !self.is_fresh(&row.cluster, now)
            })
            .map(|row| row.cluster.clone())
            .collect()
    }

    fn is_fresh(&self, cluster: &ClusterRef, now: Instant) -> bool {
        self.entries
            .get(cluster)
            .is_some_and(|entry| now.saturating_duration_since(entry.at) < PROBE_TTL)
    }

    /// Whether any probe still runs; a `--screen switcher` screenshot waits for it.
    #[cfg(any(feature = "screenshot", test))]
    pub(crate) fn is_probing(&self) -> bool {
        !self.running.is_empty()
    }

    pub(crate) fn mark_running(&mut self, clusters: &[ClusterRef]) {
        self.running.extend(clusters.iter().cloned());
    }

    pub(crate) fn is_running(&self, cluster: &ClusterRef) -> bool {
        self.running.contains(cluster)
    }

    pub(crate) fn record(&mut self, cluster: ClusterRef, result: ProbeResult, now: Instant) {
        self.running.remove(&cluster);
        self.entries.insert(cluster, ProbeEntry { result, at: now });
    }

    /// Called when the switcher closes: the probes were aborted, so those rows are due again.
    pub(crate) fn clear_running(&mut self) {
        self.running.clear();
    }

    /// The health of a row that is not the active cluster.
    pub(crate) fn row_health(&self, cluster: &ClusterRef) -> RowHealth {
        if self.is_running(cluster) {
            return RowHealth::Checking;
        }
        match self.entries.get(cluster).map(|entry| &entry.result) {
            Some(ProbeResult::Reachable { latency }) => RowHealth::Reachable(*latency),
            Some(ProbeResult::Unreachable { .. }) => RowHealth::Unreachable,
            None => RowHealth::NotChecked,
        }
    }

    /// Why the last probe failed, for the row tooltip.
    pub(crate) fn failure_reason(&self, cluster: &ClusterRef) -> Option<&str> {
        match &self.entries.get(cluster)?.result {
            ProbeResult::Unreachable { reason } => Some(reason),
            ProbeResult::Reachable { .. } => None,
        }
    }
}

/// Runs on tokio and yields one result per target as it finishes. At most `PROBE_CONCURRENCY`
/// targets are in flight; dropping the stream aborts the rest.
pub(crate) fn probe_stream(
    targets: Vec<ProbeTarget>,
) -> impl Stream<Item = (ClusterRef, ProbeResult)> + Send {
    probe_all(targets, probe_one)
}

fn probe_all<Probe, Pending>(
    targets: Vec<ProbeTarget>,
    probe: Probe,
) -> impl Stream<Item = (ClusterRef, ProbeResult)> + Send
where
    Probe: Fn(ProbeTarget) -> Pending + Send,
    Pending: Future<Output = ProbeResult> + Send,
{
    futures::stream::iter(targets)
        .map(move |target| {
            let cluster = target.cluster.clone();
            let pending = probe(target);
            async move { (cluster, pending.await) }
        })
        .buffer_unordered(PROBE_CONCURRENCY)
}

/// One `GET /version`: opens the client and reads the version, and nothing else. The latency
/// times the version request only, as the status bar does.
async fn probe_one(target: ProbeTarget) -> ProbeResult {
    let attempt = async {
        let connection = ClusterConnection::open(&target.kubeconfig, &target.context).await?;
        let started = Instant::now();
        connection.server_version().await?;
        Ok(started.elapsed())
    };
    bounded(PROBE_TIMEOUT, attempt).await
}

async fn bounded(
    limit: Duration,
    attempt: impl Future<Output = Result<Duration, ClusterError>>,
) -> ProbeResult {
    match tokio::time::timeout(limit, attempt).await {
        Ok(Ok(latency)) => ProbeResult::Reachable { latency },
        Ok(Err(error)) => ProbeResult::Unreachable {
            reason: error_text(&error),
        },
        Err(_elapsed) => ProbeResult::Unreachable {
            reason: format!("no answer in {} s", limit.as_secs()),
        },
    }
}

#[cfg(test)]
#[path = "cluster_health_tests.rs"]
mod cluster_health_tests;
