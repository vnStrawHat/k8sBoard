# 0028 · Test plan

[Back to index](README.md). **S** is the step that adds the test; the sheet and reserved-key tests from step 1 keep passing as later steps add bindings and rows. Offline and deterministic, one behavior per test. No test opens a real window: keymap tests use `TestAppContext` only to run `gpui_kit::init` and `keymap::bind_keys`, then query `cx.key_bindings()`; the few dispatch tests use the existing headless test window of `app_shell_tests.rs`.

## Keymap resolution (`keymap_tests.rs`)

Helper: `resolve(cx, "down", &["Root", "AppShell", "DataTable"]) -> Option<&'static str>` = name of the first binding from `Keymap::bindings_for_input(&[Keystroke::parse(..)], &stack)` (stack built with `KeyContext::parse`). Typed shifted characters use a `Keystroke` with `key_char` set, as the platform sends them.

| S | Test | Checks |
|---|---|---|
| 3 | `letters_resolve_in_the_workspace` | `l` in `Root > AppShell` → `ViewLogs`; same with `DataTable` below |
| 1 | `letters_do_nothing_in_text_inputs` | `?`, `/` under `Input` (step 2b adds `j`, `[`, `enter`; step 3 adds `l`) → no app action |
| 1 | `single_keys_do_nothing_in_menus_popovers_and_dialogs` | `?`, `/` (later steps add `j`, `y`) under `PopupMenu`, `Popover`, `Dialog` → no app action. Popover paths are `Root > AppShell > Popover` (the namespace picker), never one with `ClusterSwitcher` |
| 2b | `table_arrows_outrank_the_kit_table` | `down`, `up`, `home`, `end`, `pageup`, `pagedown` under `DataTable` → app row actions first |
| 2b | `table_escape_outranks_the_kit_table` | `escape` under `DataTable` → `Dismiss` |
| 2b | `escape_leaves_the_quick_filter` | `escape` under `QuickFilter > Input` → `LeaveInput` (also `Drawer`, `LogDock`) |
| 2b | `escape_stays_with_other_inputs` | `escape`, `up`, `down`, `enter` under `Root > AppShell > Popover > Input` (the namespace picker search; **no** `ClusterSwitcher` in the path) → the kit's actions, never an app action. Under `Popover > ClusterSwitcher > Input` these keys belong to 0026 (its `escape_in_switcher_filter_closes`, `arrows_move_highlight_in_filter`, `enter_in_filter_switches_to_highlight`) |
| 1 | `chords_work_inside_text_inputs` | `secondary-n`, `secondary-w`, ``ctrl-` ``, `ctrl-tab` under `Input` → app actions |
| 3 | `copy_name_is_not_bound_inside_inputs` | `secondary-c` under `Input` → the kit `Copy` |
| 1 | `question_mark_matches_a_shifted_slash` | typed `/`+shift with `key_char "?"` → `ShowShortcuts`, never `FocusQuickFilter` |
| 1 | `secondary_is_the_platform_modifier` | `Keystroke::parse("secondary-n")` has `platform` on macOS, `control` elsewhere (`cfg!` in the assertion, runs on every OS) |
| 1 | `bindings_never_share_a_keystroke_in_one_context` | no two app bindings with equal parsed keystrokes (`KeyBinding::keystrokes()`, compared as `Keystroke`, so `secondary-n` and `ctrl-n` collide off macOS) and an equal predicate |
| 1 | `bindings_avoid_reserved_keys` | each `RESERVED_KEYS` entry is parsed with `Keystroke::parse`; no app binding's keystroke equals one ([keymap.md](keymap.md) "Keys of other specs"). `RESERVED_KEYS` holds `space`, `secondary-k`, `:`, `secondary-enter`, `secondary-shift-r`, `secondary-s`; not `secondary-shift-c` or `secondary-1`…`9` (bound by 0026) |
| 1 | `every_sheet_row_has_a_binding` | each `shortcut_rows()` action has at least one binding |
| 1 | `every_bound_action_is_on_the_sheet` | each app binding's action is on the sheet, except `LeaveInput`, `SwitchToCluster2`…`9`, and the 0026 switcher-local actions |
| 1 | `cluster_switcher_chords_resolve_everywhere` | when 0026 code exists: `secondary-shift-c` → `OpenClusterSwitcher`, `secondary-1` → `SwitchToCluster1` under `AppShell`, `AppShell > Input`, and `AppShell > Popover > ClusterSwitcher > Input` |
| 1 | `settings_window_gets_no_shell_keys` | under `SettingsWindow` and `SettingsWindow > Input`: `j`, `?`, `/`, `enter`, `l`, `secondary-n`, `secondary-w` resolve to no app action; `escape` under `SettingsWindow > Input` and under `Dialog > Input` never resolves to `LeaveInput` |
| 1 | `settings_keys_keep_their_0025_contexts` | `secondary-,` → `OpenSettings` with no context (also under `Dialog`); `secondary-o` → `ImportKubeconfig` only under `SettingsWindow` |

## Pure helpers

| S | File | Test | Checks |
|---|---|---|---|
| 1 | `log_dock.rs` | `toggled_visibility_minimizes_then_restores` | Normal → Minimized → Normal; Zoomed → Minimized |
| 1 | `log_dock.rs` | `toggled_zoom_zooms_then_restores` | Normal, Minimized → Zoomed → Normal |
| 1 | `log_dock.rs` | `step_tab_wraps_both_ways` | 0 Previous of 3 → 2; 2 Next → 0 |
| 2b | `keyboard_navigation_tests.rs` | `step_row_moves_one_row` | Next 3 → 4; Previous 3 → 2 |
| 2b | same | `step_row_wraps_next_and_previous` | Next at the last row → 0; Previous at 0 → last (kit `loop_selection: true`, decision 15) |
| 2b | same | `step_row_starts_at_the_top_without_a_cursor` | `None` + Next/Previous/First/pages → 0; Last → n-1 |
| 2b | same | `step_row_pages_and_ends_clamp` | page 10: NextPage from 5 of 12 → 11; PreviousPage from 5 → 0; First, Last |
| 2b | same | `step_row_has_no_target_in_an_empty_table` | `row_count 0` → `None` |
| 2b | same | `step_row_treats_a_stale_cursor_as_the_last_row` | `current 9`, `row_count 3`: Previous → 1, NextPage → 2 |
| 2b | same | `dismiss_step_follows_the_ladder` | zoomed dock first, then open drawer, then selection, else `Propagate` (one case per rung) |
| 2a | `table_selection_tests.rs` | `take_row_echo_consumes_only_its_row` | `Some(3)` vs 3 → true and cleared; vs 4 → false and cleared; `None` → false |
| 2b | `pod_drawer.rs` | `container_display_order_groups_init_sidecar_main` | stable inside groups |
| 2b | `keyboard_navigation_tests.rs` | `step_container_clamps_in_display_order` | Next from last stays; `None` → first |
| 3 | `resource_actions_tests.rs` | `key_availability_offers_logs_only_for_pods` | pod `Run`; node, Service `NotOffered` |
| 3 | same | `key_availability_disables_mutating_keys_with_the_read_only_reason` | E, Del, R, ⇧S, C, D → `Disabled { "Read-only mode" }` where offered |
| 3 | same | `key_availability_uses_the_access_gate` | S on a pod with `CreatePodExec` denied → "Not permitted: …"; while checking → "Checking permissions…" |
| 3 | same | `key_availability_offers_restart_and_scale_from_kind_actions` | Deployments both, DaemonSets restart only, Services neither |
| 3 | same | `view_yaml_and_copy_name_always_run` | pod, node, kind row |
| 3 | `resource_kind.rs` | `kind_actions_name_their_key_action` | `Scale…` → `Scale`, `Restart rollout` → `RestartRollout`, ConfigMaps `Edit` → `EditYaml` |
| 4 | `launch_options_tests.rs` | `parses_shortcuts_and_pods_cursor_screens` | both map to Pods with the right flags |

## Headless dispatch (`app_shell_tests.rs`, no session, no network)

| S | Test | Checks |
|---|---|---|
| 1 | `question_mark_opens_the_shortcut_sheet` | `press("?")` → `window.has_active_dialog(cx)` |
| 1 | `escape_closes_the_shortcut_sheet` | then `press("escape")` → no dialog; the shell root keeps focus |
| 1 | existing `slash_is_available_before_any_click`, `focus_returns_to_the_shell_…` | still pass with the binding moved to `keymap.rs` |
| 1 | `settings_window_tests::pages_follow_w2_order` (0025, extended) | Clusters, Appearance, Keyboard Shortcuts, About |
| 2a | `closed_drawer_has_no_subject` | set `selected` to a pod key with `drawer.is_open = false` → `drawer_subject()` is `None`; with `true` → the key |
| 2a | `closing_drawer_drops_pending_subjects` | pod key selected, drawer open, `follow_drawer_subjects` → `pending_subjects` is `Some` (no session is needed: the debounce starts before any watch); `close_drawer` → `None` |
| 2a | `clearing_the_selection_closes_the_drawer` | `change_selection(None)` → `is_open == false` |

## Live checks (UAT, read-only) and ui-verifier

- Manual, release build, on Pods, Nodes, Deployments: J/K/↑/↓ move without a drawer; ⏎ opens; ↑↓ retarget it; Esc closes then clears; Y opens YAML; L opens a pod log tab; S, E, Del show the read-only notice; typing `j` in `/` filters; Esc returns to the table; Ctrl N opens the picker; Ctrl \`, Ctrl Shift M, Ctrl Tab, Ctrl W on two log tabs.
- Trace check: a closed drawer starts no object-events watch while J/K run (AC 7).
- ui-verifier, step 2a: rerun every existing `--screen *-drawer` shot (pod, node, kind drawers on each tab). They must match the last accepted shots, because launch drawers open the drawer.
- ui-verifier, step 4: `--screen shortcuts` and `--screen pods-cursor` against the wireframe key grid and W4, plus pod and node context-menu screenshots for the key hints if they can be reached. Known deviation: decision 14 (a row click while the drawer is open switches the subject), confirmed by the user.
