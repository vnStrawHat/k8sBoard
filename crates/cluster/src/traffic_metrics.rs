//! Traffic rates from a Prometheus-compatible source (spec 0049): which traffic metrics a source
//! holds, and the per-edge rates of one namespace over the last five minutes. The queries come
//! from `promql.rs`; label values are untrusted text and are cleaned before they leave this module.

use std::collections::{BTreeMap, BTreeSet};

use crate::connection::ClusterConnection;
use crate::metrics_query::{InstantRow, MetricsError, is_invisible_format};
use crate::metrics_source::MetricsSource;
use crate::promql::{QueryError, traffic_queries};

/// Most series one traffic query may contribute; more are cut and noted.
const TRAFFIC_SERIES_LIMIT: usize = 2_000;
/// Longest label value that reaches a tooltip or a label.
const LABEL_LIMIT: usize = 80;
const ISTIO_METRIC: &str = "istio_requests_total";
const POD_NETWORK_METRIC: &str = "container_network_receive_bytes_total";
/// Istio's name for a caller it could not identify.
const UNKNOWN_PEER: &str = "unknown";

/// The kinds of traffic metric a source can hold; the order is the priority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TrafficSourceKind {
    Istio,
    PodNetwork,
}

impl TrafficSourceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Istio => "Istio",
            Self::PodNetwork => "pod network bytes",
        }
    }

    fn metric(self) -> &'static str {
        match self {
            Self::Istio => ISTIO_METRIC,
            Self::PodNetwork => POD_NETWORK_METRIC,
        }
    }
}

/// A traffic metric that exists in the source; built only by `detect`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrafficMetricSource {
    kind: TrafficSourceKind,
}

impl TrafficMetricSource {
    /// The kinds whose metric is in `names`, in priority order.
    pub fn detect(names: &BTreeSet<String>) -> Vec<Self> {
        [TrafficSourceKind::Istio, TrafficSourceKind::PodNetwork]
            .into_iter()
            .filter(|kind| names.contains(kind.metric()))
            .map(|kind| Self { kind })
            .collect()
    }

    pub fn kind(self) -> TrafficSourceKind {
        self.kind
    }
}

/// One end of a flow.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TrafficEnd {
    Workload(String),
    Service(String),
    Pod(String),
    /// Another namespace or an unknown peer: `ns/name` or the raw label.
    Outside(String),
}

/// One flow of a reading. Istio fills `requests` and `errors` (per second); the pod network fills
/// `receive` and `transmit` (bytes per second).
#[derive(Clone, Debug, PartialEq)]
pub struct TrafficRate {
    pub from: Option<TrafficEnd>,
    pub to: TrafficEnd,
    pub requests: Option<f64>,
    pub errors: Option<f64>,
    pub receive: Option<f64>,
    pub transmit: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrafficReading {
    pub rates: Vec<TrafficRate>,
    /// A query held more than 2,000 series; the first 2,000 by label order were kept.
    pub was_cut: bool,
}

/// Series keyed by their cleaned `by` labels, in label order.
type KeyedValues = BTreeMap<Vec<String>, f64>;

impl ClusterConnection {
    /// The two queries of one source for `namespace`, joined on their labels. They run
    /// concurrently; the reading fails only when the first fails, and a failed second query leaves
    /// its fields `None`.
    pub async fn traffic_rates(
        &self,
        source: &MetricsSource,
        traffic: TrafficMetricSource,
        namespace: &str,
    ) -> Result<TrafficReading, MetricsError> {
        let queries = traffic_queries(traffic.kind, namespace).map_err(|error| match error {
            QueryError::InvalidName => MetricsError::InvalidName,
            QueryError::NodeDisk => MetricsError::Unsupported,
        })?;
        let (first, second) = futures::join!(
            self.instant_query(source, &queries.first),
            self.instant_query(source, &queries.second)
        );
        let (first, first_cut) = keyed(first?, traffic.kind);
        let second = second.ok().map(|rows| keyed(rows, traffic.kind));
        let was_cut = first_cut || second.as_ref().is_some_and(|(_, cut)| *cut);
        let second = second.map(|(values, _)| values);
        let rates = match traffic.kind {
            TrafficSourceKind::Istio => istio_rates(&first, second.as_ref(), namespace),
            TrafficSourceKind::PodNetwork => pod_network_rates(&first, second.as_ref()),
        };
        Ok(TrafficReading { rates, was_cut })
    }
}

/// The labels one kind groups by, in key order.
fn group_labels(kind: TrafficSourceKind) -> &'static [&'static str] {
    match kind {
        TrafficSourceKind::Istio => &[
            "source_workload",
            "source_workload_namespace",
            "destination_service_name",
        ],
        TrafficSourceKind::PodNetwork => &["pod"],
    }
}

/// Keys the rows by their cleaned labels (rows that clean to one key add up) and keeps the first
/// `TRAFFIC_SERIES_LIMIT` keys in label order.
fn keyed(rows: Vec<InstantRow>, kind: TrafficSourceKind) -> (KeyedValues, bool) {
    let labels = group_labels(kind);
    let mut values = KeyedValues::new();
    for row in rows {
        let key: Vec<String> = labels
            .iter()
            .map(|label| clean_label(row.labels.get(*label).map_or("", String::as_str)))
            .collect();
        *values.entry(key).or_insert(0.0) += row.value;
    }
    let was_cut = values.len() > TRAFFIC_SERIES_LIMIT;
    if was_cut {
        values = values.into_iter().take(TRAFFIC_SERIES_LIMIT).collect();
    }
    (values, was_cut)
}

/// Control and invisible direction characters are dropped, then the value is cut: backend label
/// values are untrusted text.
fn clean_label(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control() && !is_invisible_format(*ch))
        .take(LABEL_LIMIT)
        .collect()
}

fn istio_rates(
    totals: &KeyedValues,
    errors: Option<&KeyedValues>,
    namespace: &str,
) -> Vec<TrafficRate> {
    totals
        .iter()
        .filter_map(|(key, requests)| {
            let [workload, workload_namespace, service] = key.as_slice() else {
                return None;
            };
            if service.is_empty() {
                return None;
            }
            let from = if workload.is_empty() || workload == UNKNOWN_PEER {
                TrafficEnd::Outside(UNKNOWN_PEER.to_owned())
            } else if workload_namespace == namespace {
                TrafficEnd::Workload(workload.clone())
            } else {
                TrafficEnd::Outside(clean_label(&format!("{workload_namespace}/{workload}")))
            };
            Some(TrafficRate {
                from: Some(from),
                to: TrafficEnd::Service(service.clone()),
                requests: Some(*requests),
                errors: errors.map(|errors| errors.get(key).copied().unwrap_or(0.0)),
                receive: None,
                transmit: None,
            })
        })
        .collect()
}

fn pod_network_rates(receive: &KeyedValues, transmit: Option<&KeyedValues>) -> Vec<TrafficRate> {
    let pods: BTreeSet<&Vec<String>> = receive
        .keys()
        .chain(transmit.into_iter().flat_map(BTreeMap::keys))
        .collect();
    pods.into_iter()
        .filter_map(|key| {
            let [pod] = key.as_slice() else {
                return None;
            };
            if pod.is_empty() {
                return None;
            }
            Some(TrafficRate {
                from: None,
                to: TrafficEnd::Pod(pod.clone()),
                requests: None,
                errors: None,
                receive: receive.get(key).copied(),
                transmit: transmit.and_then(|transmit| transmit.get(key)).copied(),
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "traffic_metrics_tests.rs"]
mod traffic_metrics_tests;
