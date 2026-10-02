# 0010 · App: usage columns and bars

[Back to index](README.md) · Step 2 · Modules: `pod_table.rs`, `node_table.rs`, `usage_bar.rs` (new, tests in module), `pod_drawer.rs` (container Info), `node_drawer.rs`, `node_usage.rs` (new, pure, tests in module)

## Rows for the 0009 toolkit

0009 implements `TableRow` on `PodSummary` and `NodeSummary`. 0010 moves both impls onto borrowed wrappers that also carry the newest usage; `rebuild_view` builds a `Vec` of them (same order as the session list, so item indices still index the session list) and passes it to `TableView::rebuild`.

```rust
pub(crate) struct PodRow<'a> { pub(crate) pod: &'a PodSummary, pub(crate) usage: Option<ResourceUsage> }
pub(crate) struct NodeRow<'a> { pub(crate) node: &'a NodeSummary, pub(crate) usage: NodeUsage }
// node_usage.rs
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct NodeUsage { pub(crate) cpu: Option<f64>, pub(crate) memory: Option<f64> } // used / allocatable
pub(crate) fn node_usage(node: &NodeSummary, latest: Option<ResourceUsage>) -> NodeUsage;
pub(crate) fn node_allocatable(node: &NodeSummary) -> (Option<CpuAmount>, Option<ByteAmount>);
pub(crate) fn node_requests(node: &str, pods: &[PodSummary]) -> (CpuAmount, ByteAmount);
pub(crate) fn node_pod_count(node: &str, pods: &[PodSummary]) -> usize;
```

- `node_usage`: ratio = usage / allocatable (`cpu`, `memory` entries of `NodeSummary.resources`, parsed with `CpuAmount`/`ByteAmount`); `None` without a sample, without the entry, or with zero allocatable.
- `node_requests`: sum of main + sidecar container `cpu`/`memory` requests of pods on the node whose status tone is not `Done` (decision 22 rule). `node_pod_count`: same pods, counted. Both are used only when the scope is All (decisions 18, 23).

## Pods columns

| Logical | Column | Width | `CellValue` | Cell |
|---|---|---|---|---|
| 0–3 | Name, Status, Ready, Restarts | 0009 | 0009 | 0009 |
| 4 | CPU (new, **hidden by default**) | 70 | `Number(nanocores)` or `Absent` | mono `Measure::Cpu.format`, muted "—" |
| 5 | Memory (new) | 80 | `Number(bytes)` or `Absent` | mono `Measure::Bytes.format`, muted "—" |
| 6, 7 | Node, Age | 0009 | 0009 | 0009 |

- The Pods `TableView` is created with `hidden = {CPU}`; Columns ▾ lists CPU unchecked. A context switch keeps `hidden` (0009 decision 11).
- The `NODE` const moves to 6; "View pods on node" (0009) uses the const, so nothing else changes.
- The text filter does not match usage numbers (`Number` is not text, 0009 rule).
- A new pod reads "—" for its first 15–30 s: metrics-server needs two scrapes before it reports a pod (decision 16).

## Nodes columns

| Logical | Column | Width | `CellValue` | Cell |
|---|---|---|---|---|
| 0–5 | Name, Status, Roles, Taints, Version, Internal IP | 0009 | 0009 | 0009 |
| 6 | CPU (new) | 96 | `Number(round(ratio × 1000))` or `Absent` | `usage_bar` 46 px + mono `format_percent` |
| 7 | Memory (new) | 96 | same | same |
| 8 | Age | 0009 | 0009 | 0009 |

Bar fill and text color follow `usage_tone(ratio)` (W5: 82 % is Warn). No sample (NotReady, metrics off) → muted "—".

## Usage bar (`usage_bar.rs`)

```rust
pub(crate) struct UsageBar {
    pub(crate) fill: f32,              // clamped to 0..=1
    pub(crate) marker: Option<f32>,    // request tick, clamped; None hides it
    pub(crate) tone: Option<StatusTone>,
}
pub(crate) fn usage_bar(bar: UsageBar, width: Pixels, cx: &App) -> impl IntoElement;
```

Height 6 px, `theme.radius`-rounded track `theme.muted`; fill `tone_color(tone)` or `theme.muted_foreground`; marker 2 px wide, 12 px tall, `theme.foreground`, centered on the track (W4b `rbar`). One element for table cells and drawers.

## Container Info › Resources (0008 section)

For the `cpu` and `memory` rows of a container, with `latest_container(ns, pod, name)`:

| Usage | Limit | Row value |
|---|---|---|
| none | any | 0008 text, unchanged |
| some | set | `format_pair(usage, limit, " of ")` (`498 of 512Mi`, `310m of 1 core`; mono, toned by `usage_tone(usage / limit)`), `usage_bar` full width (fill usage / limit, marker request / limit), muted `text_xs` `request {request}` when set |
| some | none | `{usage} used`, muted `request {request} · no limit` (or `no request · no limit`); no bar |

**BestEffort containers.** A container with usage but no `cpu` or `memory` request or limit (no resource entry) gets an empty row for each missing one, so its usage still shows as `{usage} used` with `no request · no limit`. Rows are copied and ordered `cpu`, `memory`, the rest only when a row is added; otherwise the container's own list is used as is. `format_pair` keeps the unit on millicores (`44m of 300m`, never `44 of 300m`); cores and bytes share it (`1 / 15.8 cores`, `498 of 512Mi`).

Values use `Measure::format`/`format_pair`; the request keeps 0008's as-written text only in the first row case. The decision is a pure helper in `pod_drawer.rs`, rendered by the view:

```rust
pub(crate) struct UsageRow { pub(crate) value: String, pub(crate) tone: Option<StatusTone>,
    pub(crate) bar: Option<UsageBar>, pub(crate) note: Option<String> }
/// `None` when the row keeps 0008's text (no usage, or not cpu/memory).
pub(crate) fn container_usage_row(resource: &ContainerResource, usage: Option<ResourceUsage>) -> Option<UsageRow>;
```

## Node drawer

- New section **Allocatable used** after Conditions (W7 Nodes drawer order): rows CPU and Memory with `format_pair(used, allocatable, " / ")` (`9.8 / 15.8 cores`, `43 / 61Gi`) and a `usage_bar` (fill ratio, marker requested / allocatable when the scope is All); row Pods `{n} / {pods allocatable}` with a bar, only when the scope is All.
- Without a sample: values "—", no bars. With the nodes feed `Unavailable`/`Failed`: one muted line `Usage unavailable: {reason}` instead of the rows.
- **Resources** rows (0008): `cpu` via `Measure::Cpu`; `memory`, `ephemeral-storage`, `hugepages-*` via `Measure::Bytes`; anything else, or text that fails to parse, as written (0008 open item 4). Pure helper `node_quantity_text(resource: &str, text: &str) -> String` in `node_usage.rs`.
