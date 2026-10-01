# 0011 · App: Network and Disk I/O in the Monitor tab

[Back to index](README.md) · Step 3 · Modules: `usage_format.rs`, `usage_chart.rs` (+ tests), `monitor_data.rs` (+ tests), `monitor_tab.rs`, `app_shell.rs`, `screenshot.rs`

## Rate format (`usage_format.rs`)

0010's `Measure` gains `Rate` (bytes per second, decimal units as W4c `FMT.rate`):

| Value (B/s) | Text |
|---|---|
| 0, 512 | `0 B/s`, `512 B/s` |
| 420,000 | `420 KB/s` (whole KB below 1,000 KB) |
| 999,600 | `1.0 MB/s` (rounding rolls up) |
| 1,300,000; 12,400,000 | `1.3 MB/s`; `12.4 MB/s` (one decimal) |
| 2,100,000,000 | `2.1 GB/s` |

`nice_max(value, Rate)` floors at 1,000 (1 KB/s).

## Chart changes (`usage_chart.rs`)

| Change | Rule |
|---|---|
| Series colors | series 0 `chart_1`, series 1 `chart_2` (0010 decision 21); no area with two series (0010 rule) |
| Legend | two or more series: header right side lists a 8 px swatch + name per series (muted `text_xs`), replacing `now {value}` |
| `notice: Option<SharedString>` | new model field. Points in range → muted `text_xs` after the legend (truncated, full text as tooltip); no point in range → centered muted text inside the plot over the empty grid |
| Ids | `monitor-network`, `monitor-disk` (unique siblings, kit rule) |
| Tooltip | unchanged: one row per series, `not running` for `None` |

## Models (`monitor_data.rs`)

`MonitorInput` gains `kubelet: &'a KubeletFeed` and `nodes: &'a [NodeSummary]`. `monitor_charts` appends, after CPU and Memory, `Network` (series `receive`, `transmit`) and `Disk I/O` (series `read`, `write`), `Measure::Rate`, no references, no markers. Range, resolution, and point dropping follow 0010; the window end is below.

| Subject · scope | Network | Disk I/O |
|---|---|---|
| Pod · Total | `pod_rates(Network, …)` | `pod_rates(DiskIo, …, None)` |
| Pod · Part(c), Container | pod network (notice `Pod network, shared by all containers`) | `pod_rates(DiskIo, …, Some(c))` |
| Workload · Total | `owner_rates(Network, owner, None)` | `owner_rates(DiskIo, owner, None)` |
| Workload · Part(p) | `owner_rates(Network, owner, Some(p))` | `owner_rates(DiskIo, owner, Some(p))` |
| Node | `node_rates(Network, node)` | `node_rates(DiskIo, node)` |

## Notices (first match wins, per card)

| Condition | Notice |
|---|---|
| kubelet feed `Checking`/`Waiting`, or `tick_count < 2` | `Collecting… rates need two samples` |
| subject's node(s) not Ready (pod or node subject) | `Node {n} is not ready: no kubelet stats` |
| the subject's node has `node_errors[n].summary` (Network) or `.disk_io` (Disk) | `Kubelet on {n}: {error}` |
| Network, host-network pod (`PodSummary.host_network`) | `Host network: traffic is the node's (see the node's Monitor tab)`; series empty |
| Disk, Node subject, `disk_io_state == Sampled { has_root: false }` | `The kubelet reports no node-level disk I/O` |
| Disk, Pod/Container subject: its node is `Sampled` but `has_disk_series(ns, pod, container)` is false | `The kubelet reports no disk I/O for this pod` (`… for this container` in a container scope) |
| Disk, Workload: every owned pod on a `Sampled` node lacks a disk series | `The kubelet reports no disk I/O for these pods` |
| Workload, demanded nodes > polled nodes | `Covers pods on {k} of {m} nodes` (summary targets for Network, disk targets for Disk) |
| Workload Network, owned host-network pods (from `pods`) | `{h} host-network pods not counted` |
| Interrupted feed | none here; the 0010 line above the charts covers it |

`k`, `m` come from `subject_nodes(…)` against the feed's current targets; pure, so tests need no session.

## Window end

CPU and Memory keep 0010's `end` (the metrics feed's newest tick). Network and Disk use the same `end` while the metrics feed has ticks, so the four cards align; when it has none (unavailable, failed, or not yet sampled), they use `kubelet.history.newest_tick()`, else `now`.

## Body by feed status (`monitor_tab.rs`)

| Metrics feed (0010) | Kubelet feed | Body |
|---|---|---|
| any data state | any data state | toolbar, 4 cards (2 per row when expanded, 1 per row at the default width), note |
| `Unavailable`/`Failed` | data state | 0010 Alert, then toolbar and the Network/Disk cards only |
| data state | `Unavailable`/`Failed` | toolbar, CPU/Memory cards, then a warning `Alert` titled `Network and disk I/O unavailable` with the reason |
| both unavailable | | both Alerts, no toolbar |

"Data state" = `Checking`, `Waiting`, `Live`, `Interrupted`. A `Workload` subject with no `related_pods` keeps 0010's `No pods to monitor`.

## Table view

- `MonitorRow` gains `network: Option<RatePair<f64>>` and `disk: Option<RatePair<f64>>`; columns `Time · CPU · Memory · Receive · Transmit · Read · Write`, mono `text_xs`, `Measure::Rate.format`, muted `—` for `None`.
- Rows follow the metrics timeline (0010); each takes the kubelet point nearest in time within ±7.5 s. When the metrics feed has no ticks, rows follow the kubelet timeline with CPU and Memory `—`.

## Note (bottom of the tab)

0010's first sentence, then `Network and disk I/O: kubelet stats summary and cAdvisor through the API server node proxy, sampled every 15s while needed.`, then 0010's history sentence unchanged.

## Disk demand (`app_shell.rs`)

`sync_kubelet_demand` now sets `wants_disk_io` per [kubelet-session.md](kubelet-session.md); leaving the Monitor tab drops the disk targets at the next sync (a new targets value; the subscription stays).
