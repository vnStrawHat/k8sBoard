# 0035 — Port-forward and the Port Forwarding page

Status: draft; **refreshed 2026-10-03 against main `2c7dc08`**. **Mutating (connect verb). C3: Approved by the user on 2026-10-02 (one approval for all mutating specs).** Debug builds still refuse a forward unless `K8SBOARD_ALLOW_WRITES=1` (agents never set it); UAT checks stay denied-path-only. Prerequisites: merged 0024, 0026, 0027, 0028, 0029, 0030 steps 1/2a/3 (kill switch `WritePolicy`); step 1 needs 0036 step 1 (kube `ws`); step 3a needs 0030 steps 2b + 4 (in flight: `run_guarded`, session `lock`) and 0036 step 4 (`ConnectIntent`, `start_connect`, multi-check gate, `RowAction`). Lane W2, after 0036. Wireframes: W4b note 4 (Forward next to each port), W7 "Port Forwarding" (page, drawer), W7 Services/Deployments/StatefulSets "Port-forward ▸", status bar `⇄ N port-forwards`, keyboard F.

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
| 3a | Guarded start: `ConnectOpen::PortForward` in `run_guarded`, `start_forward`, Stop/Restart/Retry/Start, Forward buttons (W4b, Services; `drawer::port_row` states replace `port_forward_reason`), the `PortForward` arm of `run_available_row_key`, audit, lock pause; UAT denied path. Approved by the user on 2026-10-02 (one approval for all mutating specs). | 1, 2, 5, 9, 10, 13 |
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

- [x] 1. Quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. `Cargo.lock` gains no package beyond 0036's delta (tokio `net`/`io-util` features only).
- [x] 2. Every test in [test-plan.md](test-plan.md) exists under its name and passes offline; no test opens a real cluster connection (the window tests use fake API servers and bind only loopback listeners).
- [x] 3. Listeners bind `127.0.0.1` and best-effort `[::1]` only (tests inspect the bound addresses); no code path binds another address; a failed `[::1]` bind is ignored.
- [x] 4. `port_forward.rs` is the only `portforward` call site and a row of the 0030 allow-list; it requires a `PortForwardPermit`, whose only non-test constructor is `AccessReport::port_forward_permit` (both verbs allowed); upgrade refusals 401/403/404 map to typed errors.
- [x] 5. Debug builds without `K8SBOARD_ALLOW_WRITES=1` refuse a forward before any request (zero recorded requests); agents never set it.
- [x] 6. A pod that disappears triggers at most 5 re-resolve attempts (1, 5, 15, 30, 60 s); success → Active with a "reconnected" event; exhaustion → `Target lost`.
- [x] 7. A busy fixed local port shows `Port N in use`, a Windows-reserved one (`PermissionDenied`) `Port N is reserved by the system`, both with Retry; an auto port moves to the next free one in both cases.
- [x] 8. Forwards survive a cluster switch and a 0027 slot release; quitting the app closes every listener.
- [x] 9. Every start, restart, retry, and preset start runs the target cluster's 0030 gate and tier (`guard_for(&forward.cluster)`, never the primary) and appends one audit line (no traffic data, no per-connection lines).
- [x] 10. On UAT the SSAR answers for `get` and `create pods/portforward` are recorded; Forward buttons, menus, F, and the page Start show `Not permitted: get and create pods/portforward`; **no portforward request is ever sent** (trace).
- [x] 11. Colors come from theme tokens only (0003 grep clean).
- [ ] 12. ui-verifier (screenshots v75 taken and read by the coder; the ui-verifier pass is pending): `--screen port-forwards` (fixture rows, drawer open) matches W7 Port Forwarding with no high-severity defect.
- [x] 13. While a cluster is locked, its running forwards refuse new local connections (open ones continue) and show `Paused · read-only`; unlocking resumes them.

## As built (steps 2 to 3b)

