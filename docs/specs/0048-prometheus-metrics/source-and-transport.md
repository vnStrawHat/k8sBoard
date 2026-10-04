# 0048 · Source, detection, transport (steps 1–2)

[Back to index](README.md) · Modules (cluster crate): `metrics_source.rs` (step 1) + `metrics_source_tests.rs`, `metrics_query.rs` (step 2) + `metrics_query_tests.rs`, `service.rs` (`list_all_services`), `lib.rs` exports, `examples/probe.rs`.

## Source (`metrics_source.rs`, pure)

```rust
/// What Settings stores; no credential can be expressed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricsSourceFields { pub namespace: String, pub service: String,
    pub port: String /* number or port name */, pub scheme: MetricsScheme, #[serde(default)] pub prefix: String }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "lowercase")]
pub enum MetricsScheme { #[default] Http, Https }
/// Valid by construction: fields private, `Debug` = `display()`. Derives neither `Deserialize` nor
/// `Default`; settings deserialize into the raw `MetricsSourceFields`, then `new` validates.
#[derive(Clone, PartialEq, Eq)]
pub struct MetricsSource { /* the fields, validated */ }
impl MetricsSource {
    pub fn new(fields: &MetricsSourceFields) -> Result<Self, MetricsSourceError>;
    pub fn fields(&self) -> MetricsSourceFields;
    pub fn display(&self) -> String;            // "monitoring/vmselect-…:8481 /select/0/prometheus"
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MetricsSourceError { Namespace, Service, Port, Prefix }
```

| Field | Rule (error) | Form message |
|---|---|---|
| `namespace` | DNS label (`Namespace`) | `Use a namespace name.` |
| `service` | DNS label, ≤ 63 (`Service`) | `Use a service name.` |
| `port` | `1..=65535`, or an IANA port name: 1–15 of `[a-z0-9-]`, a letter, no leading/trailing or double `-` (`Port`) | `Use a port number or the service's port name.` |
| `prefix` | empty, or a leading `/` followed by `/`-separated non-empty segments of `[A-Za-z0-9._~-]` (no `//`), no `.`/`..` segment, no trailing `/`, ≤ 128 chars (`Prefix`) | `Use a path such as /select/0/prometheus.` |

## Detection (`metrics_candidates`, pure)

```rust
pub enum MetricsFlavor { VictoriaMetricsCluster, VictoriaMetricsQuery, VictoriaMetricsSingle, Prometheus, ThanosQuery, Mimir }
impl MetricsFlavor { pub fn label(self) -> &'static str; }   // "VictoriaMetrics cluster", …
pub struct MetricsCandidate { pub flavor: MetricsFlavor, pub fields: MetricsSourceFields }
/// Sorted by the table's rank, then namespace and name; at most 20.
pub fn metrics_candidates(services: &[ServiceSummary]) -> Vec<MetricsCandidate>;
```

| Rank | Match (`app.kubernetes.io/name`, else `app`, label value exact) | Flavor | Prefix | Port (first that exists) |
|---|---|---|---|---|
| 1 | `vmselect` | VictoriaMetricsCluster | `/select/0/prometheus` | named `http`; 8481 |
| 2 | `vmquery` | VictoriaMetricsQuery | `/select/0/prometheus` | named `http`; 8481 |
| 3 | `vmsingle`, `victoria-metrics-single` | VictoriaMetricsSingle | empty | named `http`; 8428 |
| 4 | `prometheus`; or service name `prometheus-operated`, `prometheus-server`, `prometheus-k8s` | Prometheus | empty | named `web`, `http-web`, `http`; 9090 |
| 5 | `thanos-query`, `thanos-querier` | ThanosQuery | empty | named `http`; 10902; 9090 |
| 6 | `mimir` with `app.kubernetes.io/component` = `query-frontend` | Mimir | `/prometheus` | named `http-metrics`, `http`; 8080 |

- A service with none of its candidate ports is skipped. `port` is stored as the **number** (the proxy accepts both; a number survives a port rename). Scheme `https` only when the chosen port's name starts with `https`.
- UAT result: rank 1 `monitoring/vmselect-vm-victoria-metrics-k8s-stack:8481`, rank 2 `vmquery-…:8481`.

`service.rs`: `pub async fn list_all_services(&self) -> Result<Vec<ServiceSummary>, ClusterError>` — one `list_all` over `Api::all` (action `listing services`), no watch.

## Request (`metrics_query.rs`)

```rust
enum MetricsEndpoint { Query, QueryRange }   // private; "query", "query_range" (0049 adds MetricNames)
const METRICS_DEADLINE: Duration = Duration::from_secs(20);  // head + body, success or error
const BACKEND_TIMEOUT: &str = "15s";                          // `timeout=` query parameter
const BODY_LIMIT: usize = 4 * 1024 * 1024;
const SERIES_LIMIT: usize = 64;
```

