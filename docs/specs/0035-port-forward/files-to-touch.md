# 0035 · Files to touch

[Back to index](README.md). **S** is the step; each passes the gate on its own and every new item has a production user in its step. Baseline main `2c7dc08` (0024, 0026, 0027, 0030 steps 1/2a/3 merged). Step 1 needs 0036 step 1 (`ws`); step 3a needs 0030 steps 2b + 4 and 0036 step 4 (`ConnectIntent`, `start_connect`, multi-check gate).

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
| 1 | `src/connection.rs` | the merged private `run_raw` (in `object_write.rs`) moves here as `pub(crate)` (shared by `object_write.rs`, `port_forward.rs`, 0037 `debug_shell.rs`); whichever of 0035 / 0037 lands first moves it |
| 1 | `src/fake_api.rs` (merged) | a `connect` responder for 401 / 403 / 404 upgrade refusals if 0036 has not added one (a 101 cannot be faked through `service_fn`) |

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
| 3a | `src/resource_actions.rs` (+ tests) | `PortForward` gate `Mutating { checks: [GetPodPortForward, CreatePodPortForward] }` (one check, unshipped, on main), shipped; remove `port_forward_reason` |
| 3a | `src/pod_drawer.rs`, `src/kind_drawer.rs` | pass the gate state instead of `port_forward_reason(access)` |
| 3b | `src/resource_actions.rs` (+ tests) | `Port-forward ▸` submenus for pods, Services, Deployments, StatefulSets |
| 3a | `src/drawer.rs`, `src/container_detail.rs`, `src/kind_drawer.rs` | `port_row` states (Offer / Live / Disabled) |
| 3b | `src/keyboard_navigation.rs` (0028) | the `PortForward` arm (empty on main) runs `start_forward` or opens New forward |
| 3b | `src/port_forward_page.rs` | New forward, Change local port…, Remove preset… dialogs |
| 2–3 | `src/main.rs` | `mod port_forwards; mod port_forward_page;` |

## Docs (step 3b)

| File | Change |
|---|---|
| `docs/specs/0030-guardrails-write-path/write-path.md` | the `pods/portforward` row and the `port_forward.rs` exception are already listed; mark them shipped |
| `docs/specs/0036-terminal-shell/exec-transport.md` | note: `ConnectIntent.open` is the `ConnectOpen` enum (0035) |
| `docs/roadmap/inventory-screens.md` | W4-10, PF-1, PF-2 → Done |
| `docs/roadmap/inventory-shell.md` | B3 → Done |
| `docs/roadmap/inventory-kinds.md` | Port Forwarding page; port-forward on Deployments, StatefulSets, Services → Done |
| `docs/roadmap/gap-plan-local-and-mutating.md` | 0035 entry: done; UDP and multi-port rows out of scope |

## Parallel work (lane W2: 0036 → 0035 → 0037 → 0034)

Disjoint from lane W1: `port_forward.rs`, `port_forwards.rs`, `port_forward_page.rs`, `drawer.rs` (`port_row`), `navigation.rs`, `status_bar.rs`, `settings.rs`. Shared, append-only: `access_review.rs`, `connection.rs` (`run_raw`), `write_flow.rs` (`ConnectOpen`), `resource_actions.rs`, `keyboard_navigation.rs`, `app_shell.rs` (`Screen::PortForwarding`), `launch_options.rs`, `screenshot.rs`, `main.rs`.
