# 0036 — Terminal and pod shell

Status: steps 1 to 4 built (lane W2, branch `spec-0036-3`); the allowed path awaits a write-capable cluster. **Refreshed 2026-10-03 against main `2c7dc08`**. **Mutating (project rule on exec). C3: Approved by the user on 2026-10-02 (one approval for all mutating specs).** Debug builds still refuse exec unless `K8SBOARD_ALLOW_WRITES=1` (agents never set it); UAT checks stay denied-path-only. The new dependencies (C6, [dependencies.md](dependencies.md)) are a separate approval. Prerequisites: merged 0016, 0026, 0027, 0028, 0029, 0030 steps 1/2a/3 (`WritePolicy`, `ActionGate`, `ClusterGuard`, `audit_log.rs`). Steps 1–2 run now (crate and new files only); step 3a after 0030 steps 2b + 4 merge (they edit `app_shell.rs`, `keymap.rs`); step 4 also needs 0032 2a-i (`RowAction`). Lane W2, first spec. Crates: `crates/cluster` (exec transport), `crates/app` (terminal, dock). Wireframes: W8 and W8b (Logs and Shell tabs, never side by side, ≤ 60 % height, zoom), W4 menu "Open shell ▸" and note 2, W4b note 3, keyboard map S.

## Goal

- A terminal built on `oneterm-vt` (git, pinned, `default-features = false`): grid render with kit theme fonts and colours, key encoding, selection and private copy, bracketed paste, resize → remote `SIGWINCH`, 5,000-line scrollback, Find.
- Pod exec over the kube `ws` feature (`AttachedProcess`), TTY mode, shell auto-detection (bash, ash, sh), clean exit and error states, Reconnect.
- Shell tabs in the dock beside Logs tabs, with a container picker for multi-container pods, the S key, menus, palette, and a lifecycle tied to tab close and cluster switch.
- Every open goes through the 0030 privileged gate (read-only lock, SSAR `get` and `create` on `pods/exec`, env tier, confirmation, audit).

## Non-goals

Node shell and debug containers (0037); Attach (0040); Settings › Terminal & Shell (W2 draws no content); click reporting to programs; IME composition; OSC 52 clipboard access; session recording or saved scrollback; automatic reconnect; port-forward (0035, which reuses the `ws` feature).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Cluster crate: kube `ws`, `pod_shell` stream, `drive`, `argv`, `shell_exit`, `upgrade_error`, `ExecPermit`, `GetPodExec`; fake-stream tests | 1, 2, 3, 4, 13 |
| 2 | `oneterm-vt` dependency, `TerminalSession`, palette, `TerminalElement`, `grid_size`; byte-fixture tests | 1, 2, 4, 5 |
| 3a | `LogDock` → `Dock` rename as its own mechanical commit, **merged to main at once** (about 20 files; 0034 and 0037 build on `Dock`); then `DockTab` (with `cluster()` for the merged `close_tabs_of`), `ShellTab` wiring, headless render | 1, 4, 8 |
| 3b | Input, paste sanitiser and multi-line dialog, selection, copy, Find; `Terminal` key context and the 0028 changes | 1, 4, 6, 7 |
| 4 | 0030 guarded core, multi-check gate (`ActionGate::Mutating { checks }`), the `OpenShell` arm, container picker, menus, dock "Shell into selected", Reconnect, lifecycle and the `leaving_work` release confirm, `--screen shell-fixture`, docs, live denied-path check | 1, 3, 9–12 |

## Files

| File | Contents |
|---|---|
| [dependencies.md](dependencies.md) | `oneterm-vt` pin, API used; kube `ws`; exact lock delta |
| [exec-transport.md](exec-transport.md) | cluster API, `ExecPermit`, upgrade errors, drive loop, shell detection, RBAC (both verbs), 0030 guarded core |
| [terminal-view.md](terminal-view.md) | session core, parse budget, event policy, palette, element, input, paste, copy (C1), Find |
| [shell-tab.md](shell-tab.md) | dock and tabs, Shell tab UI, entry points, lifecycle, keys |
| [decisions.md](decisions.md) · [files-to-touch.md](files-to-touch.md) · [test-plan.md](test-plan.md) | decisions; files per step; tests and checks |

## Acceptance criteria