- The spec predates 0036, so its names map onto what 0036 built: there is no `run_guarded`. `ConnectIntent.open` is the `ConnectOpen` enum (`Exec`, `PortForward`) of `write_flow.rs`, and `commit_connect` takes the matching permit from the cluster's own report (`ConnectOpen::granted`). The gate is `ActionGate::Mutating { checks: [GetPodPortForward, CreatePodPortForward], is_shipped: true }`.
- `PortForwards` (`port_forwards.rs`) is an entity of `AppShell`; `port_forward_open.rs` holds `start_forward`, `start_forward_again` (Restart, Retry, Start), `stop_forward`, `change_local_port`, the preset save and remove, the lock sync, and the F arm. The audit follows 0036: the `abandoned` line is built at start and written if the start never reports (stop, restart, quit); `applied` or `failed` is written on the first of `Resolved` and `Ended`, and an automatic port is recorded as bound.
- The lock is read in `on_slot_changed`: the cluster's running forwards get `Refuse` or `Accept` once per change. A cluster that left the view keeps its last control. Forwards add no `leaving_work` line, because a switch or a released slot keeps them (decision 20).
- Drawers and menus: `PortButtons` (the running forwards and the gate of the drawer subject's cluster) feeds `drawer::port_row`; `ForwardMenu` builds the `Port-forward ▸` item for pods (`PodMenuItems`), Services, Deployments, and StatefulSets (`MenuExtras::port_forward`). `port_forward_reason` is gone. Pod menus and F list TCP ports only; UDP entries are disabled.
- The page and drawer render from `PortForwards` and need no live cluster (`render_body` and `render_drawer` branch on `Screen::PortForwarding`). The filter is a text field above the list, not the shell quick filter. Drawer menu items: Stop forward (or Remove from list for a failed plain row), Restart or Start, Open in browser, Copy local address, Change local port…, Save as preset, Go to target, Remove preset…. No Del key is bound, so the menu shows no key hint.
- Dialogs: New forward and Change local port… are forms that take Enter themselves and submit on a fresh press only (`keymap.rs` binds `enter` to `NoAction` in `ForwardForm`); Remove preset… uses `fresh_enter.rs` with an Enter that does nothing, so only a click removes. Starting from a form closes it and then asks the cluster's tier.
- Screenshot screens (all offline, no cluster wait): `port-forwards`, `port-forwards-list` (no drawer, every column), `port-forward-new-fixture`, `port-forward-confirm-fixture`, `port-forward-remove-fixture`. Traffic is shown in decimal units (`182 MB`).
- UAT (read-only, denied path): the session review answered `get pods/portforward` allowed and `create pods/portforward` denied, so the gate says `Not permitted: get and create pods/portforward`. A pod drawer run sent 78 `connection.run` requests (46 access reviews as POST, 32 reads as GET) plus the watch GETs, none with the actions `forwarding a port` or `finding the forward target`.

## Open items

1. R2: no write-capable cluster; the allowed path is fake-tested only until the user provides one.
2. (user) Kept strict: every start, restart, retry, and preset start asks the cluster tier (PROD types the name; every other environment clicks Confirm in the dialog), like 0036 shells. A relief (for example one confirm per preset per app run) is a user decision.
3. Windows lets a process bind `127.0.0.1:N` while another holds `0.0.0.0:N` without `SO_EXCLUSIVEADDRUSE`; our forward then shadows that service for loopback clients. Accepted; documented in the drawer tooltip. Added 2026-10-03 (W2 security review): the listeners set no `SO_EXCLUSIVEADDRUSE` on Windows, so another local process could bind the same loopback port first or alongside; the risk is local-only (loopback never leaves the machine) and the same as for any dev server on Windows.
4. Namespace-only RBAC with scope All shows as denied (0030 open item 5); applies here too.
5. Asymmetry with 0036: shells close on a cluster switch, forwards keep running on a held `ClusterConnection` clone. 0026 decision 1 ("teardown releases everything of the old session") gains the exception "except port-forwards, which hold their own connection clone" (orchestrator edit).
