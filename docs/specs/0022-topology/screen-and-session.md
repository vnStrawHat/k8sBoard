# 0022 · Screen, session, and async

[Back to index](README.md) · Steps 1–2 · Modules: `topology_feeds.rs` (new, tests in module), `topology_view.rs` (new), `cluster_session.rs`, `app_shell.rs`, `workspace.rs`, `navigation.rs`, `launch_options.rs`, `screenshot.rs`, `kind_drawer.rs` (Show in Topology)

## Screen wiring

- `Screen::Topology`. `screen_of("Topology")`, and the item is enabled in the sidebar.
- `show_screen(Topology)`: explorer `None` (as on Pods), then `session.set_topology_subject(view.subject())`. Leaving the screen calls `set_topology_subject(None)`. A namespace or kind-chip change calls it again.
- `AppShell` owns `topology: Entity<TopologyView>`, created next to `pod_table`. The workspace body renders it on Topology. No filter bar, selection bar, or table appears there.
- `rebuild_visible_view`, `sync_selection`, and `check_table` get no-op `Topology` arms.

## Header and toolbar (W11)

| Place | Content |
|---|---|
| header | `Topology`, the count `ns: {ns} · {n} resources` (loading: `ns: {ns} · loading…`), right: `Fit`, `Export PNG` (step 3) |
| toolbar row 1 | segment `Resources` (on) · `Traffic` (disabled, tooltip `Needs a service mesh; not available yet`); `Namespace: {ns} ▾`; `Group by: {app/components} ▾` (step 2; shows the resolved value, decision 27); chips `Ingress` `Service` `Workload` `Config` (step 2, toggles), `RBAC` (disabled, tooltip `Not shown in this version`); `Problems only` (step 2); right: checks chip (config-checks.md) |
| under the toolbar | coverage note (muted), when present |

The namespace dropdown lists the scope's namespaces: `Named` gives one, `Several` its list, and `All` every Ready namespace from `live.namespaces`, sorted.

## View state (`topology_view.rs`)

```rust
pub(crate) struct TopologyView {
    session: Option<Entity<ClusterSession>>, shell: WeakEntity<AppShell>,
    namespace: Option<String>,                     // decision 1; defaults below
    filter: TopologyFilter, expanded: BTreeSet<NodeId>,
    pins: HashMap<(String /* context */, String /* ns */), HashMap<NodeId, GraphPoint>>,
    build: Option<Result<Rc<TopologyGraph>, TooLarge>>, layout: Option<(GraphStructure, GroupBy, Rc<TopologyLayout>)>,
    viewport: Viewport, needs_fit: bool, highlighted: Option<NodeId>, pending_focus: Option<NodeId>, drag: Drag,
    is_dirty: bool, _tick: Option<Task<()>>, _observe: Option<Subscription>,
    export: ExportState, _export: Option<Task<()>>,  // step 3
}
```

- Namespace default: `Named(ns)` → `ns`; `Several` → the first, and the header adds `1 of {n} namespaces in scope` (the dropdown lists the scope only); `All` → the namespace Topology drew last in this context (`last_namespaces`, kept while the app runs), else the one with the most pods among the pods already loaded (no new watch). A namespace that leaves the scope resets to the default. UX walk H6.
- With `None` (`All`, before the pods have loaded or when there are none) the canvas area shows `Pick a namespace above to draw its topology.` and the dropdown is the primary button, labelled `Pick a namespace`.
- A context switch (new session) clears `build`, `layout`, `expanded`, and the selection. A namespace change clears `expanded` and lays out from scratch. `pins` is keyed by context, so other contexts keep theirs.

## Feeds and watch budget (`topology_feeds.rs`, `cluster_session.rs`)

