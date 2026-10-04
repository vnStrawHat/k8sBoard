# 0048 · Settings key, session state (step 3a), Metrics page (step 3b)

[Back to index](README.md) · Step 3a (app): `cluster_registry.rs`, `settings_tests.rs`, `cluster_metrics.rs`, `active_session.rs`, `app_shell.rs`. Step 3b (app): `metrics_page.rs` (new) + `metrics_page_tests.rs`, `settings_window.rs`, `clusters_page.rs`, `launch_options.rs`, `screenshot.rs`.

## Setting (3a)

| Key | Type · default | Read where | Takes effect | Validation |
|---|---|---|---|---|
| `registry.clusters[].metrics` | `Option<cluster::MetricsSourceFields>` · `None` = metrics-server only | `ClusterRegistry::profile` → `ClusterProfile.metrics: Option<Result<MetricsSource, MetricsSourceError>>` | session start; a save for the active cluster restarts the check at once | `MetricsSource::new`; an invalid stored entry fails closed: `SourceState::Invalid`, no request, the stored text is not rewritten |

JSON: `"metrics": {"namespace":"monitoring","service":"vmselect-vm-victoria-metrics-k8s-stack","port":"8481","scheme":"http","prefix":"/select/0/prometheus"}`. Allow-list (`settings_keys_are_the_allow_list`) adds `registry.clusters.metrics`, `.namespace`, `.service`, `.port`, `.scheme`, `.prefix`. Reset to defaults (0043) clears it.

## Session state (3a, `cluster_metrics.rs`)

```rust
pub(crate) enum SourceState {
    None,                                       // metrics-server only
    Invalid,                                    // stored entry does not validate
    Checking { source: MetricsSource, _task: Task<()> },
    Ready { source: MetricsSource, check: SourceCheck },
    Failed { source: MetricsSource, error: MetricsError },
}
ClusterMetrics.source: SourceState;           // set at session start from `profile.metrics`
pub(crate) fn source_note(state: &SourceState) -> Option<String>;   // the Monitor/Traffic reason text
```

- The check runs `connection.check_metrics_source` on `ClusterRuntime`; the result lands through `cx.spawn` (one `cx.notify()`). A `cpu_series` of 0 is still `Ready`, but the Monitor offers no 7d/30d and stays on the sampler ([monitor-ranges.md](monitor-ranges.md)); 0049 may still use the source.
- `AppShell` already observes `AppSettings`; when the active cluster's `metrics` entry differs from the one the state was built from, it rebuilds the state (dropping a running check).
- `ActiveConnection` (new `Global` in `active_session.rs`): `{ cluster: ClusterRef, label: String, connection: ClusterConnection }`, set when the session goes Live, removed on switch and close. The Metrics page reads it; it never opens a connection of its own.

## Metrics page (3b, `metrics_page.rs`, entity `MetricsPage`, embedded like `ClustersPage`)

Nav order (W2): General, Clusters, Appearance, Keyboard Shortcuts, Safety, Terminal & Shell, Logs, **Metrics**, About. `SettingsPage::Metrics`, title `Metrics`. Extensions stays out.

```text
Metrics · readonly@Monitor
Where the Monitor tab reads 7- and 30-day history and Topology reads traffic. k8sBoard reaches it through the
API server service proxy with your kubeconfig credentials and stores no credential.
Source
 (•) metrics-server only (default)          CPU and memory sampled by k8sBoard while it runs; 24 hours
 ( ) VictoriaMetrics cluster   monitoring/vmselect-vm-victoria-metrics-k8s-stack:8481  /select/0/prometheus
 ( ) VictoriaMetrics query     monitoring/vmquery-vm-victoria-metrics-k8s-stack:8481   /select/0/prometheus
 ( ) Other service             [namespace] [service] [port] [http ▾] [path prefix]
[Detect again]  [Test]  Reachable · 35 ms · 1,234 CPU series                         [Save]
Saved: monitoring/vmselect-…:8481 /select/0/prometheus · Ready
```

| Element | Behaviour |
|---|---|
| Header | `Metrics · {label}` of `ActiveConnection`; without one the page shows only the muted `Connect to a cluster to choose its metrics source.` |
| Detection | on first render per page entity and on `Detect again`: `list_all_services` → `metrics_candidates` on the runtime; `Detecting…` meanwhile; an error shows `Cannot list services: {reason}. Enter the service below.` |
| Rows | the radio pattern of `confirm_dialog.rs`; the saved source is pre-selected (as `Other service` with its fields when detection did not find it); mono for names |
| Other service | five inputs; each validates on change with the [source-and-transport.md](source-and-transport.md) messages; Test and Save stay disabled while invalid |
| Test | `check_metrics_source` for the selected choice (not saved). Results: `Reachable · {ms} ms · {n} CPU series`; `Reachable, but it has no container_cpu_usage_seconds_total series` (warn tone); the `MetricsError` text (bad tone). `metrics-server only` disables Test |
| Save | writes `registry.clusters[active].metrics` through `AppSettings::update` (`None` for `metrics-server only`); a `--screenshot` run writes nothing (0024). The session re-check follows (above) |
| Saved line | the saved source and the session's `SourceState` (`Checking…`, `Ready`, `Failed: {reason}`, `Invalid entry in settings`) |

The page holds the running detection and test as `Task`s; closing the window or switching the page drops them.

## Cluster form row (3b, `clusters_page.rs`, W2 `Metrics · Source ▾`)

A fourth section `Metrics` after Safety with one row `Source`: a small outline dropdown button showing `metrics-server only` or `Prometheus-compatible · {ns}/{svc}:{port}` (the flavor is not stored). Menu items: `metrics-server only` (clears the entry), the saved source (checked) when set, a separator, `Choose on the Metrics page…` (shows the Metrics page; disabled with the tooltip `Connect to this cluster first` unless the row is the active cluster). Hint (muted): `Used for 7- and 30-day Monitor ranges and Topology traffic.`

## Screens (3b)

| Screen | Content |
|---|---|
| `settings-metrics` | live: the Metrics page of the connected cluster after detection settles (UAT: the two VictoriaMetrics rows) |
| `settings-metrics-fixture` | no cluster: a fixed candidate list (vmselect, vmquery, a Prometheus row), `Other service` filled, Test result `Reachable · 35 ms · 1,234 CPU series` |
