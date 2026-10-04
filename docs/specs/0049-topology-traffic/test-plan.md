# 0049 · Test plan

[Back to index](README.md). Offline and deterministic, one behaviour per test. Cluster tests answer through `fake_api` with fixture JSON bodies; app tests use a new `traffic_namespace()` built from the existing `topology_fixtures.rs` builders (namespace `payments`, as W11): Ingress `shop-api` → Service `payments-api` → Deployment `api` (ReplicaSet, 3 pods, one crashing); Service `ledger` → StatefulSet `ledger` (1 pod); Service `payments-legacy` with no pods; one host-network pod `node-agent-x7k2p` of a DaemonSet. No test opens a cluster connection.

## Fixture samples (`topology_fixtures.rs`)

| Name | Readings |
|---|---|
| `istio_sample` | `ledger` → `payments-api` 35 req/s, 2.1 errors/s (6 %); `api` → `ledger` 12 req/s, 0.05 errors/s; `unknown` → `payments-api` 4 req/s; `web/frontend` → `payments-api` 80 req/s |
| `bytes_sample` | three `api` pods 1.2 MB/s, 300 KB/s, 0 B/s receive; `ledger-0` 50 KB/s; `node-agent-x7k2p` 9 MB/s (host network); transmit pairs |

## Step 1 (cluster)

| Test | File | Checks |
|---|---|---|
| `metric_names_ask_for_the_last_hour` | `metrics_query_tests.rs` | path `…/label/__name__/values`, `start` = now − 1h, `end` = now; `data` array → set |
| `traffic_queries_per_source` | `promql_tests.rs` | golden text of both queries for Istio and PodNetwork |
| `detect_orders_by_priority` | `traffic_metrics_tests.rs` | both metrics → Istio, PodNetwork |
| `detect_on_uat_names_finds_only_pod_network` | `traffic_metrics_tests.rs` | the probe's present and absent names |
| `istio_rows_map_workload_to_service` | `traffic_metrics_tests.rs` | `Workload`, `Service`; foreign namespace and `unknown` → `Outside` |
| `pod_network_joins_receive_and_transmit` | `traffic_metrics_tests.rs` | one row per pod with both values |
| `label_values_are_cleaned` | `traffic_metrics_tests.rs` | a 200-char value with `\n` and `\u{1b}` → 80 chars, no control char |
| `failed_errors_query_keeps_totals` | `traffic_metrics_tests.rs` | errors 500 → `errors: None`, reading `Ok` |
| `failed_total_query_fails_the_reading` | `traffic_metrics_tests.rs` | `Err` |
| `traffic_reading_is_cut_at_2000_series` | `traffic_metrics_tests.rs` | `was_cut` |
| `invalid_namespace_sends_nothing` | `traffic_metrics_tests.rs` | `InvalidName`, no recorded request |

Live (coder-lite, UAT, read-only): `probe --metrics-source monitoring/vmselect-vm-victoria-metrics-k8s-stack:8481/select/0/prometheus --traffic monitoring` → `pod network bytes` only, rows ≈ pods of `monitoring`; trace shows only GETs to the vmselect proxy path.

## Step 2 (app, pure)

| Test | File | Checks |
|---|---|---|
| `route_edges_routes_a_slice` | `topology_route_tests.rs` | routing `&graph.edges` equals today's result in both shapes |
| `call_routes_leave_the_layout_alone` | `topology_traffic_tests.rs` | rects before and after routing the call edges are equal (Elbows and Curves) |
| `missing_pair_becomes_a_calls_edge` | `topology_traffic_tests.rs` | `ledger` → `payments-api` |
| `existing_edge_takes_the_flow` | `topology_traffic_tests.rs` | the `payments-api` → `api` `RoutesTo` edge carries the `api` pods' receive bytes, no `Calls` edge added |
| `istio_wins_over_bytes_on_an_edge` | `topology_traffic_tests.rs` | an edge with both → `Requests` |
| `unresolved_ends_count_as_outside` | `topology_traffic_tests.rs` | `web/frontend`, `unknown` |
| `pod_in_a_group_resolves_to_the_group` | `topology_traffic_tests.rs` | collapsed pods |
| `host_network_pods_add_no_bytes` | `topology_traffic_tests.rs` | `node-agent-x7k2p` ignored |
| `bytes_sum_once_per_pod` | `topology_traffic_tests.rs` | Service and workload over shared pods |
| `edges_without_requests_carry_target_bytes` | `topology_traffic_tests.rs` | `RoutesTo` and `Owns` |
| `mounts_and_access_are_hidden` | `topology_traffic_tests.rs` | `EdgeTraffic::Hidden` |
| `width_scales_by_square_root_per_unit` | `topology_traffic_tests.rs` | max → 6; a quarter → 3.75; units apart |
| `tone_thresholds` | `topology_traffic_tests.rs` | 0.9 % none, 1 % Warn, 5 % Bad; bytes no tone |
| `labels_per_unit_and_relation` | `topology_traffic_tests.rs` | req/s text, 5xx part ≥ 0.1 %, no label on `Owns` bytes |
| `label_anchor_is_the_arc_length_midpoint` | `topology_traffic_tests.rs` | an L-shaped elbow and a flattened curve |
| `bad_node_keeps_its_caption` | `topology_traffic_tests.rs` | the CrashLoopBackOff pod keeps its caption; tooltip has traffic |

## Step 3a (view state)

| Test | File | Checks |
|---|---|---|
| `traffic_button_state_per_source_state` | `topology_view_tests.rs` | every row of the state table, tooltip text |
| `first_traffic_click_loads_names_once` | `topology_view_tests.rs` | a second click reuses them |
| `relayout_reroutes_only_the_calls` | `topology_view_tests.rs` | a drag in Traffic mode keeps `calls`, recomputes `call_routes` |
| `hide_or_namespace_change_drops_the_fetch` | `topology_view_tests.rs` | fetch and timer gone |
| `failed_refresh_keeps_the_last_sample` | `topology_view_tests.rs` | chip `paused · …` |
| `chip_text_per_sources` | `topology_view_tests.rs` | Istio list; bytes-only wording |

## Step 3b (drawing)

| Test | File | Checks |
|---|---|---|
| `flow_edges_are_solid` | `topology_canvas_tests.rs` | a `RoutesTo` flow has no dash; an idle edge has `(2, 4)` |
| `export_in_traffic_mode_draws_tones_and_labels` | `topology_export_tests.rs` | SVG holds the label text, the Bad color on the edge path and its arrow `<polygon>`, `class="edge"`, the Traffic legend |

## Screens and ui-verifier

`topology-traffic-fixture` (Istio + bytes on `traffic_namespace()`, light and dark, Elbows and Curves) and live UAT `topology-traffic` (a namespace with pods, bytes fallback, `--config-dir .tmp/cfg-0048` with the vmselect entry). Checklist: segment `Traffic` selected; flow edges solid with varying widths; the `ledger` → `payments-api` edge Bad with its label at mid-route; idle edges thin and muted; chip text; legend entries; no label over a card at the first view.
