# 0019 · Steps 2a/2b: targets, membership, opening

[Back to index](README.md) · Modules: `log_target.rs` (new, pure), `log_workload.rs` (new, pure), `log_dock.rs`, `kind_row.rs`, `resource_actions.rs`, `app_shell.rs`. Streams, merge, and status: [workload-streams.md](workload-streams.md).

## Targets (`log_target.rs`, step 2a, inline tests; no GPUI types)

```rust
pub(crate) enum LogTarget { Pod(PodTarget), Workload(WorkloadTarget) }
#[derive(Clone)] pub(crate) struct PodTarget { /* the 0004 LogTarget fields */ pub(crate) choice: ContainerChoice }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub(crate) enum ContainerChoice { Default, Explicit }
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkloadTarget { pub(crate) owner: PodOwner, pub(crate) label: String }
impl LogTarget {
    pub(crate) fn of_pod(pod: &PodSummary) -> Option<Self>;                       // Default
    pub(crate) fn of_container(pod: &PodSummary, container: &str) -> Option<Self>; // Explicit; None if absent
    pub(crate) fn of_workload(owner: PodOwner) -> Option<Self>;                     // None for PodOwner::Node
    pub(crate) fn is_same(&self, other: &Self) -> bool; // pods by (namespace, pod), workloads by owner
}
/// `deploy/api`, `sts/db`, `ds/agent`, `rs/api-7d9f8c`, `job/migrate`; `None` for a node.
pub(crate) fn workload_label(owner: &PodOwner) -> Option<String>;
/// Why "Logs of selected" cannot open a tab (step 2b).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NoLogTarget { NotConnected, NotLoggable } // Display: "Not connected", "Select a pod or workload"
```

- `log_tab.rs` moves its 0004 `LogTarget` here (renamed `PodTarget`); `LogTab::is_for(&LogTarget)` delegates to `is_same`.
- `kind_row.rs`: `impl PodOwner { pub(crate) fn namespace(&self) -> Option<&str> }` (`Node` → `None`).

## Pure membership (`log_workload.rs`, step 2a, inline tests)

```rust
pub(crate) const MAX_WORKLOAD_PODS: usize = 10;
pub(crate) const MAX_WORKLOAD_STREAMS: usize = 20;
/// Pods of `owner`: ready (ready == total > 0) first, then newest `created_at` (None last), then name.
pub(crate) fn ranked_pods<'a>(owner: &PodOwner, pods: &'a [PodSummary]) -> Vec<&'a PodSummary>;
pub(crate) struct MemberChange { pub(crate) joined: Vec<String>, pub(crate) left: Vec<String> }
/// Sticky: listed members stay; up to `slots` best-ranked newcomers join.
pub(crate) fn member_change(current: &[String], ranked: &[&PodSummary], slots: usize) -> MemberChange;
/// Newcomers that fit: min(pod_limit − members, (MAX_WORKLOAD_STREAMS − live_streams) / streams_per_pod).
pub(crate) fn join_slots(members: usize, live_streams: usize, streams_per_pod: usize) -> usize;
pub(crate) fn pod_limit(selected_containers: usize) -> usize;   // clamp(20 / n, 1, 10); n = 0 → 10
pub(crate) fn pod_short_name<'a>(owner: &PodOwner, pod: &'a str) -> &'a str; // last `-` segment; StatefulSet: whole
pub(crate) fn scope_covers(scope: &NamespaceScope, namespace: &str) -> bool; // `All` covers everything
/// Container names over `members`, first-seen order, Init last (step 2b).
pub(crate) fn container_names(members: &[&PodSummary]) -> Vec<String>;
```

- `current` = pods whose `is_member` is true. `left` = current members absent from `ranked`. A listed member never leaves, even below the top `pod_limit`.
- `live_streams` counts **every** `TabStream` whose subscription is still alive, member or not (decision 4). `join_slots` saturates at 0; `streams_per_pod` = selected container count (≥ 1).
- A name that left and comes back (StatefulSet recreate) is an ordinary newcomer in `joined`; the tab handles the rejoin ([workload-streams.md](workload-streams.md)).
- A smaller `pod_limit` (more containers picked) affects only future joins.

## Opening

| Entry | Step | Code |
|---|---|---|
| `LogDock` session | 2a | field `session: Option<WeakEntity<ClusterSession>>`; `set_session(Option<WeakEntity<…>>)` called by `AppShell::start_session` next to `close_all` |
| `LogDock::open(connection, target, window, cx)` | 2a | unchanged shape; upgrades `session` (none → no-op) and calls `LogTab::new(connection, target, &session, window, cx)`. Existing pod tab + `Explicit` → `tab.pick_container(name)` (decision 27) |
| `LogTab::new(…, session: &Entity<ClusterSession>, …)` | 2a | pod tabs ignore it; workload tabs keep `session.downgrade()` and `cx.observe(session, …)` (no strong handle kept) |
| Kind row menu / drawer ⋯ | 2a | `kind_menu`: when `row.related_pods` is `Some` and `workload_label` is `Some`, first item `View logs (all pods)` (`View logs` for Jobs), gated by `action_availability(ViewLogs)`; click → `shell.open_workload_logs(owner, window, cx)` |
| `AppShell::open_workload_logs(owner, window, cx)` | 2a | live session required: `LogTarget::of_workload(owner)` → `log_dock.open(live.connection().clone(), …)` |
| `AppShell::selected_log_target(&self, cx: &App) -> Result<LogTarget, NoLogTarget>` | 2b | no live session → `NotConnected`; selected Pod → `of_pod`; selected kind row with a workload owner → `of_workload`; else `NotLoggable` |
| `AppShell::open_logs_of_selection(window, cx)` | 3 | `selected_log_target` → `log_dock.open`; `Err` → no-op (the menu item is disabled then) |
