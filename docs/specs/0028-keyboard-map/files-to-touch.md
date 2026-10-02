# 0028 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own. All in `crates/app`; `crates/cluster` is untouched.

## Cargo

No change: `KeyBinding`, actions, `Kbd`, `Dialog`, `Notification`, `PopupMenuItem::action`, and `TextSelection` are in gpui-kit 0.7 (gpui-pre 0.3.7, gpui-component 0.7, gpui-base 0.7). `git diff Cargo.lock` stays empty.

## Source

| S | File | Change |
|---|---|---|
| 1 | `src/keymap.rs` (new) | `actions!` (step 1 actions and `FocusQuickFilter` moved here; steps 2b and 3 add theirs with their bindings and sheet rows), `WINDOW`/`WORKSPACE`/`TABLE` constants and the three field predicates, `bind_keys(cx)`, `ShortcutGroup`, `ShortcutRow`, `shortcut_rows()` |
| 1 | `src/keymap_tests.rs` (new) | key resolution tests, `RESERVED_KEYS` |
| 1 | `src/shortcut_sheet.rs` (new) | `open_shortcut_sheet`, `shortcut_sheet` |
| 1 | `src/main.rs` | `mod keymap; mod shortcut_sheet;` (step 2b adds `mod keyboard_navigation;`); drop the `actions!` line; call `keymap::bind_keys` |
| 1 | `src/app_shell.rs` | drop `bind_keys`; register the step-1 handlers (`ShowShortcuts`, `OpenNamespacePicker`, dock actions); later steps add theirs |
| 1 | `src/log_dock.rs` | `key_context("LogDock")` on the root; `toggled_visibility`, `toggled_zoom`, `TabStep`, `step_tab`, the four methods; tests in module |
| 1 | `src/app_shell_tests.rs` | `keymap::bind_keys`; sheet tests |
| 1 | `src/settings_window.rs` (0025) | delete `bind_keys`: its two bindings move to `keymap::bind_keys` with the same contexts; add the Keyboard Shortcuts page at W2 position 4 ([shortcut-sheet.md](shortcut-sheet.md)) |
| 1 | `src/settings_window_tests.rs` (0025) | `pages_follow_w2_order` includes Keyboard Shortcuts |
| 1 | `src/app_shell.rs` / `src/cluster_switcher.rs` (only if 0026 landed first) | move the `secondary-shift-c` and `secondary-1`…`9` bindings into `keymap.rs` ([keymap.md](keymap.md) "Keys of other specs") |
| 2a | `src/drawer.rs` | `DrawerState.is_open` |
| 2a | `src/app_shell.rs` | `row_echo`, `select_table_row`, `drawer_subject`, `set_drawer_open`; `clear_selection` (the old `close_drawer` body) and the new `close_drawer`; `change_selection(None)` closes the drawer; `SelectRow` handlers per [cursor-and-drawer.md](cursor-and-drawer.md); drawer readers use `drawer_subject()`; `apply_selection_sync` and the launch selection go through `select_table_row`; `reveal`, `open_yaml`, and launch drawers open the drawer |
| 2a | `src/table_selection.rs` (+ `table_selection_tests.rs`) | `take_row_echo` |
| 2a | `src/workspace.rs` | `render_drawer` and the selection-bar offset read `drawer_subject()` |
| 2a | `src/screenshot.rs` | the settle input reads `drawer_subject()` |
| 2a | `src/app_shell_tests.rs` | the three 2a tests |
| 2b | `src/keyboard_navigation.rs` (new) + `keyboard_navigation_tests.rs`; `src/main.rs` `mod keyboard_navigation;` | `impl AppShell` key handlers (rows, `OpenDrawer`, `Dismiss`, `LeaveInput`, containers), `register_key_handlers(element, cx)`, `RowStep`, `step_row`, `DismissState`, `DismissStep`, `dismiss_step`, `ContainerStep`, `step_container` |
| 2b | `src/drawer.rs` | `key_context("Drawer")` on `drawer_frame` |
| 2b | `src/filter_bar.rs` | `key_context("QuickFilter")` around the `/` input |
| 2b | `src/app_shell.rs` | register the 2b handlers; plain Enter in the quick filter focuses the table |
| 2b | `src/pod_drawer.rs` | `container_display_order`, used by `container_list`; test in module |
| 3 | `src/resource_actions.rs` (+ `resource_actions_tests.rs`) | five `ResourceAction` variants, the `CopyName \| ViewYaml` Enabled arm, `action_label`, `KeyAvailability`, `key_availability`, `RowKeyNotice`; `.action(...)` hints on menu items; `kind_menu` iterates `KindAction`s |
| 3 | `src/resource_kind.rs` | `KindAction`; `read_only_actions` typed; update `deployments_offer_port_forward_and_four_read_only_actions`; new test in module |
| 3 | `src/keyboard_navigation.rs` | row-action handlers: subject → `key_availability` → run, notice, or nothing; `CopyName` text-selection check |
| 4 | `src/launch_options.rs` (+ tests) | `--screen shortcuts`, `--screen pods-cursor`; `USAGE` |
| 4 | `src/app_shell.rs` | open the sheet / leave the drawer closed for those screens, once the list has loaded |

## Docs (with the step that ships them)

| S | File | Change |
|---|---|---|
| 4 | `docs/roadmap/inventory-shell.md` | P2 → Done (0028); D6 → Done; S5 notes the Keyboard Shortcuts page as Done |
| 4 | `docs/roadmap/inventory-screens.md` | W4-7 `[` `]` → Done |
| 4 | `docs/roadmap/inventory-kinds.md` | "the Y key → 0028" → Done |
| 4 | `docs/roadmap/gap-plan-local-and-mutating.md` (the 0028 entry, line 24) | match what shipped: Ctrl , moved from 0025; `:` deferred to 0029; letters S, F, C, D, E, R, ⇧S, Del gated (no A) |
| 4 | `docs/roadmap/README.md` | status row "Keyboard map, command palette": Partial (0028 done, 0029 next) |
