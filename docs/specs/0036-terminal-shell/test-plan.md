# 0036 · Test plan

[Back to index](README.md). **S** is the step. Offline and deterministic; no test needs exec, a cluster, or a real window. The transport is tested with `tokio::io::duplex` and futures channels; the terminal with fixed byte fixtures; the view with the headless `TestAppContext` window of `app_shell_tests.rs`.

## Transport and RBAC (`crates/cluster`, paused tokio time)

| S | File | Test | Checks |
|---|---|---|---|
| 1 | `pod_shell_tests.rs` | `argv_auto_loops_bash_ash_sh_and_reports_the_pick` | the exact script with the OSC 7770 `printf`; `Bash` → `bash`; `Sh` → `sh` |
| 1 | same | `drive_forwards_stdout_as_output` | one `Output` per read, each ≤ 64 KiB |
| 1 | same | `drive_writes_input_bytes_to_stdin` | `Bytes(b"ls\r")` reaches the fake stdin unchanged |
| 1 | same | `drive_forwards_resize_latest_wins` | three resizes into a full sink → the last one delivered |
| 1 | same | `drive_ends_with_exited_after_stdout_eof` | EOF + `NonZeroExitCode` code 2 → `Exited { code: Some(2) }`, then the end |
| 1 | same | `dropping_the_stream_drops_the_owner` | a drop-flag guard passed as `owner` is dropped with the stream |
| 1 | same | `shell_exit_reads_success_failure_and_unknown` | `Success` → 0; `NonZeroExitCode` → cause; `None` → `code: None` |
| 1 | same | `upgrade_forbidden_maps_to_fixed_text` | `kube::Error::UpgradeConnection(UpgradeConnectionError::ProtocolSwitch(StatusCode::FORBIDDEN))` → `Forbidden` with the fixed message |
| 1 | same | `upgrade_unauthorized_and_not_found_map_to_typed_errors` | 401 → `Unauthorized`; 404 → `Api { code: 404 }` |
| 1 | same | `shell_debug_prints_byte_counts_only` | `format!("{:?}", ShellInput::Bytes(b"secret".to_vec()))` = `Bytes(6 bytes)`; same for `Output` |
| 1 | `access_review.rs` | `exec_permit_needs_get_and_create` | both allowed → `Some`; either denied or missing → `None` |
| 1 | `pod_shell_tests.rs` | `blocked_policy_sends_no_exec` | `WritePolicy::Blocked` → one `Failed` with the `WritesBlocked` text, then the stream ends; the `FakeApi` records zero requests |

## Terminal core (`terminal_session_tests.rs`, `terminal_input.rs`, `terminal_element.rs`)

| S | Test | Checks |
|---|---|---|
| 2 | `fed_text_appears_in_the_snapshot` | `hello\r\nworld` → rows read `hello`, `world` |
| 2 | `sgr_colors_map_to_theme_tokens` | `\x1b[31mred` → fg = the test theme's `red` |
| 2 | `device_attributes_query_is_answered_through_the_outbox` | `\x1b[c` → non-empty `take_outbox` |
| 2 | `decrqcra_is_not_answered` | `\x1b[1;1;1;1;2;2*y` → empty outbox |
| 2 | `color_query_is_answered_with_its_terminator` | `\x1b]11;?\x07` → reply ends with BEL; `\x1b]11;?\x1b\\` → reply ends with ST; colour = theme background |
| 2 | `osc52_store_and_load_are_ignored` | no outbox bytes, no clipboard access |
| 2 | `osc_7770_sets_the_resolved_shell` | `\x1b]7770;bash\x07` → `bash`; `\x1b]7770;evil\x07` → unchanged `Auto` |
| 2 | `note_strips_control_characters` | `note("a\x1bb")` renders `ab` dimmed |
| 2 | `scrollback_is_capped` | 6,000 lines → history 5,000 |
| 2 | `resize_reports_only_real_changes` | same size → `false`; new → `true` |
| 2 | `hostile_stream_never_panics` | 1 MiB of seeded random bytes in 4 KiB chunks |
| 2 | `grid_size_floors_and_keeps_a_minimum` | 805 × 410 px, 8 × 17 cells → 100 × 24; tiny → 2 × 1 |
| 2 | `terminal_palette_uses_theme_tokens_only` | 16 ANSI entries equal the mapped theme colours |
| 3b | `key_to_vt_maps_named_keys` | enter, tab, backspace, arrows, f5 |
| 3b | `key_to_vt_sends_ctrl_chords_and_drops_platform_chords` | ctrl-c → `0x03`; cmd-k → `None` |
| 3b | `sanitize_paste_strips_c0_and_c1_controls` | `a\x03b\u{9b}c\nd`, not bracketed → `abc\rd` |
| 3b | `sanitize_paste_keeps_newlines_when_bracketed` | `a\r\nb` bracketed → `\x1b[200~a\r\nb\x1b[201~` |
| 3b | `multi_line_paste_asks_first_unless_bracketed` | `ls\n` → no dialog; `a\nb` → 2 lines; bracketed → no dialog |
| 3b | `find_reports_match_count_and_order` | three `api` hits; next/previous newest first |

