# 0010 — Metrics I: metrics-server, usage columns, Monitor tab (read-only)

Status: amended after advisor review (HEAD `459e493`, 0008 steps 1–2). Crates: `crates/cluster` (step 1), `crates/app` (steps 2–3). Requires 0008 (container Info sub-tabs, node Resources) and 0009 (`TableRow`, `CellValue`, Columns ▾) merged. Wireframes: W4 (Memory column), W4b (resource bars, Monitor sub-tab), W4c (Monitor tab), W5 (CPU/Memory bars), W7 (◔ kinds, node "Allocatable used", per-kind `mon` data), stack table (Chart, metrics source). UAT serves `metrics.k8s.io/v1beta1` (metrics-server).

## Goal

- A per-session **sampler** that polls PodMetrics (session scope) and NodeMetrics every 15 s into bounded rings: 1 h at 15 s plus 24 h at 5 min.
- **Pods** Memory and CPU columns (Columns ▾ hides either); **Nodes** CPU and Memory bars as % of allocatable.
- Usage bars in container Info › Resources and the node drawer ("Allocatable used"); human-readable node quantities (0008 open item 4).
- A **Monitor tab** (CPU, Memory charts, request/limit lines, OOMKilled markers, 15m/1h/6h/24h ranges, scope selector, Table view, source note) on Pod, Node, Deployment, StatefulSet, DaemonSet, ReplicaSet, Job drawers, plus a container Monitor sub-tab.
- "Metrics unavailable · reason" when the API is missing, broken, or denied.

## Non-goals

Network, Disk I/O, PVC usage, kubelet stats (0011); Prometheus and history beyond 24 h or across app runs; Settings › Metrics (0025); Overview capacity (0021); HPA bars (0013); Namespace CPU req/Memory req columns (0012); alerts; node OOM events.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | `crates/cluster`: `quantity.rs`, `resource_metrics.rs` (types, two polling streams, backoff), two `AccessCheck`s, `review_namespaces`, probe `--metrics-seconds` | 1, 2, 3, 5 |
| 2 | App data and tables: `usage_format.rs`, `metrics_history.rs`, `cluster_metrics.rs` (gates, feeds), session wiring; fine history; Pods/Nodes columns; usage bars; node quantities | 1, 2, 3, 4, 6, 8, 9 |
| 3 | App Monitor: coarse history, series, OOM marks; `usage_chart.rs`, `monitor_data.rs`, `monitor_tab.rs`; `DrawerTab::Monitor`, `ContainerTab::Monitor`; launch screens; ui-verifier run | 1, 2, 4, 7, 8, 9, 10 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale; memory, network, CPU budget; known ceilings |
| [cluster-metrics.md](cluster-metrics.md) | step 1: quantity parser, metrics types, polling streams, access checks, probe |
| [metrics-history.md](metrics-history.md) | steps 2–3: formatting, fine and coarse rings, scope retention, series, OOM marks |
| [metrics-session.md](metrics-session.md) | step 2: access gates (per-namespace pods review), feeds, error wording, lifecycle |
| [table-columns.md](table-columns.md) | step 2: Pods/Nodes columns on the 0009 toolkit, usage bars, node quantities |
| [monitor-tab.md](monitor-tab.md) | step 3: tab layout, subjects and scopes, ranges, Table view, unavailable state, 0011 hooks |
| [usage-chart.md](usage-chart.md) | step 3: the `Plot` implementation, geometry, tooltip, theme tokens |
| [files-to-touch.md](files-to-touch.md) | modules per crate and step, doc updates |
| [test-plan.md](test-plan.md) | unit tests per step, live checks, ui-verifier checklist |

## Key decisions (all in [decisions.md](decisions.md))

15 s poll, always on per live session, no discovery step · tick-aligned history: 240 fine ticks (1 h) + 288 coarse points (24 h), ≈ 14 MB at 1,000 pods · own integer quantity parser, no new crate · own chart on the kit `Plot` trait (kit `LineChart` lacks gaps, labeled colored reference lines, markers) · per-namespace SSAR for pod metrics, "Metrics unavailable · reason" · OOM marks from container status.

## Acceptance criteria

- [x] 1. The quality gate passes, and so does `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes, offline.
- [ ] 3. No new dependency; `git diff Cargo.lock` is empty. No kube or k8s-openapi type in a public signature. The 0001 read-only grep still finds only the SSAR `create`; the new requests are `list` only. The crate spawns no task. — superseded by 0030 (write allow-list and named connect files replace the read-only grep)
- [x] 4. The 0003 AC4 color-literal grep is clean; bars and charts use theme tokens and `tone_color` only.
- [ ] 5. On UAT, `probe --metrics-seconds 40` prints at least two pod and two node samples; the pod-metrics count is within 3 of the listed pods that are running; the access report shows `list pods.metrics.k8s.io` and `list nodes.metrics.k8s.io` allowed.
- [ ] 6. On UAT, Pods show Memory for running pods and "—" for completed ones; Nodes show CPU and Memory bars; values of three spot-checked pods match the probe's same-tick output after formatting.
- [ ] 7. On UAT, the `pod-monitor`, `node-monitor`, and `deployments-monitor` screenshots show a CPU and a Memory line (≥ 2 ticks), request/limit lines where the spec sets them, four enabled range buttons (the short one muted), and the scope selector; scope switching and Table view are covered by `monitor_data` tests plus a manual spot check.
- [ ] 8. Memory bound: tests prove fine rings never exceed 240 ticks and coarse rings 288 points, unseen series are freed or dropped, and a scope change drops other namespaces.
- [ ] 9. A denied, missing, or broken metrics API shows "Metrics unavailable · reason" in the Monitor tab and "—" in the columns, and is never polled when denied; with several namespaces only the allowed ones are polled (gate tests).
- [ ] 10. Screenshots `pod-monitor`, `node-monitor`, `deployments-monitor`, `pods`, `nodes` exist; the ui-verifier reports no high-severity defect against W4, W4c, W5, W7.

## Open items

1. Screenshots capture after 2 ticks, so charts show one short segment; add a dev-only `--metrics-interval` flag only if the ui-verifier needs dense charts.
2. Prometheus (30 days) and Settings › Metrics stay backlog/0025; a range longer than the data says so in its tooltip.
3. Above ~2,000 containers the history passes 14 MB, and pods seen in the last 24 h all count; switch to on-demand history (visible namespaces only) if C13 measurements require it.
4. Hover sync across the CPU and Memory charts (one crosshair) is not in the kit `Plot`; each chart hovers alone.
