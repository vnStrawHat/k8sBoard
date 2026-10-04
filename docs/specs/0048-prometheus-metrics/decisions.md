# 0048 · Decisions

[Back to index](README.md).

| # | Decision | Rationale |
|---|---|---|
| 1 | The source is always reached through the API server **service proxy** (`services/{scheme}:{svc}:{port}/proxy`) on the session's `ClusterConnection` client | Auth is the user's kubeconfig (no new credential), the 0043 per-cluster proxy applies, and env `HTTPS_PROXY` stays ignored. The UAT probe showed a fresh `kube::Config` client picked up ambient `HTTPS_PROXY` and failed; the session client does not |
| 2 | One source per cluster, chosen by the user; detection only proposes | A cluster may run several stacks (UAT: `vmselect` and `vmquery`); guessing silently would pick wrong ones |
| 3 | **No direct URL** and no credential field | A source outside the cluster needs a second HTTP client (which must replicate 0043 proxy rules and TLS trust) and usually a token or SigV4; neither is drawn and UAT does not need it. Open item 2 |
| 4 | The cluster crate builds every PromQL text from typed targets (`UsageTarget`, `UsageMetric`, 0049 `TrafficSourceKind`); no public function takes query text | Escaping lives in one tested place; the app cannot build an injection |
| 5 | One named clippy exception: `metrics_get` in `metrics_query.rs` calls `Client::send` and reads status and body itself under one deadline | Same shape as `kubelet_text`/`kubelet_lines` (0011): a fixed endpoint allow-list, read-only GET. `send` lets success and error bodies both be capped (`http-body-util` `Limited`) and keeps the `Status` that kube's `handle_api_errors` and `ClusterConnection::run` would read whole or reclassify |
| 6 | Stored as `registry.clusters[].metrics` = `MetricsSourceFields` (namespace, service, port, scheme, prefix); `None` = metrics-server only | Per cluster like the 0043 proxy; nothing secret to store; the cluster type gets `serde` so the app needs no mirror (0043 `ShellCommand` precedent) |
| 7 | Session `SourceState` is checked once per session start and after each save (one `count(container_cpu_usage_seconds_total)` instant query); no polling of the source's health | Cheap; the Monitor fetch reports its own errors; Test on the Metrics page re-checks |
| 8 | With a **Ready** source, all four Monitor charts read the source for every range, not only 7d/30d | One data source per view (W4c note 6 names one source); history before the app started is the point |
| 9 | On a failed or empty query, ranges ≤ 24h fall back to the sampler with a notice; 7d/30d show the error | The sampler always has ≤ 24h; it has nothing older |
| 10 | Fixed step table (≤ 400 points), body cap 4 MiB, one 20 s deadline for head and body plus `timeout=15s` sent to the backend, ≤ 64 series kept per matrix | Bounded memory and time; the backend stops work it cannot finish. UAT 30d at 1h step: 0.7–1.3 s, ~24 KB |
| 11 | Workload subjects match pods by a name pattern per kind over the generated-suffix alphabet (`{name}-S{1,10}-S{5}` for Deployments …, [promql.md](promql.md)), not a `kube_pod_owner` join | Works without kube-state-metrics, includes pods deleted during the range (the point of 30 days), one selector. `ponytail:` another workload whose pod names fit the pattern also counts; upgrade path: a `kube_pod_owner` join when kube-state-metrics is present |
| 12 | Request and limit lines and OOM markers keep coming from the current pod spec and status | The source has no spec history; the footer says so |
| 13 | No SelfSubjectAccessReview for `services/proxy`; a 403 maps to `MetricsError::Denied` | The check query gives the same answer with one request fewer |
| 14 | `form_urlencoded` (already locked as a dependency of `url`) and `http-body-util` (already locked under kube) become direct dependencies of `k8sboard-cluster` | Correct percent-encoding of PromQL (`{`, `"`, `=~`, `+`); a capped body read; no new package |
| 15 | Detection lists services cluster-wide once per page open (`list services`); denied → manual entry only | UAT allows it; the list is small and needs no watch |

## UAT probe (2026-10-04, `readonly@Monitor`, v1.29.5)

| Fact | Value |
|---|---|
| Stack | VictoriaMetrics k8s stack, cluster mode, namespace `monitoring` (operator `operator.victoriametrics.com`, 24 CRDs); no `monitoring.coreos.com`, Istio, Linkerd, or Cilium CRDs; no Prometheus-named service |
| Services (`app.kubernetes.io/name`, port) | `vmselect-vm-victoria-metrics-k8s-stack` (vmselect, 8481 `http`); `vmquery-vm-victoria-metrics-k8s-stack` (vmquery, 8481 `http`); vminsert 8480; vmstorage 8482/8400/8401; vmagent 8429; vmalert 8080; vmalertmanager 9093; `vm-grafana` 80; `vm-kube-state-metrics` 8080; `vm-prometheus-node-exporter` 9101; operator 8080/9443 |
| Access | `get services/proxy` allowed in `monitoring` and cluster-wide; `get pods/proxy` in `monitoring`; `list services` cluster-wide |
| Working path | `/api/v1/namespaces/monitoring/services/http:vmselect-vm-victoria-metrics-k8s-stack:8481/proxy/select/0/prometheus/api/v1/…` |
| Names | `label/__name__/values`: 2,210 names, 91 KB, 35 ms. Present: `container_cpu_usage_seconds_total`, `container_memory_working_set_bytes`, `container_network_{receive,transmit}_bytes_total`, `kube_pod_info`, `kube_pod_owner` |
| Range cost | `sum(rate(container_cpu_usage_seconds_total[5m]))`, 30d, step 1h: 0.7–1.3 s, ~24 KB |
| Retention | vmstorage `-retentionPeriod=60d`, so 30d fits |
| Unverified | `container_fs_{reads,writes}_bytes_total` and the `node` label on `id="/"` series (open item 1) |
