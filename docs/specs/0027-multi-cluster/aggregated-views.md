# 0027 · Aggregated views

[Back to index](README.md) · Steps 3a, 3b, 4 · Modules: `cluster_rows.rs` (new, pure), `table_view.rs`, `pod_table.rs`, `node_table.rs`, `kind_table.rs`, `table_layout.rs`, `table_selection.rs`, `app_shell.rs`, `workspace.rs`, `navigation.rs`, drawers, `log_dock.rs`, `log_tab.rs`, `yaml_view.rs`. Decisions 13, 15–19, 21–23, 25.

## Merged rows (step 3a, `cluster_rows.rs`)

```rust
pub(crate) const CLUSTER_COLUMN: KindColumn = column("Cluster", 170., Align::Left);
pub(crate) struct Clustered<'a, T> { pub(crate) cluster: &'a ClusterRef, pub(crate) label: &'a str /* switcher_label */,
    pub(crate) column: usize /* logical index of the Cluster column */, pub(crate) item: &'a T }
impl<T: TableRow> TableRow for Clustered<'_, T> { /* delegates; value(cluster_column) = Text(label) */ }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RowAddress { pub(crate) slot: u16, pub(crate) item: u32 }
/// One slot's rows; the slot index is the position in `slots`, and the badge is drawn by the delegate.
pub(crate) struct SlotRows<'a, T> { pub(crate) cluster: &'a ClusterRef, pub(crate) label: &'a str, pub(crate) items: &'a [T] }
pub(crate) fn merge_rows<'a, T>(slots: &[SlotRows<'a, T>], column: usize) -> (Vec<Clustered<'a, T>>, Vec<RowAddress>);
```

Delegates hold `sessions: Vec<Entity<ClusterSession>>` (`set_sessions`); `rebuild_view` builds the merged vector, calls `TableView::rebuild(&merged, ..)`, keeps `addresses`. Render maps `item_index` → `RowAddress`. Single mode is the same code with one slot.

## Cluster column (step 3a)

- Multi mode: `CLUSTER_COLUMN` appended to every plan (logical index = old length). Cell: `environment_badge` + mono label. Sort `CellValue::Text(label)`; the quick filter matches the label; the row menu (multi mode) has `Filter by this cluster` → `FilterChip::Equals { column, title: "Cluster", value: label }`.
- Session-only (decision 23): `TableView::prefs` skips the Cluster column; `set_sessions` with one slot removes its index from `hidden` and clears a sort on it.
- Header: `{n} clusters · {total} {plural}` (W1). **Loading** in multi mode means no slot has a ready list yet.

## Identity (step 3b)

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ClusterObject { pub(crate) cluster: ClusterRef, pub(crate) key: ResourceKey }
```

`AppShell.selected`, `pending_reveal`, `PendingSubjects`, checked keys, and `RowName` use `ClusterObject`. Since 0028, `selected` is the row cursor (`Option<ClusterObject>`) and `drawer_subject()` returns it only while `drawer.is_open`; every drawer-path read and the bare-key context (`context_cluster`, `in_context`) use `drawer_subject()`, so a closed drawer never steers a reveal. Row keys act on the cursor, in the cursor's own slot. The palette lists the resources of every live slot (`ClusterObject` targets, the cluster label after the detail in multi mode), and Enter reveals the entry in its own cluster. Row menus capture `WeakEntity<ClusterSession>` + `ClusterRef` when built (decision 25). Sidebar counts: sum over slots that know the count, tooltip per slot.

## Wrong-cluster sites (step 4; each has a test)

Rule: every `live(cx)` / `self.session` read on a drawer, YAML, logs, or Monitor path becomes `slot_live(&selected.cluster, cx)` (decision 17).

| Site (today) | Change | Test |
|---|---|---|
| `sync_yaml_view` (`app_shell.rs:709-724`): `live(cx)` connection; `YamlView::is_for(&ObjectRef)` | connection from `slot_live(subject.cluster)`; `YamlView` keeps `cluster`, `is_for(&ClusterObject)` | `yaml_view_is_rebuilt_for_same_name_in_other_cluster` |
| `LogDock::open` dedup (`log_dock.rs:55-66`): `is_for(namespace, pod)` | `LogTab` keeps `cluster`; `is_for(&ClusterRef, namespace, pod)` | `same_pod_in_two_clusters_opens_two_tabs` |
| `follow_drawer_subjects` (`app_shell.rs:818-900`): reads `live(cx)` subjects | keep `subject_slot: Option<ClusterRef>`; `Stop` goes to the old slot, `Start` to the new one; `PendingSubjects::has_same_subjects` (`app_shell.rs:111-113`) also compares the cluster | `subject_moves_between_clusters_stops_old_starts_new`, `pending_subjects_differ_by_cluster` |
| `sync_kubelet_demand` / `kubelet_demand` (`app_shell.rs:729-760`) | demand to the subject's slot; `KubeletDemand::default()` to every other slot when it differs | `kubelet_demand_only_on_subject_slot` |
| Drawer content, Monitor history, related lists, events | `slot_live(selected.cluster)`; header gains badge + label in multi mode | `drawer_reads_the_row_cluster` |
| Row menus (`pod_drawer.rs`, `kind_drawer.rs`, `resource_actions.rs`) | captured weak session + `ClusterRef`; `Copy kubectl command` uses that context | `menu_uses_row_cluster_context` |

Releasing the subject's slot closes the drawer; releasing a slot closes its log tabs; log tab titles get ` · {label}` in multi mode.

## Per-slot state (step 3b)

| Slot phases | Workspace |
|---|---|
| all Failed | 0026 error view for the primary, plus `Retry all` |
| some Failed | others' rows; banner per failed slot above the filter bar: `Cannot connect to {label}: {message}` + `Retry` (`session.retry`) + `Remove from view` |
| some Live with a problem | banner `Live updates interrupted in {label}` (no buttons; it recovers on its own as today) |
| some Connecting | others' rows; muted line `Connecting to {label}…` |
| a list denied in a slot | no rows from it; muted note `Not permitted in {label}: list {plural}` |

## Screens that land later (contract)

| Screen | Multi mode rule |
|---|---|
| Issues (0020) | one table over all slots with the Cluster column; the title-bar flag sums them |
| Overview (0021) | "Needs attention" merged with a Cluster column; capacity tiles per slot, primary first |
| Topology (0022) | single cluster: toolbar dropdown of viewed slots, default primary (decision 21) |
| Port Forwarding page (0035) | Cluster column across clusters |
