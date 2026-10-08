# 0010 · Decisions

[Back to index](README.md). Architect defaults, amended after the advisor review; the user asked not to stop for questions.

## Sampling

| # | Decision | Rationale |
|---|---|---|
| 1 | Poll `metrics.k8s.io/v1beta1` PodMetrics and NodeMetrics every **15 s**, the first poll at once; keep each item's server `timestamp` | metrics-server scrapes kubelets every 15 s by default (`--metric-resolution`), so a 15 s poll lags by at most one scrape; W4c says "every 15s"; the timestamp exposes a stalled server: "stale · last sample …" shows when it has not advanced for 4 polls in a row (a tick count, so it holds for any clock or range) |
| 2 | Sampling is **always on** while the session is live and allowed, not only while a Monitor tab shows | history must exist when a drawer opens (W4c: "while the app is open"); two LISTs per 15 s fit the budget below |
| 3 | Pod metrics follow the pods watch scope (All, Named, 0009 Several: one list per allowed namespace); node metrics are cluster-wide | same RBAC and size as the pods list; a scope change restarts the pod poll |
| 4 | Polls reuse `WatchUpdate<T>`: `Snapshot` = one full sample, `Failed` = retrying | same meaning as a watch update; `subscribe` and `error_text` work unchanged |
| 5 | After a failed poll wait 30 s, 60 s, then 120 s each time; a success returns to 15 s | no hammering a broken APIService (503), a missing group (404), or a 403; recovers by itself |
| 6 | **No discovery step.** The crate always polls `v1beta1`; the app words a 404 as "metrics-server is not installed" and a 503 as "metrics.k8s.io does not answer", and keeps retrying with backoff | one path instead of discovery plus polling; installing or fixing metrics-server recovers without reconnecting; every metrics-server release serves only `v1beta1` |
| 7 | Pods feed: one `ListPodMetrics` review per namespace of the scope (one for All); poll only the allowed namespaces; `Off` only when none is allowed, naming the denied ones. Nodes feed: the session review's `ListNodeMetrics` | 0009's merged report is all-or-nothing; a user allowed in 2 of 3 namespaces still gets their metrics |

## History

| # | Decision | Rationale |
|---|---|---|
| 8 | **Tick-aligned** history: one timeline per feed (arrival time of each sample), one fine ring per container and per node | pod totals and workload sums add by index; no timestamp per point |
| 9 | Two resolutions: **fine** 240 ticks (1 h at 15 s) and **coarse** 288 points (24 h at 5 min, each the mean of 20 fine ticks). 15m/1h read fine; 6h/24h read coarse plus the fine tail. Every range button works; one longer than the data is dimmed with the tooltip "Showing data since k8sBoard connected; connect Prometheus for 30 days" | W4c note 1: without Prometheus, data since the app opened; ≈ 14 MB at 1,000 pods |
| 10 | Containers: `UsagePoint { cpu_microcores: u32, memory_kib: u32 }`, saturating (12 B with `Option`). Nodes: `Option<ResourceUsage>` (u64) | a container never nears 4,294 cores or 4 TiB; a node can pass 4 TiB |
| 11 | A fine ring unseen for 240 ticks is freed; a pod or node unseen for 24 h is removed; a pod's `controller` is filled whenever it is still `None` and the pod is in the pods list; a scope change drops other namespaces | bounded memory; a metrics tick may arrive before the pods snapshot; workload charts keep pods replaced by a rollout |
| 12 | A failed poll adds no tick; a chart line breaks at `None` and where neighbours are > 2.5 steps apart (15 s fine, 5 min coarse) | an unknown value is not "not running"; the kit `shape::Line` joins across `None`, so segments are split before painting |
| 13 | OOM marks come from container status (`Terminated` state or `last_termination` with reason OOMKilled, its `finished_at`), collected each pod tick, kept 24 h, 32 per pod | the kubelet records OOM in status, not as a pod event; same source as the 0008 WHY box |

## Quantities and tables

