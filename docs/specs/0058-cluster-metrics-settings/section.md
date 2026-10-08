# 0058 · `ClusterMetricsSection` (`cluster_metrics_section.rs`)

[Back to index](README.md) · `git mv metrics_page.rs cluster_metrics_section.rs` and `metrics_page_tests.rs cluster_metrics_section_tests.rs`, then edit. Rename `MetricsPage` → `ClusterMetricsSection`, `MetricsFixture` → `ClusterMetricsFixture`. Module doc: "The Metrics section of the selected cluster on the Clusters page (specs 0048, 0058)…".

## Moves unchanged

`Choice`, `Detection`, `TestResult`, `OtherField` (+ `ALL`, `error`), `invalid_fields`, `initial_choice`, `test_line`, `saved_line`, `blank_fields`, `candidate_detail`, `muted`; the constants `LABEL_WIDTH`, `SHORT_FIELD_WIDTH`, `FIELD_WIDTH`, `METRICS_SERVER_ONLY`, `METRICS_SERVER_DETAIL`, `PAGE_INTRO` (renamed `SECTION_INTRO`, same text); the methods `set_inputs`, `saved_entry`, `finish_detection`, `candidates`, `on_other_changed`, `choose`, `other_fields`, `selected`, `is_detecting`, `save`, `pick_scheme`, `is_settled`, `render_choices`, `render_other`, `render_detection_note`; the fixture data.

## Deleted

`NO_CLUSTER_TEXT` (no cluster now renders an empty `div()`), `open`, the public `fixture` constructor (becomes private `apply_fixture`), the cluster-switch half of `follow_connection`, the `session: Option<WeakEntity<ClusterSession>>` field (read from `ActiveConnection` when it is this cluster).

## Struct and API

```rust
pub(crate) struct ClusterMetricsSection {
    catalog: Entity<ClusterCatalog>,       // new: finds the kubeconfig of a cluster that is not open
    cluster: Option<ClusterRef>, label: String,
    detection: Detection, choice: Choice, has_chosen: bool,
    namespace, service, port, prefix: Entity<InputState>, scheme: MetricsScheme,
    test: TestResult, is_fixture: bool,
    _session_observer: Option<Subscription>, _observers: Vec<Subscription>,
}
impl ClusterMetricsSection {
    pub(crate) fn new(catalog: Entity<ClusterCatalog>, window: &mut Window, cx: &mut Context<Self>) -> Self;
    /// The page selected `row` (or nothing). Always resets, even for the same row (Reset to
    /// defaults): choice from the saved entry, inputs, detection `Idle`, test `Idle`, `has_chosen`
    /// false. Under `ClusterMetricsFixture` it applies the fixture data instead.
    pub(crate) fn show_cluster(&mut self, row: Option<&ClusterRow>, window: &mut Window, cx: &mut Context<Self>);
    /// The cluster shown; the page reads it to skip a `show_cluster(None)` that changes nothing.
    pub(crate) fn cluster(&self) -> Option<&ClusterRef>;
    #[cfg(any(feature = "screenshot", test))] pub(crate) fn is_settled(&self) -> bool; // unchanged rule
}
/// How a click reaches the cluster.
enum Reach {
    Open(ClusterConnection),                                   // `ActiveConnection` is this cluster
    OneTime { kubeconfig: Arc<Kubeconfig>, context: String, proxy: Result<ProxyChoice, ProxyUrlError> },
}
fn reach(&self, cx: &App) -> Option<Reach>;   // None: the row is no longer in the catalog
fn is_open_cluster(&self, cx: &App) -> bool;  // ActiveConnection.cluster == self.cluster
/// Runs on the tokio runtime: `Open` as is; `OneTime` through `open_cluster` under
/// `TEST_CONNECTION_TIMEOUT`, its expiry `ClusterError::TimedOut { context, action: "connecting" }`.
async fn connect(reach: Reach) -> Result<ClusterConnection, ClusterError>;
```

- `OneTime` is built like `ClustersPage::start_test`: `find_cluster(&catalog.kubeconfigs(), &cluster)`, proxy = `AppSettings::get(cx).registry.profile(&summary).proxy`, read at click time.
- `start_detection`: `runtime.spawn(async move { connect(reach).await?.list_all_services().await })`; result handling unchanged (`finish_detection`).
- `start_test`: `connect(reach)`; `Err(e)` → `MetricsError::Unexpected(format!("cannot connect: {}", error_text(&e)))`; `Ok(c)` → `c.check_metrics_source(&source)`.
- The client of a `OneTime` reach lives inside the spawned future only; dropping the `Task` (reset, page closed) drops it.
- Observers: `ActiveConnection` → `follow_session(cx)` (re-observe the session when it is this cluster, else drop `_session_observer`) + `cx.notify()`; `AppSettings` → `cx.notify()`; the four inputs → `on_other_changed` (unchanged).

## Behaviour

| Element | Open cluster | Cluster that is not open |
|---|---|---|
| Header | `Metrics · {row label}`, `text_sm`, semibold, bottom border (the form's section style) | same |
| Note under the header | `SECTION_INTRO` | `SECTION_INTRO` + muted `This cluster is not open. Detect and Test connect to it once with its kubeconfig and proxy.` |
| Detection | at first draw while `Idle` (as 0048) | only on click; nothing listed until then |
| Detect button | `Detect again`; disabled while detecting or fixture | `Detect` while `Idle`, then `Detect again`; same disabling; disabled when `reach` is `None` |
| Radio rows, Other service, messages | unchanged | unchanged (rows appear after a Detect) |
| Test | unchanged rules, through `ActiveConnection` | same rules, through a one-time client |
| Save | unchanged: `edit_entry(registry, cluster, entry.metrics = …)` | same (no connection needed) |
| Saved line | `saved_line(saved, Some(session state))` | `saved_line(saved, None)` → `Saved: {source}` |

- The saved source preselects as today: a candidate it equals, else `Other service` with its fields; `metrics-server only` without one.
- A reachable source with `cpu_series == 0` and every error text: unchanged from 0048 ([settings-page.md](../0048-prometheus-metrics/settings-page.md)).
- Fixture (`ClusterMetricsFixture` global): `show_cluster(Some(row))` sets cluster and label from the row, then the fixed candidates, `Other service` = `monitoring/thanos-query:10902`, Test `Reachable · 35 ms · 1,234 CPU series`; Detect and Save disabled and Test a no-op, as in 0048; nothing is sent.
