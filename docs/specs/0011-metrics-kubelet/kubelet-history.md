# 0011 · App: kubelet history and rates

[Back to index](README.md) · Steps 2–3 · Module: `kubelet_history.rs` (new) + `kubelet_history_tests.rs`. Uses 0010's `history_rings.rs` (`Timeline`, `Rings<P>`, `RingPoint`, `FINE_TICKS`, `TICKS_PER_COARSE`, `COARSE_POINTS`, `Resolution`) as is.

## Types

```rust
/// Bytes per second: receive/transmit (network) or read/write (disk).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RatePair<T> { pub(crate) first: T, pub(crate) second: T }
// impl RingPoint for RatePair<u32> and RatePair<u64> (field-wise mean)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RateKind { Network, DiskIo }          // step 3 (no reader before it)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiskIoState { NotSampled, Sampled { has_root: bool } }   // step 3: the node's newest disk sample, stored by `record` from then on
pub(crate) struct RateSeries {
    pub(crate) points: Vec<(jiff::Timestamp, Option<RatePair<u64>>)>,  // oldest first
    pub(crate) step: Duration,
    pub(crate) pod_count: usize,      // pods with a value in `points`
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PodKey { namespace: String, name: String }   // private
pub(crate) struct KubeletHistory { /* fine, coarse: Timeline; tick_count; nodes: BTreeMap<String, NodeKubelet>;
    pods: BTreeMap<PodKey, PodKubelet>; pvcs: BTreeMap<String /* namespace */, BTreeMap<String /* claim */, SeenPvc>> */ }
```

Private: `CounterState { at, first: u64, second: u64, last_rate: Option<RatePair<u64>> }`; `CounterRings<P> { counter: Option<CounterState>, rings: Option<Rings<P>> }` (the rings exist from the first rate on, so a new entry is not aged out in its first tick); `NodeKubelet { network, disk: CounterRings<RatePair<u64>>, last_seen }` (step 3 adds `disk_io: DiskIoState`); `PodKubelet { uid: Option<String>, controller: Option<ControllerRef>, network: Option<CounterRings<RatePair<u32>>>, containers: BTreeMap<String, ContainerKubelet>, last_seen }` (step 3 adds `node`, which has no reader before it); `ContainerKubelet { disk: CounterRings<RatePair<u32>>, last_seen }`; `SeenPvc { usage: PvcUsage, last_seen: u64 }`.

## Rate rule (pure; decisions 16–17)

```rust
fn next_rate(previous: Option<&CounterState>, at: jiff::Timestamp, first: u64, second: u64)
    -> (Option<RatePair<u64>>, CounterState);
```

| Case | Rate | New state |
|---|---|---|
| no previous | `None` | current, `last_rate: None` |
| `Δt = at − previous.at` ≤ 0 or < 1 s | `previous.last_rate` | previous kept (a backwards time is ignored) |
| `first < previous.first` or `second < previous.second` | `None` (reset) | current, `last_rate: None` |
| else | per field `(Δ as u128 × 1000 + Δms / 2) / Δms`, saturating to `u64` | current, `last_rate: Some(rate)` |

`at` is the counter's `sampled_at`, else the round time. Mixing the two sources across ticks (a kubelet that sometimes omits `time`) skews Δt by the scrape-to-poll lag (≤ 15 s); accepted, as the kubelet always sets `time` in practice. Pod and container rates saturate to `u32`.

## `record` (step 2)

```rust
pub(crate) fn record(&mut self, at: jiff::Timestamp, round: &[NodeKubeletStats],
    pods: &[PodSummary], scope: &NamespaceScope);
```

`pods` must be ordered by (namespace, name), as every pods snapshot is (`debug_assert!`); the host-network lookup is a binary search.

1. Push `at` on the fine timeline; `tick_count += 1`; push `None` to every fine ring (0010 shape).
2. Each node with `summary: Ok(s)`: node network rate from `s.network`. Each pod of `s.pods` whose namespace is in `scope`: get or create its entry; a different `uid` resets only its counters (`network` and every container's `disk`), while the rings and the controller stay with the pod name (decision 18). `uid` is an `Option`: a pod first named by a disk series has none yet, and the summary fills it in without a reset. Its `PodSummary` (binary search) says `host_network` → `network = None` and no counter (decision 21; a later-known flag frees existing rings); else the network rate. Mark seen. Each `PvcUsage` in scope replaces the stored one unless the stored `sampled_at` is newer (RWX claims appear under several pods).
3. Each node with `disk_io: Some(Ok(d))`: (step 3 also sets `disk_io = Sampled { has_root: d.node.is_some() }`, and `NotSampled` for a node with no disk sample this tick); `d.node` → node disk rate; each `ContainerDiskIo` in scope → that pod's container entry (pod created if absent) → container disk rate. A node with no disk sample this tick → `NotSampled`.
4. Every `TICKS_PER_COARSE`-th tick: coarse fold via `RingPoint::mean`; free fine rings unseen `FINE_TICKS` ticks; remove pods and nodes unseen 24 h; remove PVCs unseen `FINE_TICKS` ticks (decision 24).
5. Each `PodSummary` with an entry: fill `controller` when `None`.

Failed nodes add nothing; their rings read `None` this tick.

## Reading

| Call | Step | Result |
|---|---|---|
| `pvc_usage(namespace, claim) -> Option<&PvcUsage>` | 2 | the stored usage |
| `tick_count() -> u64`, `span()`, `newest_tick() -> Option<Timestamp>` | 2, 3 | as 0010 |
| `retain_scope(&NamespaceScope)` | 2 | drops pods and PVCs of other namespaces |
| `pod_rates(kind, ns, pod, container: Option<&str>, resolution) -> RateSeries` | 3 | Network: the pod's network, `container` ignored (shared namespace); no rings → all `None`. DiskIo: `None` → sum of its containers (`None` only when all are), `Some(c)` → `c` |
| `owner_rates(kind, owner: &PodOwner, pod: Option<&str>, resolution) -> RateSeries` | 3 | sum over pods matching 0010's `owns`; host-network pods add nothing (no rings) |
| `node_rates(kind, node, resolution) -> RateSeries` | 3 | the node's rings |
| `disk_io_state(node) -> DiskIoState` | 3 | for the disk notices |
| `has_disk_series(ns, pod, container: Option<&str>) -> bool` | 3 | a container entry exists (any container when `None`) |

Sums are `u64` per point; unknown keys give all-`None` series on the same timeline, never a panic.

## Invariants

- As 0010: a ring never exceeds its timeline or cap; a short ring aligns to the end.
- Counter state lives with its entry; a removed entry restarts with `None`.
- Nothing from `PvcUsage` is logged.
