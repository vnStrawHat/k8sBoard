# 0028 — Keyboard map

Status: implemented (steps 1–4). Open checks: AC 7 trace, 9, 10, 12, 13 need a live UAT run or ui-verifier. Amended after advisor review. HEAD `9d5af01`. Lands after 0025 and moves 0025's two key bindings. Crate: `crates/app` only. Local only: no cluster call is added and no file is written.

Wireframes: the keyboard map grid (22 keys, section `#phim`), the anatomy notes on the Drawer and the Dock, the W4/W5/W7 menu `kbd` hints, W4b note 2 (`[` `]`), the W8/W8b notes, and the W2 nav item "Keyboard Shortcuts". Roadmap: inventory P2, D6, W4-7.

## Goal

- One key map in `keymap.rs`, with named contexts:
  - chords work anywhere in the shell;
  - single keys work only outside text fields, menus, popovers, and dialogs.
- Keyboard-first tables:
  - J/K and the arrows move a row cursor;
  - ⏎ opens the drawer, and ↑↓ then retarget it;
  - Esc closes the drawer, then clears the cursor;
  - `[` `]` switch containers.
- Single-letter row actions on the cursor row: L, Y, Ctrl C, S, F, C, D, E, R, ⇧S, Del. They use the same availability checks as the menus, and the menus show the keys.
- Dock keys: Ctrl \`, Ctrl Shift M, Ctrl Tab, Ctrl W. Ctrl N opens the namespace picker.
- A `?` shortcut sheet built from the live bindings, also shown as Settings › Keyboard Shortcuts.
- Platform mapping (⌘ on macOS). Ctrl , and Ctrl O move here from 0025. Ctrl Shift C and Ctrl 1–9 are bound by 0026 (held in `keymap.rs`). The keys of 0027, 0029, 0030, and 0031 are reserved.

## Non-goals

- User rebinding, and any settings key for it (decision 25).
- Keys owned by other specs: Ctrl K and `:` (0029); Space (0027); Ctrl Shift R (0030). Ctrl Shift C and Ctrl 1–9 are 0026 behavior; 0028 only holds their bindings.
- The A key and the Attach item (a later item, no longer owned by 0036), and any enabled mutating action (0031–0036).
- Menu accelerators inside an open menu.
- Region focus cycling.
- A back key (0020 open item 3).
- New menu items, such as "Edit YAML" on every kind.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | `keymap.rs`: contexts and bindings; `/` moved; 0025 keys moved; `?` sheet and the Settings page; Ctrl N; dock keys; reserved-key and resolution tests | 1–4, 8, 10, 11 |
| 2a | Drawer split only, no new keys: `is_open`, `drawer_subject`, reader migration, `row_echo`, a click opens the drawer, the ✕/`clear_selection` rename, launch/reveal/`open_yaml` open the drawer; ui-verifier reruns the `--screen *-drawer` shots | 1, 2, 7, 14 |
| 2b | Row moves, ⏎, the Esc ladder, `LeaveInput`, `[` `]` | 1, 2, 5, 6 |
| 3 | Row actions, availability, notices, `KindAction`, menu hints | 1, 2, 9, 12 |
| 4 | `--screen shortcuts` and `--screen pods-cursor`, roadmap docs, ui-verifier run | 1, 2, 13 |

## Files

| File | Contents |
|---|---|
| [keymap.md](keymap.md) | bound keys, keys moved from 0025, keys of other specs, platform mapping |
| [contexts-and-focus.md](contexts-and-focus.md) | GPUI precedence, contexts, text-input rules, Esc ladder, focus |
| [cursor-and-drawer.md](cursor-and-drawer.md) | step 2a drawer split and `row_echo`; step 2b row steps (wrap) and containers |
| [row-actions.md](row-actions.md) | letter → `ResourceAction`, `key_availability`, notices, `KindAction`, menu hints |
| [shortcut-sheet.md](shortcut-sheet.md) | `?` dialog, Settings page, dock keys, launch screens, room for 0029 |
| [decisions.md](decisions.md) · [files-to-touch.md](files-to-touch.md) · [test-plan.md](test-plan.md) | decisions; files per step; tests and checks |

## Acceptance criteria

- [x] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`. `Cargo.lock` is unchanged.
- [x] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline. No test opens a real window.
- [x] 3. Every binding in [keymap.md](keymap.md) exists with that string and context, and all bindings live in `keymap.rs`. No still-reserved key is bound (compared as parsed keystrokes).
- [x] 4. Single keys never act while a text field, menu, popover, or dialog has focus. Chords act anywhere in the shell tree (tests).
- [x] 5. J/K/↑/↓ move the cursor and wrap at the ends; Home/End/PgUp/PgDn clamp. None of them opens the drawer. ⏎ opens it, and with it open, the moves retarget it.
- [x] 6. Esc closes a zoomed dock, else an open drawer (the row is kept), else clears the cursor. Esc in the `/` filter, the dock filter, or the YAML view returns focus to the table.
- [ ] 7. A closed drawer runs no object-events watch, related watch, YAML GET, kubelet demand, or pending debounce (tests plus a trace while pressing J).
- [x] 8. `?` opens the sheet with keys from the keymap, formatted per OS by `Kbd`, and Esc closes it. Settings › Keyboard Shortcuts shows the same grid at W2 position 4.
- [ ] 9. On UAT, S, F, E, Del, R, ⇧S, C, and D show "… is unavailable: <reason>". L opens a pod log tab, and Y opens the YAML tab. Ctrl C copies the name, or the selected text when there is any. There is no mutating API call (0001 grep clean).
- [ ] 10. Ctrl \`, Ctrl Shift M, Ctrl Tab, Ctrl Shift Tab, and Ctrl W act on the dock and do nothing without tabs. Ctrl N opens the title-bar namespace picker. Ctrl , and Ctrl O behave as in 0025.
- [x] 11. Bindings use `secondary`, except Ctrl \` and Ctrl Tab (literal `ctrl`). No binding uses the Win or Super key.
- [ ] 12. Pod, node, and kind menus show the key next to: View logs, Open shell, Port-forward, View YAML, Cordon, Drain…, Copy name, Scale…, Restart rollout, Edit, and Delete.
- [ ] 13. ui-verifier: `--screen shortcuts` and `--screen pods-cursor` match the wireframe grid and W4, with no high-severity defect. Decision 14 is a known deviation.
- [x] 14. A row click opens the drawer, even on the already-selected row. A snapshot reorder never reopens a closed drawer. The existing `--screen *-drawer` shots are unchanged.

## Open items

1. Resolved: 0025 lands first with Ctrl , and Ctrl O in `settings_window::bind_keys`. 0028 moves both bindings into `keymap.rs` and owns the Keyboard Shortcuts page.
2. Non-US layouts: Ctrl \` and `[` `]` sit on dead or AltGr keys on some layouts (German, French). Rebinding them needs a user decision and a 0024 settings key.
3. 0020 asks for a back key, which the wireframe does not show. Proposed for later: Alt+Left (⌘[ on macOS).
4. Letter accelerators inside an open menu need kit support. Until then, menu letters are hints only.
5. macOS laptops have no Delete key. 0033 decides whether ⌘⌫ also deletes.
6. Resolved: decision 14 is confirmed. A row click while the drawer is open switches the subject. This is a known deviation for ui-verifier.
