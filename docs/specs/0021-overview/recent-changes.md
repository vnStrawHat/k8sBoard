# 0021 · Recent changes (step 2)

[Back to index](README.md) · Modules: `crates/cluster/src/event.rs` (+ `event_tests.rs`), `cluster_session.rs` (+ tests), `recent_changes.rs` (new, + `recent_changes_tests.rs`), `overview.rs`, `app_shell.rs`

## Cluster crate (`event.rs`)

```rust
/// The controller events Overview shows as changes, each selected by the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeEventKind { Rollout, Rescale }

impl ClusterConnection {
    /// Events of one change kind in `scope`; keeps the `EVENT_LIMIT` most recent, like `watch_events`.
    pub fn watch_change_events(&self, scope: NamespaceScope, kind: ChangeEventKind)
        -> impl Stream<Item = WatchUpdate<EventSummary>> + Send + 'static;
}
fn change_event_selector(kind: ChangeEventKind) -> &'static str;
// Rollout → "involvedObject.kind=Deployment,reason=ScalingReplicaSet"
// Rescale → "involvedObject.kind=HorizontalPodAutoscaler,reason=SuccessfulRescale"
```

`watch_events` and `watch_change_events` share one private helper that takes `Option<&'static str>`. `EventFilter` is unchanged (the Events screen toggle stays two-valued). Requests: `list`/`watch events` with a field selector, which is read-only.

## Session (`cluster_session.rs`)

```rust
// ClusterSession: kept across Connecting and retry, like `explorer_kind`
is_overview_visible: bool,
pub(crate) fn set_overview_visible(&mut self, is_visible: bool, cx: &mut Context<Self>);

// LiveCluster
pub(crate) change_events: Option<ChangeEvents>,   // Some exactly while Overview is visible (Denied included)
/// The Overview's change feeds. Dropping it stops both watches.
pub(crate) enum ChangeEvents {
    Live { rollouts: LiveList<EventSummary>, rescales: LiveList<EventSummary>, _subscriptions: [WatchSubscription; 2] },
    Denied,
}
/// A `Known` report that denies `ListEvents` for this scope. `Checking`/`Unknown` start the watches.
fn is_change_feed_denied(access: &AccessState) -> bool;
```

| Trigger | Effect |
|---|---|
| `set_overview_visible(true)` while live, feed `None` | `is_change_feed_denied` → `Denied`; else start two `runtime.subscribe(connection.watch_change_events(scope, kind), …)` into `LiveList::Loading`; notify |
| `set_overview_visible(false)` | `change_events = None` (both watches stop) |
| `LiveCluster::start` (connect, retry) | start when `is_overview_visible` |
| `set_scope` | drop, then start again when visible (new namespaces) |
| `finish_access_review` | a `Denied` or running feed is planned again with the known report |
| watch update | `list.apply(update)`; the regular `subscribe` notifies (snapshots are batched; only while Overview is visible) |

**Watch math.**
- `OpenWatches` gains `change_events: usize`. `watch_count` sets it to `2 × scope_multiplicity(&self.scope)` (the existing helper) while `ChangeEvents::Live`, and 0 otherwise.
- `open_watch_count` adds it. Its doc comment gains "plus two change-event watches per namespace while Overview is visible", and the stated bound grows by `2N` (on top of whatever 0020 states).
- Test `open_watch_count_includes_change_events`: hidden → +0; All → +2; two namespaces → +4.

