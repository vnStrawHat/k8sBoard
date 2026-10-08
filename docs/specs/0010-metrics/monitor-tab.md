# 0010 · App: Monitor tab

[Back to index](README.md) · Step 3 · Modules: `monitor_tab.rs` (new), `monitor_data.rs` (new, pure) + `monitor_data_tests.rs`, `drawer.rs`, `pod_drawer.rs`, `node_drawer.rs`, `kind_drawer.rs`, `resource_kind.rs`, `app_shell.rs`, `launch_options.rs`, `screenshot.rs`

## Layout (W4c)

```text
[15m|1h|6h|24h]  [Container: api ▾]        step 15s · live
no access in web                                          (pods feed note, when set)
┌ CPU            now 310m ┐ ┌ Memory          now 498Mi ┐   two per row when expanded,
│ chart (usage-chart.md)  │ │ chart                     │   one per row at 420 px
└─────────────────────────┘ └───────────────────────────┘
CPU and memory: metrics-server, sampled by k8sBoard every 15s while the app is open; kept 24 hours.
```

Chart height 140 px expanded, 110 px at the default width. The body is `DrawerBody::Scrolling`, `gap_3`.

## State (`drawer.rs`)

```rust
pub(crate) struct MonitorState { pub(crate) range: MonitorRange, pub(crate) scope: MonitorScope,
    pub(crate) cache: Option<MonitorCache> }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MonitorRange { Minutes15, Hour1, Hours6, Hours24 }      // labels 15m 1h 6h 24h
impl MonitorRange { pub(crate) fn duration(self) -> Duration; pub(crate) fn resolution(self) -> Resolution; } // 6h, 24h → Coarse
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MonitorScope { Total, Part(String) }  // a container (pod) or a pod (workload)
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MonitorKey { subject: ResourceKey, container: Option<String>, ticks: u64, scope: MonitorScope, range: MonitorRange }
pub(crate) struct MonitorCache { key: MonitorKey, pub(crate) data: MonitorData }
```

