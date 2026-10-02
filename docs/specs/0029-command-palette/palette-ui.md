# 0029 · Palette UI, keys, focus

[Back to index](README.md) · Steps 2a–4 · Modules: `command_palette.rs` (new), `keymap.rs`, `title_bar.rs`, `launch_options.rs`, `app_shell.rs`. Decisions 10–14, 17–22.

## Host: kit `Dialog` + kit `Command` (verified in gpui-component 0.7)

- `gpui_kit::component::command::{Command, CommandState, CommandGroup, CommandItem}`: a search `Input`, grouped virtual list, ↑↓ / ⏎ / Esc in the `Command` key context (`command/state.rs` `init`), disabled items skipped on confirm, `on_query`, `on_confirm(IndexPath)`, `on_cancel`, `header`, `footer`, `empty`, `filterable(false)` for host-side ranking.
- Esc in `Command` clears a non-empty query first; the next Esc propagates to the hosting `Dialog`, which closes (`state.rs` `on_action_cancel`). That is the W9 "Esc close".
- Hosted with `WindowExt::open_dialog` (W9 draws a scrim and a centered panel; a `Popover` has no scrim): `.close_button(false)`, `.width(px(640.))`, `.margin_top(px(56.))`, overlay on and closable. No title.

```rust
pub(crate) struct CommandPalette { state: Entity<CommandState>, shell: WeakEntity<AppShell>,
    shell_focus: FocusHandle, entries: Vec<PaletteEntry>, query: String, highlighted: Option<IndexPath>,
    is_seed_untouched: bool, _shell_observer: Subscription }
/// Opens the dialog with `initial` typed (`""` or `":"`) and focuses the query input.
pub(crate) fn open_palette(initial: &str, shell: &Entity<AppShell>, window: &mut Window, cx: &mut App);
```

- `open_palette` creates the `CommandPalette` entity and its `CommandState` **once**, before `open_dialog`; the dialog builder closure only clones the entity handle into `.child(..)`. Nothing is created inside the builder, because it re-runs on every render.
- `CommandPalette` is rendered as the dialog's only child. Its render builds `Command::new(&self.state).filterable(false)` with one `CommandGroup` per non-empty `PaletteGroup` (headings "Actions", "Resources", "Go to") from `self.entries`.
- `on_query(text)` → `self.query = text`; `self.entries = ranked(palette_entries(..), &parse_query(text))`; notify. `_shell_observer` (`cx.observe(&shell)`) recomputes on every shell notify, so statuses stay live (W9 note 4). `on_select(IndexPath)` → `self.highlighted` (Tab preview, footer).
- `on_confirm(IndexPath)` → entry at `(group, row)` → close the dialog → run ([entries.md](entries.md)). The palette is not reused: each open builds a new entity and `CommandState` (needs the window).

## Initial `:` and Esc (k9s rule)

- Opened with `":"`, `is_seed_untouched = true`. Any `on_query` text other than `":"` and `""` sets it to `false`.
- The kit Esc clears a non-empty query (`state.rs:602`, `set_query("")` → `on_query("")`); `on_cancel` runs only on an empty query. So: `on_query("")` while `is_seed_untouched` → `window.close_dialog(cx)`. The first Esc on a bare `:` closes the palette, as in k9s; Backspace on a bare `:` closes it too (k9s does the same).
- Opened with `""`, the kit rule stands: Esc clears, the next Esc closes.

## Tab preview (open item 1 settled: the cheap, safe form)

- `tab` → `PalettePreview` (declared in `keymap.rs`), bound in context `Command > Input`, registered after `gpui_kit::init` so it wins over any kit `tab` at equal depth; handled on the `CommandPalette` root.
- It applies only when the highlighted entry is `Resource(key)`, `key` belongs to the current screen, and the row is in its filtered table (`AppShell::is_row_visible(&key)`, a new read-only accessor). Then: `shell.change_selection(Some(key))` (0028), **without** `set_drawer_open`: the table cursor moves behind the scrim; no drawer, no screen switch, no watch, no filter change. The palette stays open.
- Otherwise Tab does nothing. The footer shows `Tab preview` only while it applies.

## Rows (W9)

Each `CommandItem` uses `.label(label)` and `.child(..)` (custom content, so the kit renders no hint of its own) with: icon text (kind short, `↻`, `@`, `#`), label, muted mono `detail`, and right-aligned: status pill (`StatusLabel` tone, theme tokens), reason pill when disabled (warn tone), `Kbd` of the entry's 0028 binding (`bindings_for_action`, as in the 0028 sheet), `Kbd` `SwitchToClusterN` for cluster rows. `.disabled(true)` for `Disabled`.

## Header and footer

- Header (W9 note 2, always visible): right-aligned chips: active cluster with its 0024 `environment_badge` (theme env color) and display label; `ns: …` with the title-bar label rules (`namespaces_label`). No session → "No cluster".
- Footer (W9 note 5): `:po resource kind` · `@ cluster` · `# namespace` · `> action`, then right-aligned `↑↓ select · Tab preview · Esc close` (`Tab preview` only while it applies), plus "+N more" when a cap cut a group (decision 17).
- Empty: "No matches" plus the Resources hint from [entries.md](entries.md).

## Keys (0028 reserved keys, now bound)

| Key | Binding | Action | Context | Effect |
|---|---|---|---|---|
| Ctrl K | `secondary-k` | `OpenPalette` | `WINDOW` | `open_palette("")` |
| `:` | `:` | `OpenKindPalette` | `WORKSPACE` | `open_palette(":")` |
| ↑ ↓ ⏎ Esc | kit | kit `Command` actions | `Command` | select, confirm, clear then close |
| Tab | `tab` | `PalettePreview` | `Command > Input` | move the table cursor to the highlighted resource (above) |

- Both actions are declared in `keymap.rs`, handled on the `AppShell` root (they need the shell entity and window). `secondary-k` and `:` leave `RESERVED_KEYS`; `shortcut_rows()` gains General rows "Command palette" and "Jump to a resource kind".
- Ctrl ⏎ (`secondary-enter`) stays reserved, now for 0032 (decision 18).
- While the palette is open, focus is in its `Input` inside a `Dialog` outside `AppShell`, so no 0028 key reaches the table behind it (0028 rules 1, 2).
- Ctrl K while open does nothing (chords do not reach `AppShell`); Esc closes.

## Title-bar search box (inventory T6)

`title_bar.rs`: a ghost button in a new middle slot between the left group (switcher, namespace picker) and the right group (badge, Settings), label "Search resources or run a command…" (muted, truncated) and the `Kbd` of `OpenPalette`; click → `open_palette("")`. Sizing: `.flex_1().min_w_0().max_w(px(280.))`, so it shrinks before the side groups do.

## Focus

- Open: `CommandState::focus(window, cx)` after `open_dialog`.
- Close by Esc, overlay click, or confirm: the kit dialog restores the focus it interrupted (table, root, or field). A confirmed target that removes that element (screen switch) falls back through the existing `on_focus_lost` restore.

## Launch flag for ui-verifier

`--palette <query>`: once the first list has loaded, opens the palette with `<query>` typed (implies nothing about the screen; combine with `--screen deployments --select api`). Listed in `USAGE`. W9 shot: `--screen deployments --palette "> rest pay"`.
