# 0010 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; nothing lands before its first user (no dead code between steps).

## Cargo

No changes in any step. `DynamicObject`/`ApiResource` (kube), `serde_json`, `futures`, `tokio::time`, and the kit `plot`, `ButtonGroup`, `DropdownMenu`, `Alert` are already available. `git diff Cargo.lock` stays empty.

## `crates/cluster` (step 1)

| File | Change |
|---|---|
| `src/quantity.rs` (new) | `CpuAmount`, `ByteAmount`, private integer `Quantity` and `parse_quantity`; tests in module |
| `src/resource_metrics.rs` (new) + `resource_metrics_tests.rs` | `METRICS_INTERVAL`, `ResourceUsage`, `PodMetrics`, `ContainerMetrics`, `NodeMetrics`; `poll_pod_metrics`, `poll_node_metrics`; private `pod_metrics`, `node_metrics`, `concat_namespaces`, `poll_updates`, `next_delay` |
| `src/connection.rs` (+ `connection_tests.rs`) | `scoped_dynamic_apis(scope, &ApiResource)` |
| `src/access_review.rs` | `ListPodMetrics`, `ListNodeMetrics` (`ALL` becomes 21), `target`; `Display` adds the group when not core; `NamespaceAccess`, `review_namespaces` |
| `src/resource_watch.rs` | `WatchUpdate` doc mentions metrics polls |
| `src/lib.rs` | `mod quantity; mod resource_metrics;` and the exports |
| `examples/probe.rs` | `--metrics-seconds <n>`; usage text |

## `crates/app`

| S | File | Change |
|---|---|---|
| 2 | `src/usage_format.rs` (new) | `Measure` (`format`, `format_pair`), `format_percent`, `usage_tone`; tests in module (step 3 adds `format_offset`) |
| 2 | `src/history_rings.rs` (new) | `Timeline`, `Rings<P>`, fine push/align/freeing, constants; tests in module (step 3 adds the coarse fold, `RingPoint`, `Resolution`); shared with 0011 |
| 2 | `src/metrics_history.rs` (new) + `metrics_history_tests.rs` | fine rings: `FINE_TICKS`, `UsagePoint`, `PodUsageHistory`, `NodeUsageHistory` (step 3 adds coarse rings, `sampled_at`, `controller`, `UsageSeries`, `Resolution`, series calls, OOM marks) |
| 2 | `src/cluster_metrics.rs` (new) + `cluster_metrics_tests.rs` | `ClusterMetrics`, `MetricsFeed`, `FeedStatus`, `PodsGate`, `NodesGate`, `pods_gate`, `nodes_gate(access, check)`, `poll_error_text`, `is_metrics_settled` |
| 2 | `src/cluster_session.rs` | `LiveCluster.metrics`; pod review task; `update_metrics_feeds`; `set_scope` and `finish_access_review` hooks; the two subscriptions |
| 3 | `src/kind_row.rs` | `owns(owner, namespace, controller)`; `owns_pod` delegates |
| 2 | `src/node_usage.rs` (new) | `NodeUsage`, `node_usage`, `node_allocatable`, `node_requests`, `node_pod_count`; tests in module |
| 2 | `src/usage_bar.rs` (new) | `UsageBar`, `usage_bar` |
| 2 | `src/pod_table.rs` | `PodRow` (`TableRow` moves from `PodSummary`), CPU and Memory columns, CPU hidden by default, `NODE` → 6 |
| 2 | `src/node_table.rs` | `NodeRow`, CPU and Memory bar columns |
| 2 | `src/pod_drawer.rs` | container Info › Resources usage rows and bars |
| 2 | `src/node_drawer.rs` | Allocatable used section; formatted Resources rows |
| 2 | `src/main.rs` | module declarations |
| 3 | `src/usage_chart.rs` (new) + `usage_chart_tests.rs` | model, `UsageChart` (`#[derive(IntoPlot)]`, `Plot` impl), `usage_chart_card`, geometry |
| 3 | `src/monitor_data.rs` (new) + `monitor_data_tests.rs` | `MonitorSubject`, `MonitorInput`, `MonitorData`, `ScopeChoice`, `MonitorRow`, `monitor_data` |
| 3 | `src/monitor_tab.rs` (new) | toolbar, body by status, chart grid, Table view, note |
| 3 | `src/drawer.rs` | `DrawerTab::Monitor`, `ContainerTab::Monitor`, `MonitorState`, `MonitorRange`, `MonitorScope`, `MonitorKey`, `MonitorCache`; `drawer_tabs`, `tab_titles` |
| 3 | `src/pod_drawer.rs`, `node_drawer.rs`, `kind_drawer.rs` | Monitor bodies; container Monitor sub-tab |
| 3 | `src/resource_kind.rs` | `has_monitor` |
| 3 | `src/app_shell.rs` | `set_monitor_range`, `set_monitor_scope`, `toggle_monitor_table`; monitor cache refresh in `render`; scope reset on subject change; full reset in `show_screen` |
| 3 | `src/launch_options.rs` (+ tests) | `pod-monitor`, `node-monitor`, `{plural}-monitor`; usage text |
| 3 | `src/screenshot.rs` | monitor screens wait for `is_metrics_settled` |

## Docs (with the last step)

| File | Change |
|---|---|
| `docs/roadmap/inventory-screens.md` | W4-2, W4-11, W4c-1, W5-2 → Done (0010); W4-9 Monitor part, W4c-3 (Prometheus ranges), W5-4 → Partial or Done as built |
| `docs/roadmap/inventory-kinds.md` | Monitor tab row → Partial (Network/Disk in 0011); Nodes CPU/Mem done |
| `docs/roadmap/inventory-shell.md` | D2 (⤢ on kinds) and D3 (Monitor tab) → Done |
| `docs/roadmap/README.md` | status table: Drawer Monitor tab → Partial (0010), next 0011 |
| `docs/roadmap/cross-cutting.md` | Charts row: "own `Plot` impl on kit primitives (0010); no dependency" |
| `docs/specs/0008-pod-node-details/README.md` | open item 4 → "resolved in 0010" |
