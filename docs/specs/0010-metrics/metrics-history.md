# 0010 · App: formatting and history

[Back to index](README.md) · Steps 2–3 · Modules: `usage_format.rs` (new, tests in module), `history_rings.rs` (new, tests in module), `metrics_history.rs` (new) + `metrics_history_tests.rs`, `kind_row.rs`

## Formatting (`usage_format.rs`, pure)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Measure { Cpu, Bytes }                 // charts use it as their unit; 0011 adds a rate
impl Measure {
    pub(crate) fn format(self, value: f64) -> String;  // cores or bytes
    /// The unit once when both values share it: `9.8 / 15.8 cores`, `498 of 512Mi`; else both: `310m of 1 core`.
    pub(crate) fn format_pair(self, used: f64, total: f64, separator: &str) -> String;
}
pub(crate) fn format_percent(ratio: f64) -> String;          // rounded; may pass 100 %
pub(crate) fn usage_tone(ratio: f64) -> Option<StatusTone>;  // ≥ 0.9 Bad, ≥ 0.8 Warn, else None
pub(crate) fn format_offset(seconds: u64) -> String;         // before now (step 3)
```

| Function | Examples |
|---|---|
| `Cpu.format` | `0` → `0m`; `0.0004` → `<1m`; `0.31` → `310m`; `0.9996` → `1 core`; `2.5` → `2.5 cores`; `12.04` → `12 cores` |
| `Bytes.format` | `0` → `0B`; `512` → `512B`; `2048` → `2Ki`; `498 Mi` → `498Mi`; `1.1 Gi` → `1.1Gi`; `15.6 Gi` → `15.6Gi`; `120 Gi` → `120Gi`; `1.5 Ti` → `1.5Ti`. Ki and Mi whole; Gi and up one decimal below 100; a value rounding to 1024 moves up a unit (`1023.7Mi` → `1.0Gi`) |
| `format_pair` | `(9.8, 15.8 cores, " / ")` → `9.8 / 15.8 cores`; `(498Mi, 512Mi, " of ")` → `498 of 512Mi`; `(0.31, 1 core, " of ")` → `310m of 1 core` |
| `format_percent` / `format_offset` | `0.314` → `31%`; `0` → `now`, `45` → `-45s`, `195` → `-3m 15s`, `20400` → `-5h 40m` |

## History (`metrics_history.rs`, pure)

```rust
// history_rings.rs: the three constants and `Resolution`; the rest is metrics_history.rs
pub(crate) const FINE_TICKS: usize = 240;        // 1 h at METRICS_INTERVAL
pub(crate) const TICKS_PER_COARSE: usize = 20;   // 5 min
pub(crate) const COARSE_POINTS: usize = 288;     // 24 h
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct UsagePoint { pub(crate) cpu_microcores: u32, pub(crate) memory_kib: u32 }  // containers
impl UsagePoint { pub(crate) fn of(usage: ResourceUsage) -> Self; pub(crate) fn usage(self) -> ResourceUsage; }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Resolution { Fine, Coarse }      // Coarse: coarse points, then fine ticks newer than the last one
pub(crate) struct UsageSeries {
    pub(crate) points: Vec<(jiff::Timestamp, Option<ResourceUsage>)>,  // oldest first
    pub(crate) step: Duration,                     // 15 s or 5 min, for gap splitting
    pub(crate) oom: Vec<jiff::Timestamp>,
    pub(crate) pod_count: usize,                   // pods with a value in `points`
    pub(crate) sampled_at: Option<jiff::Timestamp>, // newest server timestamp among the included items
}
impl PodUsageHistory {
    pub(crate) fn record(&mut self, at: jiff::Timestamp, sample: &[PodMetrics], pods: &[PodSummary]);
    pub(crate) fn retain_scope(&mut self, scope: &NamespaceScope);
    pub(crate) fn latest(&self, namespace: &str, pod: &str) -> Option<ResourceUsage>;  // pod total
    pub(crate) fn latest_container(&self, namespace: &str, pod: &str, container: &str) -> Option<ResourceUsage>;
    pub(crate) fn pod_series(&self, namespace: &str, pod: &str, container: Option<&str>, resolution: Resolution) -> UsageSeries;
    pub(crate) fn owner_series(&self, owner: &PodOwner, pod: Option<&str>, resolution: Resolution) -> UsageSeries;
    pub(crate) fn tick_count(&self) -> u64;        // ticks ever recorded (monotonic)
    pub(crate) fn span(&self) -> Option<Duration>; // oldest kept point to newest tick
}
// NodeUsageHistory: record(at, &[NodeMetrics]), latest(node), node_series(node, resolution), tick_count, span.
```

Shared shape (`history_rings.rs`, `pub(crate)`, reused by 0011's kubelet history): fine and coarse `Timeline`s (`VecDeque<jiff::Timestamp>`); `Rings<P> { fine: Option<VecDeque<Option<P>>>, coarse: VecDeque<Option<P>> }` with push, align, freeing, and the coarse fold; `FINE_TICKS`, `TICKS_PER_COARSE`, `COARSE_POINTS`, and `Resolution` live there too. The fold averages through `pub(crate) trait RingPoint: Copy { fn mean(points: &[Self]) -> Self; }` (field-wise mean of the `Some` values), implemented here for `UsagePoint` and `ResourceUsage`. Private here: `P` = `UsagePoint` for containers and `ResourceUsage` (u64, decision 10) for nodes; `PodHistory { controller: Option<ControllerRef>, containers: BTreeMap<String, Rings<UsagePoint>>, oom: Vec<OomMark>, sampled_at: Option<Timestamp>, last_seen: u64 }`.

Step split (dead-code rule: nothing lands before its first reader): step 2 lands `history_rings.rs` with fine rings and `metrics_history.rs` (`record` steps 1, 2, 4), `retain_scope`, `latest*`, `tick_count`; step 3 adds the coarse rings and `RingPoint` (step 3), `sampled_at`, `controller`, OOM marks (step 5), `UsageSeries`, `Resolution`, `span`, the `*_series` calls, `owns`, and `format_offset`; `record` gains its `pods` parameter in step 3.

### Invariants

- After `record`, every ring's newest entry belongs to this tick (fine) or the newest fold (coarse); no ring is longer than its timeline or its cap. A short ring aligns to the end: ring index `i` ↔ timeline index `timeline.len() - ring.len() + i`.
- `latest*` read only the newest fine tick: a pod absent from it reads `None` ("—").

### `record` (pods)

1. Push `at` on the fine timeline (cap `FINE_TICKS`); `tick_count += 1`.
2. Push `None` to every fine ring; for each `PodMetrics` set (or create) its rings' newest entry to `Some(UsagePoint::of(..))`, store `sampled_at`, mark the pod seen.
3. Every `TICKS_PER_COARSE`-th tick: push `at` on the coarse timeline (cap `COARSE_POINTS`) and to every coarse ring the mean of the `Some` values among the last 20 fine entries (missing entries count as `None`; all `None` → `None`; a freed fine ring → `None`).
4. Free a fine ring unseen for `FINE_TICKS` ticks; remove a pod unseen for 24 h (`FINE_TICKS × 24` ticks).
5. For each `PodSummary` (binary search; the list is sorted) with a history entry: fill `controller` when it is still `None` (decision 11); each `Termination` of a container (`ContainerState::Terminated`, `last_termination`) with reason `StatusReason::OomKilled` and a `finished_at` within 24 h adds `OomMark { container, at }` unless present; marks older than 24 h drop; at most 32 (oldest drop first).

`NodeUsageHistory::record` is steps 1–4 with one `Rings<ResourceUsage>` per node.

### Series

| Call | `points[i]` | `oom` | `pod_count` |
|---|---|---|---|
| `pod_series(ns, pod, None, r)` | sum of the pod's containers; `None` when all are `None` | the pod's marks | 1 if any value |
| `pod_series(ns, pod, Some(c), r)` | container `c` | marks of `c` | 1 if any value |
| `owner_series(owner, None, r)` | sum over pods whose namespace and `controller` match `owns` | union | pods with a value |
| `owner_series(owner, Some(pod), r)` | that pod, if it matches | its marks | 0 or 1 |
| `node_series(node, r)` | the node's rings | empty | 0 |

Sums are u64 per point. Unknown keys give a series of `None`s on the same timeline, never a panic.

`kind_row.rs`: `owns_pod` delegates to `pub(crate) fn owns(owner: &PodOwner, namespace: &str, controller: Option<&ControllerRef>) -> bool`.

### `retain_scope`

`All` keeps everything; `Named`/`Several` keep pods whose namespace is in `scope.namespaces()`. Called when the pods scope changes; timelines stay.
