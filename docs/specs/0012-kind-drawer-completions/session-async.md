# 0012 · Session, watches, async

[Back to index](README.md) · Steps 2, 4a, 5 · Modules: `related_objects.rs` (new) + tests in module, `cluster_session.rs` (+ tests), `app_shell.rs`, `screenshot.rs`, `navigation.rs`

All network I/O stays on the cluster runtime; results reach the session through `subscribe` (watches) or `cx.spawn` (counts), batched before `cx.notify()` as today.

## Related watch (step 2; `ConfigMapValues` in step 4b)

```rust
// related_objects.rs (pure)
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RelatedSubject {
    ReplicaSets { namespace: String, deployment: String, selector: String }, // selector terms joined ","
    Jobs { namespace: String, cron_job: String },
    ConfigMapValues { namespace: String, name: String },
}
pub(crate) fn related_subject(kind: ResourceKind, row: &KindRow) -> Option<RelatedSubject>; // from row.object
// cluster_session.rs
pub(crate) struct RelatedObjects { pub(crate) subject: RelatedSubject, pub(crate) list: RelatedList, _subscription: WatchSubscription }
pub(crate) enum RelatedList {
    ReplicaSets(LiveList<ReplicaSetSummary>), Jobs(LiveList<JobSummary>), ConfigMapValues(LiveList<ConfigMapValues>),
}
impl ClusterSession { pub(crate) fn set_related_subject(&mut self, subject: Option<RelatedSubject>, cx: &mut Context<Self>); }
impl LiveCluster {
    pub(crate) fn related_subject(&self) -> Option<&RelatedSubject>;
    pub(crate) fn related_of(&self, subject: &RelatedSubject) -> Option<&RelatedList>; // None for another subject
}
```


- Same lifecycle as object events (0006): `set_related_subject` with the same subject is a no-op; a new one drops the old subscription first; `None` stops it. A session that is not live ignores it.
- One subscription: the typed stream is mapped on tokio into `RelatedUpdate::{ReplicaSets(WatchUpdate<_>), Jobs(_), ConfigMapValues(_)}`; `RelatedList::apply` ignores a variant that does not match.
- A row without a subject (any other kind, Pods, Nodes, Events) stops the watch. A context switch drops the session and with it the watch.

### One debounce for both drawer watches (`app_shell.rs`)

`follow_event_subject` becomes `follow_drawer_subjects`, called from `change_selection`:

```rust
drawer_subject_task: Option<Task<()>>,   // replaces event_subject_task; one 250 ms timer for both watches
fn follow_drawer_subjects(&mut self, cx: &mut Context<Self>);
pub(crate) fn subject_change<S: PartialEq>(running: Option<&S>, next: Option<S>) -> SubjectChange<S>; // 0006, now generic
```

- Next subjects: `event_subject(key)` and `related_subject(kind, row)` (the row is read from `live.kind_list(kind)`).
- Each watch whose change is `Stop` or `Start` is stopped at once. If any change is `Start`, one task waits `DRAWER_SUBJECT_DELAY` and then starts every pending subject in one `update`; a newer selection replaces the task. `Keep` leaves that watch running.
- Screenshot settle: `is_content_pending` waits for `drawer_subject_task` plus a `Loading` events or related list.

## Endpoint slices companion (step 4a)

```rust
pub(crate) struct KindList { /* … */ endpoint_slices: Option<EndpointSlices> }
struct EndpointSlices { list: LiveList<EndpointSliceSummary>, _subscription: WatchSubscription }
impl LiveCluster { pub(crate) fn endpoint_slices(&self) -> Option<&LiveList<EndpointSliceSummary>>; }
```

- `KindList::start(Services, …)` also starts `watch_endpoint_slices(scope)`, unless the access report is `Known` and denies `ListEndpointSlices` (then `None`, and the drawer shows the reason). A scope change restarts both with the explorer.
- If access becomes `Known` and denied after the start, the next explorer restart drops the companion; the running watch shows its own 403 failure until then.

## Join triggers (step 4a)

`LiveCluster::join_explorer(&mut self)` calls `kind_join::join_rows` on the explorer's `items_mut()` (new `LiveList::items_mut`, `Ready` only) after: an explorer snapshot, a pods snapshot or failure (Services, ConfigMaps, Namespaces; 4b adds the last two), or an endpoint slices snapshot or failure (Services). Pause stream (0009) applies only to Events, so frozen rows never need a join. One notify per update, as today.

## Sidebar counts (step 5)

```rust
pub(crate) struct KindCounts { scope: NamespaceScope, counts: HashMap<ResourceKind, u64>, refreshed_at: Option<Instant>, _task: Option<Task<()>> }
impl ClusterSession { fn refresh_kind_counts(&mut self, cx: &mut Context<Self>); }
```

- **Trigger**: `finish_access_review` with a `Known` report starts a run when `KindCounts.scope` differs from the live scope (once per scope: a retried review for the same scope does not recount); a scope change clears the counts. `AppShell::show_screen` also asks for a run, which starts only when 30 s passed since `refreshed_at`. A new run replaces the task.
- A run counts every kind in `ResourceKind::ALL` whose list check is allowed, on the cluster runtime: `stream::iter(kinds).map(count_objects).buffer_unordered(4)`, collected, then one `update` and one `notify`. Failures and `None` leave that kind without a count; only the kind and the error text are logged.
- `NavigationCounts` gains `kinds: HashMap<ResourceKind, usize>`; a live explorer list's count wins, then the counted one, else none.

## Watch count (AC 8, step 2; 4a adds the companion)

```rust
pub(crate) struct OpenWatches { pub(crate) namespaces: usize /* scope multiplicity N, All = 1 */,
    pub(crate) explorer: usize /* 0; 1 for Namespaces; else N */, pub(crate) endpoint_slices: bool, pub(crate) object_events: bool, pub(crate) related: bool }
fn open_watch_count(watches: OpenWatches) -> usize; // replaces open_watch_count(explorer, object_events)
```

`= 2 (namespaces list, nodes) + N (pods) + explorer + N·endpoint_slices + object_events + related` ≤ `3N + 4` (19 at N = 5). `LiveCluster::watch_count` (status bar) fills it from the session. Counts and YAML GETs are one-shot requests, not watches.
