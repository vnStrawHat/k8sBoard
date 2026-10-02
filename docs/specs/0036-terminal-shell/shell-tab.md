# 0036 · Shell tab, dock, entry points, lifecycle

[Back to index](README.md) · Steps 3–4 · Modules: `log_dock.rs` → `dock.rs`, `shell_tab.rs` (new; tests `shell_tab_tests.rs`), `resource_actions.rs`, `keymap.rs`, `app_shell.rs`. Decisions 23–32.

## Dock (W8, W8b, roadmap v0.5 rules)

- `LogDock` becomes `Dock` (`dock.rs`, key context `"Dock"`; 0028's `LogDock > Input` predicate is renamed with it). Tabs: `enum DockTab { Logs(Entity<LogTab>), Shell(Entity<ShellTab>) }`. One tab per session, **never side by side**; the existing rules stay: height ≤ 60 % of the workspace, ⤢ zoom over the workspace, minimize, Ctrl W / Ctrl Tab / Ctrl \` / Ctrl Shift M (0028), tabs kept across screen navigation.
- `Dock::open_shell(target, connection, window, cx)`: a new tab every time (two shells into one container are normal), activated; Minimized → Normal. Cap: 8 Shell tabs; the ninth shows the notice "Close a shell tab first (8 open)".
- Tab label `›_ shell · {pod suffix}/{container}` (W8: `shell · m8n2p/api`; the suffix rule is the Logs tab's); a dim dot when the session has ended.

## Shell tab (`ShellTab` entity, W8b pane)

```rust
pub(crate) struct ShellTarget { pub(crate) cluster: ClusterRef /* 0026 */, pub(crate) namespace: String,
    pub(crate) pod: String, pub(crate) container: String }
pub(crate) enum ShellState { Connecting, Live, Ended(ShellEnd) }
pub(crate) enum ShellEnd { Exited { code: Option<i32> }, Failed { reason: SharedString }, NoShell }
```

- Header row (W8b): `›_ {pod} · {container} · {shell}` | `Shell: Auto ▾` (Auto, bash, sh; a change reconnects with the new shell) | `Find` | `Clear` | `Reconnect`. `{shell}` is the resolved shell: `Auto` until the exec starts; for `Bash`/`Sh` the name; for `Auto` the shell the script picked, which it reports before `exec` with a private OSC (`printf '\033]7770;%s\007' bash`), routed `Forward` (`OscRoutes::route(7770, OscRoute::Forward)`) and read only for the names `bash`, `ash`, `sh`. The cluster appears only in the banner line (`# exec -n … ({cluster})`).
- Body: the terminal element, full width. `Connecting` shows the banner and "Connecting…"; `Ended` appends a note (`[process exited with code 1]`, `[connection lost: …]`, `No shell in this container; try Debug container… (0037)`) and stops sending input.
- Clear: local only, feeds `\x1b[H\x1b[2J\x1b[3J` (screen and scrollback). The remote shell is not told.
- Reconnect: enabled when `Ended`, and on `Live` after a "Restart this shell?" confirm that is skipped when the 0030 tier dialog will open anyway; goes through `start_connect` again (0030 gate, lock, tier, audit), keeps the scrollback, adds a separator note, opens a new exec. No automatic reconnect: a new exec is a new shell, its state is lost, so the user decides.
- Session wiring: `ClusterRuntime::subscribe(connection.pod_shell(request, input_rx), cx, apply, on_closed)`; `apply` feeds `Output` into `TerminalSession`, sends the outbox, sets state on `Started`/`Exited`/`Failed`; `subscribe` notifies once per update (each update is one ready read, already batched). Input: `futures::channel::mpsc::UnboundedSender<ShellInput>` held by the tab.

## Entry points

| Where | What |
|---|---|
| Pod row menu and drawer ⋯ "Open shell ▸" (W4) | submenu of containers with MAIN/SIDECAR tags (W4 note 2), Init containers left out; a single container opens directly; a container not `Running` is disabled "Container is not running" |
| S key (0028 `OpenShell`) | the cursor pod's default container: first running Main container, else the first running one |
| Container detail ⋯ menu (W4b note 3), if 0008/0019 built it | "Open shell" for that container |
| Palette (0029) `>` Open shell | same as S, through the same 0028 action |
| Dock "+ ▾" (W8 note 5), if 0019 built it | "Shell into selected" = S |

All of them read `action_availability(OpenShell, guard)` and then call `start_connect`; `Disabled { reason }` shows the 0028 notice and opens nothing. Node rows' "Open node shell" stays disabled ("Comes with node shell, 0037").

## Lifecycle

| Event | Effect |
|---|---|
| Tab closed (× or Ctrl W while focus is outside the terminal) | dropping `ShellTab` drops the subscription → the tokio task is aborted → the WebSocket closes; the remote shell gets a hangup |
| Cluster or context switch (0026 tears down) | in `switch_cluster`, when `dock.shell_tab_count() > 0`, a kit `Dialog` "{N} shells will close" (Enter confirms, Esc keeps the cluster); no tier logic. Then `Dock::close_all` as today. Under 0027 only the shells of the released slot count and close |
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
- `WINDOW` chords that no shell uses stay app chords inside the terminal: Ctrl \`, Ctrl Tab, Ctrl Shift Tab, Ctrl Shift M, Ctrl ,, Ctrl 1–9. On macOS every `secondary` chord is ⌘, which shells never receive, so ⌘K, ⌘N, ⌘W keep their app meaning.
- Ctrl W closes the active dock tab only while focus is outside the terminal (inside, `ctrl-w` is `NoAction` and reaches the program; on macOS ⌘W still closes the tab).
- Esc always goes to the program (vim) and never un-zooms the dock while the terminal has focus; Ctrl Shift M still toggles zoom. Leaving the terminal by keyboard: Ctrl \` minimizes the dock (focus returns to the shell root).
- `ctrl-shift-c` in the terminal outranks 0026's switcher chord (`secondary-shift-c` on Windows and Linux): the switcher is reachable from the terminal by the title bar only (decision 30).
