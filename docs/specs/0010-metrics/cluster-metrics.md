# 0010 · Cluster: quantities, metrics polling, access

[Back to index](README.md) · Step 1 · Modules: `quantity.rs` (new), `resource_metrics.rs` (new) + `resource_metrics_tests.rs`, `connection.rs`, `access_review.rs`, `resource_watch.rs` (doc only), `lib.rs`, `examples/probe.rs`

## Quantities (`quantity.rs`, tests in module)

```rust
/// CPU in nanocores, from a Kubernetes quantity such as `250m`, `0.5`, `2`, or `1234567n`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CpuAmount { nanocores: u64 }
impl CpuAmount { pub fn parse(text: &str) -> Option<Self>; pub fn from_nanocores(n: u64) -> Self;
    pub fn nanocores(self) -> u64; pub fn cores(self) -> f64; }
/// Bytes, from a quantity such as `512Mi`, `1G`, `129e6`, or `16384256Ki`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteAmount { bytes: u64 }
impl ByteAmount { pub fn parse(text: &str) -> Option<Self>; pub fn from_bytes(b: u64) -> Self; pub fn bytes(self) -> u64; }
/// Private: `mantissa × 10^exponent × binary`, exact.
struct Quantity { mantissa: u128, exponent: i32, binary: u128 }
fn parse_quantity(text: &str) -> Option<Quantity>;
```

| Part | Rule |
|---|---|
| Text | trimmed; empty → `None`; optional `+`; `-` → `None` (never negative here) |
| Number | digits with an optional `.` fraction (`5`, `5.`, `.5`); at least one digit; digits go into the u128 mantissa (over 38 digits → `None`), each fraction digit lowers `exponent` by 1 |
| Then exactly one of | a binary suffix `Ki`…`Ei` (`binary` = 2^10…2^60); a decimal suffix `n` −9, `u` −6, `m` −3, `k` 3, `M` 6, `G` 9, `T` 12, `P` 15, `E` 18 (added to `exponent`); an exponent `e`/`E` + optional sign + digits (`129e6`, `1E3`; `E` alone is exa); or nothing |
| Rejected | `1.2.3`, `5x`, `Mi`, `1 Gi`, exponent with a suffix (`1e3Ki`, `1.5e2m`) |
| Conversion | target unit: nanocores (`exponent + 9`) or bytes; a positive power multiplies, a negative one divides rounding half up; checked u128 math; above `u64::MAX` → `None` |

## Metrics types (`resource_metrics.rs`)

```rust
pub const METRICS_INTERVAL: Duration = Duration::from_secs(15);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResourceUsage { pub cpu: CpuAmount, pub memory: ByteAmount }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PodMetrics { pub namespace: String, pub name: String,
    /// The server's scrape time (`timestamp`).
    pub sampled_at: Option<jiff::Timestamp>,
    /// In API order; at least one.
    pub containers: Vec<ContainerMetrics> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerMetrics { pub name: String, pub usage: ResourceUsage }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeMetrics { pub name: String, pub sampled_at: Option<jiff::Timestamp>, pub usage: ResourceUsage }

impl ClusterConnection {
    /// Polls PodMetrics in `scope` every `METRICS_INTERVAL`, the first poll at once; snapshots
    /// ordered by (namespace, name). Never ends; dropping the stream stops it.
    pub fn poll_pod_metrics(&self, scope: NamespaceScope) -> impl Stream<Item = WatchUpdate<PodMetrics>> + Send + 'static;
    /// Same for NodeMetrics, ordered by name.
    pub fn poll_node_metrics(&self) -> impl Stream<Item = WatchUpdate<NodeMetrics>> + Send + 'static;
}
```

## Requests

