# 0021 · Headline, Capacity, Nodes heatmap (step 1)

[Back to index](README.md) · Modules: `overview.rs` (headline, stats), `cluster_capacity.rs` (new, + `cluster_capacity_tests.rs`), `node_heatmap.rs` (new; model and render, tests in module), `usage_bar.rs` (`CapacityBar`), `usage_format.rs` (`format_shared`). Every model is pure: no GPUI context, and the inputs are the Ready snapshots.

## Headline and stats (`overview.rs`)

```rust
/// `{context} · Kubernetes {git_version}`, plus ` · {region}` when known.
fn headline_text(context: &str, version: &ServerVersion, nodes: Option<&[NodeSummary]>) -> String;
/// The `topology.kubernetes.io/region` of the labelled nodes: one value → it; several → `{n} regions`; none → None.
fn cluster_region(nodes: &[NodeSummary]) -> Option<String>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Counted { ready: usize, total: usize }        // nodes: Ready; pods: running
struct ClusterStats { nodes: Option<Counted>, pods: Option<Counted>, namespaces: Option<usize> }
fn cluster_stats(live: &LiveCluster) -> ClusterStats; // a part is None while its list is not Ready
```

- A Ready node has `status.readiness == NodeReadiness::Ready`. A running pod is `PodStatus::Reason(StatusReason::Running) | PodStatus::NotReady` (phase Running).
- Numbers use `group_digits` from `workspace.rs` (make it `pub(super)`).

## Shared-unit format (`usage_format.rs`)

```rust
/// The numbers of `values`, with the unit printed only on the last one when every value shares it
/// (`["104", "131", "168 cores"]`); otherwise each keeps its own unit. `format_pair` is built on it.
pub(crate) fn format_shared(self, values: &[f64]) -> Vec<String>;
```

## Capacity model (`cluster_capacity.rs`)

```rust
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Layers {
    pub(crate) used: Option<f64>,        // None: no node feed, or no node has a sample
    pub(crate) requested: Option<f64>,   // None: scope is not All
    pub(crate) allocatable: f64,         // > 0, else the row is omitted
    pub(crate) unsampled_nodes: usize,   // nodes without a metrics sample
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CapacityRow {
    Cpu(Layers),                                                     // cores
    Memory(Layers),                                                  // bytes
    Pods { taking_room: Option<usize>, allocatable: u64 },           // None: scope is not All
    Volumes(VolumeTotals),
}
pub(crate) struct VolumeTotals { pub(crate) used: u64, pub(crate) capacity: u64, pub(crate) claims: usize, pub(crate) limited: Option<String> }
impl CapacityRow {
    pub(crate) fn name(&self) -> &'static str;      // "CPU", "Memory", "Pods", "Volumes"
    pub(crate) fn label(&self) -> String;           // right-hand figures, below
    pub(crate) fn note(&self) -> Option<String>;    // muted line under the bar
    pub(crate) fn ceiling(&self) -> Option<&'static str>; // tooltip (decision 13)
}
pub(crate) struct CapacityInputs<'a> {
    pub(crate) nodes: &'a [NodeSummary],
    pub(crate) pods: Option<&'a [PodSummary]>,           // Some only when the scope is All and pods are Ready
    pub(crate) node_usage: Option<&'a NodeUsageHistory>, // None unless the node feed is Live/Interrupted
    pub(crate) volumes: Option<VolumeTotals>,            // None unless the kubelet feed is Live/Interrupted
}
pub(crate) fn cluster_capacity(inputs: &CapacityInputs) -> Vec<CapacityRow>; // CPU, Memory, Pods, Volumes
pub(crate) fn volume_totals<'a>(usages: impl Iterator<Item = &'a PvcUsage>) -> VolumeTotals;
```

| Row | Built from (all through `node_usage.rs`, no second quantity math) | `label()` | `note()` |
|---|---|---|---|
| Cpu / Memory | used: Σ `latest(node)` over sampled nodes. requested: Σ `node_requests(node, pods)`. allocatable: Σ `node_allocatable` | each figure with its own unit → `104 cores used · 131 cores req · 168 cores`. A missing used prints `—`; a missing req drops its part | `used from {k} of {n} nodes` when `unsampled_nodes > 0` |
| Pods | taking_room: `takes_room` count. allocatable: Σ `node_pod_limit` | `1,284 running / 4,620 capacity` (`— running / …` while loading; `4,620 capacity` when the scope is not All) | none; the card carries the scope note |
| Volumes | `volume_totals(kubelet.history.pvc_usages())` | `Measure::Bytes.format_pair(used, capacity, " / ")` + ` · {claims} PVCs` | `limited` (`Volume usage: 10 of 42 nodes polled.`) |