## Headless view (`shell_tab_tests.rs`, `app_shell_tests.rs`; no session)

| S | Test | Checks |
|---|---|---|
| 3a | `shell_tab_renders_fed_bytes` | a `ShellTab` on a fake target renders a frame; snapshot text = fixture; measured bounds give the expected `GridSize` |
| 3a | `resizing_the_window_queues_one_resize` | two frames at a new size → one `Resize` on the fake input receiver |
| 3a | `header_shows_auto_until_started` | `Auto`, then `bash` after the OSC 7770 fixture |
| 3b | `typing_sends_encoded_bytes` | `press("l")`, `press("ctrl-c")` → `l`, `0x03` |
| 3b | `ended_session_ignores_input` | after `Exited`, `press("a")` sends nothing |
| 4 | `switch_with_open_shells_asks_first` | two Shell tabs + `switch_cluster` → dialog "2 shells will close"; Esc keeps both tabs |

## Keys (`keymap_tests.rs`, extended)

| S | Test | Checks |
|---|---|---|
| 3b | `single_keys_do_nothing_in_the_terminal` | `j`, `y`, `?`, `escape` under `Dock > Terminal` → no app action |
| 3b | `shell_keys_reach_the_program_on_windows_and_linux` | `ctrl-k`, `ctrl-n`, `ctrl-w`, `ctrl-c`, `tab` under `Terminal` → `NoAction` first |
| 3b | `terminal_copy_and_paste_chords_resolve` | `ctrl-shift-c` → `TerminalCopy`; `cmd-v` → `TerminalPaste` |
| 3b | `dock_chords_still_work_in_the_terminal` | ``ctrl-` ``, `ctrl-tab`, `secondary-shift-m` → 0028 dock actions |
| 3b | `escape_in_find_closes_find_not_leave_input` | `escape` under `Dock > ShellFind > Input` → `CloseTerminalFind`, not `LeaveInput` |

## Gating and dock (step 4)

| Test | Checks |
|---|---|
| `open_shell_needs_get_and_create` | both allowed → Enabled; one denied → `Not permitted: get and create pods/exec`; checking or unknown → disabled |
| `open_shell_follows_the_0030_gate` | locked → `{cluster} is read-only`; `confirm_step` gets `Change` |
| `start_connect_never_opens_when_blocked` | locked at confirm time, dialog cancelled, or no permit → `open` not called, no audit line |
| `start_connect_audits_one_line_without_bytes` | 0030 `AuditEntry` keys; fields `container`, `command` only |
| `container_picker_lists_running_main_and_sidecars` | init left out; terminated disabled; one container → no submenu |
| `ninth_shell_tab_is_refused` | notice, no new tab |
| `close_all_ends_shell_sessions` | the fake input sender reports closed after `close_all` |

## Live checks and ui-verifier

- UAT (1.29.5, read-only): record both SSAR answers (`get` and `create` on `pods/exec`) from the session report trace; S, the submenu, and the palette show the gate text; **no exec request is ever sent** (trace), whatever the answers.
- `--screen shell-fixture` (debug, `screenshot` feature): Pods with a zoomed dock and a Shell tab fed the W8b transcript from a `const` in `screenshot.rs`; ui-verifier compares with W8b. Debug builds parse 10–30× slower; bulk-output smoothness is not judged there.
- Write-capable cluster (risks R2), release build, before release: bash and sh images, `stty size` after a resize, vim, Ctrl C, paste (single and multi-line), copy, Reconnect, pod deletion, and a bulk-output timing (`cat` of a 50 MB file: trace feed time per update, budget about 1 ms).
