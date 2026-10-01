# 0005 · App: kind model and lazy watch

[Back to index](README.md) · Modules: `resource_kind.rs`, `kind_row.rs`, `workload_rows.rs`, `network_rows.rs`, `config_map_rows.rs`, `namespace_rows.rs`, `cluster_session.rs`, `status_tone.rs`

## `ResourceKind` (`resource_kind.rs`): per-kind data, no trait

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ResourceKind { Namespaces, Deployments, StatefulSets, DaemonSets, ReplicaSets,
    Jobs, CronJobs, Services, Ingresses, ConfigMaps }
impl ResourceKind {
    pub(crate) const ALL: [ResourceKind; 10];
    pub(crate) fn label(self) -> &'static str;     // "StatefulSets": sidebar item and title, equal to SECTIONS text
    pub(crate) fn singular(self) -> &'static str;  // "statefulset": count label and "Delete statefulset…"
    pub(crate) fn plural(self) -> &'static str;    // "statefulsets": count label and the --screen slug (kubectl name)
    pub(crate) fn badge(self) -> &'static str;     // W7 icon: Ns De Ss Ds Rs Jb Cj Sv In Cm
    pub(crate) fn is_namespaced(self) -> bool;     // false only for Namespaces
    pub(crate) fn access_check(self) -> AccessCheck;           // ListDeployments …
    pub(crate) fn columns(self) -> &'static [KindColumn];      // after Name (kind-columns.md)
    pub(crate) fn read_only_actions(self) -> &'static [&'static str];
    pub(crate) fn delete_label(self) -> &'static str;          // "Delete deployment…"
    pub(crate) fn has_port_forward(self) -> bool;              // Deployments, StatefulSets, Services
    pub(crate) fn from_label(text: &str) -> Option<Self>;
    pub(crate) fn from_plural(text: &str) -> Option<Self>;
    /// The only per-kind `match` over cluster calls: watch, then map to rows on tokio.
    pub(crate) fn watch_rows(self, connection: &ClusterConnection, scope: NamespaceScope)
        -> BoxStream<'static, WatchUpdate<KindRow>>;
}
pub(crate) struct KindColumn { pub(crate) name: &'static str, pub(crate) width: f32, pub(crate) align: Align }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Align { Left, Right } // mapped to `Column::text_right()` and `header_cell`
```

`watch_rows` arm: `connection.watch_deployments(scope).map(|update| rows(update, deployment_row)).boxed()`, where the private `fn rows<T>(update: WatchUpdate<T>, row: fn(&T) -> KindRow) -> WatchUpdate<KindRow>` maps a `Snapshot` and passes `Failed` through. Namespaces ignore `scope` and use `watch_namespaces()`.

## `KindRow` (`kind_row.rs`): what the table and the drawer read

```rust
pub(crate) struct KindRow {
    pub(crate) namespace: Option<String>,        // None for cluster-scoped kinds
    pub(crate) name: String,
    pub(crate) created_at: Option<jiff::Timestamp>,
    pub(crate) status: StatusLabel,              // drawer subtitle
    pub(crate) cells: Vec<KindCell>,             // exactly kind.columns().len(); Name is not included
    pub(crate) sections: Vec<DetailSection>,     // drawer body, in order
    pub(crate) related_pods: Option<PodOwner>,
    pub(crate) labels: Vec<SharedString>,
}
pub(crate) enum KindCell {
    Text(SharedString), Mono(SharedString), Toned(StatusLabel), Absent,
    /// Rendered at paint time, so ages never go stale. `tone` colours it (CronJob last schedule).
    Age { at: Option<jiff::Timestamp>, tone: Option<StatusTone> },
    /// `format_age(started_at, finished_at.unwrap_or(now))`; Absent when not started.
    Duration { started_at: Option<jiff::Timestamp>, finished_at: Option<jiff::Timestamp> },
}
pub(crate) struct DetailSection { pub(crate) title: &'static str, pub(crate) rows: Vec<DetailRow> }
pub(crate) enum DetailRow {
    Field { label: SharedString, value: KindCell },
    Chips(Vec<SharedString>),
    Port { text: SharedString },                 // + the disabled Forward button
}
pub(crate) enum PodOwner {
    Controller { namespace: String, kind: &'static str, name: String }, // StatefulSet, DaemonSet, ReplicaSet, Job
    Deployment { namespace: String, name: String },
}
pub(crate) fn owns_pod(owner: &PodOwner, pod: &PodSummary) -> bool;
```

- `KindRow` is `Send` (`SharedString` and `StatusLabel` are), so rows are built on tokio. `StatusLabel` gains `#[derive(Clone, Debug, PartialEq, Eq)]` (builders clone it between cells and status; tests compare it).
- Step 2 adds only what Namespaces and Deployments use: `ResourceKind::{Namespaces, Deployments}`, `PodOwner::Deployment`, `DetailRow::{Field, Chips, Port}`, and `KindCell` without `Duration`. Step 3 adds the other 8 variants, `PodOwner::Controller`, and `KindCell::Duration` with their first users.
- `owns_pod`: the namespace must be equal, and then:
  - `Controller`: the pod's `controller` must equal `(kind, name)`;
  - `Deployment`: the controller kind is `ReplicaSet`, and its name minus the prefix `"{name}-"` is non-empty and uses only the pod-template-hash alphabet `bcdfghjklmnpqrstvwxz2456789` (Kubernetes `SafeEncodeString`). So `api` matches `api-7d9f8c`, and does not match `api-worker-7d9f8c` (`-`) or `api-canary` (`a`, `y`).
- The row builders are pure and take only the summary. Section contents are listed in [kind-drawers.md](kind-drawers.md), and cells in [kind-columns.md](kind-columns.md):
  - `workload_rows.rs`: `deployment_row`, `stateful_set_row`, `daemon_set_row`, `replica_set_row`, `job_row`, `cron_job_row`;
  - `network_rows.rs`: `service_row`, `ingress_row`;
  - `config_map_rows.rs`: `config_map_row`, `format_bytes`;
  - `namespace_rows.rs`: `namespace_row`.

## Session state (`cluster_session.rs`)

```rust
pub(crate) struct ClusterSession { /* … */ explorer_kind: Option<ResourceKind> } // survives Connecting and retry
pub(crate) struct LiveCluster { /* … */ pub(crate) explorer: Option<KindList> }
/// The visible explorer kind's list. Dropping it stops the watch.
pub(crate) struct KindList { pub(crate) kind: ResourceKind, pub(crate) list: LiveList<KindRow>, _subscription: WatchSubscription }

impl ClusterSession {
    pub(crate) fn new(kubeconfig, summary, requested_namespace, explorer_kind: Option<ResourceKind>, cx) -> Self;
    /// Starts, replaces, or stops the explorer watch.
    pub(crate) fn set_explorer_kind(&mut self, kind: Option<ResourceKind>, cx: &mut Context<Self>);
}
```

| Event | Effect |
|---|---|
| `set_explorer_kind(Some(k))`, live, a different kind or none | `explorer = Some(KindList { k, Loading, subscribe(k.watch_rows(conn, scope)) })`; the old subscription drops first |
| `set_explorer_kind(Some(k))`, same kind | no-op (no re-list) |
| `set_explorer_kind(None)` | `explorer = None` (watch stopped) |
| not live yet | only `explorer_kind` is stored. `LiveCluster::start` starts it |
| `set_scope`, explorer kind namespaced | restart, like pods (`Loading`, new subscription) |
| `set_scope`, Namespaces | keep |
| context switch | the whole session drops; the new session gets `explorer_kind` from `AppShell.screen` |

- `apply` guards on `explorer.kind == kind` before `list.apply(update)`, and `on_closed` calls `mark_stopped` the same way. That is defense in depth, because dropping the subscription already cancels the receiver.
- At most 4 watches are live per session: namespaces, pods, nodes, and the explorer. Pods keep running while a kind screen is shown, because related pods read them.
