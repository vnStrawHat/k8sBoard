# 0020 · Feeds, coverage, session, async

[Back to index](README.md) · Step 1a (core feeds, Warning events, board in the session, tick, silent subscription) and step 2 (condition feeds, watch count) · Modules: `issue_feeds.rs` (new, + `issue_feeds_tests.rs`), `cluster_session.rs`, `cluster_runtime.rs`, `kubelet_history.rs`, `metrics_history.rs`

All network I/O stays on the cluster runtime. Feeds reach the session through `subscribe_silent`; the board runs on the main thread from one session task.

## Which rule needs which feed

| Feed | Watch / poll | Rules |
|---|---|---|
| Pods, Nodes, Namespaces | existing watches | pod and node rules; NamespaceStuck |
| Warning events | **new** `watch_events(scope, WarningsOnly)`, `EVENT_LIMIT` cap | event rules, PodStuck text, pod probe texts, PvcPending |
| Pod / node metrics, kubelet | existing polls (0010, 0011) | PodMemory, PodCpu, NodeMemory, NodeCpu, VolumeFull |
| Deployments, DaemonSets, Jobs, HPAs, PDBs, ResourceQuotas, PVCs, TLS Secrets | **new**, the existing typed watches (`watch_deployments`, …, `watch_tls_secrets`) mapped to `KindObject` on tokio | [kind-rules.md](kind-rules.md) |

## Types (`issue_feeds.rs`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum IssueFeed { Pods, Nodes, Namespaces, WarningEvents, PodMetrics, NodeMetrics, VolumeUsage, Kind(ResourceKind) }
// Step 1a declares Pods, Nodes, WarningEvents, PodMetrics, NodeMetrics, VolumeUsage; Namespaces and Kind(..) arrive in step 2 with NamespaceStuck and the condition feeds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FeedState { Live, Loading, Limited(String), Off(String) }
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Coverage { pub(crate) feeds: Vec<(IssueFeed, FeedState)> }
impl Coverage {
    pub(crate) fn is_partial(&self) -> bool;          // any Loading or Off; Limited does not count
    pub(crate) fn note(&self) -> Option<String>;      // text below
}

/// The Warning feed, re-sorted by involved object on each snapshot so a pod's events are one slice.
pub(crate) struct WarningEvents { list: LiveList<EventSummary>, by_object: HashMap<IssueObject, Range<usize>> }
impl WarningEvents {
    fn apply(&mut self, update: WatchUpdate<EventSummary>);   // sort by (namespace, kind, name, last_seen desc), rebuild index
    pub(crate) fn of(&self, object: &IssueObject) -> Option<&[EventSummary]>; // None unless Ready; Some(&[]) when none
    pub(crate) fn recent(&self) -> impl Iterator<Item = &EventSummary>;
}

/// The watches the issues need beyond the core lists; in `LiveCluster`. Dropping it stops them.
pub(crate) struct IssueFeeds { pub(crate) events: WarningEvents, _events: WatchSubscription, pub(crate) conditions: Vec<ConditionFeed> }
pub(crate) struct ConditionFeed { pub(crate) kind: ResourceKind, pub(crate) list: LiveList<KindObject>,
    off: Option<String> /* Off reason */, _subscription: Option<WatchSubscription> }   // None once Off
