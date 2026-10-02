//! Pod and node usage from `metrics.k8s.io/v1beta1` (metrics-server), polled with plain
//! list requests. Only counts are traced, never names or values.

use std::future::Future;
use std::str::FromStr;
use std::time::Duration;

use futures::Stream;
use futures::stream;
use kube::Api;
use kube::api::{ApiResource, DynamicObject};
use serde_json::Value;

use crate::connection::{ClusterConnection, ClusterError};
use crate::metrics_api::METRICS_GROUP;
use crate::namespace::NamespaceScope;
use crate::quantity::{ByteAmount, CpuAmount};
use crate::resource_watch::WatchUpdate;

/// metrics-server scrapes kubelets every 15 s by default, so a faster poll sees nothing new.
pub const METRICS_INTERVAL: Duration = Duration::from_secs(15);

const POD_ACTION: &str = "listing pod metrics";
const NODE_ACTION: &str = "listing node metrics";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResourceUsage {
    pub cpu: CpuAmount,
    pub memory: ByteAmount,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PodMetrics {
    pub namespace: String,
    pub name: String,
    /// The server's scrape time (`timestamp`).
    pub sampled_at: Option<jiff::Timestamp>,
    /// In API order; at least one.
    pub containers: Vec<ContainerMetrics>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerMetrics {
    pub name: String,
    pub usage: ResourceUsage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeMetrics {
    pub name: String,
    pub sampled_at: Option<jiff::Timestamp>,
    pub usage: ResourceUsage,
}

impl ClusterConnection {
    /// Polls PodMetrics in `scope` every `METRICS_INTERVAL`, the first poll at once; snapshots
    /// ordered by (namespace, name). A new pod is absent until metrics-server has scraped it
    /// twice. Never ends; dropping the stream stops it.
    pub fn poll_pod_metrics(
        &self,
        scope: NamespaceScope,
    ) -> impl Stream<Item = WatchUpdate<PodMetrics>> + Send + 'static {
        let connection = self.clone();
        let apis = self.scoped_dynamic_apis(&scope, &metrics_resource("PodMetrics", "pods"));
        poll_updates(move || {
            let connection = connection.clone();
            let apis = apis.clone();
            async move { list_pod_metrics(&connection, apis).await }
        })
    }

    /// Same for NodeMetrics, ordered by name.
    pub fn poll_node_metrics(
        &self,
    ) -> impl Stream<Item = WatchUpdate<NodeMetrics>> + Send + 'static {
        let connection = self.clone();
        let api: Api<DynamicObject> = Api::all_with(
            self.client().clone(),
            &metrics_resource("NodeMetrics", "nodes"),
        );
        poll_updates(move || {
            let connection = connection.clone();
            let api = api.clone();
            async move { list_node_metrics(&connection, api).await }
        })
    }
}

fn metrics_resource(kind: &str, plural: &str) -> ApiResource {
    ApiResource {
        group: METRICS_GROUP.to_owned(),
        version: "v1beta1".to_owned(),
        api_version: format!("{METRICS_GROUP}/v1beta1"),
        kind: kind.to_owned(),
        plural: plural.to_owned(),
    }
}

async fn list_pod_metrics(
    connection: &ClusterConnection,
    apis: Vec<(Option<String>, Api<DynamicObject>)>,
) -> Result<Vec<PodMetrics>, ClusterError> {
    let namespace_count = apis.len();
    let mut results = Vec::with_capacity(namespace_count);
    for (namespace, api) in apis {
        let listed = connection
            .list_all(api, POD_ACTION)
            .await
            .map(|objects| objects.iter().filter_map(pod_metrics).collect::<Vec<_>>());
        let has_failed = listed.is_err();
        results.push((namespace, listed));
        // The first failure decides the outcome, so the other namespaces are not asked.
        if has_failed {
            break;
        }
    }
    let mut pods = concat_namespaces(results, namespace_count)?;
    pods.sort_by(|left, right| (&left.namespace, &left.name).cmp(&(&right.namespace, &right.name)));
    let containers: usize = pods.iter().map(|pod| pod.containers.len()).sum();
    tracing::debug!(pods = pods.len(), containers, "listed pod metrics");
    Ok(pods)
}

async fn list_node_metrics(
    connection: &ClusterConnection,
    api: Api<DynamicObject>,
) -> Result<Vec<NodeMetrics>, ClusterError> {
    let objects = connection.list_all(api, NODE_ACTION).await?;
    let mut nodes: Vec<NodeMetrics> = objects.iter().filter_map(node_metrics).collect();
    nodes.sort_by(|left, right| left.name.cmp(&right.name));
    tracing::debug!(nodes = nodes.len(), "listed node metrics");
    Ok(nodes)
}

/// `None` when the object has no namespace, no name, or no readable container.
fn pod_metrics(object: &DynamicObject) -> Option<PodMetrics> {
    let namespace = object.metadata.namespace.clone()?;
    let name = object.metadata.name.clone()?;
    let containers: Vec<ContainerMetrics> = object
        .data
        .get("containers")?
        .as_array()?
        .iter()
        .filter_map(container_metrics)
        .collect();
    if containers.is_empty() {
        return None;
    }
    Some(PodMetrics {
        namespace,
        name,
        sampled_at: sampled_at(&object.data),
        containers,
    })
}

fn container_metrics(container: &Value) -> Option<ContainerMetrics> {
    Some(ContainerMetrics {
        name: container.get("name")?.as_str()?.to_owned(),
        usage: resource_usage(container.get("usage")?)?,
    })
}

/// `None` when the object has no name or no readable usage.
fn node_metrics(object: &DynamicObject) -> Option<NodeMetrics> {
    Some(NodeMetrics {
        name: object.metadata.name.clone()?,
        sampled_at: sampled_at(&object.data),
        usage: resource_usage(object.data.get("usage")?)?,
    })
}

fn resource_usage(usage: &Value) -> Option<ResourceUsage> {
    Some(ResourceUsage {
        cpu: CpuAmount::parse(usage.get("cpu")?.as_str()?)?,
        memory: ByteAmount::parse(usage.get("memory")?.as_str()?)?,
    })
}

fn sampled_at(data: &Value) -> Option<jiff::Timestamp> {
    jiff::Timestamp::from_str(data.get("timestamp")?.as_str()?).ok()
}

/// One namespace of a listing with its result; `None` is every namespace.
type NamespaceListing<T> = (Option<String>, Result<Vec<T>, ClusterError>);

/// Concatenates per-namespace results in order, or returns the first error. With several
/// namespaces (`namespace_count`, which may exceed `results` when listing stopped early) the
/// error names the one that failed.
fn concat_namespaces<T>(
    results: Vec<NamespaceListing<T>>,
    namespace_count: usize,
) -> Result<Vec<T>, ClusterError> {
    let is_several = namespace_count > 1;
    let mut merged = Vec::new();
    for (namespace, result) in results {
        match result {
            Ok(items) => merged.extend(items),
            Err(source) => {
                return Err(match namespace {
                    Some(namespace) if is_several => ClusterError::Namespace {
                        namespace,
                        source: Box::new(source),
                    },
                    _ => source,
                });
            }
        }
    }
    Ok(merged)
}

/// The wait before the next poll after `failures` failures in a row.
fn next_delay(failures: u32) -> Duration {
    match failures {
        0 => METRICS_INTERVAL,
        1 => Duration::from_secs(30),
        2 => Duration::from_secs(60),
        _ => Duration::from_secs(120),
    }
}

struct PollState<Fetch> {
    fetch: Fetch,
    failures: u32,
    is_first: bool,
}

/// The testable core: one fetch at once, then one after each delay. The delay starts after
/// the previous fetch ended, so requests never stack. Spawns nothing.
fn poll_updates<T, Fetch, Fut>(fetch: Fetch) -> impl Stream<Item = WatchUpdate<T>> + Send + 'static
where
    Fetch: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = Result<Vec<T>, ClusterError>> + Send + 'static,
    T: Send + 'static,
{
    let state = PollState {
        fetch,
        failures: 0,
        is_first: true,
    };
    stream::unfold(state, |mut state| async move {
        if !state.is_first {
            tokio::time::sleep(next_delay(state.failures)).await;
        }
        state.is_first = false;
        let update = match (state.fetch)().await {
            Ok(items) => {
                state.failures = 0;
                WatchUpdate::Snapshot(items)
            }
            Err(error) => {
                state.failures = state.failures.saturating_add(1);
                WatchUpdate::Failed(error)
            }
        };
        Some((update, state))
    })
}

#[cfg(test)]
#[path = "resource_metrics_tests.rs"]
mod resource_metrics_tests;
