# 0035 · Decisions

[Back to index](README.md). Architect defaults; items marked (user) need confirmation.

## Transport

| # | Decision | Rationale |
|---|---|---|
| 1 | kube `Api::<Pod>::portforward` over the `ws` feature that 0036 enables; no SPDY, no own WebSocket code | already in the lock; one transport for exec and forward |
| 2 | **One WebSocket per accepted local TCP connection**, one port each | kube's `Portforwarder` gives one duplex stream per port per socket; a browser opens several connections; this is kube's own `port_forward_bind` pattern |
| 3 | Bind `127.0.0.1`, plus a best-effort second listener on `[::1]` with the same port (its failure is ignored; its connections join the same set); never `0.0.0.0` or `::` | a forward must not expose a cluster port to the network; browsers resolve `localhost` to `::1` first |
| 4 | The forward is a `Stream` from the cluster crate polled by `ClusterRuntime::subscribe`; the crate calls no `spawn` (kube's `Portforwarder` spawns its own loop, which our socket guard aborts on drop) | same shape as `pod_logs` and 0036 `pod_shell`; dropping the stream closes the listener and every socket |
| 5 | Kill switch: `port_forward` checks the connection's 0030 `WritePolicy`; `Blocked` fails before any request | C3: the first real forward needs the user's approval; agents run debug builds |
| 6 | `PortForwardPermit` (not `Clone`), built only by `AccessReport::port_forward_permit`; required by `port_forward` | the 0036 `ExecPermit` pattern; an ungated forward does not compile |
| 7 | Gate on **both** `get` and `create` `pods/portforward`; fail closed | servers before 1.35 authorize the upgrade as `get`; KEP-4006 adds `create` in 1.35 |
| 8 | Upgrade refusals (`UpgradeConnection(ProtocolSwitch(code))`) map 401/403/404 to typed errors with fixed text | the non-101 response has no `Status` body |
| 9 | Cap 64 concurrent local connections per forward, 20 forwards running app-wide | each connection is a WebSocket to the API server |

## Targets and reconnect

| # | Decision | Rationale |
|---|---|---|
| 10 | Service → ready backing pod via the Service selector and existing `Selector::matches`; `targetPort` number, name (container port name in the chosen pod), or absent (= port) | kubectl semantics; one list call, no Endpoints watch |
| 11 | Deployment / StatefulSet → a ready pod matching `spec.selector`; the port is a template container port | W7 note "Port-forward picks a ready pod" |
| 12 | Ready pod = phase Running, condition Ready true, no `deletionTimestamp`; the first by name | deterministic, testable |
| 13 | Reconnect only after an abnormal upstream end **and** a GET showing the pod gone, not Running, or deleting; 5 tries at 1, 5, 15, 30, 60 s (about 2 min); Service/workload forwards may land on another pod | W7 "Reconnecting 2/5", "auto-reconnect up to 5"; a refused connection with a live pod is the app's problem, not the pod's |
| 14 | While reconnecting, new local connections are closed at once | holding them adds a queue and timeouts for little value |
| 15 | 403 or 401 on any open ends the forward (`Not permitted` / credentials); no reconnect | retrying cannot fix RBAC |
| 16 | Auto-reconnect and per-connection opens do not re-run the gate or the tier; they belong to the started forward | one consent per start, like a 0036 session; manual Restart/Retry/Start re-run it |

## Ports

| # | Decision | Rationale |
|---|---|---|
| 17 | Default local port = `10000 + remote` when `remote ≤ 55535`, else `remote`; `Auto` tries it then the next 20, then an OS-assigned port | matches the wireframe (5432 → 15432, 8080 → 18080, 9090 → 19090); avoids privileged ports |
| 18 | A port the user typed (dialog, preset) is `Exact`: `AddrInUse` → `Port N in use`, Windows `PermissionDenied` (WSAEACCES, an excluded port range) → `PortReserved` `Port N is reserved by the system`, both with Retry, never moved silently. `Auto` treats both errors as busy and tries the next port | the user expects that address; Hyper-V and WinNAT reserve port ranges |
| 19 | Conflicts across our own forwards are refused before binding (`Port N is used by another forward`) | clearer than an OS error |

## App and lifecycle

| # | Decision | Rationale |
|---|---|---|
| 20 | Forwards live in an app-level `PortForwards` entity owned by `AppShell`, keyed by `ClusterRef`; each running stream keeps a `ClusterConnection` clone, also for clusters no longer viewed | the page lists forwards of every cluster (W7); a cluster switch or slot release must not stop them. Amends 0026 decision 1 (teardown releases everything except these clones); asymmetric with 0036, whose shells close on a switch |
| 21 | Starting needs the target cluster to be viewed (`guard_for` is `Some`); otherwise `Open {cluster} to start this forward` | the 0030 gate, lock, and tier need a live session |
| 22 | Lock does not end running forwards, but while a viewed cluster is locked its forwards refuse **new** local connections (`Paused · read-only`; open ones continue); unlocking resumes; new starts are blocked | a locked cluster should not gain new traffic paths; ending open DB sessions would surprise more |
| 23 | Stop, Stop all, Copy, Open in browser, Save/Remove preset need no gate and no confirm | local actions; Stop reduces risk |
| 24 | Port-forward start is `ActionRisk::Change`; the cluster tier applies on every start, restart, retry, and preset start (strict). A relief, e.g. one confirm per preset per run, is a user decision | 0036 decision 11 for exec; README open item 2 |
| 25 | Audit: one line per start (action `Port-forward`, object, fields `remote_port`, `local_port`); never traffic, never per connection | C10 records actions, not sessions |
| 26 | Presets in `port_forward.presets` (0024 key): cluster, namespace, target kind and name, remote port, local port; never started at launch | W7 "Stopped · preset … Start"; starting is a guarded action |
| 27 | Keep the last 20 events per forward, in memory only | drawer "Recent events" |
| 28 | Traffic sampled at most once per second, only when changed | one notify per second per busy forward |
| 29 | Open in browser uses `http://127.0.0.1:{port}`; the page shows `localhost:{port}` (W7), the drawer `127.0.0.1:{port}` | the forward is IPv4 only; `localhost` may resolve to `::1` first |
| 30 | Each socket polls `take_error` concurrently with the copy (`select!`); every reason string that reaches the UI (error channel, WebSocket close, resolve errors) goes through one `reason_text` (control characters stripped, ≤ 200 chars) | an error-channel message must end the socket at once, not after the copy; one sanitizer for every path |
