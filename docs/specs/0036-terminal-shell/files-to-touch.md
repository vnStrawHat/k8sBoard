# 0036 · Files to touch

[Back to index](README.md). **S** is the step; each passes the gate on its own. Baseline main `2c7dc08`: 0016 (`secret_clipboard`), 0026/0027 (`release_all`, `release_slot`, `close_tabs_of`), 0028 (`keymap.rs`, `key_availability_of`), 0030 steps 1/2a/3 (`WritePolicy`, `ActionGate`, `audit_log.rs`). Step 3a after 0030 steps 2b + 4; step 4 after 0032 2a-i.

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
| 3a | `src/log_dock.rs` → `src/dock.rs` (rename alone in its own mechanical commit first, merged at once) | `Dock`, `DockTab` (+ `cluster()`), `open_shell`, the 8-tab cap, key context `"Dock"`; every `LogDock` / `log_dock` user renamed: `app_shell.rs`, `app_shell_view.rs`, `workspace.rs`, `resource_actions.rs`, `keyboard_navigation.rs`, `keymap.rs`, `pod_table.rs`, `pod_drawer.rs`, `issue_table.rs`, `overview.rs`, `log_tab.rs`, `launch_options.rs`, `screenshot.rs`, `main.rs`, and the tests (`app_shell*_tests.rs`, `keymap_tests.rs`, `launch_options_tests.rs`) |
| 3b | `src/keymap.rs` (+ `keymap_tests.rs`) | registered after every 0028 binding (and after the `FIELDS` loop); `TerminalCopy`, `TerminalPaste`, `TerminalFind`, `CloseTerminalFind`; the `Terminal` bindings and `NoAction`s; `WORKSPACE` gains `!Terminal`; `LogDock > Input` → `Dock > Input`; sheet rows for copy, paste, Find |
| 4 | `src/write_flow.rs` (0030) | `ConnectIntent`; `start_connect` as a thin wrapper over 0030 `run_guarded` (`GuardedKind::Connect`) |
| 4 | `crates/cluster` allow-list (0030 `write-path.md` table and grep) | the `pods/exec` row; `pod_shell.rs` in the expected file list |
| 4 | `src/resource_actions.rs` (+ tests) | `ActionGate::Mutating { checks: Vec<AccessCheck>, .. }` and the pair reason; `OpenShell` gate on `GetPodExec` and `CreatePodExec`, shipped; the "Open shell ▸" container submenu (`on_click` with `RowContext`); `default_shell_container`; remove `open_shell_reason` |
| 4 | `src/keyboard_navigation.rs` (0028) | the `OpenShell` arm → `start_connect` for the cursor pod's default container (the `OpenNodeShell` arm stays empty for 0037) |
| 4 | `src/app_shell.rs`, `src/app_shell_view.rs` | `leaving_work` + `ReleaseCheck` at `switch_cluster` / `view_clusters` / `remove_from_view` (unless 0031 step 3 already added it: then one more line); dock "Shell into selected" dispatches S |
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

## Parallel work (lane W2, first: 0036 → 0035 → 0037 → 0034)

- Steps 1–2 now, beside the in-flight 0030 2b/4: `pod_shell.rs`, `terminal_session.rs`, `terminal_element.rs` are new; only `access_review.rs`, `lib.rs`, Cargo files, and `main.rs` are shared (append-only).
- Step 3a's rename touches ~20 app files: merge it alone and early, before lane W1 starts 0032 2a-ii, so W1 rebases once.
- Shared with lane W1 later, append-only: `resource_actions.rs`, `keyboard_navigation.rs` (own arm), `keymap.rs`, `write_flow.rs` (`Connect` branch), `app_shell.rs` (`leaving_work`), `launch_options.rs`, `screenshot.rs`.
