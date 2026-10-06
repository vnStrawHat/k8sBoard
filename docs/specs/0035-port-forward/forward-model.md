# 0035 · App model, lifecycle, guarded start

[Back to index](README.md) · Steps 2–3 · Modules: `port_forwards.rs` (new) + `port_forwards_tests.rs`, `write_flow.rs` (0030/0036), `app_shell.rs`, `settings.rs` (0024), `resource_actions.rs`. Decisions 16–29.

## Entity (`port_forwards.rs`)

```rust
pub(crate) struct PortForwards { forwards: Vec<Forward>, next_id: u64 }   // Entity owned by AppShell
pub(crate) struct ForwardId(u64);
pub(crate) struct Forward {
    pub(crate) id: ForwardId, pub(crate) cluster: ClusterRef, pub(crate) cluster_label: SharedString,
    pub(crate) spec: ForwardSpec, pub(crate) state: ForwardState, pub(crate) local: Option<SocketAddrV4>,
    pub(crate) pod: Option<String>, pub(crate) traffic: ForwardTraffic, pub(crate) events: VecDeque<ForwardLogLine>, // ≤ 20
    pub(crate) started_at: Option<jiff::Timestamp>, pub(crate) is_preset: bool,
    run: Option<ForwardRun>,   // Some while running
}
struct ForwardRun { _subscription: WatchSubscription, control: UnboundedSender<ForwardControl> } // the stream owns its `ClusterConnection` clone, also after the cluster leaves the view
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ForwardSpec { pub(crate) namespace: String, pub(crate) target: TargetSpec, pub(crate) remote_port: u16,
    pub(crate) local_port: LocalPortSpec }   // TargetSpec { kind: pod|service|deployment|statefulset, name }
pub(crate) enum ForwardState { Starting, Active, Paused, Reconnecting { attempt: u8 }, Failed(ForwardFailure), Stopped }
pub(crate) enum ForwardFailure { PortInUse(u16), PortReserved(u16), NotPermitted, TargetLost, Other(SharedString) }
```

- `apply(&mut self, id, ForwardUpdate)` (pure state transition, unit-tested): `Listening` → `local`; `Resolved` → `Active`, `pod`; `Event(Reconnecting{n})` → `Reconnecting{n}`; `Event(Reconnected)` → `Active`; `Event(Paused)` → `Paused`; `Event(Resumed)` → `Active`; `Traffic` → counters; `Ended(e)` → `Failed(..)`, `run = None`. Each event appends a line with the local time.
- Stop: `run = None` (drops the subscription: listener closed, sockets aborted), state `Stopped`; a non-preset stopped row is removed; a preset row stays `Stopped · preset`.
- Status text (W7): `Starting…`, `Active`, `Reconnecting {n}/5`, `Paused · read-only`, `Port {n} in use`, `Port {n} is reserved or needs more rights`, `Not permitted`, `Target lost`, `{text}`, `Stopped · preset`. Tokens: success, warning, danger, muted.
- `active_count()` feeds the status bar and the sidebar.

## Guarded start (reuses 0036 `ConnectIntent`, generalized)

0036 leaves "how 0035's permit enters `Connect`" to 0035. 0035 generalizes the open callback:

```rust
pub(crate) enum ConnectOpen {
    Exec(Box<dyn FnOnce(ExecPermit, &mut Window, &mut App)>),             // 0036
    PortForward(Box<dyn FnOnce(PortForwardPermit, &mut Window, &mut App)>),// 0035
}
pub(crate) struct ConnectIntent { pub(crate) object: AuditObject, pub(crate) fields: Vec<AuditField>, pub(crate) open: ConnectOpen }
```

`run_guarded` takes the matching permit from the guard's `AccessReport` (`exec_permit` / `port_forward_permit`) right before `open`; `None` → the gate reason, nothing opens, no audit line.

`AppShell::start_forward(spec, cluster, existing: Option<ForwardId>, window, cx)`:

1. `guard_for(&cluster)`; `None` → notice `Open {cluster} to start this forward`.
2. Own-port check (decision 19) for `Exact` ports.
3. `start_connect(GuardedIntent { cluster, action: PortForward, label: "Port-forward {target}:{port}", risk: Change, expected_name: None, kind: Connect(ConnectIntent { object, fields: [remote_port, local_port], open: PortForward(..) }) })`.
4. `open`: insert or reuse the row (`Starting`), `connection = slot_live(&cluster)?.connection().clone()` (0027; `slot_live` is private in `app_shell.rs` on main, so the same `pub(crate)` accessor 0030 step 4 needs for `checked_write`), `ClusterRuntime::subscribe(connection.port_forward(permit, request, control_rx), cx, apply, on_closed)` on the `PortForwards` entity.

Restart = Stop + `start_forward(existing)`; Retry and Start (preset) = `start_forward(existing)`. The 20-forward cap is checked in step 2 (`Stop a forward first (20 running)`).

## Lifecycle

| Event | Effect |
|---|---|
| Cluster switch (0026 `release_all`) or 0027 slot released (`release_slot`) | nothing: forwards keep running on their own connection clone; their rows keep the cluster label (0026 decision 1 amended; 0036 shells close instead). Neither path touches `PortForwards`, and forwards add no `leaving_work` line |
| Lock toggled on (viewed cluster) | `PortForwards` observes each slot session's `lock` (0030 step 2b, in flight; the merged `ClusterSession::guard` still derives the lock from the profile) and sends `Refuse` to that cluster's running forwards: new local connections are closed, open ones continue, rows show `Paused · read-only`; unlock sends `Accept`. Start/Restart/Retry show `{cluster} is read-only`. A cluster that leaves the view keeps its last control |
| Kubeconfig credentials rotate | handled by the kube client of the held connection |
| App quit | with a running forward that is not a saved preset (or a live shell), the window close asks first through `leaving_work` ("{N} port-forwards will stop", UX walk I6); then `PortForwards` is dropped with `AppShell` and runtime shutdown aborts the tasks (existing 2 s timeout) |
| Pod deleted | transport reconnect (forward-transport.md step 7) |
| Settings presets edited elsewhere (0025 window) | `observe_global` reloads preset rows that are `Stopped`; running rows are untouched |

Multi-cluster (0027): the page lists forwards of every cluster in both modes; the Cluster column is always shown (W7). Forward buttons in a drawer use the drawer subject's cluster (`ClusterObject.cluster`), never the primary.

## Presets (`port_forward.presets`, 0024 reserved key)

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ForwardPreset { pub(crate) cluster: ClusterRef, pub(crate) spec: ForwardSpec }
```

- Save as preset: appends (dedup by cluster + spec without `local_port`); the row gets `is_preset`. Remove preset…: a click-confirm dialog, then removed from settings; a running row keeps running as a non-preset.
- Startup: presets become `Stopped · preset` rows; nothing starts automatically (decision 26).
- Preset ports are `Exact` (decision 18). Added to the 0024 key allow-list test. No secrets: names and numbers only.

## Port choice (pure, `port_forwards.rs`)

`fn local_port_for(spec_port: Option<u16>, remote: u16) -> LocalPort`: typed → `Exact`; none → `Auto(default_local_port(remote))`. `Change local port…` stores `Exact(n)` and restarts a running forward through `start_forward` (guarded).
