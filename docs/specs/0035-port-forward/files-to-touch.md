# 0035 · Files to touch

[Back to index](README.md). **S** is the step; each passes the gate on its own and every new item has a production user in its step. Prerequisites merged: 0030, 0036 (at least step 4: `ws`, `ConnectIntent`, `start_connect`), 0024, 0026, 0027.

## Cargo

| S | File | Change |
|---|---|---|
| 1 | `crates/cluster/Cargo.toml` | tokio `features = ["net", "io-util"]` (already unified in the lock; declared because `port_forward.rs` uses them). `Cargo.lock`: no package change (AC 1) |

The kube `ws` feature is already on (0036). No new dependency.

## `crates/cluster`

| S | File | Change |
|---|---|---|
| 1 | `src/port_forward.rs` (new) + `port_forward_tests.rs` | `PortForwardPermit` (+ `#[cfg(test)] for_tests`), `ForwardTarget`, `LocalPort`, `ForwardRequest`, `ForwardUpdate`, `ForwardTraffic`, `ForwardEvent`, `ForwardError`, `ClusterConnection::port_forward`, `default_local_port`; private `resolve_target`, `ready_pod`, `service_target_port`, `bind_local`, `candidate_ports`, `ForwardSocket` (abort guard), `Counting`, `socket_outcome`, `upgrade_error`, `RECONNECT_DELAYS`; the named `#[allow(clippy::disallowed_methods)]` on the `portforward` call |
| 1 | `src/access_review.rs` (+ tests) | `AccessCheck::GetPodPortForward` in `ALL`; `AccessReport::port_forward_permit` |
| 1 | `src/lib.rs` | `mod port_forward;` and its `pub use` |
| 1 | `src/connection.rs` | 0030's `run_raw` moves here as `pub(crate)` (shared by `object_write.rs`, `port_forward.rs`, 0037 `debug_shell.rs`) unless 0036 already did |
| 1 | `src/fake_api.rs` (0030) | a `connect` responder for 101 / 401 / 403 / 404 upgrade answers if 0036 has not added one |

## `crates/app`

| S | File | Change |
|---|---|---|
| 2 | `src/port_forwards.rs` (new) + `port_forwards_tests.rs` | `PortForwards`, `Forward`, `ForwardSpec`, `TargetSpec`, `LocalPortSpec`, `ForwardState`, `ForwardFailure`, `ForwardPreset`, `apply`, status texts, `running_for`, `active_count`, `local_port_for`, `parse_target` |
| 2 | `src/port_forward_page.rs` (new) | page table, drawer, empty state, header buttons |
| 2 | `src/app_shell.rs` | `Screen::PortForwarding`; `port_forwards: Entity<PortForwards>`; navigation to the page |
| 2 | `src/navigation.rs` | "Port Forwarding" item → `Screen::PortForwarding`; running count |
| 2 | `src/status_bar.rs` | `⇄ N port-forwards`, click → page |
| 2 | `src/settings.rs` (0024) (+ tests) | `port_forward.presets: Vec<ForwardPreset>`; allow-list test |
| 2 | `src/screenshot.rs`, `src/launch_options.rs` (+ tests) | `--screen port-forwards` fixture |
| 3a | `src/write_flow.rs` (0030/0036) | `ConnectOpen::{Exec, PortForward}`; `run_guarded` picks the matching permit |
| 3a | `src/app_shell.rs` | `start_forward`, Stop / Stop all / Restart / Retry; lock observer sending `ForwardControl` / Start handlers |
| 3a | `src/resource_actions.rs` (+ tests) | `PortForward` gate on `GetPodPortForward` + `CreatePodPortForward`, `mutates: true`, shipped; remove `port_forward_reason` |
| 3b | `src/resource_actions.rs` (+ tests) | `Port-forward ▸` submenus for pods, Services, Deployments, StatefulSets |
| 3a | `src/drawer.rs`, `src/container_detail.rs`, `src/kind_drawer.rs` | `port_row` states (Offer / Live / Disabled) |
| 3b | `src/keyboard_navigation.rs` (0028) | F runs `start_forward` or opens New forward |
| 3b | `src/port_forward_page.rs` | New forward, Change local port…, Remove preset… dialogs |
| 2–3 | `src/main.rs` | `mod port_forwards; mod port_forward_page;` |

## Docs (step 3b)

| File | Change |
|---|---|
| `docs/specs/0030-guardrails-write-path/write-path.md` | allow-list row `pods/portforward`; grep file list gains `port_forward.rs` |
| `docs/specs/0036-terminal-shell/exec-transport.md` | note: `ConnectIntent.open` is the `ConnectOpen` enum (0035) |
| `docs/roadmap/inventory-screens.md` | W4-10, PF-1, PF-2 → Done |
| `docs/roadmap/inventory-shell.md` | B3 → Done |
| `docs/roadmap/inventory-kinds.md` | Port Forwarding page; port-forward on Deployments, StatefulSets, Services → Done |
| `docs/roadmap/gap-plan-local-and-mutating.md` | 0035 entry: done; UDP and multi-port rows out of scope |
