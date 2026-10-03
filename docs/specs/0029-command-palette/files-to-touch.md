# 0029 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own. All in `crates/app`; `crates/cluster` is untouched. Builds on 0028 (keymap, actions, `shortcut_rows`, `key_availability`), 0024 (`environment_badge`), and 0026 (`switcher_sections`, `switch_cluster`).

## Cargo

No change. `Command`, `CommandState`, `CommandGroup`, `CommandItem`, `Dialog`, `WindowExt`, and `Kbd` are in gpui-component 0.7. No fuzzy crate is added (decision 1). `git diff Cargo.lock` stays empty.

## Source

| S | File | Change |
|---|---|---|
| 1 | `src/fuzzy_score.rs` (new) | `fuzzy_score`; tests in module |
| 1 | `src/palette_search.rs` (new) + `palette_search_tests.rs` | `PaletteMode`, `PaletteQuery`, `parse_query`, `entry_score`; step 2b adds `PaletteGroup`, `PaletteTarget`, `PaletteEntry`, `EntryState`, `PaletteInput`, `palette_entries` (commands, screens, namespaces), `ranked`; step 3 adds resources, row actions, clusters, caps |
| 1 | `src/resource_kind.rs` | `short_names`; test `short_names_cover_every_kind` |
| 1 | `src/main.rs` | `mod fuzzy_score; mod palette_search;` (step 2a adds `mod command_palette;`) |
| 2a | `src/command_palette.rs` (new) | `CommandPalette` entity, `open_palette` (entity and `CommandState` created once, outside the dialog builder), `on_query` with the initial `:` Esc rule, an empty list |
| 2a | `src/keymap.rs` (+ `keymap_tests.rs`) | `OpenPalette`, `OpenKindPalette`; `secondary-k` (`WINDOW`), `:` (`WORKSPACE`); two General sheet rows; `RESERVED_KEYS` drops both and relabels `secondary-enter` to 0032 |
| 2a | `src/app_shell.rs` (+ `app_shell_tests.rs`) | `on_action` for both actions → `open_palette`; a `pub(crate)` accessor for the root `FocusHandle`; headless tests |
| 2b | `src/command_palette.rs` | header chips, footer, row content, `on_confirm` / `on_select` wiring, run targets, Tab preview handler |
| 2b | `src/keymap.rs` | `PalettePreview` on `tab` in `Command > Input` (after kit init) |
| 2b | `src/app_shell.rs` | `is_row_visible(&ResourceKey)` (read-only accessor); `change_selection` reachable as `pub(crate)` (0028) |
| 2b | `src/navigation.rs` | `kind_availability` and `KindAvailability` become `pub(crate)` |
| 2b | `src/title_bar.rs` | search box in a middle slot (inventory T6) |
| 3 | `src/palette_search.rs` | resource entries from pods, nodes, the visible explorer kind; row actions via `key_availability`, `action_label`, and `key_action`; cluster rows from 0026 `cluster_switcher_rows`; caps |
| 3 | `src/palette_search_tests.rs` | test `every_offered_row_action_maps` (uses the existing `ResourceAction::key_action`) |
| 3 | `src/command_palette.rs` | status and reason pills, `Kbd` hints, Resources hint text |
| 4 | `src/launch_options.rs` (+ tests) | `--palette <query>`; `USAGE` |
| 4 | `src/app_shell.rs` | open the palette for `--palette` once the first list has loaded (like `open_pending_logs`) |
| 4 | `src/screenshot.rs` | settle waits until the palette dialog is open and its entries are built |

## Docs (step 4)

| File | Change |
|---|---|
| `docs/roadmap/inventory-shell.md` | P1 → Done (0029) with the deviations of decisions 10, 19; T6 → Done |
| `docs/roadmap/cross-cutting.md` | C6 row "Fuzzy matching": own scorer, no dependency (0029 decision 1) |
| `docs/roadmap/gap-plan-local-and-mutating.md` | the 0029 entry: Tab preview in its cheap form (cursor only), Ctrl ⏎ to 0032, no new dependency |
| `docs/roadmap/README.md` | status row "Keyboard map, command palette" → Done (with notes) |
| `docs/specs/0028-keyboard-map/keymap.md` | reserved table: Ctrl K and `:` bound by 0029; Ctrl ⏎ owner 0032 (orchestrator applies if 0028 is merged) |
