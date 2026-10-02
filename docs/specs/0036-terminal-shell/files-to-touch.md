# 0036 · Files to touch

[Back to index](README.md). **S** is the step; each passes the gate on its own. Builds on 0028 (keymap, actions, `key_availability`), 0016 (`secret_clipboard`), 0030 (gate, confirm, audit, allow-list), and 0026 (switch teardown).

## Cargo (needs user approval)

| S | File | Change |
|---|---|---|
| 1 | `crates/cluster/Cargo.toml` | `kube = { workspace = true, features = ["ws"] }` |
| 2 | `Cargo.toml` | the commented `oneterm-vt` line becomes the pinned git dependency ([dependencies.md](dependencies.md)) |
| 2 | `crates/app/Cargo.toml` | `oneterm-vt.workspace = true` |
| 1–2 | `Cargo.lock` | exactly `tokio-tungstenite`, `tungstenite`, `sha1`, `data-encoding` (step 1) and `oneterm-vt` (step 2) |

## `crates/cluster`

| S | File | Change |
|---|---|---|
| 1 | `src/pod_shell.rs` (new) + `pod_shell_tests.rs` | `ShellRequest`, `GridSize`, `ShellCommand`, `ShellInput`, `ShellUpdate`, `ShellExit`, `ClusterConnection::pod_shell`, private `drive`, `argv`, `shell_exit`, `SHELL_ACTION` |
| 1 | `src/lib.rs` | `mod pod_shell;` and its `pub use` |
| 1 | `src/pod_shell.rs` | `upgrade_error`: the mandatory `ProtocolSwitch` 401/403/404 mapping ([exec-transport.md](exec-transport.md)); manual `Debug` for `ShellInput`, `ShellUpdate`; `ExecPermit` with `#[cfg(test)] for_tests` |
| 1 | `src/access_review.rs` | `AccessCheck::GetPodExec` (in `ALL`); `AccessReport::exec_permit` (the only non-test `ExecPermit` constructor) |
| 1 | `examples/probe.rs` | no exec (the probe stays read-only) |

## `crates/app`

| S | File | Change |
|---|---|---|
| 2 | `src/terminal_session.rs` (new) + `terminal_session_tests.rs` | `TerminalSession`, `SCROLLBACK_LINES`, event policy, `terminal_palette` |
| 2 | `src/terminal_element.rs` (new) | the GPUI element, `grid_size`, cell metrics, paint |
| 2 | `src/main.rs` | `mod terminal_session; mod terminal_element;` (step 3a: `log_dock` → `dock`, `shell_tab`; step 3b: `terminal_input`) |
| 3b | `src/terminal_input.rs` (new) | `key_to_vt`, paste encoding, mouse and wheel handling; tests in module |
| 3a | `src/shell_tab.rs` (new) + `shell_tab_tests.rs` | `ShellTarget` (`cluster: ClusterRef`), `ShellState`, `ShellEnd`, `ShellTab` view, header, Clear, Reconnect, session wiring, headless render; step 3b adds Find, the paste dialog, copy |
| 3a | `src/log_dock.rs` → `src/dock.rs` (rename alone in its own mechanical commit first) | `Dock`, `DockTab`, `open_shell`, the 8-tab cap, key context `"Dock"`; every `LogDock` user renamed (`app_shell.rs`, `workspace.rs`, `resource_actions.rs`, `pod_table.rs`, `pod_drawer.rs`, `screenshot.rs`, tests) |
| 3b | `src/keymap.rs` (+ `keymap_tests.rs`) | registered after every 0028 binding; `TerminalCopy`, `TerminalPaste`, `TerminalFind`, `CloseTerminalFind`; the `Terminal` bindings and `NoAction`s; `WORKSPACE` gains `!Terminal`; `LogDock > Input` → `Dock > Input`; sheet rows for copy, paste, Find |
| 4 | `src/write_flow.rs` (0030) | `ConnectIntent`; `start_connect` as a thin wrapper over 0030 `run_guarded` (`GuardedKind::Connect`) |
| 4 | `crates/cluster` allow-list (0030 `write-path.md` table and grep) | the `pods/exec` row; `pod_shell.rs` in the expected file list |
| 4 | `src/resource_actions.rs` (+ tests) | `OpenShell` gate on `GetPodExec` and `CreatePodExec`, `mutates: true`, shipped; the "Open shell ▸" container submenu; `default_shell_container` |
| 4 | `src/app_shell.rs` | the `OpenShell` handler opens a Shell tab through `start_connect`; `switch_cluster` asks "{N} shells will close" when `dock.shell_tab_count() > 0`, then `close_all` |
| 4 | `src/screenshot.rs`, `src/launch_options.rs` (+ tests) | `--screen shell-fixture` (W8b transcript const, `screenshot` feature only) |
| 4 | the 0025 About page (`settings_window.rs`) | list `oneterm-vt` (Apache-2.0) with its NOTICE text; see open item 6 |

## Docs (step 4)

| File | Change |
|---|---|
| `docs/roadmap/inventory-shell.md` | K6 Shell tabs → Done |
| `docs/roadmap/inventory-screens.md` | W4-3 Open shell, W4-4 container submenu (shell part) → Done |
| `docs/roadmap/cross-cutting.md` | C6 rows: `oneterm-vt` pin recorded; kube `ws` enabled by 0036 (0035 reuses it) |
| `docs/roadmap/gap-plan-local-and-mutating.md` | the 0036 entry: Attach ownership moves out of 0036 to a later item; Settings › Terminal & Shell moved out |
| `docs/specs/0028-keyboard-map/` | orchestrator note (orchestrator applies): `WORKSPACE` gains `!Terminal`; `LogDock` context renamed `Dock`; the A key and Attach are no longer owned by 0036 (a later item) |
| `docs/specs/0030-guardrails-write-path/` | done in this amendment: `run_guarded`, `GuardedIntent`, `GuardedKind`, `audit_entry` |
