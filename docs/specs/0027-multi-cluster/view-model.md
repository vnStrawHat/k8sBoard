# 0027 · View model

[Back to index](README.md) · Step 1 · Modules: `cluster_view.rs` (new: pure plan + slot list), `app_shell.rs`, `title_bar.rs`, `status_bar.rs`, `namespace_picker.rs`, `cluster_session.rs` (test seams). Decisions 2–8, 11, 12, 14, 20, 22, 24, 26.

## Types (`cluster_view.rs`)

```rust
pub(crate) const MAX_VIEWED_CLUSTERS: usize = 5;
pub(crate) struct ViewSlot { pub(crate) cluster: ClusterRef, pub(crate) summary: ContextSummary,
    pub(crate) profile: ClusterProfile, pub(crate) session: Entity<ClusterSession>,
    pub(crate) has_reported_live: bool, _observer: Subscription }
#[derive(Default)]
pub(crate) struct ClusterView { slots: Vec<ViewSlot>, primary: usize }
impl ClusterView {
    pub(crate) fn primary(&self) -> Option<&ViewSlot>;
    pub(crate) fn slots(&self) -> &[ViewSlot];
    pub(crate) fn slot_of(&self, cluster: &ClusterRef) -> Option<usize>;
    pub(crate) fn is_multi(&self) -> bool;                 // slots.len() >= 2
    pub(crate) fn riskiest(&self) -> Option<Environment>;  // border only (decision 6)
}
pub(crate) struct ViewPlan { pub(crate) keep: Vec<ClusterRef>, pub(crate) release: Vec<ClusterRef>,
    pub(crate) connect: Vec<ClusterRef>, pub(crate) primary: ClusterRef }
/// Pure. `current_primary` stays primary when it is in `wanted`; else the first in display order.
/// `Ok(None)`: nothing viewable was wanted (every wanted cluster is outside `display_order`).
pub(crate) fn plan_view(current: &[ClusterRef], current_primary: Option<&ClusterRef>,
    wanted: &[ClusterRef], display_order: &[ClusterRef]) -> Result<Option<ViewPlan>, TooManyClusters>;
```

`AppShell.session` (0026) becomes `view: ClusterView`. `active` (0026) is `view.primary()`; `previous` keeps 0026 meaning and is set only by single switches. `AppShell::slot_live(&self, cluster: &ClusterRef, cx) -> Option<&LiveCluster>` reads one slot; `live(cx)` (today) becomes the primary's and is not used on drawer paths (aggregated-views.md).

## Applying a set

```rust
impl AppShell {
    /// One cluster: 0026 `switch_cluster`. Two or more: multi apply.
    pub(crate) fn view_clusters(&mut self, wanted: &[ClusterRef], cx: &mut Context<Self>);
}
```

1. `plan_view` (Err → switcher notice; `None` → nothing to do). Targets missing from the catalog are dropped with the 0026 decision 20 notice.
2. Remember: each Live slot → `remember_scope` (0026).
3. Release: for each released slot, close its log tabs; close the drawer if its subject is in that slot; remove the slot (the entity is released).
4. Reorder kept slots to display order; set `primary`; tables `set_sessions(view.sessions())`; `cx.notify()`.
5. `cx.defer`: re-check that this plan is still the latest request; for each `connect` cluster, `ClusterSession::new(.., view_scope, kind, cx)`, insert the slot at its display position (`has_reported_live = false`), `set_sessions` again.
6. Filters: kept when the primary is unchanged, else reset (decision 11).

`view_scope` = the primary's current scope (coming from single mode, its Live scope; else its 0026 `start_scope`).

## Fan-out to sessions

| Call | Goes to |
|---|---|
| `set_scope`, `set_explorer_kind`, `refresh_kind_counts`, `set_event_filter`, `set_explorer_paused` | every slot |
| `set_event_subject`, `set_related_subject`, `set_kubelet_demand` | the drawer subject's slot; every other slot gets `None` / `KubeletDemand::default()` (decision 22) |

## Slot going Live

`on_session_changed` runs per slot observer. On a slot's first Live (its `has_reported_live`): if `live.scope != view_scope`, call `set_scope(view_scope)` on it (decision 12); if it is the primary, write `last_used` (0024, decision 24).

## Namespace scope

- `set_namespace(scope)` fans out (table above); `view_scope` is updated.
- Picker list: the union of `live.namespaces` names over Live slots whose namespace list is ready, sorted; one muted line per other slot: `Loading namespaces of {label}…` (or `Not permitted in {label}` when denied). The optional muted `{n}/{m} clusters` after names missing in some slots is not built.
- `MAX_NAMESPACES` (5) still applies.

## Title bar

| Part | Single (0024/0026) | Multi |
|---|---|---|
| Badge | slot env | primary env |
| Label | `switcher_label` | primary label + muted chip `+{n−1}` |
| Top border | slot env color | `environment_color(view.riskiest())` |
| Tooltip | — | `Viewing: {label}, {label}, …` |

## Status bar

`Watching {k} resource types · {n} clusters` (k = sum of `watch_count()`). Optional: `API {min}–{max} ms` over Live slots. Interrupted slots are per-slot banners (aggregated-views.md), not status text.

## Test seams (`cluster_session.rs`, `#[cfg(test)]`)

```rust
impl ClusterSession {
    /// Enters Live at once from a connection built by `ClusterConnection::open` (no request is
    /// made by `open`); watches start and fail against the fixture's dead port, harmlessly.
    pub(crate) fn live_fixture(kubeconfig: Arc<Kubeconfig>, summary: &ContextSummary,
        connection: ClusterConnection, scope: NamespaceScope, cx: &mut Context<Self>) -> Self;
    /// Applies `WatchUpdate::Snapshot(pods)` through `LiveList::apply`.
    pub(crate) fn seed_pods(&mut self, pods: Vec<PodSummary>, cx: &mut Context<Self>);
}
```

`live_fixture` builds `Connected { connection, server_version: v1.29.5 fixture, scope, access: Err("fixture") }` and calls `LiveCluster::start`. Tests build the connection with `runtime.block_on(ClusterConnection::open(..))` on the 0026 fixture runtime. Add `seed_namespaces` / `seed_nodes` the same way only when a test needs them.
