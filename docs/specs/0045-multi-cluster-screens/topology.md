# 0045 · Topology for a chosen cluster

[Back to index](README.md) · Step 3 · Modules: `app_shell.rs`, `app_shell_view.rs`, `workspace.rs`, `topology_view.rs`, `resource_actions.rs`, `cluster_rows.rs`. Decisions 8–11.

## Which cluster the graph draws

```rust
// AppShell
topology_cluster: Option<ClusterRef>,   // the user's choice; `None` = primary
impl AppShell {
    /// The viewed slot Topology draws: `topology_cluster` while it is viewed, else the primary.
    fn topology_slot(&self) -> Option<&ViewSlot>;
    /// Header dropdown and Show in Topology. Same cluster: no-op.
    pub(crate) fn set_topology_cluster(&mut self, cluster: &ClusterRef, cx: &mut Context<Self>);
    /// `key` in the cluster Topology draws (replaces `primary_object` on the Topology paths).
    fn topology_object(&self, key: Option<ResourceKey>) -> Option<ClusterObject>;
}
```

- `sync_view_sessions` passes `topology_slot()`'s session to `TopologyView::set_session` (was the primary's). When the chosen slot is released by an apply, `topology_cluster` is cleared and the primary is drawn.
- A single switch (0026, Ctrl 1–9) clears `topology_cluster`.
- `set_topology_cluster` closes the drawer when its subject is a Topology-only row of the old cluster (the drawer would read the wrong feeds), then calls `sync_view_sessions`.

## Feed hand-over (`TopologyView::set_session`)

Today a new session is assumed to replace a released one. In multi mode the old slot stays alive, so its feeds would keep running. Change:

1. If the old session differs from the new one, call `old.update(cx, |s, cx| s.set_topology_subject(None, cx))` first (feeds dropped; `SubjectChange::Stop`).
2. Then the existing reset (`namespace = None`, `expanded`, graph cleared) and `sync_subject` on the new session.

Pins are keyed by context (0022 decision 23), so each cluster keeps its own dragged positions.

## Header dropdown (W11 header, multi mode only)

`topology_header_buttons` (`workspace.rs`) gets, before `Fit`:

| Part | Content |
|---|---|
| Button | outline small, `Cluster: ` + `environment_badge` + mono label, `dropdown_caret(true)`, tooltip `Cluster the graph draws` |
| Menu | one item per viewed slot in slot order, `checked` on the drawn one; a slot that is not Live shows its state (`Connecting…`, `Failed`) and is still selectable (the body then shows that state) |

Single mode: no button (AC 11). The `ns: {ns} · {n} resources` count is unchanged.

## Body (`render_multi_body`)

The primary-only branch becomes the topology slot's phase: Live → `self.topology` element; Failed → `error_view("Cannot connect to {label}", …, Retry → retry_cluster)` (hint text `Pick another cluster from the Cluster menu.`); Connecting → `busy_view`. The `Showing {label} only` notice is removed for Topology.

## Selection and drawer

- `select_on_topology(key)` → `topology_object(key)` (was `primary_object`). The cursor is a `ClusterObject` of the topology cluster, so `render_drawer` reads `slot_session(&object.cluster)` and `live.row_of(key)` finds the row in that slot's topology feeds.
- Row keys (`run_row_key`) act on that cursor in its own cluster: L, Y, E, S, F, Del on a Topology node work as on a table row (no key change).
- Double-click reveal (`with_shell` → `reveal`) uses `in_context`, which is the drawer subject's cluster: unchanged and correct.

## Show in Topology (row menus, any cluster)

| Item | Today | Change |
|---|---|---|
| `topology_menu(kind, namespace, scope, other_primary)` | `Disabled("Topology draws only the primary cluster ({primary})")` for other clusters | parameter removed; `Disabled` only for a namespace outside the scope |
| `show_in_topology_item(key, shell)` | `shell.show_in_topology(&key)` | `show_in_topology_item(object: ClusterObject, shell)`; built with `context.object(key)` |
| `AppShell::show_in_topology(&ResourceKey)` | primary | `show_in_topology(&ClusterObject)`: `set_topology_cluster(&object.cluster)`, then `TopologyView::show_object(namespace, id)`, then `show_screen(Topology)` |

`RowContext::{is_primary, primary_label}` and `SlotSession::{is_primary, primary_label}` lose their last readers here and in step 1; remove them (dead-code rule) unless a remaining reader exists when the step lands — the coder checks with a grep and records it in the step report.

## Interplay with the RBAC layer (0022 step 4)

The RBAC feeds of 0022 step 4 run in the topology slot only, like every other feed. No extra rule.
