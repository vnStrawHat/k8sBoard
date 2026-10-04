# 0049 — Topology Traffic mode (W11)

Status: **draft 2026-10-04** against main `94e9c21`, amended after the opus advisor review (R8, first-cut sources, notes); approved by the user on 2026-10-04 together with 0048. **Starts after 0050 (curved edges) merges.** **Read-only: instant `query` GETs and one `label/__name__/values` per session** through the 0048 transport (`metrics_query.rs`, same single clippy exception). Crates: `crates/cluster` (step 1), `crates/app` (steps 2–3b). Prerequisites: 0048 steps 1–3a, 0022, 0022b, 0050. Roadmap: [wireframe-gap-audit.md](../../roadmap/wireframe-gap-audit.md) row W11 n1. Wireframes: W11 toolbar segment `Resources | Traffic`, note 1 ("Resources … and Traffic (needs a service mesh or eBPF, later phase)"), the legend, note 5 (Export PNG).

## Goal

- The **Traffic** segment works when the cluster has a Ready 0048 source: the same layout and card positions as Resources, with flow edges drawn **solid**, **width by rate**, a **label** with rate and 5xx share, and an **error tone**, over a fixed **5-minute** window refreshed every 30 s.
- First-cut sources: **Istio** request metrics (edges between workloads and Services) and, as the fallback, **per-pod network bytes** (cAdvisor), drawn on the Resources edges and marked as per-pod throughput, not traffic between objects. UAT has only the fallback.
- Without a source, or without either metric, the segment stays disabled with the reason in its tooltip.
- Works with both 0050 edge shapes (`Edges: Elbows | Curves`): traffic styling is per edge and ignores the shape.

## Non-goals

**Later sources** (each one row of the [traffic-sources.md](traffic-sources.md) table when a cluster needs it): OpenTelemetry service graph, ingress-nginx, Kong, OpenTelemetry span metrics, Linkerd, Hubble/Cilium, Pixie. Also: a window picker or history replay; objects outside the namespace as nodes (counted only); traffic in Overview, Issues, or drawers; persisting the mode; animated traffic edges; multi-namespace graphs (0022); edge hit-testing (0050 non-goal).

## Decisions

Full list in [decisions.md](decisions.md). Key: sources are table rows built in `promql.rs` (1) · the names list picks the rows once per session (2) · the Resources layout is never redone for traffic: only the extra `Calls` edges are routed, over the same rects (4) · width ∝ √(rate / max) per unit, tones at 1 % and 5 % 5xx (6, 7) · UAT has only the bytes fallback; Istio is verified with fixtures (9).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Cluster: `MetricsEndpoint::MetricNames` + `metric_names` (moved from 0048), `traffic_metrics.rs` (`TrafficSourceKind`, `TrafficSource::detect`, `TrafficEnd`, `TrafficRate`, `traffic_rates`), Istio and pod-network templates in `promql.rs`, probe `--traffic <ns>` | 1–4 |
| 2 | App, pure: `Relation::Calls` (stroke, legend, export arm), `route_edges` over an edge slice, `topology_traffic.rs` (resolve, call edges, aggregation, widths, tones, labels, node text, label anchors) | 1–3, 5–8 |
| 3a | App view: `TopologyMode`, segment state and reasons, names load, fetch and refresh, call-edge routing on sample and relayout, chip, `--screen topology-traffic` | 1–3, 9, 11, 12 |
| 3b | App drawing: `paint_edges` for flows (solid, widths, tones, idle, hidden), labels, node text, Traffic legend, export, `--screen topology-traffic-fixture`, ui-verifier | 1–3, 10, 13, 14 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale; UAT facts |
| [traffic-sources.md](traffic-sources.md) | step 1: names list, source table (first cut and later rows), queries, mapping, caps, API |
| [traffic-view.md](traffic-view.md) | steps 2–3b: overlay model, call-edge routing, resolution, look, chip, legend, degradation, refresh, export |
| [files-to-touch.md](files-to-touch.md) · [test-plan.md](test-plan.md) | files per step and doc follow-ups; tests, fixtures, live checks, screens |

## Acceptance criteria

- [ ] 1. Gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`; no new `#[allow]`; `clippy.toml` and `Cargo.lock` unchanged.
- [ ] 2. Every test in [test-plan.md](test-plan.md) for the step exists under its name and passes offline; no test opens a cluster connection.
- [ ] 3. Read-only: the only new requests are 0048-transport GETs to `…/api/v1/query` and, once per session (and per Reconnect), `…/api/v1/label/__name__/values?start={now−1h}`; every name in a query goes through `string_literal`/`regex_literal`; no `pub` function takes PromQL text.
- [ ] 4. Bounds: ≤ 2 queries per detected source per refresh, `time` = now, `[300s]` windows; a reading above 2,000 series is cut and noted; label values are cut to 80 chars with control characters stripped; 0048's 4 MiB and 20 s caps apply. UAT `probe --traffic monitoring` names `pod network bytes` as the only source and prints pod counts.
- [ ] 5. Istio and pod-network fixtures resolve to the expected nodes and edges; unresolved ends (`unknown`, other namespaces) count in `outside`; host-network pods add no bytes.
- [ ] 6. A pair with no Resources edge becomes one `Calls` edge; a pair on an existing edge in the same direction annotates it; Istio wins over bytes on an edge.
- [ ] 7. Bytes fallback: pod rates sum up to pod groups, ReplicaSets, workloads, Services (once per pod), and Ingresses; edges take their target's receive rate.
- [ ] 8. Card positions in Traffic mode equal the Resources layout: `layout()` is not called for a sample; only the `Calls` edges are routed (`route_edges` over the same rects and bands, with the current `EdgeShape`) and appended.
- [ ] 9. Disabled reasons: no source, invalid, checking, unreachable, no traffic metric — each tooltip text as [traffic-view.md](traffic-view.md).
- [ ] 10. Traffic look: flow edges solid, width 1.5–6 graph units by √ share per unit; idle edges thin and muted; Mounts and Access edges hidden; labels at the arc-length midpoint of the route, `{n} req/s · {p}% 5xx` or `{rate}`, at zoom ≥ `MIN_TEXT_ZOOM`; Warn ≥ 1 %, Bad ≥ 5 %; theme tokens only; the same in Elbows and Curves.
- [ ] 11. The chip reads `Traffic · Istio · last 5 min · {HH:MM:SS}` (plus `, pod network bytes` when both); bytes only: `Traffic · pod network bytes (per pod, not per connection) · last 5 min · {HH:MM:SS}`; a failed refresh keeps the last sample and shows `paused · {reason}`.
- [ ] 12. Refresh every 30 s only while Topology is visible in Traffic mode; a namespace change, mode change, hide, or session switch drops the fetch and timer.
- [ ] 13. Export PNG/SVG in Traffic mode draws widths, tones (stroke and the arrow `<polygon>` fill), labels, and the Traffic legend; edge paths keep `class="edge"`.
- [ ] 14. ui-verifier (light, dark, Elbows and Curves): `topology-traffic-fixture` (Istio + bytes) and live UAT `topology-traffic` (bytes fallback) show no high-severity defect against W11; the coder-lite UAT trace shows only the GETs of AC 3.

## Open items

1. The 5-minute window is fixed; add a window picker if users ask.
2. Istio is verified on fixtures only (UAT has no mesh); the first cluster with Istio metrics should get a live check recorded in as-built.
