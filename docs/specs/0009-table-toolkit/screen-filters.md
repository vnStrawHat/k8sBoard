# 0009 · App: per-screen filters and stream pause

[Back to index](README.md) · Step 3 · Modules: `node_summary.rs` (new), `node_table.rs`, `filter_bar.rs`, `workspace.rs`, `resource_actions.rs`, `cluster_session.rs`, `app_shell.rs`

## Nodes summary chips (W5 note 1)

```rust
// node_summary.rs (pure)
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NodeGroup { Ready, NotReady, Cordoned, Version(String) }
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct NodeCounts {
    pub(crate) total: usize, pub(crate) ready: usize, pub(crate) not_ready: usize,
    pub(crate) cordoned: usize,
    pub(crate) versions: Vec<(String, usize)>,   // count desc, then natural version desc
    pub(crate) common_version: Option<String>,    // versions[0]
}
pub(crate) fn node_counts(nodes: &[NodeSummary]) -> NodeCounts;
pub(crate) fn node_in_group(node: &NodeSummary, group: &NodeGroup) -> bool;
pub(crate) fn role_counts(nodes: &[NodeSummary]) -> Vec<(String, usize)>; // count desc, then name
```

| Group | Member when |
|---|---|
| `Ready` | readiness `Ready` and scheduling `Enabled` |
| `NotReady` | readiness `NotReady` or `Unknown` |
| `Cordoned` | scheduling `Disabled` (any readiness; a cordoned NotReady node is in both) |
| `Version(v)` | `kubelet_version == v` |

- The Nodes filter bar starts with `All {total}` then `Ready n`, `NotReady n`, `Cordoned n` (each only when n > 0), then one `{version} × n` per version. Each is a small `Button`, `.selected(..)` when it is the preset (`All` when the preset is `None`); status chips use `tone_color` (Ok, Bad, Warn) for their text; a version other than `common_version` uses Warn.
- A click sets `FilterPreset::Nodes(group)`; clicking the active chip or `All` sets `None` (`AppShell::set_preset`).
- Counts read all nodes (not the filtered view), so they never change with the text filter.
- `NodeTableDelegate` keeps `common_version` from `rebuild_view`; the Version cell text is Warn-toned when it differs (skew tint).
- Header when not filtering: `{n} nodes · {count} {role}` for up to three roles from `role_counts`; nodes without roles add nothing (no `worker` inference, 0003).

## ReplicaSets: Hide inactive

- `ReplicaSets` screen only: header button `Hide inactive`, built like Warnings only (primary when on, outline when off, `.selected().toggled()`), tooltip "Hide ReplicaSets scaled to zero".
- On: `set_preset(Some(HideInactive))`; off: `None`. `KindRow::in_preset(HideInactive)` is `tone != Done`; a ReplicaSet with zero desired replicas is `Done` (`replica_tone`).
- **On by default** (decision 26, supersedes 0005 decision 23): `default_filter(Screen::Kind(ReplicaSets))` has `preset: Some(HideInactive)`; a context switch restores it, while `Clear filters` turns it off.

## Nodes: View pods on node

- `node_menu` gains `View pods on node · {n}` after View YAML, `n` = pods of the current scope whose `node_name` is the node. Enabled even when `n == 0`.
- `AppShell::view_pods_on_node(name, cx)`: `show_screen(Screen::Pods)`, then the Pods view filter becomes `TableFilter::on_node(name)` = exactly `chips: [Equals { column: NODE, title: "Node", value: name }]` (other Pods filters are dropped so every pod on the node shows).
- `node_menu` takes the pods slice (`&LiveCluster` already reachable at both call sites).

## Events: Filter similar

- The event menu (row menu and drawer ⋯, one builder) gains `Filter similar` after Go to object. Disabled with "This event has no reason" when the reason is empty.
- `AppShell::filter_similar(reason, cx)`: on the Events view, `filter.set_equals(1, "Reason", reason)`: removes any `Equals` chip on that column, then adds the new one. The drawer stays: its row has that reason.

## Events: Pause stream

```rust
// cluster_session.rs
/// Whether new snapshots reach the list. A paused list keeps its rows; only the newest held
/// snapshot is kept.
pub(crate) enum StreamFlow<T> { Live, Paused { held: Option<Vec<T>> } }
impl<T> StreamFlow<T> {
    fn receive(&mut self, list: &mut LiveList<T>, update: WatchUpdate<T>);
    fn resume(&mut self, list: &mut LiveList<T>);
}
pub(crate) struct KindList { /* … */ flow: StreamFlow<KindRow> }   // starts Live
impl ClusterSession { pub(crate) fn set_explorer_paused(&mut self, paused: bool, cx: &mut Context<Self>); }
impl LiveCluster { pub(crate) fn explorer_flow(&self) -> Option<FlowState>; }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FlowState { Live, Paused { has_held: bool } }
```

| Event | Effect |
|---|---|
| `receive`, Live | `list.apply(update)` (today) |
| `receive`, Paused, `Snapshot(items)` | `held = Some(items)` (replaces any older one) |
| `receive`, Paused, `Failed` | `list.apply(update)`: an interruption still shows |
| `resume` | apply `held` as a snapshot if any; become Live |
| `set_explorer_paused(true)` | only when the explorer list is `Ready`; else no-op |
| any `KindList::start` (scope, kind, Warnings only) | a new list starts `Live` |

- The explorer subscription callback calls `explorer.flow.receive(&mut explorer.list, update)`.
- Header (Events only), right of Warnings only: `Button::new("pause-stream")`, label `Pause stream` or `Resume`, same variant rule as Warnings only; tooltip "Hold the list still; new events wait". Count text appends ` · paused`, and ` · new events waiting` when `has_held`.
- Pause while the list is `Loading` or `Failed`: the button is disabled (tooltip "Nothing to pause yet"); a `Ready` list never becomes `Failed`, so a paused list only gains an interruption.
- The status bar is unchanged: the watch still runs.
- Toolkit actions keep working on the frozen rows; `rebuild_view` reads the shown list.