- `ApiResource { group: "metrics.k8s.io", version: "v1beta1", api_version: "metrics.k8s.io/v1beta1", kind: "PodMetrics" | "NodeMetrics", plural: "pods" | "nodes" }`, read through `Api<DynamicObject>` like `object_yaml.rs`. No serde derive, no new dependency.
- `connection.rs` gains `scoped_dynamic_apis(scope, &ApiResource) -> Vec<(Option<String>, Api<DynamicObject>)>`, the `DynamicObject` twin of 0009's `scoped_apis`. Each api is listed with `list_all(api, "listing pod metrics")` in order; the results go through the pure `concat_namespaces(results: Vec<(Option<String>, Result<Vec<T>, ClusterError>>)) -> Result<Vec<T>, ClusterError>`: concatenation in order, or the first error, wrapped in 0009's `ClusterError::Namespace` when it names a namespace and there are several. Nodes: one `Api::all_with`, action `"listing node metrics"`.
- A 404 or 503 surfaces as `ClusterError::Api { code, .. }`; the app words it (decision 6). `metrics_api()` (0001) stays for the probe only.
- Mapping (private, pure): `pod_metrics(&DynamicObject) -> Option<PodMetrics>` reads metadata namespace and name, `data["timestamp"]` (RFC 3339 via jiff; bad text → `None`), and `data["containers"][*]` with `name`, `usage.cpu`, `usage.memory`. A container missing a field or failing to parse is skipped; no container left → `None`. `node_metrics` reads `data["timestamp"]` and `data["usage"]`. Labels and `window` are not kept.
- Snapshots are sorted like watch snapshots. Only counts are traced (`tracing::debug!(pods, containers)`), never values or names.

## Poll loop (private core, tested with paused tokio time)

```rust
fn poll_updates<T, F, Fut>(fetch: F) -> impl Stream<Item = WatchUpdate<T>> + Send + 'static
where F: FnMut() -> Fut + Send + 'static,
      Fut: Future<Output = Result<Vec<T>, ClusterError>> + Send + 'static, T: Send + 'static;
fn next_delay(failures: u32) -> Duration;   // 0 → 15 s, 1 → 30 s, 2 → 60 s, ≥ 3 → 120 s
```

`stream::unfold` over `{ fetch, failures, is_first }`: sleep `next_delay(failures)` (not before the first fetch), fetch, yield `Snapshot` (failures = 0) or `Failed` (failures + 1). The sleep starts after a fetch ends, so requests never stack; each still has the 30 s `run` timeout. No task is spawned.

## Access (`access_review.rs`)

| Variant | Verb | Group | Resource | Namespaced |
|---|---|---|---|---|
| `ListPodMetrics` | list | `metrics.k8s.io` | pods | yes |
| `ListNodeMetrics` | list | `metrics.k8s.io` | nodes | no |

- Appended to `AccessCheck::ALL`, which becomes `[AccessCheck; 21]`. `Display` adds the group when not core: `list pods.metrics.k8s.io`.
- New, for decision 7 (reuses 0009's private `review_checks`; SSAR `create` only):

```rust
pub struct NamespaceAccess { pub namespace: Option<String>, pub decision: AccessDecision }
impl ClusterConnection {
    /// One review of `check` per namespace of `scope` (one cluster-wide review for `All`), in order.
    pub async fn review_namespaces(&self, check: AccessCheck, scope: &NamespaceScope)
        -> Result<Vec<NamespaceAccess>, ClusterError>;
}
```

## Exports and docs

`lib.rs`: `ByteAmount`, `CpuAmount`, `ContainerMetrics`, `METRICS_INTERVAL`, `NamespaceAccess`, `NodeMetrics`, `PodMetrics`, `ResourceUsage`. `WatchUpdate`'s doc becomes "One item of a resource watch or a metrics poll".

## Probe (`examples/probe.rs`)

`--metrics-seconds <n>` runs both polls for `n` seconds and prints one line per update: `pod metrics: 98 pods (listed 104, not running 6), 187 containers, cpu 3.214 cores, memory 41.2 GiB`, `node metrics: 6 nodes, cpu …, memory …`, or `pod metrics failed: {error}`. "Not running" = listed pods with no container in `ContainerState::Running`. Sums are formatted in the probe. Nothing from the kubeconfig is printed.