| # | Decision | Rationale |
|---|---|---|
| 14 | Own integer parser in the cluster crate (`CpuAmount`, `ByteAmount`: u128 mantissa plus a decimal exponent); no quantity crate | exact for every valid quantity; ~70 lines; quantity crates pull decimal or regex dependencies; unblocks 0008 open item 4 |
| 15 | Format: CPU `310m`, `<1m`, `1 core`, `2.5 cores`; memory binary `0B`, `498Mi`, `1.1Gi`, `15.6Gi`, `120Gi`; a pair shows the unit once when both share it (`9.8 / 15.8 cores`, `498 of 512Mi`), else both (`310m of 1 core`) | matches W4, W4b, W7 node drawer, and the W4c axis format |
| 16 | Pods: Memory and CPU columns shown (Columns ▾ can hide CPU; it was hidden by default until the UX walkthrough of 2026-10); both sort by value; "—" without a sample (a new pod reads "—" until metrics-server has two scrapes of it) | W4 shows Memory only; metrics-server needs two scrapes to compute a CPU rate |
| 17 | Nodes: CPU and Memory = bar + percent of **allocatable**; `usage_tone`: Warn ≥ 80 %, Bad ≥ 90 % | W5 colors 82 % yellow; W4b shows 97 % red; one rule for every bar |
| 18 | Usage bars in container Info › Resources (fill = usage / limit, tick = request) and the node drawer "Allocatable used" (CPU, Memory, and Pods when the scope is All) | W4b note 5; W7 Nodes drawer |
| 19 | Node Resources rows (0008) print cpu, memory, ephemeral-storage, hugepages human-readable | 0008 open item 4 |

## Monitor tab

| # | Decision | Rationale |
|---|---|---|
| 20 | Charts: own `UsageChart` implementing the kit `Plot` trait with kit primitives, `#[derive(IntoPlot)]` | kit `LineChart` draws one series, joins across gaps, has no colored or labeled reference lines and no markers; `Plot` gives the hover crosshair; no new dependency (C6) |
| 21 | Colors: series `chart_1` (0011 adds `chart_2`), area `chart_1` at 10 % opacity, grid `chart_grid`, request and allocatable lines `muted_foreground`, limit line and OOM dots `tone_color(Bad)` | theme tokens only; W4c chart engine |
| 22 | Lines: a container uses its own request/limit; pod total and workloads sum main + sidecar containers; request labelled `request (partial)` when some have none; a limit line only when every one has a limit | the effective limit is unbounded when one container has none; init containers do not run alongside |
| 23 | Node lines: `allocatable` always; `requested` only when the scope is All | a namespace scope would understate it |
| 24 | Monitor tab on Pod, Node, Deployment, StatefulSet, DaemonSet, ReplicaSet, Job; shown even when metrics are unavailable | W7 ◔ list (CronJob has none); a tab with a reason beats a missing tab |
| 25 | Scope: Pod → Pod total + each container; workload → All pods + each pod; Node → Node total | W4c note 2, W7 Monitor toolbar |
| 26 | Container sub-tabs `Info · Env · Mounts · Monitor`; 0019 inserts `Logs` before Monitor (W4b order) | W4b; one chart path |
| 28 | The range survives subject changes; scope resets on a new subject; `show_screen` resets all; chart data is memoized per (subject, tick count, scope, range) | like `tab` and `selected_container`; rendering never re-sums 2,000 rings per frame |

## Budget (1,000 pods, 2,000 containers, 50 nodes, scope All)

| Item | Cost |
|---|---|
| Container | fine 240 × 12 B + coarse 288 × 12 B + ≈ 150 B entry ≈ 6.5 KB; × 2,000 ≈ 13 MB |
| Pods, nodes (24 B points), timelines, marks | ≈ 1 MB |
| **History total** | **≈ 14 MB steady** (C13 budget 150 MB) |
| Transient per tick | the PodMetrics list as `DynamicObject`s ≈ 1–3 MB on tokio, dropped after mapping |
| Network | 2 LISTs per 15 s; ≈ 0.5–1 KB per pod → ≈ 35–70 KB/s at 1,000 pods |
| Main thread | one ring push per series per tick, a coarse fold every 20 ticks (< 2 ms), plus the 0009 visible-table rebuild |

## Known ceilings

- Memory grows with pods seen in the last 24 h (≈ 3.5 KB each after their fine ring is freed); CronJob-heavy clusters pay for every run. Upgrade path: history for visible namespaces only.
- A tick repeats the last value when metrics-server has not scraped again; charts show short flat steps.
- Status keeps one last termination, so two OOM kills inside one tick record only the newest.
