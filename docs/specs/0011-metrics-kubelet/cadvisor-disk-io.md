# 0011 · Cluster: cAdvisor disk I/O reader

[Back to index](README.md) · Step 1 · Module: `cadvisor_text.rs` (new, tests in module); called from `kubelet_stats.rs`

## Public types

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiskIoCounters { pub sampled_at: Option<jiff::Timestamp>, pub read_bytes: u64, pub write_bytes: u64 }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerDiskIo { pub namespace: String, pub pod: String, pub container: String, pub counters: DiskIoCounters }
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiskIoSample {
    /// The root cgroup (`id="/"`), summed over devices; `None` when cAdvisor has no such series.
    pub node: Option<DiskIoCounters>,
    /// Ordered by (namespace, pod, container).
    pub containers: Vec<ContainerDiskIo>,
}
```

## Reading (in `kubelet_stats.rs`)

```rust
const CADVISOR_BYTE_LIMIT: u64 = 64 * 1024 * 1024;
async fn read_disk_io(connection: &ClusterConnection, node: &str) -> Result<DiskIoSample, ClusterError>;
```

1. `connection.kubelet_lines(node, KubeletPath::MetricsCadvisor)` for the response head ([kubelet-stats.md](kubelet-stats.md)).
2. Read the body with `futures::AsyncBufReadExt::lines()` under `tokio::time::timeout(REQUEST_TIMEOUT)`; count bytes (line length + 1); feed every line to `DiskIoBuilder::push_line`.
3. Errors, with action `KubeletPath::MetricsCadvisor.action()`: timeout → `ClusterError::TimedOut`; a body read error or invalid UTF-8 → `UnexpectedResponse` with the fixed source `"cAdvisor body could not be read"` (the io text is not kept); more than `CADVISOR_BYTE_LIMIT` bytes → stop, `UnexpectedResponse` with `"cAdvisor metrics exceed 64 MiB"`.
4. The body is never kept; success bodies are never traced; only `tracing::debug!(node, containers, bytes)`.

`REQUEST_TIMEOUT` becomes `pub(crate)` in `connection.rs` if it is not already.

## Line parser (private, pure)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction { Read, Write }
struct DiskIoLine<'a> { direction: Direction, id: Option<Cow<'a, str>>, namespace: Option<Cow<'a, str>>,
    pod: Option<Cow<'a, str>>, container: Option<Cow<'a, str>>, value: u64, at: Option<jiff::Timestamp> }
fn disk_io_line(line: &str) -> Option<DiskIoLine<'_>>;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ContainerKey { namespace: String, pod: String, container: String }
#[derive(Default)]
struct Accumulated { read_bytes: u64, write_bytes: u64, sampled_at: Option<jiff::Timestamp> }
#[derive(Default)]
struct DiskIoBuilder { node: Option<Accumulated>, containers: BTreeMap<ContainerKey, Accumulated> }
impl DiskIoBuilder { fn push_line(&mut self, line: &str); fn finish(self) -> DiskIoSample; }
```

| Rule | Detail |
|---|---|
| Families | only lines starting with `container_fs_reads_bytes_total{` (Read) or `container_fs_writes_bytes_total{` (Write); every other line, `#` comments included, is skipped before any parsing |
| Grammar | `name{key="value",…} value [timestamp_ms]`; label values unescape `\\`, `\"`, `\n`; a trailing comma is allowed; a malformed line → `None` |
| Labels kept | `id`, `namespace`, `pod`, `container`; others (`image`, `name`, `device`, …) are skipped without allocation |
| Value | `f64` text (`1.2e+06` allowed); NaN, infinite, or negative → `None`; else truncated to `u64` |
| Timestamp | optional integer milliseconds → `jiff::Timestamp`; bad → `None` for the time only |
| Classify | `id == "/"` → node; `container` non-empty and not `POD`, with non-empty `namespace` and `pod` → that container; anything else (pod cgroups, system slices) → ignored |
| Accumulate | sum `value` per key and direction over devices (saturating); `sampled_at` = newest line time |
| Finish | a key with only one direction keeps 0 for the other; containers sorted by key |

## Why streaming

The body holds ≈ 60 series per container, mostly other families; reading line by line keeps one line in memory instead of 1–4 MB, and the prefix check skips the rest cheaply (decision 3).