pub(crate) const CONDITION_KINDS: [ResourceKind; 8];  // the 7 kinds + Secrets (TLS only)
pub(crate) fn condition_plan(scope: &NamespaceScope, access: &AccessState) -> Vec<(ResourceKind, FeedPlan)>;
pub(crate) enum FeedPlan { Start { watch_scope: NamespaceScope }, Wait, Off(String) }
```

- Multiplicity `N = max(1, scope.namespaces().len())`. `condition_plan`: review `Checking` → `Wait`; N ≤ 2 and a `Known` report denying the kind's list check (`ListSecrets` for TLS) → `Off("not permitted: {check}")`; else `Start` with `watch_scope` = the session scope for N ≤ 2, `NamespaceScope::All` for N > 2.
- An All-scope feed for a narrower scope drops objects outside `scope.namespaces()` in the tokio map, before the snapshot reaches the session. Its first `Failed(ClusterError::Forbidden { .. })` turns it Off(`not permitted cluster-wide`) and drops the subscription (no retry storm); a scope change or retry plans again.
- `LiveCluster.issue_feeds` is built in `LiveCluster::start` (Warning events at once); `finish_access_review` and a scope change call `IssueFeeds::restart_conditions`; a scope change also restarts the events watch, **debounced by 1 s** (`EVENTS_RESTART_DELAY`): an events watch scans every event of the scope in etcd, so picking namespaces one after another must not run one scan per click. While the restart waits the feed reads Loading and counts no watch.
- Core states come from the lists (`Loading`; `Ready` → Live; `Failed` → Off(message); a failed list still counts as settled for core readiness, so the header reads "partial", not "checking" forever) and the metrics `FeedStatus` (`Live`/`Interrupted` → Live; `Unavailable`/`Failed` → Off; `Checking`/`Waiting` → Loading). `VolumeUsage` is `Limited("{k} of {n} nodes polled")` when the kubelet feed polls fewer Ready nodes than exist, while `Live`, `Interrupted`, or `Waiting` (with no node to poll no round ever comes, so waiting must not read as loading).

## Coverage note

`None` when every feed is Live. Else, grouped by state, feed labels (`pods`, `warning events`, `rollouts` for Deployments+DaemonSets, `jobs`, `HPAs`, `PDBs`, `quotas`, `volume claims`, `certificates`, `metrics`, `volume usage`): `Not checked: certificates (not permitted cluster-wide). Loading: warning events.` Limited feeds add a muted sentence (`Volume usage: 10 of 42 nodes polled.`) that is not toned Warn. Reasons never contain secret data (`error_text` rules).

## Silent subscription (`cluster_runtime.rs`)

```rust
pub(crate) fn subscribe_silent<V: 'static, U: Send + 'static>(&self, updates: impl Stream<Item = U> + Send + 'static,
    cx: &mut Context<V>, apply: impl Fn(&mut V, U, &mut Context<V>) + 'static,
    on_closed: impl FnOnce(&mut V, &mut Context<V>) + 'static) -> WatchSubscription;
```

Same as `subscribe` without `cx.notify()`; both share one private helper with `enum UpdateNotice { Notify, Silent }`. Issue feed callbacks update their list and call `session.issues.mark_dirty()`; core list and metrics callbacks also mark dirty (and keep notifying).

## Board and tick (`ClusterSession`, decisions 12, 13, 22)

```rust
pub(crate) struct ClusterSession { /* … */ issues: IssueBoard, is_issues_visible: bool, _issue_tick: Task<()> }
pub(crate) fn issues(&self) -> &IssueBoard;                      // sidebar, title bar, Issues table, 0021
pub(crate) fn set_issues_visible(&mut self, is_visible: bool);   // AppShell::show_screen
fn refresh_issues(&mut self, cx: &mut Context<Self>);            // the tick calls it every ISSUE_TICK
```

`refresh_issues`: return unless live and `run_due(now)` is `Some(reason)`; build `IssueInputs` from `LiveCluster` (Ready lists only) and `Coverage`; notify when `refresh` returns true, or when `reason == TimeRefresh` and `is_issues_visible`. The board survives `retry` and scope changes; a new context makes a new `ClusterSession`. First-seen times are memory only: they do not survive an app restart.

## Watch count (status bar; extends 0012 `OpenWatches`)

`OpenWatches` gains `issue_feeds: usize` = N (events) + 8 × (N for N ≤ 2, else 1). Bound: `12N + 4` for N ≤ 2 (16 at N = 1, 28 at N = 2), `4N + 12` for N > 2. `open_watch_count` tests cover N = 1, 2, 3.

## History accessors

- `kubelet_history.rs`: `pub(crate) fn pvc_usages(&self) -> impl Iterator<Item = &PvcUsage>` (newest sample per claim).
- `metrics_history.rs`: `pub(crate) fn latest_container_pair(&self, namespace, pod, container) -> Option<[ResourceUsage; 2]>` (the last two fine ticks, both present).
