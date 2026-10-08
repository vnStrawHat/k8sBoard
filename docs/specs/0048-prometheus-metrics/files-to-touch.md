# 0048 · Files to touch

[Back to index](README.md). **S** = step. Each step passes the gate alone; every new item has a production reader in its step or the next (step 1 items are read by step 2 in the same crate; if clippy flags dead code, merge steps 1 and 2). Base: main `81bbcaa`.

## Cargo

| S | File | Change |
|---|---|---|
| 2 | `crates/cluster/Cargo.toml` | `form_urlencoded = "1"` (locked via `url`) and `http-body-util` (locked via kube, same version as the lock) as direct dependencies; no new `[[package]]` |

## Cluster crate

| S | File | Change |
|---|---|---|
| 1 | `promql.rs` (new) + `promql_tests.rs` | escaping, `UsageTarget`, `WorkloadKind`, `UsageMetric`, `RangeSpec`, `RangeError`, query text builders, step table |
| 1 | `metrics_source.rs` (new) + `metrics_source_tests.rs` | `MetricsSourceFields` (serde), `MetricsScheme`, `MetricsSource`, `MetricsSourceError`, `MetricsFlavor`, `MetricsCandidate`, `metrics_candidates` |
| 2 | `metrics_query.rs` (new) + `metrics_query_tests.rs` | `MetricsEndpoint`, `metrics_get` (the one exception, `Client::send`), `MetricsAnswer`, capped body, deadline, status mapping, decode, `MetricsError`, `SourceCheck`, `UsageSeries`, `check_metrics_source`, `usage_range` |
| 2 | `service.rs` | `list_all_services` |
| 2 | `lib.rs` | `mod` lines and `pub use` of the public types above |
| 2 | `examples/probe.rs` | `--metrics-source <ns>/<svc>:<port>[<prefix>]`: prints the check, one 30d CPU `query_range` line (series, points, bytes, ms), and the three promql.md facts; never a body |

## App crate

| S | File | Change |
|---|---|---|
| 3a | `cluster_registry.rs` | `ClusterEntry.metrics`, `ClusterProfile.metrics`, reset clears it |
| 3a | `cluster_metrics.rs` | `SourceState`, `source_note`, `ClusterMetrics.source`, check start |
| 3a | `active_session.rs` | `ActiveConnection` global |
| 3a | `app_shell.rs` | set/clear `ActiveConnection`; rebuild `SourceState` on a settings change of the active entry |
| 3a | — | if clippy flags `ActiveConnection` fields as dead code before 3b reads them, move the global to 3b |
| 3b | `metrics_page.rs` (new) + `metrics_page_tests.rs` | `MetricsPage`: detection, choices, Other service inputs, Test, Save, saved line |
| 3b | `settings_window.rs` | `SettingsPage::Metrics`, nav order, embed `MetricsPage` |
| 3b | `clusters_page.rs` | `Metrics` section, `Source` dropdown |
| 3a | `settings_tests.rs` | allow-list keys |
| 3b | `settings_window_tests.rs` | nav order |
| 3b | `launch_options.rs`, `screenshot.rs` | `settings-metrics`, `settings-metrics-fixture` |
| 4 | `drawer.rs` | `MonitorRange::{Days7, Days30}`, `SAMPLER`, `SOURCE`, `source_step`, `is_long` |
| 4 | `monitor_source.rs` (new) + `monitor_source_tests.rs` | `SourceKey`, `SourceFetch`, `SourceResult`, `source_target`, `refresh_after`, `source_monitor_data`, `SourceView` |
| 4 | `monitor_data.rs` | share the reference-line and OOM-mark builders with `monitor_source.rs` (`pub(crate)`, no copy) |
| 4 | `monitor_tab.rs` | range set (the click indexes the shown set, not `ALL`), tooltip text, toolbar text, footer, notices, Alert |
| 4 | `app_shell.rs` | `source_fetch: Option<SourceFetch>` field; `sync_monitor_source(&mut Context)` next to `sync_kubelet_demand` (drops the fetch when `!shows_monitor()`), range reset; `refresh_monitor_cache` skips long ranges |
| 4 | `launch_options.rs`, `screenshot.rs` | `pod-monitor-source-fixture` |

## Docs (in the step that ships the code)

| S | File | Change |
|---|---|---|
| 2 | `docs/specs/0030-guardrails-write-path/write-path.md` | exception table row `metrics_query.rs` · `metrics_get` · 0048; grep paragraph |
| 3b | `docs/specs/0043-settings-pages/README.md` | non-goals: "Metrics … page (0048)" |
| 4 | `docs/roadmap/wireframe-gap-audit.md` | W4c n1 / W2 Metrics row → Done (0048) |
| 4 | `docs/roadmap/cross-cutting.md` | UAT table row stays as written by the architect; tick when verified |
| 4 | `as-built.md` (new, this folder) | deviations, the node-label fact (open item 1), checks run |
