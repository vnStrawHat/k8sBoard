# 0048 · Monitor tab with a source (step 4)

[Back to index](README.md) · Modules (app): `drawer.rs`, `monitor_source.rs` (new) + `monitor_source_tests.rs`, `monitor_tab.rs`, `monitor_data.rs`, `app_shell.rs` (`sync_monitor_source`, `refresh_monitor_cache`), `launch_options.rs`, `screenshot.rs`. Base layout: 0010 [monitor-tab.md](../0010-metrics/monitor-tab.md), 0011 [monitor-charts.md](../0011-metrics-kubelet/monitor-charts.md).

## Ranges

```rust
pub(crate) enum MonitorRange { Minutes15, Hour1, Hours6, Hours24, Days7, Days30 }  // labels 7d, 30d
impl MonitorRange {
    pub(crate) const SAMPLER: [Self; 4] = [Minutes15, Hour1, Hours6, Hours24];   // replaces today's ALL
    pub(crate) const SOURCE: [Self; 6] = [.., Days7, Days30];
    pub(crate) fn source_step(self) -> Duration;   // promql.md step table
    pub(crate) fn is_long(self) -> bool;           // Days7, Days30
}
```

- **Long ranges skip the sampler.** `resolution()` (drawer.rs) stays defined for the four sampler ranges only; `Days7`/`Days30` return `Coarse` but no code path reads the sampler for them: `refresh_monitor_cache` skips `monitor_data` (monitor_data.rs `monitor_data`, `is_short_for`) when `range.is_long()`, and the tab builds from the source result alone.
- The toolbar (`monitor_tab.rs`, the range `ButtonGroup`) iterates the shown set, and the click handler indexes that same set (today `MonitorRange::ALL.get(index)`), so a click on `30d` cannot map to the wrong range.
- The shown set is `SOURCE` when `SourceState::Ready` with `cpu_series > 0` and the subject has a target (Node may have none, [promql.md](promql.md) facts), else `SAMPLER`. A Ready source with `cpu_series == 0` counts as no source for the Monitor (note `Metrics source has no container CPU series`).
- With `SOURCE`, nothing is dimmed (`is_short_for` is not consulted). With `SAMPLER`, dimming is as today and the tooltip becomes `Showing data since k8sBoard connected; choose a metrics source in Settings › Metrics for up to 30 days`.
- If the shown set drops to `SAMPLER` while 7d or 30d is chosen, the range resets to 24h in `sync_monitor_source`.

## Fetch (`monitor_source.rs`)

```rust
pub(crate) struct SourceKey { subject: ResourceKey, container: Option<String>, scope: MonitorScope,
    range: MonitorRange, source: MetricsSource }
pub(crate) struct SourceFetch { key: SourceKey, started: Instant, result: Option<SourceResult>,
    _task: Option<Task<()>>, _refresh: Option<Task<()>> }          // AppShell field, Option
pub(crate) struct SourceResult { pub(crate) end: jiff::Timestamp, pub(crate) step: Duration,
    pub(crate) metrics: [(UsageMetric, Result<UsageSeries, MetricsError>); 6] }
pub(crate) fn source_target(subject: &MonitorSubject, scope: &MonitorScope) -> Option<UsageTarget>;
pub(crate) fn refresh_after(range: MonitorRange) -> Duration;   // 30 s for 15m and 1h, 5 min otherwise
/// The charts from a result, with the same reference lines and OOM marks as `monitor_data`.
pub(crate) fn source_monitor_data(input: &MonitorInput, result: &SourceResult) -> SourceView;
pub(crate) enum SourceView { Charts(MonitorData), Fallback(String) /* the reason */ }
```

- `AppShell::sync_monitor_source(&mut self, cx: &mut Context<Self>)` is its own sync step next to `sync_kubelet_demand` (called from the same place in `render`), not inside `refresh_monitor_cache` (that takes `&App` and never notifies).
  - `!shows_monitor()` → `source_fetch = None` (drops the task and the timer, so a hidden tab stops refreshing, AC 12).
  - Otherwise it starts a fetch when the key differs from the current one, or the result is older than `refresh_after`. Equal and fresh → nothing. A new key drops the old `SourceFetch` (its `RuntimeTask` aborts the requests).
- One fetch = the six `usage_range` calls on `ClusterRuntime`, at most 3 at a time (`buffer_unordered(3)`), joined, then one `cx.update` + `cx.notify()`. The previous result stays visible until the new one lands.
- `_refresh`: a `cx.spawn` timer of `refresh_after` that only calls `cx.notify()`; the next render's `sync_monitor_source` decides.
- **Host-network pods** (0011 decision 21): `source_target` still targets them, but the Network chart of a host-network pod, or of a workload with any host-network pod, shows the 0011 notice instead of source data (their counters are the node's).

## What the tab shows (Ready source)

| Result | Body |
|---|---|
| no result yet | toolbar, then muted `Querying {source display}…` (≤ 24h: the sampler charts meanwhile, with that line on top) |
| CPU and Memory `Ok` with at least one value | charts from the source: CPU, Memory (request/limit lines and OOM marks as 0010), Network (receive, transmit), Disk I/O (read, write) |
| Network or Disk `Err`, or all `None` | that card keeps its frame with the notice `No {metric} series in the source` or the error text (0011 notice slot) |
| CPU or Memory `Err` or empty, range ≤ 24h | `SourceView::Fallback`: the sampler view of today, under a muted `Prometheus: {reason}. Showing k8sBoard samples.` |
| same, 7d or 30d | warning `Alert` titled `Metrics source query failed`, message the reason; the toolbar stays |
| a refresh failed, an older result exists | the older result plus muted `Last query failed: {reason}. Showing older results.` |

`was_cut` on any series adds the muted line `Some series were left out (more than 64).`

## Toolbar and footer

- Right text: `step {step} · metrics source` (`step 2h · metrics source`), `querying…` before the first result.
- Footer note (W4c note 6), replacing `SOURCE_NOTE` while the source view shows: `CPU, memory, network, and disk I/O: {source display}, step {step}. Request and limit lines show the current spec.`
- State not Ready and a source is saved: a muted line under the toolbar from `source_note`: `Metrics source: checking…`, `Metrics source unreachable: {reason} (Settings › Metrics)`, `Metrics source in settings is not valid (Settings › Metrics)`. The sampler view follows as today.

## Unchanged

Scope choices (current pods and containers), Pods/Nodes columns, node usage bars, Overview, the sampler and kubelet feeds (they keep running: the Pods columns need them and the fallback uses them).

## Screens

| Screen | Content |
|---|---|
| `pod-monitor-source-fixture` | no cluster: a fixture pod, `SourceState::Ready`, range 30d, synthetic series (CPU wave, memory sawtooth with one OOM mark, network and disk pairs), footer naming `monitoring/vmselect-…:8481` |
| `pod-monitor` (live) | unchanged screen; with a config dir whose UAT entry has the vmselect source it shows six ranges and the source footer |
