# 0049 · Traffic overlay and view (steps 2–3b)

[Back to index](README.md) · Modules (app): `topology_traffic.rs` (new, pure, no GPUI context) + `topology_traffic_tests.rs`, `topology_graph.rs`, `topology_route.rs`, `topology_canvas.rs`, `topology_export.rs`, `topology_fixtures.rs` (step 2); `topology_view.rs`, `cluster_metrics.rs` (3a); `topology_canvas.rs`, `topology_card.rs`, `topology_export.rs`, `launch_options.rs`, `screenshot.rs` (3b). Base: 0050 merged (`EdgeShape`, `route_edges(.., shape)`, `ports()` + `bezier()`, export `<polygon>` arrows, `class="edge"`).

## Graph and routing (step 2)

- `Relation::Calls` (after `Access`): stroke width 1.5, solid, the `RoutesTo` accent color; legend text `calls`; export arm. Only Traffic mode creates such edges; they never enter `TopologyGraph.edges`.
- `route_edges(edges: &[TopologyEdge], rects) -> Vec<EdgeRoute>`: takes the edge slice instead of the graph (it only reads `from`, `to`, `relation`); `layout()` passes `&graph.edges`. No other routing change.

## Overlay (`topology_traffic.rs`)

```rust
pub(crate) struct TrafficSample { pub(crate) at: jiff::Timestamp,
    pub(crate) readings: Vec<(TrafficSource, Result<TrafficReading, MetricsError>)> }
/// Pairs with no Resources edge, as `Calls` edges (indexes into `graph.nodes`).
pub(crate) fn call_edges(graph: &TopologyGraph, pods: &[PodSummary], sample: &TrafficSample) -> Vec<TopologyEdge>;
pub(crate) fn traffic_overlay(graph: &TopologyGraph, calls: &[TopologyEdge], pods: &[PodSummary],
    sample: &TrafficSample) -> TrafficOverlay;
pub(crate) struct TrafficOverlay {
    pub(crate) edges: Vec<EdgeTraffic>,          // graph.edges, then calls, same order
    pub(crate) nodes: Vec<Option<NodeTraffic>>,  // one per graph node
    pub(crate) sources: Vec<TrafficSourceKind>,  // that produced data, for the chip
    pub(crate) outside: Vec<String>,             // unresolved ends, deduplicated, at most 20 kept
    pub(crate) notes: Vec<String>,               // cut readings, failed sources
}
pub(crate) enum EdgeTraffic { Hidden /* Mounts, Access */, Idle, Flow(EdgeFlow) }
pub(crate) struct EdgeFlow { pub(crate) rate: f64, pub(crate) unit: TrafficUnit, pub(crate) error_share: Option<f64>,
    pub(crate) width: f32, pub(crate) tone: Option<StatusTone>, pub(crate) label: Option<SharedString> }
#[derive(Clone, Copy, PartialEq, Eq)] pub(crate) enum TrafficUnit { Requests, Bytes }
pub(crate) struct NodeTraffic { pub(crate) requests: Option<f64>, pub(crate) error_share: Option<f64>,
    pub(crate) receive: Option<f64>, pub(crate) transmit: Option<f64>, pub(crate) text: SharedString }
/// The point at half the arc length of a route's polyline (works for any polyline).
pub(crate) fn label_anchor(route: &EdgeRoute) -> GraphPoint;
```

### Resolution (inside the Topology namespace, decision 10)

| End | Node |
|---|---|
| `Workload(n)` | the Deployment, StatefulSet, or DaemonSet node named `n` |
| `Service(n)` | the Service node `n` |
| `Pod(n)` | the pod's node, or the `PodGroup` that holds it (by the pod's controller in `pods`); a host-network pod → no node, no bytes (decision 14) |
| `Outside(s)`, anything not found | no node; `s` (or the name) goes to `outside` |

### Rules

1. Istio rows with both ends resolved and distinct: an existing edge between the two nodes in that direction (`RoutesTo`, `Owns`) gets the flow; otherwise `call_edges` made a `Calls` edge for it. Rows on the same edge add up. A row with an unresolved `from` adds to the target node's `requests` and errors only.
2. PodNetwork: each pod's receive and transmit go to its node; a node's bytes are the sum over the distinct pods reachable from it along `Owns` and `RoutesTo` edges (pod group, ReplicaSet, workload, Service, Ingress). Every `RoutesTo` or `Owns` edge without an Istio flow gets a `Bytes` flow of its target's receive rate.
3. `Mounts` and `Access` edges → `Hidden`; any other edge without flow, or with a rate of 0 → `Idle`.
4. Width (decision 6): `1.5 + 4.5 × √(rate / max)`, `max` over flows of the same unit; a single flow gets 6.
5. Tone (decision 7): `error_share` = errors ÷ requests; ≥ 5 % Bad, ≥ 1 % Warn, else none; bytes flows have no tone.
6. Labels: requests `"{rate} req/s"` (1 decimal below 10) plus `" · {p}% 5xx"` when the share is ≥ 0.1 %; bytes labels (`Measure::Rate`, `1.2 MB/s`) only on `RoutesTo` edges; `Owns` bytes edges have none.
7. Node text: `"{n} req/s · {p}% 5xx"` when `requests` is known, else `"↓ {rx}  ↑ {tx}"`; it replaces the caption only when the node's tone is not Warn or Bad (decision 11); the tooltip always ends with it.

