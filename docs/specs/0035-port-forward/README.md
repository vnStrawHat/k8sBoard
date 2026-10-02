# 0035 — Port-forward and the Port Forwarding page

Status: draft, HEAD `1c859ae`. **Mutating (connect verb). C3: Approved by the user on 2026-10-02 (one approval for all mutating specs).** Debug builds still refuse a forward unless `K8SBOARD_ALLOW_WRITES=1` (agents never set it); UAT checks stay denied-path-only. Lands after 0030 (gate, tiers, audit, `run_guarded`, kill switch), 0036 (kube `ws`, `ConnectIntent`, `start_connect`), 0024 (presets key), 0026/0027 (switch, slots). Wireframes: W4b note 4 (Forward next to each port), W7 "Port Forwarding" (page, drawer), W7 Services/Deployments/StatefulSets "Port-forward ▸", status bar `⇄ N port-forwards`, keyboard F.

## Goal

- Forward a local port on **loopback only** (`127.0.0.1`, plus a best-effort `[::1]` on the same port) to a pod port, through kube's `ws` `Api::portforward`, one WebSocket per accepted local TCP connection.
- Targets: pod, Service (resolved to a ready backing pod), Deployment or StatefulSet (a ready pod).
- Many concurrent forwards across clusters, owned by the app, not by a session: they survive a cluster switch and slot changes.
- Auto-reconnect up to 5 tries (1, 5, 15, 30, 60 s) when the target pod goes away; new local connections are refused while the cluster is locked; port choice and conflicts; presets; traffic counters; recent events.
- Network › **Port Forwarding** page (list, inline Stop/Start/Retry, drawer), live Forward buttons, status bar count.
- Every start goes through 0030 `run_guarded` (`Connect`) with a `PortForwardPermit`; both `get` and `create` on `pods/portforward` (KEP-4006).

## Non-goals

UDP; binding any non-loopback address (never `0.0.0.0` or `::`); forwarding several ports in one row (one row per port); SPDY; forwarding to nodes or external hosts; auto-start of presets at launch; Ingress "Open URL" through a forward.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Cluster crate: `GetPodPortForward`, `PortForwardPermit`, `port_forward.rs` (resolve, listener, per-connection socket, counters, reconnect, upgrade errors); fake tests | 1–4, 6 |
| 2 | App: `port_forwards.rs` entity (pure state, presets, port choice), `Screen::PortForwarding` page and drawer with fixture rows, status bar; `--screen port-forwards` | 1, 2, 7, 8, 11 |
| 3a | Guarded start: `ConnectOpen::PortForward` in `run_guarded`, `start_forward`, Stop/Restart/Retry/Start, Forward buttons (W4b, Services), audit, lock pause; UAT denied path. Approved by the user on 2026-10-02 (one approval for all mutating specs). | 1, 2, 5, 9, 10, 13 |
| 3b | Menus (`Port-forward ▸`), F, palette, New forward, Change local port…, Remove preset… dialogs | 1, 2, 9 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale |
| [forward-transport.md](forward-transport.md) | cluster API, permit, RBAC, resolution, listener, sockets, reconnect, errors |
| [forward-model.md](forward-model.md) | app entity, states, lifecycle (switch, multi-cluster, lock, quit), presets, port choice, guarded start |
| [forwarding-page.md](forwarding-page.md) | page, drawer, Forward buttons, menus, F, dialogs, status bar |
| [files-to-touch.md](files-to-touch.md) · [test-plan.md](test-plan.md) | files per step; tests and checks |

## Acceptance criteria

- [ ] 1. Quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. `Cargo.lock` gains no package beyond 0036's delta (tokio `net`/`io-util` features only).
- [ ] 2. Every test in [test-plan.md](test-plan.md) exists under its name and passes offline; no test opens a real cluster connection.
- [ ] 3. Listeners bind `127.0.0.1` and best-effort `[::1]` only (tests inspect the bound addresses); no code path binds another address; a failed `[::1]` bind is ignored.
- [ ] 4. `port_forward.rs` is the only `portforward` call site and a row of the 0030 allow-list; it requires a `PortForwardPermit`, whose only non-test constructor is `AccessReport::port_forward_permit` (both verbs allowed); upgrade refusals 401/403/404 map to typed errors.
- [ ] 5. Debug builds without `K8SBOARD_ALLOW_WRITES=1` refuse a forward before any request (zero recorded requests); agents never set it.
- [ ] 6. A pod that disappears triggers at most 5 re-resolve attempts (1, 5, 15, 30, 60 s); success → Active with a "reconnected" event; exhaustion → `Target lost`.
- [ ] 7. A busy fixed local port shows `Port N in use`, a Windows-reserved one (`PermissionDenied`) `Port N is reserved by the system`, both with Retry; an auto port moves to the next free one in both cases.
- [ ] 8. Forwards survive a cluster switch and a 0027 slot release; quitting the app closes every listener.
- [ ] 9. Every start, restart, retry, and preset start runs the target cluster's 0030 gate and tier and appends one audit line (no traffic data, no per-connection lines).
- [ ] 10. On UAT the SSAR answers for `get` and `create pods/portforward` are recorded; Forward buttons, menus, F, and the page Start show `Not permitted: get and create pods/portforward`; **no portforward request is ever sent** (trace).
- [ ] 11. Colors come from theme tokens only (0003 grep clean).
- [ ] 12. ui-verifier: `--screen port-forwards` (fixture rows, drawer open) matches W7 Port Forwarding with no high-severity defect.
- [ ] 13. While a cluster is locked, its running forwards refuse new local connections (open ones continue) and show `Paused · read-only`; unlocking resumes them.

## Open items

1. R2: no write-capable cluster; the allowed path is fake-tested only until the user provides one.
2. (user) Kept strict: every start, restart, retry, and preset start asks the cluster tier (PROD types the name; every other environment clicks Confirm in the dialog), like 0036 shells. A relief (for example one confirm per preset per app run) is a user decision.
3. Windows lets a process bind `127.0.0.1:N` while another holds `0.0.0.0:N` without `SO_EXCLUSIVEADDRUSE`; our forward then shadows that service for loopback clients. Accepted; documented in the drawer tooltip.
4. Namespace-only RBAC with scope All shows as denied (0030 open item 5); applies here too.
5. Asymmetry with 0036: shells close on a cluster switch, forwards keep running on a held `ClusterConnection` clone. 0026 decision 1 ("teardown releases everything of the old session") gains the exception "except port-forwards, which hold their own connection clone" (orchestrator edit).
