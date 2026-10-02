# 0036 · Decisions

[Back to index](README.md). Architect defaults; "user" marks a decision the user made. The whole spec needs the user's approval for exec (project rule, C3).

## Dependencies

| # | Decision | Rationale |
|---|---|---|
| 1 | `oneterm-vt` pinned to `e2f9c24b…`, `default-features = false`; not `alacritty_terminal` | project rule; without `pty` the engine adds no new transitive crate |
| 2 | Pin by `rev`, not branch or tag | the crate is unpublished and its tags predate it |
| 3 | kube `ws` only in `crates/cluster` | the app has no kube dependency |
| 4 | Do not depend on OneTerm's `terminal-view`; read it as a reference | an application crate bound to its workspace |
| 5 | Lock delta is exactly five packages (probed) | AC 2 catches a surprise dependency |

## Transport, RBAC, gating

| # | Decision | Rationale |
|---|---|---|
| 6 | The session is a `Stream`; it owns the `AttachedProcess`, whose `Drop` aborts kube's message-loop task; `pod_shell.rs` calls no `spawn` | dropping the stream is the one way to end a session; same shape as `pod_logs` |
| 7 | Each ready stdout read (≤ 64 KiB) is one update; no batch timer | keystroke echo must feel immediate; a read already coalesces bursts |
| 8 | `Auto` runs one `sh -c` loop over bash, ash, sh and reports the pick with private OSC 7770 | one exec, no probing round-trips; the header can show the real shell |
| 9 | TTY mode (`interactive_tty`), stderr merged; exec is one `GET` upgrade | an interactive shell needs a TTY; kubectl does the same |
| 10 | One guarded core: 0030 `run_guarded` with `GuardedKind::Connect`; `start_connect` is a thin wrapper; 0036 sets no policy | lock, SSAR, tier, confirmation, and audit live in one place; the dry-run is the only branch |
| 11 | Open shell is `ActionRisk::Change`; the cluster's confirm tier applies as is | one rule for every guarded action; the tier is editable in Settings › Safety |
| 12 | Audit one line per session start, never stream bytes | C10 records actions; C1 forbids recording output |
| 13 | RBAC needs **both** `get` and `create` on `pods/exec`; fail closed; the UAT check records both SSAR answers and never attempts an exec | servers before 1.35 authorize a WebSocket exec as `get` (kubernetes#78741); KEP-4006 adds `create` in 1.35; UAT is 1.29.5 |
| 33 | `ExecPermit` is required by `pod_shell`; only `AccessReport::exec_permit` makes one | the type system keeps an ungated exec out of production code |
| 34 | Upgrade refusals (`ProtocolSwitch` 401, 403, 404) map to typed errors with fixed text | the upgrade response has no `Status` body; a raw "protocol switch" error tells the user nothing |

## Terminal

| # | Decision | Rationale |
|---|---|---|
| 14 | Parse on the GPUI thread, ≤ 64 KiB per update, ~1 ms budget per update (release) | no lock, no thread; `ponytail:` a reader thread plus a mutex if bulk output stalls frames |
| 15 | Scrollback 5,000 lines per tab, 8 Shell tabs max | about 8 MB per tab at 200 columns (8 bytes per cell), about 64 MB worst case for 8 tabs; inside the C13 budget only if few shells are open, so the cap stays |
| 16 | Palette from kit theme tokens; `ColorQuery` answered from it with the query's terminator | theme rule; programs that ask for colours get the app's |
| 17 | Remote clipboard access (OSC 52) ignored both ways; `allow_screen_readback` pinned `false` | a container must not read the clipboard or read back screen text it did not write |
| 18 | Paste: strip C0 (except `\t \r \n`) and C1 controls; newline mapping only when not bracketed; "Paste N lines?" dialog for multi-line non-bracketed pastes in every environment (user) | paste injection guard; a pasted script must not run line by line unseen |
| 19 | No click reporting to programs in 0036; wheel only | keeps the input path small |
| 20 | Copy uses 0016's private clipboard write, **without** the 30 s auto-clear (user) | output may hold secrets, so it stays out of history and cloud sync; the user picked the text to paste it later |
| 21 | No title, bell, or notification handling | the tab label is fixed by W8 |
| 22 | Reshape visible rows every frame | simplest correct renderer; a row cache only after a profile |

## Tab and lifecycle

| # | Decision | Rationale |
|---|---|---|
| 23 | Rename `LogDock` → `Dock` (its own mechanical commit, step 3a) with `DockTab::{Logs, Shell}` | it holds both tab kinds |
| 24 | A new tab per "Open shell", even for the same container | two shells into one container are normal |
| 25 | No auto-reconnect; explicit Reconnect | a reconnect is a new process |
| 26 | Cluster switch with open shells asks "{N} shells will close" (kit `Dialog`, Enter confirms), no tier logic; under 0027 only the released slot's shells count (user) | closing live shells silently loses work |
| 27 | Clear is local only | matches W8b; no remote cooperation needed |
| 28 | S opens the default running Main container; menus show a picker | one key, one shell (W4 note 2) |
| 29 | Init containers never offered | not running after start |
| 30 | In the terminal, Ctrl Shift C is copy, not the cluster switcher | the terminal convention wins where the user types |
| 31 | Ctrl K, N, W, C, Tab go to the program on Windows and Linux; Ctrl W closes a tab only outside the terminal; Esc never un-zooms from the terminal (Ctrl Shift M does) | core shell keys and vim |
| 32 | Out of scope: Attach (moves out of 0036 to a later item), node shell and debug containers (0037), Settings › Terminal & Shell | W2 draws no Terminal & Shell content |