## View state and fetch (step 3a, `topology_view.rs`)

- `TopologyMode { Resources, Traffic }`, `Resources` at start; not persisted. The segment's selected button follows it. The 0050 `Edges:` dropdown stays as is and applies in both modes.
- `TrafficLayer { sample: TrafficSample, calls: Vec<TopologyEdge>, call_routes: Vec<EdgeRoute>, overlay: TrafficOverlay }`, `Option` on the view, `Some` only in Traffic mode with a sample. Built on a new sample and after every `relayout`/`install` (drag, shape change, graph rebuild): `call_edges` → `route_edges(&calls, &layout.rects, &band_rects, self.edge_shape)` → `traffic_overlay`. `layout()` is never called for it (decision 4).
- **Traffic button state** from `live.metrics.source` (0048 `SourceState`) and the session's traffic sources:

| State | Button | Tooltip |
|---|---|---|
| `None` | disabled | `Choose a metrics source in Settings › Metrics` |
| `Invalid` | disabled | `The metrics source in Settings is not valid` |
| `Checking` | disabled | `Checking the metrics source…` |
| `Failed` | disabled | `Metrics source unreachable: {reason}` |
| `Ready`, names not loaded | enabled | `Show traffic from {source display}` |
| `Ready`, names loaded, `detect` empty | disabled | `No traffic metrics in {source display}` |
| `Ready`, names failed | enabled | `Show traffic (the metric list failed: {reason}; retrying)` |

- `ClusterMetrics.traffic_sources: Option<Result<Vec<TrafficSource>, MetricsError>>`, loaded by the first Traffic click (`metric_names` + `detect`), kept for the session, reloaded on Reconnect; a failure is retried at the next refresh.
- Fetch: `traffic_rates` for every detected source on `ClusterRuntime`, joined into one `TrafficSample`, one `cx.notify()`. A 30 s `cx.spawn` timer starts the next one. Fetch and timer are dropped on mode Resources, `set_visible(false)`, a namespace change, or `set_session`.
- **Chip** (right of the toolbar; the checks chip stays left of it): `Traffic · {labels joined by ", "} · last 5 min · {HH:MM:SS}`; bytes only: `Traffic · pod network bytes (per pod, not per connection) · last 5 min · {HH:MM:SS}`; before the first sample `Traffic · loading…`; a failed refresh `Traffic · paused · {reason}` (the last sample stays drawn). Tooltip: `notes`, then `{n} peers outside this namespace: a, b, …`.

## Drawing (step 3b)

- **Canvas** (`paint_edges`): iterates `graph.edges` with `layout.routes`, then `calls` with `call_routes`, reading `overlay.edges[i]`. `Hidden` not drawn; `Idle` 0.75 px, muted, dashed `(2, 4)`; `Flow` **solid** (decision 13) at its width × zoom, `tone_color(tone)` or the relation color, `Owns` flows at 60 % alpha; the arrow takes the same color. No flow animation in Traffic mode.
- **Labels**: small `text_xs` mono divs centered on `label_anchor(route)` (the arc-length midpoint), on the theme background with a border, at zoom ≥ `MIN_TEXT_ZOOM`, skipped when they would overlap a card.
- **Node text** (`topology_card.rs`): rule 7.
- **Legend** in Traffic mode: `routes to · width = req/s` (bytes only: `width = receive bytes/s per pod`), `calls`, `owns`, a Warn swatch `≥ 1% 5xx`, a Bad swatch `≥ 5% 5xx`.
- **Export** (`topology_export.rs`): the same widths and tones; the tone also fills the arrow `<polygon>`; edge paths keep `class="edge"`; labels as `<text>` at the anchor; the Traffic legend.

## Degradation summary

| Source holds | Traffic shows |
|---|---|
| Istio (and pod network) | request edges with tones; new `Calls` edges; bytes on the other edges |
| pod network only (UAT) | bytes widths and node throughput; chip says per pod, not per connection |
| neither | button disabled (`No traffic metrics …`) |
