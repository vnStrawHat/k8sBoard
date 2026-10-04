# 0026 · Health probes

[Back to index](README.md) · Step 4 · Module: `cluster_health.rs` (new; pure board + probe stream). Decisions 9–13. C4.

## Model

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProbeResult { Reachable { latency: Duration }, Unreachable { reason: String } }
pub(crate) struct ProbeEntry { pub(crate) result: ProbeResult, pub(crate) at: Instant }
#[derive(Default)]
pub(crate) struct HealthBoard { entries: HashMap<ClusterRef, ProbeEntry>, running: HashSet<ClusterRef> }
pub(crate) enum RowHealth { Live(Duration), Connecting, Interrupted, Reachable(Duration), Unreachable,
    Checking, NotChecked }
pub(crate) struct ProbeCandidate { pub(crate) cluster: ClusterRef, pub(crate) auth: AuthKind, pub(crate) is_active: bool }
pub(crate) struct ProbeTarget { pub(crate) cluster: ClusterRef, pub(crate) kubeconfig: Arc<Kubeconfig>, pub(crate) context: String }
pub(crate) const PROBE_TTL: Duration = Duration::from_secs(60);
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const PROBE_CONCURRENCY: usize = 4;
impl HealthBoard {
    /// Clusters to probe on open: in-process auth, not active, not running, stale or unknown.
    pub(crate) fn due(&self, rows: &[ProbeCandidate], now: Instant) -> Vec<ClusterRef>;
    pub(crate) fn mark_running(&mut self, clusters: &[ClusterRef]);
    pub(crate) fn is_running(&self, cluster: &ClusterRef) -> bool;
    pub(crate) fn record(&mut self, cluster: ClusterRef, result: ProbeResult, now: Instant);
    /// Called when the switcher closes: probes were aborted, so those rows are due again.
    pub(crate) fn clear_running(&mut self);
    pub(crate) fn row_health(&self, cluster: &ClusterRef) -> RowHealth; // non-active rows
}
pub(crate) fn is_probed_automatically(auth: &AuthKind, origin: RowOrigin) -> bool; // false for Exec, AuthProvider, and every watched-folder row (0043)
```

`AuthKind` is 0025 step 1's `Kubeconfig::connection_info(..).auth`. `Instant` is passed in so tests are deterministic. `ClusterRef` is a map key (0024 derive amended with `Hash`).

## When

| Trigger | Action |
|---|---|
| Switcher opens | probe `due(..)`: rows with `is_probed_automatically`, not active, no entry younger than `PROBE_TTL`, not running |
| A row of a watched folder (0043) | never `due`, whatever its auth kind (`ProbeCandidate.origin`, `is_probed_automatically(auth, origin)`): a dropped file can aim `tokenFile` at a real token and `server` at any host; only the per-row Check probes it |
| Retry on an Unreachable non-active row, Check on a NotChecked row | probe that row only, any auth kind (the user asked) |
| Retry on the **active** row (session Failed) | `ClusterSession::retry` (same as the workspace Retry); no probe |
| Retry/Check on a row that `is_running` | no-op |
| Switcher closes | `probes.clear()` drops every subscription (aborts running probes), then `clear_running()`; aborted rows have no new entry, so they are due on the next open |
| Switcher closed | no probe at all (no background polling) |

The active cluster is never probed; its row shows `Live(api_latency)`, `Connecting`, `Interrupted` (`live.has_problem()`), or `Unreachable` (Failed) from the session.

## Probe stream

```rust
/// Runs on tokio. Yields one result per target as it finishes.
pub(crate) fn probe_stream(targets: Vec<ProbeTarget>) -> impl Stream<Item = (ClusterRef, ProbeResult)> + Send;
```

- `futures::stream::iter(targets).map(probe_one).buffer_unordered(PROBE_CONCURRENCY)`.
- `probe_one` = `tokio::time::timeout(PROBE_TIMEOUT, async { let c = ClusterConnection::open(&kubeconfig, &context).await?; let start = Instant::now(); c.server_version().await?; Ok(start.elapsed()) })`. Latency times `server_version` only, the same as the status bar (switch-lifecycle.md).
- Timeout → `Unreachable { reason: "no answer in 5 s" }`; error → `Unreachable { reason: error_text(&error) }` (credential-free, 0001). The reason is the row tooltip only.
- Testability: `probe_stream` delegates to a private generic `probe_all<F>(targets, probe: F)` so a test can count in-flight probes.

The shell starts each stream with `ClusterRuntime::subscribe(stream, cx, apply = record + notify, on_closed = nothing)` and pushes the `WatchSubscription` into `ClusterSwitcherState.probes`. One-target Retry/Check streams are separate subscriptions, so they **bypass `PROBE_CONCURRENCY`**; they are user-paced clicks, so the bound on the open-time fan-out is what matters.

## Cost (C4)

- Per open: at most one connection setup + one `GET /version` per in-process-auth cluster, 4 at a time, then nothing for 60 s.
- No watch, no list, no SSAR. The `ClusterConnection` is dropped right after the version call.
- Each connect logs the existing "ignoring the proxy from the environment" info line when a proxy variable is set; repeated lines are harmless (host only, no credential).
- Exec plugins (EKS, GKE, AKS) never run without a click (decision 9).

## Read-only

`server_version` is a GET. No new call site in the cluster crate. The 0001 grep is unchanged.
