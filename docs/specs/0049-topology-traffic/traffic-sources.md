# 0049 · Traffic sources (step 1)

[Back to index](README.md) · Modules (cluster crate): `traffic_metrics.rs` (new) + `traffic_metrics_tests.rs`, `metrics_query.rs` (`MetricNames`, instant-vector decoder), `promql.rs` (templates), `lib.rs`, `examples/probe.rs`. Transport, caps, decoding, and errors are 0048's ([source-and-transport.md](../0048-prometheus-metrics/source-and-transport.md)).

## Names list (moved from 0048)

```rust
// metrics_query.rs: the private enum gains `MetricNames` ("label/__name__/values"); still one exception
impl ClusterConnection {
    /// Metric names seen in the last hour: `start` = now − 1h, `end` = now.
    pub async fn metric_names(&self, source: &MetricsSource) -> Result<BTreeSet<String>, MetricsError>;
}
```

## API

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]   // Ord = priority
pub enum TrafficSourceKind { Istio, PodNetwork }
impl TrafficSourceKind { pub fn label(self) -> &'static str; }   // "Istio", "pod network bytes"
/// A row whose metric exists in the source; built only by `detect`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrafficSource { kind: TrafficSourceKind }
impl TrafficSource {
    pub fn detect(names: &BTreeSet<String>) -> Vec<TrafficSource>;   // in priority order
    pub fn kind(self) -> TrafficSourceKind;
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TrafficEnd { Workload(String), Service(String), Pod(String),
    /// Another namespace or an unknown peer: `ns/name` or the raw label.
    Outside(String) }
pub struct TrafficRate { pub from: Option<TrafficEnd>, pub to: TrafficEnd,
    pub requests: Option<f64>, pub errors: Option<f64>,       // per second (Istio)
    pub receive: Option<f64>, pub transmit: Option<f64> }      // bytes per second (PodNetwork)
pub struct TrafficReading { pub rates: Vec<TrafficRate>, pub was_cut: bool }
impl ClusterConnection {
    /// The two queries of one source for `namespace`, joined on their labels.
    pub async fn traffic_rates(&self, source: &MetricsSource, traffic: TrafficSource, namespace: &str)
        -> Result<TrafficReading, MetricsError>;
}
```

- `namespace` passes the DNS-label check first (`InvalidName`, no request).
- The two queries run concurrently; the reading fails only when the first query fails. A failed errors query leaves `errors: None` (no tone).
- Every label value is cut to 80 chars with control characters stripped before it enters a `TrafficEnd` (decision 12).

## Source table (first cut)

`NS` = `string_literal(namespace)`; `W` = `[300s]`. Both queries are `sum by (…) (rate(M{…}[W]))`.

| Kind | Metric `M` (detect) | `by` labels | First query matchers | Second query | Mapping |
|---|---|---|---|---|---|
| Istio | `istio_requests_total` | `source_workload`, `source_workload_namespace`, `destination_service_name` | `reporter="destination",destination_service_namespace=NS` | same + `response_code=~"5.."` (errors) | from `Workload(source_workload)` when its namespace is NS; `Outside(ns/name)` otherwise; `Outside("unknown")` for `unknown`; to `Service(destination_service_name)` |
| PodNetwork | `container_network_receive_bytes_total` | `pod` | `namespace=NS,interface!="lo"` | `container_network_transmit_bytes_total`, same matchers | to `Pod(pod)`, `receive` / `transmit` |

- Rows are joined on their `by` labels; a total of 0 is kept (the edge exists, idle). `NaN`/`Inf` values drop the row.
- `detect` returns Istio first when present; PodNetwork is offered whenever its metric exists, alongside Istio.
- If the 0048 live check found pod network counted twice, PodNetwork uses the same sandbox filter as the 0048 Monitor query ([promql.md](../0048-prometheus-metrics/promql.md) facts).

## Later rows (non-goal now; one row each)

| Kind | Metric | Ends |
|---|---|---|
| OpenTelemetry service graph | `traces_service_graph_request_total` (+ `_failed_total`) | `client` → `server` (names resolved to workload or Service) |
| ingress-nginx | `nginx_ingress_controller_requests` | `ingress` → `service`; namespace label `exported_namespace` or `namespace` |
| Kong | `kong_http_requests_total` | Kong Ingress Controller names `{ns}.{svc}.{port}` / `{ns}.{ingress}.…` |
| OpenTelemetry span metrics | `traces_spanmetrics_calls_total`, `calls_total` | node-level only (`service_name`) |

## Caps

`TRAFFIC_SERIES_LIMIT` = 2,000 per query (more → the first 2,000 by label order, `was_cut`). Body 4 MiB, deadline 20 s, `timeout=15s` (0048). At most 2 queries per source per refresh: ≤ 4 per refresh; UAT 2.

## Probe (live, read-only)

`probe --metrics-source <…> --traffic <namespace>`: loads the names, prints the detected kinds, then one line per kind: rows, `was_cut`, bytes, ms. UAT expectation: `pod network bytes` only.
