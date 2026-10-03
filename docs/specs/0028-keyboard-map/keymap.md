# 0028 · Key map

[Back to index](README.md) · Steps 1–3 · Module: `crates/app/src/keymap.rs` (new). Decisions 1–9.

Source: the wireframe keyboard map grid (section `#phim`, 22 rows), the W4/W5/W7 menu `kbd` hints, the anatomy Drawer and Dock notes, W8/W8b notes. Contexts are defined in [contexts-and-focus.md](contexts-and-focus.md).

## Bound in 0028

All actions are unit structs from `gpui_kit::actions!(k8sboard, [...])` in `keymap.rs` (`FocusQuickFilter` moves there from `main.rs`).

| Wireframe key | Binding string | Action | Context | Effect |
|---|---|---|---|---|
| `?` | `?` | `ShowShortcuts` | WORKSPACE | opens the shortcut sheet ([shortcut-sheet.md](shortcut-sheet.md)) |
| `/` | `/` | `FocusQuickFilter` | WORKSPACE | unchanged behavior (0009); context widened (decision 4) |
| J / K, ↓ / ↑ | `j`, `down` / `k`, `up` | `SelectNextRow` / `SelectPreviousRow` | WORKSPACE, TABLE (arrows only) | moves the row cursor, wrapping at the ends as ↑↓ do today; the drawer follows only when open |
| — (kit keys) | `home`, `end`, `pageup`, `pagedown` | `SelectFirstRow`, `SelectLastRow`, `SelectPreviousPage`, `SelectNextPage` | WORKSPACE, TABLE | same, re-bound so no kit key opens the drawer (decision 6) |
| ⏎ | `enter` | `OpenDrawer` | WORKSPACE | opens the drawer on the cursor row (first row when none) |
| Esc | `escape` | `Dismiss` | WORKSPACE, TABLE | ladder in [contexts-and-focus.md](contexts-and-focus.md) |
| Esc (in a field) | `escape` | `LeaveInput` | FIELD | focus leaves the text field |
| `[` / `]` | `[` / `]` | `PreviousContainer` / `NextContainer` | WORKSPACE | pod drawer: previous/next container |
| L | `l` | `ViewLogs` | WORKSPACE | [row-actions.md](row-actions.md) |
| Y | `y` | `ViewYaml` | WORKSPACE | drawer opens on the YAML tab |
| Ctrl C (menu) | `secondary-c` | `CopyName` | WORKSPACE | copies the name; a text selection wins (decision 8) |
| S | `s` | `OpenShell` | WORKSPACE | gated |
| F | `f` | `PortForward` | WORKSPACE | gated |
| C | `c` | `Cordon` | WORKSPACE | gated (nodes) |
| D | `d` | `Drain` | WORKSPACE | gated (nodes) |
| E | `e` | `EditYaml` | WORKSPACE | gated |
| R (W7 menu) | `r` | `RestartRollout` | WORKSPACE | restart the cursor row (0032, shipped) |
| ⇧S (W7 menu) | `shift-s` | `Scale` | WORKSPACE | gated until the 0032 Scale popover |
| — (menus, palette) | unbound | `PauseRollout`, `RollBack`, `SuspendCronJob`, `TriggerCronJob`, `RerunJob` | — | unit actions the W7 menus and the palette dispatch; no key in the wireframe, not on the sheet (0032) |
| Del (W4 menu) | `delete` | `Delete` | WORKSPACE | gated; offered on every subject, beyond the wireframe (W4 pods only) |
| Ctrl N | `secondary-n` | `OpenNamespacePicker` | WINDOW | opens the title-bar namespace picker |
| Ctrl \` | ``ctrl-` `` | `ToggleDock` | WINDOW | Normal/Zoomed → Minimized; Minimized → Normal |
| Ctrl Shift M | `secondary-shift-m` | `ToggleDockZoom` | WINDOW | Zoomed → Normal; Normal/Minimized → Zoomed |
| Ctrl Tab | `ctrl-tab` | `NextDockTab` | WINDOW | next dock tab, wraps |
| — | `ctrl-shift-tab` | `PreviousDockTab` | WINDOW | previous dock tab, wraps |
| Ctrl W | `secondary-w` | `CloseDockTab` | WINDOW | closes the active dock tab |

Dock actions do nothing when the dock has no tabs. Letters are lowercase in binding strings: `"J"` would parse as `shift-j`.

## Moved from 0025 (0025 lands first)

0025 binds both in `settings_window::bind_keys`. 0028 moves the two `KeyBinding`s into `keymap::bind_keys` with their contexts unchanged and deletes `settings_window::bind_keys`. The actions stay declared in `settings_window.rs`.

