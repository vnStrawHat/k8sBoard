# 0036 · Shell tab, dock, entry points, lifecycle

[Back to index](README.md) · Steps 3–4 · Modules: `log_dock.rs` → `dock.rs`, `shell_tab.rs` (new; tests `shell_tab_tests.rs`), `resource_actions.rs`, `keyboard_navigation.rs`, `keymap.rs`, `app_shell.rs`, `app_shell_view.rs`. Decisions 23–32, 35–37.

## Dock (W8, W8b, roadmap v0.5 rules)

- `LogDock` becomes `Dock` (`dock.rs`, key context `"Dock"`; the `keymap.rs` `FIELDS` entry `LogDock > Input` is renamed with it). It keeps what 0027 added: `LogOrigin { cluster, label, session, connection }`, `set_multi`, `close_tabs_of(cluster)`, `close_all`, and the "+ ▾" menu. Tabs: `enum DockTab { Logs(Entity<LogTab>), Shell(Entity<ShellTab>), Drain(Entity<DrainTab>) }` (`Drain` joined with 0034; a running drain's tab cannot be closed by the user). One tab per session, **never side by side**; the existing rules stay: height ≤ 60 % of the workspace, ⤢ zoom over the workspace, minimize, Ctrl W / Ctrl Tab / Ctrl \` / Ctrl Shift M (0028), tabs kept across screen navigation.
- `Dock::open_shell(target, connection, window, cx)` (`connection` = `slot_live(&target.cluster)?.connection().clone()`, the pattern of `LogOrigin`, never the primary's): a new tab every time (two shells into one container are normal), activated; Minimized → Normal. Cap: 8 Shell tabs; the ninth shows the notice "Close a shell tab first (8 open)".
- Tab label `›_ shell · {workload}-{suffix}/{container}` (`shell · api-m8n2p/api`; `pod_tab_name`, the name the Logs tab shows too: a Deployment pod reads as the Deployment and its pod suffix, a StatefulSet pod keeps its ordinal, a pod without a controller keeps its name); a dim dot when the session has ended.

## Shell tab (`ShellTab` entity, W8b pane)

```rust
pub(crate) struct ShellTarget { pub(crate) cluster: ClusterRef /* 0026 */, pub(crate) namespace: String,
    pub(crate) pod: String, pub(crate) container: String }
pub(crate) enum ShellState { Connecting, Live, Ended(ShellEnd) }
pub(crate) enum ShellEnd { Exited { code: Option<i32> }, Failed { reason: SharedString }, NoShell }
```

- Header row (W8b): `›_ {pod} · {container} · {shell} · {namespace} · {context}` | `Shell: Auto ▾` (Auto, bash, sh; a change reconnects with the new shell) | `Find` | `Clear` | `Reconnect`. `{shell}` is the resolved shell: `Auto` until the exec starts; for `Bash`/`Sh` the name; for `Auto` the shell the script picked, which it reports before `exec` with a private OSC (`printf '\033]7770;%s\007' bash`), routed `Forward` (`OscRoutes::route(7770, OscRoute::Forward)`) and read only for the names `bash`, `ash`, `sh`. The cluster appears only in the banner line (`# exec -n … ({cluster})`).
- Body: the terminal element, full width. `Connecting` shows the banner and "Connecting…"; `Ended` appends a note (`[process exited with code 1]`, `[connection lost: …]`, `No shell in this container; try Debug container… (0037)`) and stops sending input.
- Clear: local only, feeds `\x1b[H\x1b[2J\x1b[3J` (screen and scrollback). The remote shell is not told.
- Reconnect: enabled when `Ended` and on `Live` (the 0030 tier dialog, always shown, is its confirmation); goes through `start_connect` again (0030 gate, lock, tier, audit), keeps the scrollback, adds a separator note, opens a new exec. No automatic reconnect: a new exec is a new shell, its state is lost, so the user decides.
- Session wiring: `ClusterRuntime::subscribe(connection.pod_shell(request, input_rx), cx, apply, on_closed)`; `apply` feeds `Output` into `TerminalSession`, sends the outbox, sets state on `Started`/`Exited`/`Failed`; `subscribe` notifies once per update (each update is one ready read, already batched). Input: `futures::channel::mpsc::UnboundedSender<ShellInput>` held by the tab.

## Entry points

| Where | What |
|---|---|
| Pod row menu and drawer ⋯ "Open shell ▸" (W4) | submenu of containers with MAIN/SIDECAR tags (W4 note 2), Init containers left out; a single container opens directly; a container not `Running` is disabled "Container is not running; see Previous logs or Debug container" |
| S key (0028 `OpenShell`) | the `OpenShell` arm of `run_available_row_key` (empty on main): the cursor pod's default container (first running Main, else the first running one), in the cursor's slot; a pod with several containers shows the container picker (the Open shell submenu list in a dialog) first, then the confirm |
| Container detail ⋯ menu (W4b note 3), if 0008/0019 built it | "Open shell" for that container |
| Palette (0029) `>` Open shell | dispatches the S key action: the same arm |
| Dock "+ ▾" (W8 note 5, exists on main) | "Shell into selected": today `disabled_menu_item(.., open_shell_reason(subject_live))`; becomes an item that dispatches S (the drawer subject is the cursor) |

The submenu items carry an argument (the container), so they keep an `on_click` that captures the row's `RowContext` and calls `start_connect` for that slot; S, the palette, and "Shell into selected" run the arm. All read `action_availability(OpenShell, guard)` of the row's own slot (`open_shell_reason` is removed); `Disabled { reason }` shows the 0028 notice and opens nothing. Node rows' "Open node shell" stays `Comes in a later version` until 0037.

## Lifecycle

| Event | Effect |
|---|---|
| Tab closed (× or Ctrl W while focus is outside the terminal) | dropping `ShellTab` drops the subscription → the tokio task is aborted → the WebSocket closes; the remote shell gets a hangup |
| Cluster switch (0026 `release_all` → `dock.close_all`) or slot release (0027 `release_slot` → `dock.close_tabs_of`) | before either runs, `leaving_work` (one kit `Dialog`, shared with 0031 / 0034 / 0037) lists "{N} shells will close" for the leaving clusters only; Esc keeps everything. Confirm → the release closes those tabs (`DockTab::cluster()`), each drop ends its session |
| Screen navigation, drawer changes, dock minimize or zoom | sessions keep running |
| Pod deleted or restarted | the server ends the exec → `Ended(Failed)` with its reason; Reconnect then fails "pod not found" |
| App quit | runtime shutdown aborts the tasks (existing 2 s timeout) |
| Theme change | `set_palette`, full repaint |

## Keys (extends 0028)

| Binding | Action | Context | Why |
|---|---|---|---|
| `ctrl-shift-c`, `cmd-c` | `TerminalCopy` | `Terminal` | Ctrl C must reach the program as SIGINT |
| `ctrl-shift-v`, `cmd-v` | `TerminalPaste` | `Terminal` | same |
| `ctrl-shift-f`, `cmd-f` | `TerminalFind` | `Terminal` | opens Find |
| `escape` | `CloseTerminalFind` | `ShellFind > Input` | closes Find and refocuses the terminal. Same depth as 0028 `LeaveInput` (`Dock > Input`), so it must win by order: `keymap::bind_keys` registers every 0036 binding after the 0028 field bindings (test `escape_in_find_closes_find_not_leave_input`) |
| `tab`, `shift-tab`, `ctrl-c`, `ctrl-k`, `ctrl-n`, `ctrl-w` | `NoAction` | `Terminal` | suppress the kit Root focus cycling and copy, and the 0028/0029 chords Ctrl K, N, W on Windows and Linux, so the key reaches `on_key_down` and the program (completion, kill-line, history, word delete) |

- 0028 `WORKSPACE` gains `&& !Terminal`: single keys never act on the table while a shell has focus.
- `WINDOW` chords that no shell uses stay app chords inside the terminal: Ctrl \`, Ctrl Tab, Ctrl Shift Tab, Ctrl Shift M, Ctrl ,. On macOS every `secondary` chord is ⌘, which shells never receive, so ⌘K, ⌘N, ⌘W keep their app meaning.
- Ctrl W closes the active dock tab only while focus is outside the terminal (inside, `ctrl-w` is `NoAction` and reaches the program; on macOS ⌘W still closes the tab).
- Esc always goes to the program (vim) and never un-zooms the dock while the terminal has focus; Ctrl Shift M still toggles zoom. Leaving the terminal by keyboard: Ctrl \` minimizes the dock (focus returns to the shell root).
- `ctrl-shift-c` in the terminal outranks 0026's switcher chord (`secondary-shift-c` on Windows and Linux): the switcher is reachable from the terminal by the title bar only (decision 30).
- 0026 binds `secondary-1…9` (`SwitchToCluster1…9`) and `secondary-shift-c` in the shell root context. Inside a Terminal, `ctrl-shift-c` is `TerminalCopy` (above), and `ctrl-1…9` must be bound or consumed in the `Terminal` context (`NoAction`, like Ctrl K), so Ctrl n inside a pod shell reaches the program and does not switch clusters. On macOS ⌘1…9 stay app chords (shells never receive ⌘).
- The `!Terminal` part of `WORKSPACE` also keeps 0028's `secondary-c` (`CopyName`) out of the terminal, and with it every letter key. The terminal element must set `key_context("Terminal")`, or the table keys (and Ctrl C as "copy name") would be captured while a shell has focus.

## UX walk fixes (I14, I15)

- Find has ↑ / ↓ buttons beside `1 of N` (previous and next match, like Shift+Enter and Enter). The terminal body has `px_2` padding so text keeps off the sidebar border.
- The exec, attach, and forward confirms hide the "Dry-run not supported" line (a stream start has nothing to check), say "this action won't be logged" when no audit folder is set, and the forward confirm words its fields `Remote port` and `Local port` (the audit line keeps `remote_port` and `local_port`).
- The Debug container and node shell Image fields keep the start of a long image; under the field a muted line shows it cut in the middle (registry and tag or digest tail), with the whole image as its tooltip.
