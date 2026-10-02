# 0028 · Key contexts, text inputs, focus

[Back to index](README.md) · Steps 1, 2b · Modules: `keymap.rs`, `keyboard_navigation.rs` (new), `app_shell.rs`, `filter_bar.rs`, `drawer.rs`, `log_dock.rs`. Decisions 3–7.

## How GPUI picks a binding (gpui-pre 0.3.7 `keymap.rs`)

- A predicate matches at the **depth** of the deepest context node where it holds; a deeper match wins; at equal depth the **later registered** binding wins.
- `A > B` means B below A (any distance). `!X` means no `X` anywhere in the focused path.
- If the winning action has no handler or calls `cx.propagate()`, the next matching binding runs.
- `keymap::bind_keys` runs after `gpui_kit::init` (as today), so app bindings outrank kit bindings at equal depth.

## Contexts (constants in `keymap.rs`)

| Name | Predicate | Element that sets it | Holds |
|---|---|---|---|
| `WINDOW` | `AppShell` | AppShell root (exists) | chords; they work inside text fields too |
| `WORKSPACE` | `AppShell && !Input && !PopupMenu && !Popover && !Dialog` | — | single keys: letters, `? / [ ]`, arrows, Enter, Esc. `!Dialog` is redundant, because dialogs sit outside `AppShell`. It is kept so the predicate states the rule |
| `TABLE` | `AppShell > DataTable` | the kit table (exists) | overrides of the kit's `up down home end pageup pagedown escape` |
| `FIELD` | three bindings: `QuickFilter > Input`, `Drawer > Input`, `LogDock > Input` | new wrappers, below | Esc → `LeaveInput` |

New `key_context` wrappers: `"QuickFilter"` around the `/` input in `filter_bar.rs`; `"Drawer"` on the root of `drawer_frame` in `drawer.rs`; `"LogDock"` on the root of `LogDock::render`. `Input`, `PopupMenu`, `Popover`, `Dialog`, `DataTable` are the kit's own context names.

Wireframe context → GPUI context: workspace = `WINDOW`/`WORKSPACE`; table = `TABLE`; drawer = `WORKSPACE` + `Drawer` (the drawer never holds focus, see below); dock = `WINDOW` + `LogDock`; menus = the kit's `PopupMenu`/`Popover`/`Dialog`, where 0028 binds nothing.

## Text input conflict rules

1. **Single keys never act in a text field.** `WORKSPACE` excludes `Input`, so `j`, `?`, `[`, `/`, Enter, arrows, Esc type or edit as usual in the `/` filter, the dock filter, the namespace search, and the YAML view (a read-only kit editor, also `Input`).
2. **Chords work anywhere inside the shell's element tree** (`WINDOW`), text fields included. Kit dialogs (the `?` sheet among them) are `Root` layers outside `AppShell`, so `WINDOW` chords do nothing while a dialog is open. Only `OpenSettings`, which has no context, still fires there. The kit `Input` binds none of the 0028 chords (checked in gpui-base 0.7: no Ctrl or Cmd chord on `n`, `w`, `` ` ``, `tab`, or `shift-m`). `secondary-c` is bound twice in the kit: `Copy` in `Root` (gpui-base `root.rs:19-21`, copies selected text) and the `Input` copy. `CopyName` is a `WORKSPACE` key. It outranks `Root` because it is deeper, and the `!Input` part of `WORKSPACE` keeps it out of text fields.
3. **Esc leaves a field**: in the three `FIELD` contexts Esc moves focus to the visible table (the shell root while no table is drawn); the field keeps its text. It outranks the kit `Input` Esc (equal depth, later), whose extra duties (multi-cursor, inline completion) do not occur in these fields. 0031's editor binds its own Esc deeper.
4. **Open menus, popovers, and dialogs own the keyboard.** Single keys are off there; their own arrows, Enter, Esc (kit) work. Menu letters are hints, not accelerators (open item 4).
5. **Kit table keys**: `TABLE` re-binds every kit key that moves the row (decision 6). `tab`, `shift-tab`, `left`, `right` stay with the kit.
6. **Copy**: `secondary-c` copies the cursor row's name unless the window has a text selection (`gpui_kit::base::TextSelection::has_selection`); then the handler calls `cx.propagate()` and the kit Root `Copy` copies the text.

## Esc ladder (`Dismiss`, pure fn in `keyboard_navigation.rs`)

```rust
pub(crate) struct DismissState { pub(crate) is_dock_zoomed: bool, pub(crate) is_drawer_open: bool, pub(crate) has_selection: bool }
pub(crate) enum DismissStep { UnzoomDock, CloseDrawer, ClearSelection, Propagate }
pub(crate) fn dismiss_step(state: DismissState) -> DismissStep; // first that applies, in this order
```

Before the ladder, the kit handles Esc in an open dialog, menu, or popover (deeper contexts). Esc while a drawer button or tab has focus (after a click) runs the ladder, because buttons bind no Esc. `Propagate` calls `cx.propagate()`. W8b note 2: Esc returns a zoomed dock to its height.

## Focus targets and movement

| Target | Context path | Gets focus when |
|---|---|---|
| Shell root (`AppShell.focus_handle`) | `Root > AppShell` | start; a focused element leaves the tree (existing `on_focus_lost` restore) |
| Visible table (`TableState::focus_handle`) | `… AppShell > DataTable` | a row click (existing `focus_table`); any row-move key, `Enter`, `LeaveInput` |
| `/` filter input | `… QuickFilter > Input` | `/` (existing) |
| Namespace picker | `… Popover` | Ctrl N or a click; closing returns focus (kit), else the root restore |
| Drawer buttons, YAML view | `… Drawer > …` | a click only; the drawer is never focused by a key |
| Dock filter input | `… LogDock > Input` | a click only |
| Shortcut sheet | `… Dialog` | `?`; Esc closes it and the kit restores focus |

- Every `WORKSPACE` key works from the root and from the table, so a screen switch needs no focus move. A row-move key focuses the visible table, so the kit's own `tab`/`left`/`right` work next.
- `/` filter: Enter on plain text (not `label:`) also focuses the table; `label:` Enter keeps today's chip behavior.
- No region cycling (sidebar → table → drawer → dock): the wireframe shows none. `tab`/`shift-tab` keep the kit Root focus cycling.