- [x] 1. Quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`.
- [x] 2. `Cargo.lock` gains exactly `tokio-tungstenite`, `tungstenite`, `sha1`, `data-encoding`, `oneterm-vt` (git, rev `e2f9c24b…`); `oneterm-vt` has `default-features = false`; no `alacritty_terminal` anywhere.
- [x] 3. `pod_shell.rs` is the only exec call site and a `pods/exec` row of the 0030 allow-list (the grep expects `access_review.rs`, `object_write.rs`, `pod_shell.rs`). It calls no `spawn`: kube's message-loop task belongs to the `AttachedProcess`, which `drive` owns in its stream state, so dropping the stream aborts it. No kube type in a public signature. Opening goes through 0030 `run_guarded` (confirm tier, one audit line).
- [x] 4. Every test of the step in [test-plan.md](test-plan.md) exists and passes offline; none needs exec, a cluster, or a real window.
- [x] 5. Terminal colours come from kit theme tokens only (0003 colour grep clean); the font is the theme mono font.
- [x] 6. Typing reaches the program (letters, Esc, Tab, Ctrl C, and Ctrl K/N/W on Windows and Linux); 0028 single keys never act while the terminal has focus; Ctrl \`, Ctrl Tab, Ctrl Shift M still work; Ctrl W closes a tab only outside the terminal.
- [x] 7. C1: copy uses the 0016 private write (no auto-clear); paste drops C0 (except tab, CR, LF) and C1 controls and asks before a multi-line non-bracketed paste; OSC 52 and DECRQCRA are ignored; `ShellInput` and `ShellUpdate` have a manual `Debug` that prints byte counts only; no session byte is logged, traced, or stored.
- [ ] 8. Resizing the dock or window sends one resize per change (done, tested); the remote sees the new size (`stty size`): open until an allowed-path run.
- [x] 9. On UAT (1.29.5), both SSAR answers for `get` and `create` on `pods/exec` are recorded; the gate is disabled unless both allow; S, the submenu, and the palette show `Not permitted: get and create pods/exec` when either is denied; **no exec request is ever sent** (trace).
- [x] 10. Closing a tab ends its session. A switch, view change, or `Remove from view` that releases clusters with open shells asks "{N} shells will close" first (`leaving_work`, before `release_all` / `release_slot`), then ends only those shells; screen navigation keeps them.
- [ ] 11. ui-verifier (screenshots v73 taken and read by the coder; the ui-verifier pass is pending): `--screen shell-fixture` matches the W8b shell pane (header `pod · container · shell`, controls, transcript) with no high-severity defect.
- [ ] 12. One allowed-path run on a write-capable cluster before release, including the bulk-output timing ([test-plan.md](test-plan.md) last section).
- [x] 13. `pod_shell` requires an `ExecPermit`; its only non-test constructor is `AccessReport::exec_permit` (both exec verbs allowed); upgrade refusals 401, 403, 404 map to typed errors with fixed text.

## As built (steps 3a to 4)

- The guarded entry is `AppShell::start_connect` with `ConnectIntent` in `write_flow.rs`; 0030 as built kept one `WriteIntent`, so there is no `run_guarded` or `GuardedKind` yet. `ConnectIntent.open` is `Rc<dyn Fn(&mut AppShell, ExecPermit, ClusterConnection, ..)>`: the permit and the connection are read from the pod's own slot at confirm time, and the closure may update the shell. `DryRunState::NotSupported` is the dialog line `Dry-run not supported for this action`; the audit-note checkbox is for writes only.
- The audit line is written by `shell_open.rs` from the tab's `ShellEvent` (`Opened` gives `applied`, `OpenFailed` gives `failed` with the error), once per start; a reconnect writes its own line. A debug trace in `AccessState::from_review` records both exec answers.
- `leaving_work` is `leaving_work.rs`: `LeavingWork { shells }` plus `confirm_leaving` (no `ReleaseCheck`). It asks in `switch_cluster`, `view_clusters` (multi), and `remove_from_view`, and runs the release on Continue.
- Find lists matches newest first: `1 of N` is the bottom-most, Enter goes to the next older match, Shift Enter back, both wrap. The wheel gives arrow keys on the alternate screen with alternate scroll, and mouse reports when the program asked for them. A paste over 128 KiB is refused whole. Tests copy to the app test clipboard (`copy_private`), never the real one.
- Extra screenshot-only screens: `shell-dock-fixture`, `shell-paste-fixture`, `shell-picker-fixture` (the container list in a dialog, since a submenu cannot be held open by a flag). The Terminal group is on the shortcut sheet; `cmd-c`, `cmd-v`, `cmd-f` are bound on macOS only. About lists `oneterm-vt`.
- Review fixes: the two alert dialogs (`leaving_work`, the multi-line paste) use `fresh_enter.rs`, which binds `enter` to `NoAction` in its own key context and confirms on a fresh Enter only, like the write confirm dialog. On Windows, AltGr (reported as Ctrl+Alt) with a printable `key_char` sends that text, so `@ \ { } [ ] | ~` work on German, French, and Polish layouts (`CtrlAltMeans`; only the decision input is platform-gated). A start that never reports (tab closed, app quit, Reconnect replacing it) writes an `abandoned` audit line (`ShellStarts` in `shell_open.rs`; the quit path appends synchronously). Ctrl Shift R is `NoAction` in the terminal. The tab label is `shell · m8n2p/api` (`short_pod_name`). Find searches again after a resize, because a reflow gives rows new identities. `PasteDecision` and `PasteAsk` print sizes only in `Debug`.
- The shell fixture screens (`shell-fixture`, `-find-`, `-paste-`, `-picker-`, `-confirm-`) draw from fixed data and wait for no cluster; `shell-dock-fixture` still opens a Logs tab when a cluster is live.
- `Debug container…` and the 0028 note (`WORKSPACE` gains `!Terminal`, context `Dock`, A and Attach no longer owned) are for the orchestrator.

## Open items

1. 0030 steps 1/2a/3 are merged; `run_guarded`, `GuardedIntent`, `GuardedKind::Connect`, and `audit_entry` come with 0030 step 4 (in flight). Step 4 follows the merged signatures; where they differ from 0030 write-flow.md, follow the code and note the deviation.
2. No write-capable test cluster (risks R2): the allowed path is only fake-tested until one exists.
3. Decided (user): no 30 s auto-clear for terminal copies (decision 20); "{N} shells will close" confirm on cluster switch (decision 26); multi-line paste dialog in every environment, skipped in bracketed mode (decision 18).
4. The 0025 About page has no third-party licence list yet; `oneterm-vt` (Apache-2.0, NOTICE) must be credited somewhere before release.
5. Done in step 2: the `scroll_viewport` accessor path is `Terminal::grid_mut().screen_mut()` ([dependencies.md](dependencies.md)).