## Model (`recent_changes.rs`)

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ChangeWindow { #[default] FifteenMinutes, OneHour }
impl ChangeWindow {
    pub(crate) const ALL: [Self; 2];
    pub(crate) fn label(self) -> &'static str;   // "Last 15 min", "Last 1 h"
    pub(crate) fn span(self) -> jiff::SignedDuration;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChangeKind { Deployment, Autoscaler, Node, Namespace }
impl ChangeKind { pub(crate) fn label(self) -> &'static str; } // "Deployment", "HPA", "Node", "Namespace"

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChangeEntry {
    pub(crate) at: jiff::Timestamp,
    pub(crate) kind: ChangeKind,
    pub(crate) object: String,             // "payments/api", "ip-10-0-3-17"
    pub(crate) text: String,               // "Scaled up replica set api-7d9f8c to 3", "became NotReady"
    pub(crate) count: u32,                 // the event count; 1 for state rows
    pub(crate) actor: Option<String>,
    pub(crate) target: Option<ResourceKey>,
}

pub(crate) struct ChangeInputs<'a> {
    pub(crate) rollouts: Option<&'a [EventSummary]>,   // None while that feed is not Ready
    pub(crate) rescales: Option<&'a [EventSummary]>,
    pub(crate) nodes: Option<&'a [NodeSummary]>,
    pub(crate) namespaces: Option<&'a [NamespaceSummary]>,
    pub(crate) window: ChangeWindow,
    pub(crate) now: jiff::Timestamp,
}
/// Every change newer than `now − span` (a time slightly ahead of `now` counts: clocks differ), newest first; ties are broken by object.
pub(crate) fn recent_changes(inputs: &ChangeInputs) -> Vec<ChangeEntry>;
pub(crate) const CHANGE_ROWS: usize = 8;
```

| Source | Entry |
|---|---|
| `rollouts` event | `Deployment`, `{ns}/{name}`, `message_line(message)`, count, actor |
| `rescales` event | `Autoscaler` (label `HPA`), same |
| Node `Ready` condition with `changed_at` in the window, node not joined in the window | `Node`, name, `became Ready` / `became NotReady` / `stopped reporting (Unknown)`; actor `kubelet` (True/False) or `node-controller` (Unknown) |
| Node `created_at` in the window | `Node`, name, `joined the cluster`, no actor |
| Namespace `created_at` in the window | `Namespace`, name, `created`, no actor |

- **Selection.** The server selects event rows (decision 4); no client allow-list.
- **Time.** `at` = `last_seen`, so an aggregated event moves to its newest occurrence. Events without `last_seen` are skipped.
- **Actor.** `actor` = `source` up to the first ` on ` (`deployment-controller`, `horizontal-pod-autoscaler`); `None` when empty.
- **Target.** `target` = `ResourceKey::of_object(kind, namespace, name)`. That covers Node, Namespace, and Deployment, plus HPA once 0013 adds the kind; anything else is `None`.
- **No tracing.** Event messages are arbitrary text, so this module has no `tracing::` call (0006 rule).

## Panel rows (W3 `.tl`: 44 px | 1fr | auto)

```
h_flex().gap_2().px_3().py(px(6.)).border_b_1().text_xs()
  time  w(px(44)) mono muted:  local `HH:MM` (`jiff` system zone)
  text  flex_1 min_w_0 truncate:  `{kind.label()} ` + mono `{object}` + ` {text}` + ` ×{count}` when count > 1
  who   mono muted text_xs:  actor or nothing
```

- **Rows.** The panel shows the first `CHANGE_ROWS` entries, then the footnote ([layout.md](layout.md)).
- **Window state.** The window lives in `AppShell.overview.window: ChangeWindow` (new field; default 15 min; reset on restart). `set_change_window(window, cx)` notifies.

## Last 24 h and View all (UX round 3, N2 and N3)

- **`Last 24 h`** is the third range. The API server keeps events about an hour, so for this range only, the session also watches the ReplicaSets of the scope (`RolloutHistory`, one watch per namespace, started by `set_rollout_history` and dropped with the range). Each ReplicaSet owned by a Deployment and created in the window is a Deployment row: `rev 8 · 1.26-alpine · release test` (revision, first image tag, `kubernetes.io/change-cause`), at its creation time. It replaces the `Scaled up replica set …` event that made the ReplicaSet (within two minutes of its creation); a rollback reuses its old ReplicaSet, so that rollout has no row of its own and shows as its event, at the time of the event; scale-downs, rescales, nodes, and namespaces stay. The panel shows `Loading rollouts…` until the first ReplicaSet snapshot; a failed list leaves the events alone.
- **Time column.** The 24 h range reaches back to yesterday, so a row from another day reads `04-30 22:10` (`ChangeClock`, a wider column); today's rows stay `HH:MM`.
- **`View all →`** opens Events with every event fetched (a Warnings-only list would hide Normal events) and the **Changes** chip on, so the list is newest first with no Warnings above it. Changes are the Normal events of `event_rows::is_change`: a rollout (`ScalingReplicaSet`, the `Killing` of `Stopping container`), an HPA `SuccessfulRescale`, a node going Ready or NotReady, and the other Normal events of a Deployment, StatefulSet, DaemonSet, Job, or CronJob except the routine `SuccessfulCreate`, `SuccessfulDelete`, `Completed`, and `SawCompletedJob`. Pulls, starts, and scheduling are never changes. The chip (`FilterPreset::Changes`) removes with its ×; the header says `276 total, 206 hidden (not changes)`.