- Path: `/api/v1/namespaces/{ns}/services/{scheme}:{service}:{port}/proxy{prefix}/api/v1/{endpoint}`; every part comes from a validated `MetricsSource` and the private enum. Query string: `form_urlencoded::Serializer` (`query`, `time`/`start`/`end`/`step`, `timeout`). Built as `http::Request::get(uri)` with an empty body and `Accept: application/json`.
- **The one exception** (decision 5): `async fn metrics_get(&self, source, endpoint, query: String) -> Result<MetricsAnswer, MetricsError>` with `#[allow(clippy::disallowed_methods)]` and the comment `// Read-only GET of the 0048 metrics endpoint allow-list through the service proxy: the metrics_query.rs row of the 0030 table.` Inside it: `tokio::time::timeout_at(deadline, async { self.client().send(request).await, then the body })`. Not `self.run`: its 30 s `REQUEST_TIMEOUT` and `classify_error` would drop the `Status`. No other function in the file calls a raw method.
- `MetricsAnswer { status: http::StatusCode, body: Vec<u8> }` (private). The body, **success and error alike**, is read with `http_body_util::Limited::new(body, BODY_LIMIT)` + `collect()` under the same deadline; the limit error → `TooLarge`.
- The status is mapped here, not by kube ([Errors](#errors)); JSON is parsed with `serde_json::from_slice` into private docs (`Status` subset: `code`, `reason`, `message`; backend `{status, errorType, error}`); serde errors become fixed text (the body is never echoed).
- Trace: `tracing::debug!(endpoint, status, bytes, series, ms, "read metrics")`; never a body or a header. kube's `TraceLayer` records the request URL, PromQL included, in a debug span under `kube_client::client::builder`: names only, never a credential; the coder does not add more.

## Public calls

```rust
impl ClusterConnection {
    /// `count(container_cpu_usage_seconds_total)` at now.
    pub async fn check_metrics_source(&self, source: &MetricsSource) -> Result<SourceCheck, MetricsError>;
    pub async fn usage_range(&self, source: &MetricsSource, target: &UsageTarget, metric: UsageMetric,
        range: &RangeSpec) -> Result<UsageSeries, MetricsError>;
}
pub struct SourceCheck { pub latency: Duration, pub cpu_series: u64 }      // 0: reachable, no cAdvisor data
pub struct UsageSeries { pub points: Vec<(jiff::Timestamp, Option<f64>)>, pub was_cut: bool }
```

Decoding: `status` must be `success`; `matrix` values `[t, "v"]` with `t` float seconds; `"NaN"`, `"+Inf"`, `"-Inf"`, unparsable → `None`. Each sample goes to the step index `round((t − start) / step)` (nearest step, never float equality); indexes outside `0..=points` are dropped; a missing step is `None` (a gap, as 0010 charts draw). `usage_range` sums the (at most 64) series per index (queries already `sum`; this only guards a backend that splits them).

## Errors

```rust
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MetricsError { Denied(String), Unreachable(String), NoApiAtPrefix, Rejected(String), TooLarge,
    TimedOut, InvalidName, Unexpected(String) }
```

| Answer | Variant | `Display` |
|---|---|---|
| 403 whose body parses as a `Status` (API server RBAC) | `Denied` | `not permitted: {message}` |
| 404 / 503 whose body parses as a `Status` (`services "x" not found`, `no endpoints available for service`) | `Unreachable` | `the metrics service cannot be reached: {message}` |
| 404 whose body is not a `Status` (the backend answered: wrong prefix or port) | `NoApiAtPrefix` | `nothing answers at this path prefix; check the prefix and port` |
| any code whose body parses as `{"status":"error","error":…}`, including 200 | `Rejected` | `the metrics backend refused the query: {error}` |
| body over 4 MiB (any code) | `TooLarge` | `the metrics answer is larger than 4 MiB` |
| deadline | `TimedOut` | `the metrics backend did not answer within 20 s` |
| a name failing its DNS check (no request sent) | `InvalidName` | `a name is not valid for a metrics query` |
| transport (`kube::Error` of `send`), decode, other codes | `Unexpected` | `the metrics request failed: {message}` (other codes: `HTTP {code}`) |

Every `{message}` is cut to 200 chars on one line (control chars dropped). A backend error text may quote the query (names only, never a credential).

## 0030 table row (step 2, docs)

| File | Item | Spec |
|---|---|---|
| `metrics_query.rs` | `metrics_get` (read-only GETs of the 0048 metrics endpoint allow-list through the API server service proxy, `Client::send` with a capped body) | 0048 |

Add `metrics_query.rs` to the "Shipped so far" sentence of the grep paragraph (it has no mutating method, so the grep result is unchanged).
