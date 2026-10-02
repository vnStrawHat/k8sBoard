# 0026 · Switch lifecycle

[Back to index](README.md) · Step 1 · Modules: `app_shell.rs`, `cluster_registry.rs` (0024), `cluster_session.rs`, `workspace.rs`, `status_bar.rs`, `pod_drawer.rs`, `kind_drawer.rs`. Decisions 1–5, 17, 19–22.

## Entry point

All go through one method; nothing else creates a session after start.

```rust
impl AppShell {
    /// Switches the only session to `target`. No-op when it is already the active target.
    pub(crate) fn switch_cluster(&mut self, target: &ClusterRef, cx: &mut Context<Self>);
}
```

Callers: switcher row click and Enter, `SwitchToClusterN`, "Back to {previous}", and the first start (0024 `start_choice`).

## Order of a switch

1. **Resolve**: find the `Arc<Kubeconfig>` in `ClusterCatalog` whose `ContextSummary` matches `target` (source + name). Missing → shell notice `'{label}' is no longer in its kubeconfig`, stop (decision 20).
2. **Remember**: if the current session is Live, `remember_scope(&mut self.scope_memory, current_ref, live.scope.clone())`.
3. **Tear down** (decision 1):
   - `close_drawer` (pending subjects, object events, related watches, YAML GET);
   - `log_dock.close_all` (every log stream);
   - tables `set_session(None)`, `self._session_observer = None`, `self.session = None`;
   - reset UI state from decision 2 (`reset_filter` on every view, `namespace_picker` default, `pending_launch_screen`, `pending_reveal`, `quick_filter_screen = None`, `has_reported_live = false`).
4. **Bookkeeping**: `previous = Some(current_ref)` whenever a current target existed and differs from `target` (decision 21: always the cluster you came from); `active = Some(summary.clone())`; `cx.notify()`.
5. **Connect later**: `let this = cx.weak_entity(); cx.defer(move |cx| { let _ = this.update(cx, |shell, cx| shell.connect_active(namespace, cx)); });` (`App::defer`, gpui-pre `app.rs:2070`). `flush_effects` calls `release_dropped_entities` before every effect (`app.rs:1784-1786`), so the deferred call runs after the old entity is released (decision 1), so the old `LiveCluster`, its `WatchSubscription`s, `RuntimeTask`s, metrics and kubelet polls, or a pending `Connecting` task are gone before the new connect starts. `connect_active` re-checks that `active` still equals the target (a second switch in between wins).
6. `connect_active`: `ClusterSession::new(kubeconfig, &summary, namespace, self.screen.kind(), cx)`; observe; tables `set_session(Some)`.

`namespace = start_scope(&self.scope_memory, target, &profile)` (pure, below).

## Session holders

Strong `Entity<ClusterSession>` holders after step 1: `AppShell.session` and the three table delegates only. The row-menu closures that clone the session today become `WeakEntity<ClusterSession>` (`pod_drawer.rs:111` `let session = session.clone();`, `kind_drawer.rs:143` the same) and upgrade on click; a failed upgrade does nothing. Rendered elements are rebuilt each frame, so after two draws no closure of the old frame holds the entity (test `switch_releases_the_old_session` draws twice).

## Pure helpers (`cluster_registry.rs`, tested without GPUI)

```rust
pub(crate) type ScopeMemory = HashMap<ClusterRef, NamespaceScope>;   // ClusterRef: Hash (0024 amended)
pub(crate) fn remember_scope(memory: &mut ScopeMemory, cluster: ClusterRef, scope: NamespaceScope);
/// memory > profile.default_namespace (as a Named scope) > None (today's default).
pub(crate) fn start_scope(memory: &ScopeMemory, target: &ClusterRef, profile: &ClusterProfile)
    -> Option<NamespaceScope>;
```

## State on `AppShell`

```rust
active: Option<ContextSummary>,   // 0024; the selected target, also while failed
previous: Option<ClusterRef>,     // the cluster you came from (decision 21)
scope_memory: ScopeMemory,        // app session only
has_reported_live: bool,          // 0024 last_used write, reset per session
```

## Session going Live

0024 (amended, decision 22 there): `on_session_changed` writes `last_used` the first time the phase is Live per session. 0026 adds nothing here; there is no `last_used` write in `start_session`.

## Failure

`workspace.rs` `SessionPhase::Failed` view, extended:

| Part | Content |
|---|---|
| Title | `Cannot connect to {label}` (`switcher_label`) |
| Message | the existing `error_text` (credential-free, 0001) |
| Buttons | `Retry` (`session.retry`); `Back to {previous label}` **only when `previous` resolves** in the catalog (decision 22) |

`error_view` gains an optional second action `(label, handler)`; no other caller changes. The title bar keeps the target's badge and label; the status bar shows `Disconnected` (existing); the switcher row shows `Unreachable` from the session (decision 11).

## Status bar latency

`connect_cluster` times only `connection.server_version()` (`Instant` before and after; not the client setup); `Connected` and `LiveCluster` gain `api_latency: Duration`. `status_bar` renders `API {git_version} · {ms} ms` (`as_millis()`, at least 1). The switcher's `Live · {n} ms` uses the same value; probes time the same call (health-probes.md).

## Room for 0027

- `switch_cluster` is the single-session case of a future `view_clusters(&[ClusterRef])`; 0027 turns `session` into an ordered list and the tables' `set_session` into a list setter.
- Identity everywhere is `ClusterRef`; `scope_memory`, health, and `previous` stay valid with several sessions.
- The row model has no checkbox field; 0027 adds `is_ticked` and the footer.

## Not changed

Kubeconfig loading (0024/0025 catalog), access reviews and kind counts (restart with the session), the `Connecting to …` busy view (label becomes `switcher_label`). The connect logs the existing "ignoring the proxy from the environment" info line once per connect and per probe; repeated lines are harmless (no secret: host only).
