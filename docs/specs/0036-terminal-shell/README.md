# 0036 — Terminal and pod shell

Status: draft, HEAD `1c859ae`. **Mutating: needs the user's approval before implementation** (project rule on exec; C3). Lands after 0028, 0016 (private clipboard), 0030 (guarded core, tiers, audit, allow-list), and 0026. Crates: `crates/cluster` (exec transport), `crates/app` (terminal, dock). Wireframes: W8 and W8b (Logs and Shell tabs, never side by side, ≤ 60 % height, zoom), W4 menu "Open shell ▸" and note 2, W4b note 3, keyboard map S.

## Goal

- A terminal built on `oneterm-vt` (git, pinned, `default-features = false`): grid render with kit theme fonts and colours, key encoding, selection and private copy, bracketed paste, resize → remote `SIGWINCH`, 5,000-line scrollback, Find.
- Pod exec over the kube `ws` feature (`AttachedProcess`), TTY mode, shell auto-detection (bash, ash, sh), clean exit and error states, Reconnect.
- Shell tabs in the dock beside Logs tabs, with a container picker for multi-container pods, the S key, menus, palette, and a lifecycle tied to tab close and cluster switch.
- Every open goes through the 0030 privileged gate (read-only lock, SSAR `get` and `create` on `pods/exec`, env tier, confirmation, audit).

## Non-goals

Node shell and debug containers (0037); Attach (ownership moves out of 0036 to a later item); Settings › Terminal & Shell (W2 draws no content); click reporting to programs; IME composition; OSC 52 clipboard access; session recording or saved scrollback; automatic reconnect; port-forward (0035, which reuses the `ws` feature).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Cluster crate: kube `ws`, `pod_shell` stream, `drive`, `argv`, `shell_exit`, `upgrade_error`, `ExecPermit`, `GetPodExec`; fake-stream tests | 1, 2, 3, 4, 13 |
| 2 | `oneterm-vt` dependency, `TerminalSession`, palette, `TerminalElement`, `grid_size`; byte-fixture tests | 1, 2, 4, 5 |
| 3a | `LogDock` → `Dock` rename as its own mechanical commit; then `DockTab`, `ShellTab` wiring, headless render | 1, 4, 8 |
| 3b | Input, paste sanitiser and multi-line dialog, selection, copy, Find; `Terminal` key context and the 0028 changes | 1, 4, 6, 7 |
| 4 | 0030 guarded core, container picker, S and menus, Reconnect, lifecycle and switch confirm, `--screen shell-fixture`, docs, live denied-path check | 1, 3, 9–12 |

## Files

| File | Contents |
|---|---|
| [dependencies.md](dependencies.md) | `oneterm-vt` pin, API used; kube `ws`; exact lock delta |
| [exec-transport.md](exec-transport.md) | cluster API, `ExecPermit`, upgrade errors, drive loop, shell detection, RBAC (both verbs), 0030 guarded core |
| [terminal-view.md](terminal-view.md) | session core, parse budget, event policy, palette, element, input, paste, copy (C1), Find |
| [shell-tab.md](shell-tab.md) | dock and tabs, Shell tab UI, entry points, lifecycle, keys |
| [decisions.md](decisions.md) · [files-to-touch.md](files-to-touch.md) · [test-plan.md](test-plan.md) | decisions; files per step; tests and checks |

## Acceptance criteria

- [ ] 1. Quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`.
- [ ] 2. `Cargo.lock` gains exactly `tokio-tungstenite`, `tungstenite`, `sha1`, `data-encoding`, `oneterm-vt` (git, rev `e2f9c24b…`); `oneterm-vt` has `default-features = false`; no `alacritty_terminal` anywhere.
- [ ] 3. `pod_shell.rs` is the only exec call site and a `pods/exec` row of the 0030 allow-list (the grep expects `access_review.rs`, `object_write.rs`, `pod_shell.rs`). It calls no `spawn`: kube's message-loop task belongs to the `AttachedProcess`, which `drive` owns in its stream state, so dropping the stream aborts it. No kube type in a public signature. Opening goes through 0030 `run_guarded` (confirm tier, one audit line).
- [ ] 4. Every test of the step in [test-plan.md](test-plan.md) exists and passes offline; none needs exec, a cluster, or a real window.
- [ ] 5. Terminal colours come from kit theme tokens only (0003 colour grep clean); the font is the theme mono font.
- [ ] 6. Typing reaches the program (letters, Esc, Tab, Ctrl C, and Ctrl K/N/W on Windows and Linux); 0028 single keys never act while the terminal has focus; Ctrl \`, Ctrl Tab, Ctrl Shift M still work; Ctrl W closes a tab only outside the terminal.
- [ ] 7. C1: copy uses the 0016 private write (no auto-clear); paste drops C0 (except tab, CR, LF) and C1 controls and asks before a multi-line non-bracketed paste; OSC 52 and DECRQCRA are ignored; `ShellInput` and `ShellUpdate` have a manual `Debug` that prints byte counts only; no session byte is logged, traced, or stored.
- [ ] 8. Resizing the dock or window sends one resize per change; the remote sees the new size (`stty size`, allowed-path check).
- [ ] 9. On UAT (1.29.5), both SSAR answers for `get` and `create` on `pods/exec` are recorded; the gate is disabled unless both allow; S, the submenu, and the palette show `Not permitted: get and create pods/exec` when either is denied; **no exec request is ever sent** (trace).
- [ ] 10. Closing a tab ends its session; switching cluster with open shells asks "{N} shells will close" first, then ends them; screen navigation keeps them.
- [ ] 11. ui-verifier: `--screen shell-fixture` matches the W8b shell pane (header `pod · container · shell`, controls, transcript) with no high-severity defect.
- [ ] 12. One allowed-path run on a write-capable cluster before release, including the bulk-output timing ([test-plan.md](test-plan.md) last section).
- [ ] 13. `pod_shell` requires an `ExecPermit`; its only non-test constructor is `AccessReport::exec_permit` (both exec verbs allowed); upgrade refusals 401, 403, 404 map to typed errors with fixed text.

## Open items

1. 0030 is committed as a draft spec; this amendment adds its `run_guarded` / `GuardedIntent` / `audit_entry` changes. Step 4 waits for 0030's code to merge.
2. No write-capable test cluster (risks R2): the allowed path is only fake-tested until one exists.
3. Decided (user): no 30 s auto-clear for terminal copies (decision 20); "{N} shells will close" confirm on cluster switch (decision 26); multi-line paste dialog in every environment, skipped in bracketed mode (decision 18).
4. The 0025 About page has no third-party licence list yet; `oneterm-vt` (Apache-2.0, NOTICE) must be credited somewhere before release.
5. The coder confirms the `scroll_viewport` accessor path in step 2 ([dependencies.md](dependencies.md)).
