# 0049 · Decisions

[Back to index](README.md).

| # | Decision | Rationale |
|---|---|---|
| 1 | Each traffic source is one row of a table: a metric name, a "total" query, an "errors" query, and a label mapping; the queries are built in `promql.rs` from the namespace only | A later source (service graph, ingress-nginx, Kong, span metrics, Linkerd, Hubble) is one row; escaping stays in one tested place (0048 decision 4) |
| 2 | Which rows run is decided by `metric_names` (`label/__name__/values` with `start` = now − 1h, so stale names drop out), loaded once per session on the first Traffic use | One 91 KB request on UAT instead of probing each metric every refresh; moved here from 0048, which has no reader for it |
| 3 | Instant queries at now over a fixed `[300s]` window, refreshed every 30 s | Matches "what is flowing now"; a window picker is open item 1 |
| 4 | The Resources layout is used as is: `layout()` is not called for traffic. The extra `Calls` edges are routed with `route_edges(edges, rects, bands, shape)` over the layout's rects and bands and kept beside it; a sample or a relayout (drag, shape change) re-routes only them | Mode switches and samples move no card (AC 8 by construction); no per-sample layout cost |
| 5 | Traffic mode hides `Mounts` and `Access` edges but keeps their nodes; `Owns` stays, dimmed unless it carries bytes | Hiding nodes would change the layout |
| 6 | Width = `1.5 + 4.5 × √(rate / max)` graph units, `max` over flows of the same unit (req/s or bytes/s) in the graph | √ keeps small flows visible next to one big one; units never share a scale |
| 7 | Tone from the 5xx share: Warn ≥ 1 %, Bad ≥ 5 %; no tone without an errors query | Common SLO thresholds; theme status tokens (0003) |
| 8 | First cut: Istio (edge-level) and pod network bytes (fallback). Istio wins over bytes on an edge | Istio is the most common mesh with a stable metric; bytes is all UAT has. The advisor scoped the rest out (README non-goals) |
| 9 | UAT gets the bytes fallback only; Istio is verified by fixtures | Probe 2026-10-04: `istio_requests_total` and every other edge-level metric absent; `container_network_{receive,transmit}_bytes_total` present |
| 10 | Names resolve inside the Topology namespace only; anything else counts in `outside` and is listed in the chip tooltip | The graph is one namespace (0022) |
| 11 | A node with a Warn or Bad caption keeps its caption; traffic text goes to its tooltip | A crash must stay visible in Traffic mode |
| 12 | Series cap 2,000 per reading; label values cut to 80 chars, control characters stripped, before they reach a tooltip or label | Bounded memory; backend label values are untrusted text |
| 13 | Flow edges are drawn solid, whatever their relation's dash (`RoutesTo` is dashed in Resources mode) | Width must read as rate; dashes break that |
| 14 | Host-network pods add no bytes (0011 decision 21) | Their counters are the node's; UAT `monitoring` runs node-exporter on the host network |
| 15 | No new dependency, no new cluster module besides `traffic_metrics.rs` | Reuses the 0048 transport, decoding, errors, and source |

## UAT facts (2026-10-04)

| Fact | Value |
|---|---|
| Mesh, CNI | no Istio, Linkerd, or Cilium CRDs; Calico CNI |
| Gateways | Kong in namespace `kong` (no Kong Prometheus metrics in the source), no ingress-nginx metrics |
| OpenTelemetry | `opentelemetry/opentelemetry-collector` 6/6; no span-metrics or service-graph series in the source |
| Fallback data | `container_network_receive_bytes_total`, `container_network_transmit_bytes_total` present |
| Retention | 60 days (0048) |