```rust
pub(crate) const TOPOLOGY_FEED_KINDS: [ResourceKind; 10] = [Ingresses, Services, Deployments, StatefulSets,
    DaemonSets, ReplicaSets, ConfigMaps, Secrets, PersistentVolumeClaims, HorizontalPodAutoscalers];
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TopologySubject { pub(crate) namespace: String, pub(crate) kinds: BTreeSet<KindFilter> }
pub(crate) struct TopologyFeeds { pub(crate) subject: TopologySubject, pub(crate) feeds: Vec<TopologyFeed> } // chip-disabled kinds absent
pub(crate) struct TopologyFeed { pub(crate) kind: ResourceKind, pub(crate) list: LiveList<KindRow>, off: Option<String>, _subscription: Option<WatchSubscription> }
pub(crate) fn feed_plan(kind: ResourceKind, access: &AccessState) -> FeedStart; // Known + denied list check → Off("not permitted"), else Start
impl TopologyFeeds { pub(crate) fn open_count(&self) -> usize; pub(crate) fn rows(&self, kind) -> FeedRows<'_>; }
impl ClusterSession { pub(crate) fn set_topology_subject(&mut self, subject: Option<TopologySubject>, cx: &mut Context<Self>); }
impl LiveCluster {
    pub(crate) fn topology(&self) -> Option<&TopologyFeeds>;
    /// The kind row of `key`: the explorer list first, then the topology feeds. Replaces the
    /// `kind_list(kind)?…find(is_row)` lookups of the drawer, the kind menu, and the YAML tab only (decision 26).
    pub(crate) fn row_of(&self, key: &ResourceKey) -> Option<&KindRow>;
}
```

- Each feed is `runtime.subscribe(kind.watch_rows(&connection, NamespaceScope::Named(ns), EventFilter::default()), ..)`. Rows are mapped on tokio, exactly like the explorer. Apply updates the list (`LiveList::apply`), and `subscribe` notifies.
- Feeds start for `TOPOLOGY_FEED_KINDS` whose `TopologyKind::filter()` is in `subject.kinds` (decisions 3, 5: Secrets run only with Config on). Same subject → no-op. A changed subject drops the feeds of removed kinds, starts added ones, and keeps the rest when the namespace is unchanged; a new namespace restarts all. `None` drops them all. A scope change that removes the namespace drops them, and the view picks its default. `finish_access_review` re-plans Off feeds.
- **Budget** (AC 5): `OpenWatches.topology = feeds.open_count()` (≤ 10, all `Named`; chip-disabled and Off kinds count 0). With Topology hidden it is 0. On Topology the explorer is 0, so the bound grows by at most 10 over the 0020/0021 bound.
- Memory: rows of one namespace. Secrets rows hold no values (0016). An oversized namespace still lists; the build then stops at `RAW_LIMIT` before any work (decision 14).

## Async and refresh (decision 29)

- `TopologyView` observes the session (`cx.observe`) and only sets `is_dirty`. While the view is visible, a `cx.spawn` loop wakes every `TOPOLOGY_TICK` (500 ms). When dirty, it builds (`build_topology`), wraps the graph in an `Rc`, lays out per layout.md "When to pass `previous`", and notifies once if anything changed. Leaving the screen drops the task.
- Everything runs on the main thread: the input is bounded by `RAW_LIMIT`, and the budget test covers the realistic case (AC 6). Network I/O stays on the cluster runtime.
- `pending_focus` (Show in Topology, check click) resolves after a build that contains the id: `center_on` its rect, select it, then clear it.

## Drawer and selection (decision 26)

- `AppShell::select_on_topology(key: Option<ResourceKey>)` = `change_selection(key)` without `show_screen`. The drawer renders over the workspace through `row_of`; pods use `live.pods`. Monitor and related lists read the explorer only (README open item 5). Drawer links (owner, pods, Open pod) still `reveal` and leave Topology.
- `TopologyView` reads `shell.selected` in render to draw the selection ring. Closing the drawer clears it.

## Show in Topology (step 2, decision 36)

The Service and Ingress kind menus get a `Show in Topology` item → `AppShell::show_in_topology(key)`. It sets `view.namespace`, sets `pending_focus = Object { kind, name }`, and calls `show_screen(Topology)`. It is disabled with `Namespace {ns} is outside the scope` when the scope does not contain the namespace.

## Launch and screenshots

- `--screen topology` (`LaunchScreen::Topology`, `USAGE`). The existing `--namespace <ns>` sets the scope, so the namespace default above picks it.
- `--screen topology-problems` (step 2, `LaunchScreen::TopologyProblems`): the same screen with Problems only on.
- Screenshot settle (`screenshot.rs`): wait until every started feed is Ready or Off, pods are Ready, and `build` exists.