- `DrawerState` gains `monitor: MonitorState` (15m, Total, charts, no cache). `AppShell::set_monitor_range`, `set_monitor_scope`, each `cx.notify()`. Scope resets on a subject change; `show_screen` resets everything (decision 28).
- Memo (decision 28): before building a drawer that shows Monitor (tab or container sub-tab), `AppShell::render` (`&mut self`) compares the current `MonitorKey` (`ticks` = the feed's `tick_count()`) with `cache.key` and calls `monitor_data` only when it differs. Hover repaints and unrelated notifies reuse the cache.

## Data (`monitor_data.rs`)

```rust
pub(crate) enum MonitorSubject<'a> { Pod(&'a PodSummary), Container { pod: &'a PodSummary, container: &'a str },
    Node(&'a NodeSummary), Workload(&'a PodOwner) }
/// Plain data, so tests need no session; the caller fills it from `LiveCluster`.
pub(crate) struct MonitorInput<'a> { pub(crate) subject: MonitorSubject<'a>, pub(crate) scope: &'a MonitorScope,
    pub(crate) range: MonitorRange, pub(crate) pods: &'a [PodSummary], pub(crate) pod_history: &'a PodUsageHistory,
    pub(crate) node_history: &'a NodeUsageHistory, pub(crate) is_all_namespaces: bool }
pub(crate) struct MonitorData { pub(crate) charts: Vec<Rc<UsageChartModel>>,
    pub(crate) choices: Vec<ScopeChoice>, pub(crate) stale_since: Option<jiff::Timestamp>, pub(crate) span: Option<Duration>, pub(crate) scope: MonitorScope }
pub(crate) fn monitor_data(input: &MonitorInput) -> MonitorData;
```

| Subject · scope | Series (name) | Reference lines | OOM |
|---|---|---|---|
| Pod · Total | `pod_series(.., None, r)` (`pod total`) | `request` / `request (partial)`, `limit`: main + sidecar sums (decision 22) | all containers |
| Pod · Part(c), Container | `pod_series(.., Some(c), r)` (`c`) | the container's own | `c` |
| Workload · Total | `owner_series(owner, None, r)` (`{n} pods`) | sums over owned pods now in `pods` that still take room (not Completed, Succeeded, Failed, Evicted, or Error), like `node_requests` | union |
| Workload · Part(p) | `owner_series(owner, Some(p), r)` (`p`) | that pod's sums | that pod |
| Node | `node_series(node, r)` (`used`) | `requested` (All only), `allocatable` (decision 23) | none |

- The window ends at the feed's newest tick (`end`) and starts at `end − range`; older points drop. Requests and limits parse with `CpuAmount`/`ByteAmount` (unparsable → no line).
- `charts`: `[CPU, Memory]`, ids `monitor-cpu`, `monitor-memory`; 0011 appends.
- `choices`: Pod → `Pod total` + main and sidecar containers (`Container: {name}`); Workload → `All pods` + owned pods (`Pod: {name}`); Node → `Node total`. A `Part` no longer offered falls back to Total.
- `stale_since`: the series' `sampled_at` once the server timestamp has not advanced for 4 polls in a row (counted in ticks, so it does not depend on the clock or the range; a pod that did not report the newest tick is not stale). `is_short_for(range)`: the history `span()` plus one step of the range's resolution is shorter than the range, so a full history (at most 24 h less a coarse step) does not dim 24h forever.

## Toolbar

- Range: kit `ButtonGroup` of four small buttons, all enabled, the current one selected. Each range button for which `is_short_for(range)` holds is dimmed and has the tooltip "Showing data since k8sBoard connected; connect Prometheus for 30 days".
- Scope: small outline `Button` with the current choice and a dropdown caret + `DropdownMenu` of `choices`; Node shows the muted text `Node total`. Hidden in the container sub-tab.
- Right (`ml_auto`, muted `text_xs`): `Live` → `step 15s · live` (`step 5m` for coarse ranges), or `stale · last sample {age} ago` when `stale_since` is set; `Checking`/`Waiting` → `collecting…`; `Interrupted` → `paused · retrying` with the error as tooltip.
- Pods feed `note` (e.g. `no access in web`): a muted `text_xs` line under the toolbar.

## Body by feed status

| Status | Body |
|---|---|
| `Unavailable(reason)` or `Failed(reason)` | kit `Alert` (warning) titled `Metrics unavailable`, message `reason` (`Failed` adds `Retrying.`); no toolbar |
| `Checking`/`Waiting`, no tick | toolbar, then muted `Collecting the first sample…` |
| `Live` | toolbar, charts, note |
| `Interrupted(reason)` | as Live plus a muted `Last poll failed: {reason}. Showing older samples.` |

Pod, Container, and Workload read the pods feed; Node reads the nodes feed.

## Tabs

- `DrawerTab::Monitor`, title `Monitor`. `drawer_tabs`: Pod `Overview · Containers · Monitor · YAML · Events`; Node and `has_monitor` kinds `Overview · Monitor · YAML · Events`; no-YAML variants drop only YAML. Others unchanged.
- `ResourceKind::has_monitor(self) -> bool`: Deployments, StatefulSets, DaemonSets, ReplicaSets, Jobs. A kind row with `related_pods == None` shows muted `No pods to monitor`.
- `ContainerTab::Monitor` (label `Monitor`) after Mounts: `Info · Env · Mounts · Monitor`; 0019 inserts `Logs` before Monitor (W4b order). Body: `MonitorSubject::Container`, no scope selector.
- ⤢ exists on every drawer already (0005/0007); inventory row D2 flips to Done.

## Launch screens

`pod-monitor`, `node-monitor`, `{plural}-monitor` for `has_monitor` kinds (`deployments-monitor`, …); any other `-monitor` is a usage error. They settle at 2 ticks of their feed or `Unavailable`/`Failed` ([metrics-session.md](metrics-session.md)).

## Hooks for 0011

`charts` is a list (0011 appends Network and Disk I/O with two series each and a rate `Measure`); the grid lays out any count, two per row; legends render for two or more series; the note gains 0011's part.
