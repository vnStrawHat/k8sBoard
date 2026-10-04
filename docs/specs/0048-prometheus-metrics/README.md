# 0048 — Prometheus-compatible metrics source, Settings › Metrics, 30-day Monitor ranges

Status: **draft 2026-10-04** against main `81bbcaa`, amended after the opus advisor review (R1–R7 and notes); approved by the user on 2026-10-04 ("viết spec cho Prometheus và trang Metrics, chế độ Traffic trong Topology và implement"). **Read-only: GET requests only** (`query`, `query_range`; 0049 adds `label/__name__/values`) through the API server service proxy with the session's own client; no new mutating path, no stored credential. Crates: `crates/cluster` (steps 1–2), `crates/app` (steps 3–4). Prerequisites: merged 0010, 0011, 0024, 0025, 0030, 0043, 0046. Consumer: 0049 (Topology Traffic). Roadmap: [wireframe-gap-audit.md](../../roadmap/wireframe-gap-audit.md) row W4c n1 / W2 Metrics. Wireframes: W2 nav `Metrics` and the cluster form `Metrics · Source metrics-server ▾`; W4c note 1 (longer ranges need Prometheus), note 6 (source named in the footer), `.mon-note` ("Connect Prometheus in Settings › Metrics to see up to 30 days"); stack table row "Metrics for the Monitor tab".

## Goal

- A per-cluster **metrics source**: a Prometheus-compatible HTTP API (Prometheus, VictoriaMetrics single or cluster, Thanos Query, Mimir) reached as `/api/v1/namespaces/{ns}/services/{scheme}:{svc}:{port}/proxy{prefix}/api/v1/…` with the user's kubeconfig auth and the 0043 per-cluster proxy, because it rides the session's `ClusterConnection`.
- **Settings › Metrics** page: read-only auto-detection of candidate services in the active cluster, manual entry, Test, Save. The W2 cluster form gets its drawn `Metrics · Source ▾` row.
- **Monitor tab**: with a reachable source, every chart (CPU, Memory, Network, Disk I/O) comes from the source and two more ranges appear, **7d** and **30d**; without one, today's metrics-server sampler and kubelet feeds stay as they are.

## Non-goals

A direct URL (outside the cluster) or any credential field, token, header, or keyring (decision 3); arbitrary PromQL from the UI; Prometheus for the Pods/Nodes columns, Overview, or node heatmap (they keep the sampler); alerts, recording rules, dashboards; series discovery beyond one names list; several sources per cluster; the Extensions page; anything multi-cluster (0046).

## Decisions

Full list with rationale in [decisions.md](decisions.md). Key: service proxy only, no direct URL (1, 3) · the cluster crate builds every PromQL text from typed targets, the app never sends free text (4) · one named clippy exception in `metrics_query.rs`, `Client::send` with a capped body (5) · source stored in `registry.clusters[].metrics` without credentials (6) · all Monitor charts from the source when it is Ready, sampler fallback for ≤ 24h on error (8, 9) · fixed step table, ≤ 400 points, 4 MiB body, 20 s deadline (10).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Cluster, pure: `promql.rs` (escaping, `UsageTarget`, `UsageMetric`, query text, `RangeSpec`), `metrics_source.rs` (`MetricsSource`, validation, serde fields, `metrics_candidates`) | 1–4 |
| 2 | Cluster transport: `metrics_query.rs` (endpoint allow-list, the named `send` exception, capped body, deadline, status mapping, decode, `MetricsError`), `list_all_services`, probe `--metrics-source` (settles the [promql.md](promql.md) facts); 0030 table row | 1–6 |
| 3a | App session: `registry.clusters[].metrics`, `ClusterProfile.metrics`, `SourceState` and its check, `ActiveConnection` global | 1–3, 8 |
| 3b | App UI: `metrics_page.rs`, `SettingsPage::Metrics`, cluster form row, `--screen settings-metrics[-fixture]` | 1–3, 7–9, 13 |
| 4 | App Monitor: `MonitorRange::{Days7, Days30}`, `monitor_source.rs` (fetch, refresh, chart models), fallback and notes, Table view, `--screen pod-monitor-source-fixture`, ui-verifier | 1–3, 10–14 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale; UAT probe facts |
| [source-and-transport.md](source-and-transport.md) | steps 1–2: source type, detection table, request path, allow-list, caps, errors, clippy row |
| [promql.md](promql.md) | step 1: escaping, targets, query table, step table |
| [settings-page.md](settings-page.md) | steps 3a (settings key, session state) and 3b (Metrics page, cluster form row, screens) |
| [monitor-ranges.md](monitor-ranges.md) | step 4: ranges, fetch and refresh, fallback, notes, Table view |
| [files-to-touch.md](files-to-touch.md) · [test-plan.md](test-plan.md) | files per step, doc follow-ups; tests, live checks, screens |

