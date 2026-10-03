# 0046 · The single-session model

[Back to index](README.md) · Steps 2–4.

## Types and API after step 4

```rust
// active_session.rs (replaces cluster_view.rs; same fields as the old ViewSlot)
pub(crate) struct ActiveSession {
    pub(crate) cluster: ClusterRef, pub(crate) summary: ContextSummary,
    pub(crate) profile: ClusterProfile, pub(crate) label: String,
    pub(crate) session: Entity<ClusterSession>,
    pub(crate) has_reported_live: bool, pub(crate) _observer: Subscription,
}
// AppShell
active: Option<ContextSummary>,          // 0026, set before the deferred connect (unchanged)
active_session: Option<ActiveSession>,   // was `view: ClusterView`; None between release and connect
pub(crate) fn session(&self) -> Option<&Entity<ClusterSession>>;          // unchanged
pub(crate) fn active_session(&self) -> Option<&ActiveSession>;             // was `view()`
pub(crate) fn active_cluster(&self) -> Option<ClusterRef>;                 // was `primary_cluster()`
fn slot_session(&self, cluster: &ClusterRef) -> Option<&Entity<ClusterSession>>; // None unless cluster == active
fn slot_live<'a>(&self, cluster: &ClusterRef, cx: &'a App) -> Option<&'a LiveCluster>;
fn slot_connection(&self, cluster: &ClusterRef, cx: &App) -> Option<ClusterConnection>;
pub(crate) fn guard_for<'a>(&'a self, cluster: &ClusterRef, cx: &'a App) -> Option<ClusterGuard<'a>>; // body unchanged
fn in_context(&self, key: ResourceKey) -> Option<ClusterObject>;          // active cluster + key

// row_context.rs (replaces cluster_rows.rs)
#[derive(Clone)] pub(crate) struct TableSession { pub(crate) cluster: ClusterRef,
    pub(crate) label: String, pub(crate) session: Entity<ClusterSession> }   // was SlotSession
impl TableSession { pub(crate) fn row_context(&self, cx: &App) -> RowContext; }
#[derive(Clone)] pub(crate) struct RowContext { pub(crate) cluster: ClusterRef, pub(crate) label: String,
    pub(crate) context: String, pub(crate) session: WeakEntity<ClusterSession> }
// delegates (pod, node, kind, issue)
pub(crate) fn set_session(&mut self, session: Option<TableSession>); // clears ticks + anchor when the cluster changes
```

`slot_session` body: `self.active_session.as_ref().filter(|open| open.cluster == *cluster).map(|open| &open.session)`. Every `slot_*` and `guard_for` routes through it, so the cluster check lives in one place.

## Decisions

| # | Decision | Rationale |
|---|---|---|
| 1 | One `ActiveSession`; a switch is 0026 `switch_cluster` (release all, then deferred `connect_active`) | the only lifecycle left; already tested by `app_shell_switch_tests.rs` |
| 2 | `ClusterObject { cluster, key }` stays everywhere | see [write-safety.md](write-safety.md) K1 |
| 3 | `guard_for(&ClusterRef)` and the `slot_*` helpers keep their signatures and names | the cluster check is the write path's first refusal; ~15 call-site files are shared with parallel lanes |
| 4 | `primary_cluster()` → `active_cluster()`; `lock_target()`, `context_cluster()`, `primary_object()`, `reveal_in_primary()`, `scope_live()` are deleted in favor of `active_cluster()`, `in_context()`, `reveal()`, `live()` | "primary" means nothing with one cluster; the drawer subject and the cursor are always in the active cluster (`release_all` clears them) |
| 5 | `RowContext` keeps `cluster` + weak `session` captured when a menu is built; drops `is_primary`, `primary_label`, `is_multi` | a menu left open over a switch acts on its captured cluster, whose guard is then `None` |
| 6 | Ticks and the range anchor are cleared when a delegate's session changes cluster; `RowName` loses its cluster | today the cluster in `RowName` prunes old ticks; without merged rows the clear is explicit (AC 10) |
| 7 | `space` → `gpui_kit::NoAction` on `ClusterSwitcher` and `ClusterSwitcher > Input`; `normalize_query` stays | without it the kit `Popover` `space → Confirm` would close the popover; a typed space is ignored by the filter |
| 8 | `--view` and `--screen pods-multi` are deleted; `--context` is the only start choice | no multi start exists |
| 9 | The "Select rows of one cluster" guards stay unchanged | they are 3-line refusals that, with `guard_for(first.cluster)`, prove every row is of the active cluster; see [write-safety.md](write-safety.md) K3 |
| 10 | `running_batches: HashSet<ClusterRef>` stays | a batch of A may still be ending its loop right after A → B (`leaving_work` warned) |
| 11 | Port forwards keep their `cluster`, `cluster_label`, the page's Cluster column, and `Open {cluster} first` | forwards survive a switch (0035 AC 8), so the page can list several clusters |
| 12 | `leaving_work(&[ClusterRef])` and `confirm_leaving` stay; the only caller is `switch_cluster` | 0036/0037/0034 lines unchanged |
| 13 | `app_shell_view.rs` stays as the active-session lifecycle (`new_slot`, `sync_view_sessions`, `refresh_slot_labels`, `on_first_live`); its module doc is rewritten | small file, no rename churn |
| 14 | Overview, Issues, Topology read `session()` as today; `Showing {label} only` goes | they already draw one cluster |
| 15 | Settings: nothing to drop. No viewed set was ever saved (0027 decision 14); `last_used` is one `ClusterRef`; a `Cluster` table pref (never written, decision 23) is dropped by `apply_prefs` (unknown name) | AC 13; `unknown_fields_are_ignored` already covers stray keys |
| 16 | Each step deletes what it leaves unused (dead-code rule); no temporary shims | gate green per step without `#[allow]` |

## Single-cluster texts (unchanged from 0026)

| Place | Text |
|---|---|
| Trigger | active env badge + `display_name` |
| Lock badge | `Read-only` / `Unlocked`; tooltip `{label}: {state}`; click toggles (Ctrl Shift R too) |
| Status bar | `Watching {k} resource types` + API latency + version |
| Header | `{count} {plural}` / `{shown} of {total} match` |
| Dock tab | `{label}` (no ` · {cluster}`) |
