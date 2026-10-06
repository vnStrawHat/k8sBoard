# 0035 · Port Forwarding page, Forward buttons, menus, dialogs

[Back to index](README.md) · Steps 2–3 · Modules: `port_forward_page.rs` (new), `app_shell.rs` (`Screen::PortForwarding`), `navigation.rs`, `status_bar.rs`, `drawer.rs` (`port_row`), `container_detail.rs`, `kind_drawer.rs`, `resource_actions.rs`, `keyboard_navigation.rs` (0028), `screenshot.rs`, `launch_options.rs`. Wireframes: W7 Port Forwarding, W4b note 4, W7 Services/Deployments/StatefulSets, status bar.

## Page (`Screen::PortForwarding`, Network › Port Forwarding)

Not a Kubernetes kind (W7 note): a local list rendered with the 0009 table toolkit from `PortForwards`, not a `ResourceKind`.

| Part | Content |
|---|---|
| Header | `Port Forwarding` + muted `{total} ({active} active)`; buttons `+ New forward`, `Stop all` (disabled with no running forward) |
| Columns | Target (`{ns}/{pod|svc|deploy|sts}/{name}`), Ports (`{remote} → localhost:{local}`, local `—` before bind), Status (theme token pill, decision texts in forward-model.md), Cluster (env badge + label), Uptime (`2h 14m`, `—` when not Active), action |
| Action cell | Active / Paused / Reconnecting / Starting → `■ Stop`; Failed → `↻ Retry`; Stopped preset → `▶ Start` (W7 rows) |
| Empty state | `No port forwards. Use Forward next to a port, or + New forward.` |
| Sort / filter | 0009 toolkit: filter text over Target and Cluster; default sort: running first, then target |

Sidebar item count = running forwards (W7 sidebar `3`); hidden when 0.

## Drawer (W7 "pod/postgres-0 · 5432")

- Title `{target} · {remote}`; status pill; meta `{ns} · {cluster} · since {HH:MM}`.
- ⋯ menu (W7 order): `Stop forward` (Del), `Restart`, `Open in browser`, `Copy local address`, `Change local port…`, `Save as preset`, `Go to target` (⏎), separator, `Remove preset…` (danger; presets only). Stop-type items follow the state; Restart/Start-type items read the gate (forward-model.md step 1–3) and show its reason when disabled.
- Section **Forward**: Target (`pod/{pod} · container {c}` when known), Remote port `{n}/TCP`, Local address `127.0.0.1:{n}`, Bind `localhost only`, Auto-reconnect `on · up to 5 tries`.
- Section **Traffic**: Open connections, Received, Sent (binary units, `182 MB`).
- Section **Recent events**: newest first, `HH:MM` + text, success/warning tokens; e.g. `reconnected after pod restart`, `connection lost: pod deleted`, `started from drawer of postgres-0`.
- Go to target: `reveal(ResourceKey)` in that cluster; disabled `Open {cluster} first` when not viewed.

## Forward buttons (W4b note 4, W7 Services)

- On main `drawer::port_row(text, id, reason, cx)` renders an always-disabled Forward button whose tooltip is `resource_actions::port_forward_reason(access)` (callers `pod_drawer.rs`, `kind_drawer.rs`). It gets a state instead: `Offer { on_click }`, `Live { local, on_stop }`, `Disabled { reason }`; `port_forward_reason` is removed, the reason comes from `action_availability(PortForward, guard)` of the drawer subject's slot (`drawer_subject()`, never the primary). Live renders the address `● localhost:{n}` in the success token (W4b; click copies it, tooltip `Copy localhost:{n}`) and a separate small `Stop` button that stops that forward (UX walk I3). A start that reports `Up` pushes the notice `Forwarding localhost:{n} → svc/api:80` with Copy and Open in browser buttons.
- Live match = same cluster, namespace, target, and remote port as a running forward (`PortForwards::running_for`).
- Pod container ports (W4b, 0008 `container_detail.rs`) → `ForwardTarget::Pod`, remote = container port. Service ports (W7 drawer) → `Service`, remote = the Service port. UDP ports → `Disabled("UDP ports cannot be forwarded")`. ExternalName or selector-less Services → `Disabled("Forward a pod: this Service selects no pods")`.
- Enabled click → `start_forward(spec with LocalPort::Auto)`.

## Menus and key F

| Where | Item |
|---|---|
| Pod row / drawer ⋯ (W4) | `Port-forward ▸`: one item per declared TCP container port, `{container} · {name} {port}/TCP` with MAIN/SIDECAR tag (W4 note 2); a single port starts directly; none declared → `Port-forward…` opens New forward prefilled with the pod |
| Deployment / StatefulSet ⋯ (W7) | `Port-forward ▸`: template container ports → `Deployment` / `StatefulSet` target |
| Service ⋯ (W7) | `Port-forward ▸`: Service ports → `Service` target |
| F (0028 `PortForward`) | the `PortForward` arm of `run_available_row_key`, on the cursor row and its slot: one port → start; several or none → New forward dialog prefilled |
| Palette `>` Port-forward (0029) | dispatches the F key action: the same arm |

All read `action_availability(PortForward, guard)` with the guard of the row's own slot (0030 order: shipped → checking → `Not permitted: get and create pods/portforward` → `{cluster} is read-only`; the pair text comes from 0036's multi-check gate). On main pods and `has_port_forward` kinds show `action_item(PortForward, guard)`; the submenus replace it. Submenu items carry an argument (the port), so they keep an `on_click` that captures the `RowContext` (cluster + weak session) of the row; the plain item without a port dispatches F. `port_forward_reason` goes away (`READ_ONLY_FEATURE_REASON` is already gone).

## Dialogs (kit `Dialog`, width 420)

- **New forward**: Cluster (select of viewed clusters, default the primary; env badge), Namespace (input, default the single scoped namespace), Target (input `pod/NAME`, `svc/NAME`, `deploy/NAME`, `sts/NAME`; pure `parse_target`), Remote port (1–65535), Local port (empty = auto). `Forward` runs `start_forward`; invalid fields show inline errors and keep the dialog.
- **Change local port…**: one number input, preset value or current; `Apply` validates 1–65535 and decision 19; restarts a running forward (guarded).
- **Remove preset…**: `Remove the preset {target}:{port}?`, `Cancel` / `Remove` (danger), click only.

## Status bar (wireframe `⇄ 3 port-forwards`)

`status_bar.rs`: `⇄ {n} port-forward(s)` when `n ≥ 1`; click → `Screen::PortForwarding`. Counts running forwards of every cluster.

## Screenshot (`--screen port-forwards`, `screenshot` feature only)

Fills `PortForwards` with the five W7 fixture rows (no subscription, no request): Active ×2, Reconnecting 2/5, `Port 3000 in use`, `Stopped · preset`, across two fixture cluster labels; opens the drawer of the first row with fixture traffic and three events. Listed in `USAGE`.

- UX batch 5c: the list is not a kit table, so its header borrows the table look (`table_head` background, `table_head_foreground`, the row text size), and the Target column has a fixed width so a long `namespace/kind/name` is cut in the middle (`cell_truncation::middle_truncate`) with the whole text in the tooltip.
