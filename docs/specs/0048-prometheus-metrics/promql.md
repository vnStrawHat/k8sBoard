# 0048 · PromQL builders (step 1)

[Back to index](README.md) · Module (cluster crate): `promql.rs` (new, pure, no request) + `promql_tests.rs`. 0049 adds its traffic templates here.

## Escaping

```rust
/// A PromQL double-quoted string: `\` → `\\`, `"` → `\"`, `\n` → `\\n`, other control chars dropped.
pub(crate) fn string_literal(value: &str) -> String;           // returns the text with its quotes
/// For `=~` / `!~`: RE2 metacharacters `\ . + * ? ( ) | [ ] { } ^ $` get a `\`, then `string_literal`
/// (so `a.b` becomes `"a\\.b"` in the query). PromQL regexes are fully anchored.
pub(crate) fn regex_literal(value: &str) -> String;
```

Defense in depth: every name is checked first (`InvalidName` otherwise, no request): namespaces and container names DNS label; pod, node, and workload names DNS subdomain (`dns_name.rs`).

## Targets

```rust
pub enum UsageTarget {
    Pod { namespace: String, pod: String, container: Option<String> },
    Workload { namespace: String, kind: WorkloadKind, name: String },
    Node { name: String },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkloadKind { Deployment, StatefulSet, DaemonSet, ReplicaSet, Job }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsageMetric { Cpu, Memory, NetworkReceive, NetworkTransmit, DiskRead, DiskWrite }
pub struct RangeSpec { /* start, end, step: private */ }
impl RangeSpec {
    /// Err when `step` < 1 s, `end` ≤ `start`, or more than `MAX_POINTS` (400) points.
    pub fn new(start: jiff::Timestamp, end: jiff::Timestamp, step: Duration) -> Result<Self, RangeError>;
}
```

The app's Monitor scopes map as: Pod · Total → `Pod { container: None }`; Pod · Part(c) and the container sub-tab → `Pod { container: Some(c) }`; Workload · Total → `Workload`; Workload · Part(p) → `Pod { pod: p, container: None }`; Node → `Node`.

## Selectors

| Target | Pod selector `P` | Container filter `C` |
|---|---|---|
| Pod | `namespace="{ns}",pod="{pod}"` | `container="{c}"`, or `container!="",container!="POD"` for the whole pod |
| Workload | `namespace="{ns}",pod=~"{pattern}"` | `container!="",container!="POD"` |
| Node | `id="/",node="{node}"` | none |

Pod-name patterns (decision 11; `{name}` is `regex_literal`-escaped):

| Kind | Pattern |
|---|---|
| Deployment | `{name}-S{1,10}-S{5}` |
| StatefulSet | `{name}-[0-9]+` |
| DaemonSet, ReplicaSet, Job | `{name}-S{5}` |

`S` = `[bcdfghjklmnpqrstvwxz2456789]`, the alphabet of Kubernetes' generated suffixes (`rand.SafeEncodeString`; no vowels, no `0 1 3`), so CronJob timestamps (`api-28312790-…`) and most words do not match. `ponytail:` a sibling workload whose name plus a dash and a word of that alphabet fits the pattern (`api` and `api-v2x`) still counts; upgrade path: a `kube_pod_owner` join when kube-state-metrics is present (decision 11).

## Query table

`W` = the rate window = `max(step, 120 s)` written in whole seconds (`[120s]`, `[7200s]`): at least four scrapes at a 30 s interval, so a 15 s step never yields an empty `rate`. `V` = the virtual-interface filter `interface!~"lo|(veth|cali|cni|flannel|cilium|lxc|docker|tunl|vxlan|kube-|weave|br-).*"` (the 0011 node rule); pods use `interface!="lo"`.

| Metric | Pod, Workload | Node |
|---|---|---|
| Cpu | `sum(rate(container_cpu_usage_seconds_total{P,C}[W]))` | `sum(rate(container_cpu_usage_seconds_total{N}[W]))` |
| Memory | `sum(container_memory_working_set_bytes{P,C})` | `sum(container_memory_working_set_bytes{N})` |
| NetworkReceive | `sum(rate(container_network_receive_bytes_total{P,interface!="lo"}[W]))` (pod level for a container too, W4c note 5) | `sum(rate(container_network_receive_bytes_total{N,V}[W]))` |
| NetworkTransmit | same with `transmit` | same with `transmit` |
| DiskRead | `sum(rate(container_fs_reads_bytes_total{P,C}[W]))` | `sum(rate(container_fs_reads_bytes_total{N}[W]))` |
| DiskWrite | same with `writes` | same with `writes` |

`N` is the Node selector `id=~"/|",pod="",node="{n}"` (decision 16: the `id` label may be absent), written twice joined by `or`, the second with `kubernetes_io_hostname` in place of `node`: `sum(A{N1}) or sum(A{N2})`. Node DiskRead and DiskWrite build no query (decision 17). Units: CPU cores, memory bytes, rates bytes/s (the 0010/0011 `Measure`s).

Facts the step 2 live check settles and records in as-built (open item 1):

| Fact | If true | If false |
|---|---|---|
| root-cgroup series `id="/"` carry a `node` label | `N` as above | node-exporter: `N` uses its CPU (`node_cpu_seconds_total{mode!="idle"}`), memory (`node_memory_MemTotal_bytes - node_memory_MemAvailable_bytes`), network, and disk families with the node label it has (record the label: `instance`, `node`, or `kubernetes_node`); none usable → `UsageTarget::Node` is removed and Node Monitor stays on the sampler for every range |
| `container_fs_reads_bytes_total` / `container_fs_writes_bytes_total` exist | Disk I/O as above | the Disk I/O card shows `No disk I/O series in the source` |
| pod network counted once: one pod's 1h receive rate is within ±20 % of the 0011 kubelet summary rate | as above | the query keeps only the pod-sandbox series (`container="POD"` or `container=""`, whichever the check shows) |

## Step table

| Range | Step | Points |
|---|---|---|
| 15m | 15 s | 61 |
| 1h | 15 s | 241 |
| 6h | 1 min | 361 |
| 24h | 5 min | 289 |
| 7d | 30 min | 337 |
| 30d | 2 h | 361 |

`end` = now rounded down to the step (so refreshes reuse the backend cache); `start` = `end − range`. Instant queries (`check_metrics_source`, 0049) send `time` = now.

## Tests (names in [test-plan.md](test-plan.md))

Exact query text per metric × target kind (golden strings); escaping of `"`, `\`, newline, `.`; a pod name with a regex metachar never reaches the query unescaped; `RangeSpec` limits; step table covers every range at ≤ 400 points.
