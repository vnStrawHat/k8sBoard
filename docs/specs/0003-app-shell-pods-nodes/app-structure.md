# 0003 · App structure and state

[Back to index](README.md)

## Crate decision

Everything stays in `crates/app`. The style guide says to extract a crate only when a second crate needs the code, and nothing else consumes UI code. GPUI upgrade risk is contained because only `crates/app` depends on `gpui-kit`. Modules are flat files named after concepts, with no `ui/` folder.

## Cargo (`crates/app/Cargo.toml`)

| Dependency | Why |
|---|---|
| `tokio` (workspace) | runtime, `mpsc`, `JoinHandle` |
| `futures` (workspace) | `Stream`/`StreamExt` for watch streams |
| `jiff` (workspace) | `Timestamp::now()` for ages |
| feature `screenshot = ["gpui-kit/test-support"]` | dev-only hook; **never** in `default` ([screenshot-hook.md](screenshot-hook.md)) |

There is still no `kube` or `k8s-openapi` dependency: the app sees only `cluster` domain types.

## Modules (`crates/app/src`)

| File | Concept |
|---|---|
| `main.rs` | tracing, parse `LaunchOptions`, build runtime, `application().run`, exit code |
| `launch_options.rs` | CLI flags and kubeconfig path resolution (pure) |
| `cluster_runtime.rs` | `ClusterRuntime` global, `RuntimeTask`, `WatchSubscription`, `subscribe` (0002 contract) |
| `cluster_session.rs` | one connected context: connection, phase, live lists, access report |
| `app_shell.rs` | root view: layout of the six regions, screen and drawer state, key context |
| `title_bar.rs` | cluster switcher, namespace picker, read-only lock, settings button |
| `navigation.rs` | sidebar groups and kinds |
| `status_bar.rs` | watch state, API version, kubeconfig user, app version |
| `pod_table.rs` | `PodTableDelegate` (columns, cells, context menu) |
| `node_table.rs` | `NodeTableDelegate` |
| `drawer.rs` | overlay frame: header (kind badge, name, ⋯ ⤢ ✕), width, tab bar |
| `pod_drawer.rs` | pod Overview and Containers tabs |
| `node_drawer.rs` | node Overview |
| `resource_actions.rs` | pod and node menus, `ActionAvailability` gating |
| `status_tone.rs` | `StatusTone`, labels and tones for pod, node, and container states, `tone_color` |
| `age.rs` | `format_age`, used by both tables and both drawers, so it is a module of its own rather than a local helper |
| `screenshot.rs` | screen setup, readiness wait, PNG capture |

## Entity graph and ownership

```text
AppShell (root view, Entity)
 ├─ kubeconfig: KubeconfigState            Loading | Loaded(Kubeconfig) | Failed(String)
 ├─ session: Option<Entity<ClusterSession>> replaced on context switch (drop = cancel all)
 ├─ screen: Screen                          Pods | Nodes
 ├─ pod_table:  Entity<TableState<PodTableDelegate>>   delegate reads the session
 ├─ node_table: Entity<TableState<NodeTableDelegate>>
 └─ drawer: DrawerState                     tab, expanded, selected container
ClusterSession (Entity)
 ├─ context: String, user: Option<String>
 ├─ phase: SessionPhase
 └─ (when Live) connection, server_version, scope, access, namespaces, pods, nodes, subscriptions
```

```rust
enum SessionPhase {
    Connecting { _task: Task<()> },          // open + server_version + initial scope choice (bootstrap.md)
    Live(Box<LiveCluster>),
    Failed { message: String },              // ClusterError Display; retry button
}
struct LiveCluster {
    connection: ClusterConnection,
    server_version: ServerVersion,
    scope: NamespaceScope,                   // decided before Live, so never "unknown"
    access: AccessState,                     // Checking { _task } | Known(AccessReport) | Unknown { message }
    namespaces: LiveList<NamespaceSummary>,
    pods: LiveList<PodSummary>,
    nodes: LiveList<NodeSummary>,
    subscriptions: Subscriptions,            // namespaces, pods, nodes: WatchSubscription
}
enum LiveList<T> {
    Loading,
    Ready { items: Vec<T>, interruption: Option<String> }, // Some = last Failed after data
    Failed { message: String },              // failed before the first snapshot; retrying
}
impl<T> LiveList<T> { fn apply(&mut self, update: WatchUpdate<T>); } // pure, tested
```

- `LiveList::apply`:
  - `Snapshot` → `Ready { items, interruption: None }`;
  - `Failed` in `Loading`/`Failed` → `Failed { message }`;
  - `Failed` in `Ready` → keep the items and set `interruption`.
- Delegates hold `Entity<ClusterSession>` (or `None`) and read rows in `rows_count` and `render_td` through `cx: &App`. There is one source of truth, and rows are never copied.
- Selection is held as a key, not a row index ([tables.md](tables.md)).
- `AppShell` observes the session (`cx.observe`). On notify it re-syncs selection and refreshes the tables.
