# 0049 · As built

[Back to index](README.md). What differs from the spec, and the facts the live checks settled. Steps 1, 2, 3a, and 3b.

## Commits

Step 1 is its own commit. Steps 2, 3a, and 3b ship as one: the overlay, the `Calls` edges, the Traffic state and fetch, and the drawing read each other's items, and clippy flags each as dead code until its reader exists (the 0048 precedent).

## Deviations

| # | Spec | Built | Why |
|---|---|---|---|
| 1 | `TrafficSource` | `TrafficMetricSource` (kind `TrafficSourceKind` as specced) | `TrafficSource` is the network-policy test type already exported by the cluster crate |
| 2 | `MetricsEndpoint` read by the one exception | `instant_query` (`pub(crate)`, takes PromQL text) is the only reader beside `check_metrics_source` and `usage_range`; `Envelope<T>`, `decode<T>`, and an `InstantRow` decoder are shared | `traffic_rates` lives in `traffic_metrics.rs`; no `pub` function takes query text |
| 3 | `call_edges(graph, &[PodSummary], ..)`, same for the overlay | `&[&PodSummary]` | The view passes the pods of the namespace without cloning them |
| 4 | `ClusterMetrics.traffic_sources: Option<Result<..>>` | `TrafficSources { NotLoaded, Loading { _task }, Loaded(Result<..>) }` | The load needs an in-flight state so a second click sends nothing; the entry resets to `NotLoaded` when the stored source changes, and a reconnect builds a new `ClusterMetrics` |
| 5 | A 30 s `cx.spawn` timer | The existing 500 ms topology tick starts the fetch when the last one is 30 s old | One timer fewer; it already stops on hide, and every other drop rule drops the fetch the same way |
| 6 | `TrafficLayer` in `topology_view.rs` | In `topology_traffic.rs`, behind `Rc` fields (`sample`, `overlay`) | The canvas and the export read it; a drag clones two `Rc`s and re-routes the `Calls` only |
| 7 | Labels at the arc-length midpoint | The midpoint first, then 0.58, 0.42, 0.66, 0.34, 0.74, 0.26 of the arc length, the first spot that is on the canvas and clear of every card and of the placed labels; none clear means no label | A gutter (50 units) is narrower than a label, so the midpoint of an edge in a gutter is beside a card almost always. `label_anchor` is the midpoint and is tried first |
| 8 | Legend: swatches `≥ 1% 5xx`, `≥ 5% 5xx` always | The tone swatches and `calls` only when Istio is among the sources that answered; `owns` is a solid flow swatch | Bytes flows have no tone and make no `Calls` edge |
| 9 | `LEGEND` arm for `Calls` | `Calls` is in the Traffic legend only (`legend_entries`); the Resources legend is unchanged | Resources mode has no `Calls` edge |
| 10 | HPA to workload `Owns` edge carries the target's bytes | It stays `Idle`, and an HPA is not an ancestor when pods are summed up | A scaler is not in the data path |
| 11 | Namespace `payments` in the fixtures | `shop`, like every other topology fixture | The builders in `topology_fixtures.rs` are fixed to `shop` |
| 12 | `istio_sample`, `bytes_sample` in `topology_fixtures.rs` | In `topology_traffic_fixture.rs`, which also holds a hand-written copy of the fixture graph and pods for the screenshot build; a test checks the copy equals what `build_topology` makes | `topology_fixtures.rs` is test-only, and the `topology-traffic-fixture` screen needs the data in a screenshot build |
| 13 | Screens `topology-traffic`, `topology-traffic-fixture` | Also `topology-traffic-curves` and `topology-traffic-fixture-curves` | The ui-verifier checks both edge shapes |
| 14 | Session tests in `topology_view_tests.rs` | `first_traffic_click_loads_names_once` and two more in `app_shell_metrics_tests.rs` | They need the session fixtures of that file |
| 15 | A failed names read retried "at the next refresh" | A `Loaded(Err)` is read again once the last try is 30 s old | Same, driven by the tick |

Unchanged: `MetricsEndpoint::MetricNames` (`label/__name__/values`, `start` = now − 1 h); `metrics_get` stays the one clippy exception; no `#[allow]`; `clippy.toml` and `Cargo.lock` untouched.

## Live facts (UAT `readonly@Monitor`, 2026-10-04)

| Fact | Result |
|---|---|
| `probe --metrics-source monitoring/vmselect-vm-victoria-metrics-k8s-stack:8481/select/0/prometheus --traffic monitoring` | 2,210 names in 42 ms; sources: `pod network bytes` only; 19 rows, all with traffic, not cut, 88 ms |
| The same for `argocd` | 7 rows, all with traffic, 62 ms |
| `topology-traffic --namespace argocd` and `monitoring` | The Traffic segment is selected, the chip reads `Traffic · pod network bytes (per pod, not per connection) · last 5 min · {HH:MM:SS}`, edges carry the bytes of their target pods (`↓ 20 KB/S ↑ 11 KB/S` on Services, workloads, and Ingress, `20 KB/s` on a Service to pod edge), idle edges are thin and dotted, Mounts and Access are not drawn |
| Host-network pods | `node-exporter` (host network) adds no bytes and is not listed as outside |
| Istio | Not on UAT; verified with the fixtures only (open item 2 of the README stays) |

## Screens (`.tmp/shots-0049/`, light unless named)

`topology-traffic-fixture` (light, dark, curves-light): the 35 req/s · 6 % 5xx call `ledger` → `payments-api` is Bad with its label, the 12 req/s call is neutral, Service to pod edges carry 1.2 MB/s and 300 KB/s, the crashing pod keeps its caption, the idle edges are dotted. `topology-traffic-argocd-{light,dark}`, `topology-traffic-monitoring-light`, `topology-traffic-monitoring-curves-dark`: the live bytes fallback.

## Not done

- The coder-lite UAT trace of AC 14 (only GETs): the only code that sends is `metrics_get`, covered by the fake-server tests (`metric_names_ask_for_the_last_hour`, `traffic_queries_per_source`), and the probe and the screens above ran without a write.

## After the review

- A pod, Service, or workload of the namespace that is not a node (ended, not delivered yet, hidden by a filter) is skipped, not counted as an outside peer; only `TrafficEnd::Outside` is.
- A source in the `Checking` state is waited for; Traffic mode ends only for a missing, invalid, or unreachable source.
- Istio's `source_workload` carries no kind, so a Deployment wins over a StatefulSet or DaemonSet of the same name.