## Acceptance criteria

- [ ] 1. Gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`; no new `#[allow]` but the one named in AC 5; no `unsafe`; `Cargo.lock` gains no `[[package]]` (step 2 adds `form_urlencoded` and `http-body-util` as direct dependencies of `k8sboard-cluster`; both are already locked).
- [ ] 2. Every test in [test-plan.md](test-plan.md) for the step exists under its name and passes offline; no test opens a cluster connection.
- [ ] 3. Read-only: every new request is a `GET` whose path is `…/services/{scheme}:{svc}:{port}/proxy{prefix}/api/v1/query` or `…/query_range` or a `list services`; `object_write.rs` and the connect files are unchanged; `crates/app` still has no `kube` dependency.
- [ ] 4. Every Kubernetes name in a query passes its DNS check and goes through `string_literal` or `regex_literal`; tests cover `"`, `\`, `.`, and a newline; the app cannot pass PromQL text (no `pub` function takes a query string).
- [ ] 5. `clippy.toml` is unchanged; `metrics_query.rs` holds exactly one `#[allow(clippy::disallowed_methods)]` (on `metrics_get`, the one function that calls `Client::send`, not through `ClusterConnection::run`), with a comment naming its row; the row is added to the 0030 [write-path.md](../0030-guardrails-write-path/write-path.md) exception table and grep paragraph.
- [ ] 6. Bounds: success and error bodies over 4 MiB fail `TooLarge`; one 20 s deadline covers head and body; a range above 400 points is refused before sending; a matrix above 64 series is cut with a notice; the crate's own trace has endpoint, status, bytes, series, ms only, never a body or header (kube's `TraceLayer` debug span holds the URL with the PromQL: names only). UAT: `probe --metrics-source monitoring/vmselect-vm-victoria-metrics-k8s-stack:8481/select/0/prometheus` prints the check and a 30d `query_range` line (series, points, bytes, ms) and records the three [promql.md](promql.md) facts in as-built.
- [ ] 7. Settings › Metrics sits between Logs and About; on UAT it lists `vmselect-vm-victoria-metrics-k8s-stack` first (VictoriaMetrics cluster, port 8481, prefix `/select/0/prometheus`) and `vmquery-…` second; Test shows `Reachable · {ms} ms · {n} CPU series`.
- [ ] 8. Save writes `registry.clusters[].metrics` with namespace, service, port, scheme, prefix only; a hand-edited invalid entry fails closed (`Invalid` state, no request); the cluster form's `Metrics · Source` row shows `metrics-server only` or the saved source, and `metrics-server only` clears it.
- [ ] 9. Denied `services/proxy`, a missing service, no endpoints, a backend error, and a timeout each show their own message on the Metrics page and in the Monitor note (unit tests on `MetricsError` mapping).
- [ ] 10. With a Ready source that has CPU series (`cpu_series > 0`) the range group reads `15m 1h 6h 24h 7d 30d`, nothing is dimmed, and the footer names the source and step; without one it reads `15m 1h 6h 24h` as today and the dim tooltip points at Settings › Metrics.
- [ ] 11. A failed or empty query on ≤ 24h falls back to the sampler with a muted `Prometheus: {reason}. Showing k8sBoard samples.` line; on 7d/30d it shows a warning Alert; a source going away resets 7d/30d to 24h. A host-network pod (or a workload with one) shows the 0011 Network notice, not source data. 7d and 30d never read the sampler memo.
- [ ] 12. Refresh: every 30 s for 15m and 1h, every 5 min above, only while the Monitor tab is visible; a subject, scope, or range change drops the running fetch (task aborted).
- [ ] 13. Colors come from theme tokens only (0003 color-literal grep clean).
- [ ] 14. ui-verifier (light, dark): `settings-metrics-fixture`, `pod-monitor-source-fixture` (30d), and the live UAT `settings-metrics` and `pod-monitor` with a saved source show no high-severity defect against W2 and W4c; coder-lite UAT trace shows only the GETs of AC 3.

## Open items

1. Node subjects need the cAdvisor `node` label on root-cgroup series (`id="/"`), else node-exporter; `container_fs_*` and single counting of pod network are unverified on UAT. Step 2's live check settles all three ([promql.md](promql.md) facts table).
2. A direct URL for a source outside the cluster (managed Prometheus) needs a separate HTTP client and credentials; deferred until a cluster needs it (decision 3).