| Wireframe key | Binding string | Action | Context | Effect |
|---|---|---|---|---|
| Ctrl , | `secondary-,` | `OpenSettings` | none (every window, dialogs included) | opens or focuses the Settings window (0025) |
| Ctrl O (W2) | `secondary-o` | `ImportKubeconfig` | `SettingsWindow` | import flow (0025) |

## Keys of other specs

`RESERVED_KEYS` (test const in `keymap_tests.rs`) lists the keys that no 0028 binding may take. A key leaves the list in the change that binds it, made by its owner spec. If the owner has already landed when 0028 is implemented, the key is not in the list, and the test that checks for duplicate keys covers it.

| Wireframe key | Binding string | Action | Context | Bound by | Notes |
|---|---|---|---|---|---|
| Space (W1) | `space` | tick a cluster | `ClusterSwitcher` only | 0027 | bound by 0027 (`cluster_switcher::bind_keys`); left `RESERVED_KEYS`. No binding in WORKSPACE. The kit `Popover` binds `space` → `Confirm` (gpui-base 0.7 `popover.rs:21`), so 0027 binds `space` → `ToggleClusterTick` on `ClusterSwitcher` and `ClusterSwitcher > Input` ([0027 switcher-multi.md](../0027-multi-cluster/switcher-multi.md)); the switcher filter then takes no spaces |
| Ctrl K | `secondary-k` | `OpenPalette` | WINDOW | 0029 | bound (0029); left `RESERVED_KEYS` |
| `:` | `:` | `OpenKindPalette` (palette opened with `:` typed) | WORKSPACE | 0029 | bound (0029); left `RESERVED_KEYS` |
| Ctrl ⏎ (W9) | `secondary-enter` | `ScaleCursorRow`: turns the query into a replicas field on an enabled Scale entry | `Command > Input` only | 0032 | bound (0032 step 2a-ii); left `RESERVED_KEYS`. Escape in that field (`PaletteArgument`) steps back to the list |
| Ctrl Shift R | `secondary-shift-r` | `ToggleReadOnly` | WINDOW | 0030 | bound in 0030 step 2b; it toggles the lock of the cursor cluster, else the primary |
| Ctrl S (W10) | `secondary-s` | Apply | editor context only | 0031 | |

Each owner also adds a row to `shortcut_rows()`, so `every_bound_action_is_on_the_sheet` keeps passing.

## Bound by 0026 (accepted spec; held in `keymap.rs`, not in `RESERVED_KEYS`)

| Wireframe key | Binding string | Action | Context | Notes |
|---|---|---|---|---|
| Ctrl Shift C | `secondary-shift-c` | `OpenClusterSwitcher` (toggle) | WINDOW | 0026 [keys.md](../0026-cluster-switcher/keys.md). If 0026 code lands first, it binds in `app_shell::bind_keys`, and 0028 moves the binding into `keymap.rs` |
| Ctrl 1 … 9 | `secondary-1` … `secondary-9` | `SwitchToCluster1` … `9` | WINDOW | as above; one sheet row (shortcut-sheet.md) |
| ↓ ↑ ⏎ Esc in the switcher | `down`, `up`, `enter`, `escape` | `SwitcherNext`, `SwitcherPrevious`, `SwitcherConfirm`, `CloseClusterSwitcher` | `ClusterSwitcher`, `ClusterSwitcher > Input` | stay in `cluster_switcher::bind_keys` (0026 keys.md); not on the sheet (popover-local) |

## Platform modifiers

GPUI parses `secondary` as Cmd on macOS and Ctrl elsewhere; `ctrl` is literal everywhere.

| Rule | Keys | Windows / Linux | macOS |
|---|---|---|---|
| Wireframe "Ctrl" means the primary modifier | Ctrl N, Ctrl W, Ctrl C, Ctrl Shift M, and every reserved Ctrl key | Ctrl | ⌘ |
| Literal Ctrl where ⌘ is taken by the OS | Ctrl \`, Ctrl Tab, Ctrl Shift Tab | Ctrl | ⌃ (⌘\` cycles windows, ⌘Tab switches apps) |
| No modifier | letters, `?`, `/`, `[`, `]`, arrows, Home, End, PgUp, PgDn, Enter, Esc, Delete | — | — |

- Linux: the Super key is never used. Windows: the Win key is never used.
- Labels come from the kit `Kbd` element, which formats per OS (`Ctrl+N`, `⌘N`, `⌃\``); no label is hand-written.
- `?` and `:` are shifted characters on US layouts. GPUI matches them through `key_char` on every OS, so the binding is the character itself, never `shift-/`.
- macOS Delete: the ⌫ key is `backspace`; only `delete` (fn ⌫) is bound. 0033 decides on ⌘⌫ (open item 5).
