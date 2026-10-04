# 0049 · Files to touch

[Back to index](README.md). **S** = step. Each step passes the gate alone. Base: main with 0048 steps 1–3a and **0050 merged** (`EdgeShape`, `route_edges(.., shape)`, export `<polygon>` arrows); 0048 steps 3b and 4 are independent.

## Cluster crate

| S | File | Change |
|---|---|---|
| 1 | `metrics_query.rs` | `MetricsEndpoint::MetricNames`, `metric_names` (`start` = now − 1h), an instant-vector decoder shared with `check_metrics_source`; `metrics_get` stays the one exception |
| 1 | `traffic_metrics.rs` (new) + `traffic_metrics_tests.rs` | `TrafficSourceKind`, `TrafficSource`, `detect`, `TrafficEnd`, `TrafficRate`, `TrafficReading`, `traffic_rates`, row mapping, label cleaning, `TRAFFIC_SERIES_LIMIT` |
| 1 | `promql.rs` + `promql_tests.rs` | Istio and pod-network templates |
| 1 | `lib.rs` | `mod traffic_metrics;` and `pub use` of the public types |
| 1 | `examples/probe.rs` | `--traffic <namespace>` |

## App crate

| S | File | Change |
|---|---|---|
| 2 | `topology_graph.rs` | `Relation::Calls` |
| 2 | `topology_route.rs` | `route_edges` takes `&[TopologyEdge]`; `layout()` passes `&graph.edges` |
| 2 | `topology_canvas.rs` | `relation_stroke`, `edge_color`, `LEGEND` arms for `Calls` |
| 2 | `topology_export.rs` | `Calls` arm |
| 2 | `topology_traffic.rs` (new) + `topology_traffic_tests.rs` | `TrafficSample`, `call_edges`, `traffic_overlay`, `TrafficOverlay`, `EdgeTraffic`, `EdgeFlow`, `TrafficUnit`, `NodeTraffic`, `label_anchor` |
| 2 | `topology_fixtures.rs` | `traffic_namespace()` plus `istio_sample` and `bytes_sample` |
| 3a | `cluster_metrics.rs` | `ClusterMetrics.traffic_sources`, load and reload |
| 3a | `topology_view.rs` | `TopologyMode`, `TrafficLayer`, segment state and tooltips, names load, fetch, timer, call routing after each relayout, chip |
| 3a | `launch_options.rs`, `screenshot.rs` | `topology-traffic` (live) |
| 3b | `topology_canvas.rs` | `paint_edges` reads `EdgeTraffic` (solid flows, idle, hidden), label divs at `label_anchor`, Traffic legend entries |
| 3b | `topology_card.rs` | node text and tooltip from `NodeTraffic` |
| 3b | `topology_export.rs` | widths, tones on stroke and arrow polygon, labels, Traffic legend |
| 3b | `launch_options.rs`, `screenshot.rs` | `topology-traffic-fixture` |

## Docs (in the step that ships the code)

| S | File | Change |
|---|---|---|
| 1 | `docs/specs/0048-prometheus-metrics/source-and-transport.md` | `MetricsEndpoint` gains `MetricNames` (already noted) |
| 3b | `docs/specs/0022-topology/README.md` | non-goals: Traffic mode → 0049 |
| 3b | `docs/roadmap/wireframe-gap-audit.md` | W11 n1 row → Done (0049) |
| 3b | `as-built.md` (new, this folder) | deviations, the UAT live check, fixture screenshots |