- **Ceilings.** `ceiling()` for Cpu/Memory: `Init-container requests are not counted. Allocatable includes NotReady and cordoned nodes.`; for Pods, the second sentence only.
- **Volumes.** Only usages with `capacity: Some` count (`used: None` adds 0); `claims` counts them, and the row is omitted when `claims == 0` and no claim was skipped as shared (a row of skipped claims shows `—` and the note). A claim whose kubelet capacity is larger than its own size reports the node disk (0014 `is_shared_filesystem`, read against the always-on PVC condition feed): it is skipped and counted in `shared_claims`; `is_sharing_unchecked` is set while that feed is not Live. Without a polling feed the row shows `—`. The store is scoped (0011 `retain_scope`), so a narrower scope adds ` in {namespaces_label}`. `limited` uses the 0020 `FeedState::Limited` text. `pvc_usages()` comes from 0020; add it here if 0020 has not merged.
- **Cost.** `node_requests` scans pods once per node, O(pods × nodes) per render. Fold it into one pass only if `capacity_budget` (1,000 pods × 50 nodes, release) exceeds 2 ms. `ponytail:` per-node scan first.
- **Tones.** Used and requested figures are toned by `usage_tone(x / allocatable)`.

## `CapacityBar` (`usage_bar.rs`)

```rust
#[derive(Debug, PartialEq)]
pub(crate) struct CapacityBar { pub(crate) used: Option<f32>, pub(crate) requested: Option<f32> } // clamped 0..=1
impl CapacityBar { pub(crate) fn of_row(row: &CapacityRow) -> Self; }    // ratios over allocatable/capacity; NaN → 0
pub(crate) fn capacity_bar(bar: CapacityBar, cx: &App) -> impl IntoElement; // full width, 10 px, theme.radius
```

- **Bar layers.** Track `theme.muted`; requested `theme.foreground.opacity(0.28)`; used `theme.foreground` on top.
- **Row layout.** `v_flex().gap_1()`: an `h_flex` (bold name left, mono muted label right, toned figures), then the bar, then the note. The row tooltip is `ceiling()`. Panel body: `p_3().gap_3()`.

## Heatmap (`node_heatmap.rs`)

```rust
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HeatCell { pub(crate) node: String, pub(crate) usage: NodeUsage, pub(crate) readiness: NodeReadiness, pub(crate) is_cordoned: bool }
impl HeatCell {
    pub(crate) fn tone(&self) -> Option<StatusTone>;      // usage_tone of the higher of CPU and memory (warn 80 %, bad 90 %); None without a sample or when not Ready
    pub(crate) fn usage_line(&self, state: UsageState) -> String; // `CPU 31% · MEM 56%`; `Loading usage…` while the feed loads; the status when the feed is absent
    pub(crate) fn is_not_ready(&self) -> bool;     // NotReady or Unknown
    pub(crate) fn tooltip(&self) -> String;        // `ip-10-0-3-17 · CPU 62% · Memory 48% · Ready` (+ ` · SchedulingDisabled`); `—` when missing
}
pub(crate) fn heat_cells(nodes: &[NodeSummary], usage: Option<&NodeUsageHistory>) -> Vec<HeatCell>; // list order
pub(crate) fn node_heatmap(cells: &[HeatCell], state: UsageState, cx: &Context<AppShell>) -> impl IntoElement;
```

- **Render.** `h_flex().flex_wrap().gap(px(4.)).p_3()`. Each cell is a 168 px card, `id("node-{name}")` (stable across list changes): the node name (truncated, semibold) and the usage line (mono muted) under it. A cell with a tone gets that tone as its border and a 16 % tint (decision 20); a not-ready cell gets `border_2` Bad and its status as the line. Each cell has a tooltip; a click calls `shell.reveal(ResourceKey::Node { name })`. When the node feed is absent the card says `Node usage unavailable: {reason}.` once under the grid and the cells show their status only.
- **Inputs.** `usage` = `node_usage(node, history.latest(&node.name))` (0010); cordon comes from `NodeScheduling::Disabled`.
- **Size ceiling.** About 3 cells fit per row in a 560 px panel, so 500 nodes is ~170 rows and the page scrolls. `ponytail:` fixed 168 px cards; group by node pool above ~300 nodes.
