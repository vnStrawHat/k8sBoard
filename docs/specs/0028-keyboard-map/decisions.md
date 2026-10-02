# 0028 · Decisions

[Back to index](README.md). Architect defaults; the user confirms.

## Key map and contexts

| # | Decision | Rationale |
|---|---|---|
| 1 | Bind every wireframe key whose feature exists. Move Ctrl , and Ctrl O from 0025 into `keymap.rs`. Keys owned by later specs are listed in `RESERVED_KEYS` until their owner binds them: Space (0027), Ctrl K, `:`, Ctrl ⏎ (0029), Ctrl Shift R (0030), Ctrl S (0031). Ctrl Shift C and Ctrl 1–9 are bound by 0026 (accepted) and live in `keymap.rs` | a key that does nothing is worse than no key, and the owner spec knows the behavior; `keymap.rs` stays the one place that holds every binding (exception: 0026's switcher-local keys in `cluster_switcher.rs`, keymap.md) |
| 2 | Unit actions only, from `gpui_kit::actions!` | the facade macro is unit-only; a data action's derive needs the `gpui` crate path; ~30 unit structs are plain |
| 3 | Contexts are predicates over existing names (`AppShell`, kit `Input`, `PopupMenu`, `Popover`, `Dialog`, `DataTable`) plus three wrappers (`QuickFilter`, `Drawer`, `LogDock`) | no context per screen; the predicates read like the wireframe rule |
| 4 | `/` moves to `WORKSPACE` (adds `!PopupMenu && !Popover && !Dialog`) | supersedes 0009 decision 13; one rule for all single keys |
| 5 | Chords work inside text fields; single keys never do | wireframe: letters only "when not typing"; Ctrl N or Ctrl W from the filter is expected |
| 6 | Re-bind the kit table's row-move keys (`up down home end pageup pagedown escape`) in `TABLE` | every keyboard move then belongs to the shell, which is what lets a click open the drawer and a key not (decision 12) |
| 7 | Esc leaves the `/` filter, the dock filter, and drawer fields (YAML view) | without it, focus in a field is a keyboard dead end |
| 8 | Ctrl C copies the cursor's name unless text is selected | the kit Root copy of selected drawer text must keep working |
| 9 | `secondary` for every wireframe Ctrl key except Ctrl \` and Ctrl Tab, which stay literal Ctrl | ⌘ is the macOS primary; ⌘\` and ⌘Tab belong to the OS |

## Cursor and drawer

| # | Decision | Rationale |
|---|---|---|
| 10 | Split the row cursor from the drawer: `DrawerState.is_open` | wireframe: J/K move, ⏎ opens, Esc closes. Rejected: keep "selected = open" and alias J/K to ↑↓ (⏎ would mean nothing, and browsing would always cover 420 px of the table) |
| 11 | Drawer readers use `drawer_subject()`; a closed drawer runs no events, related, YAML, or kubelet fetch | cursor browsing must stay as cheap as today's table |
| 12 | Tell a click from a shell move with a one-shot `row_echo` mark | a mouse-capture flag also fires on headers and scrollbars, which emit no `SelectRow`, and would leak into the next key move |
| 13 | ✕ and the first Esc keep the row highlighted; the second Esc clears it | the user returns to the same row with ⏎ |
| 14 | A row click with the drawer open still switches the subject (0003), instead of following the anatomy note "click on the table area closes it". **Confirmed by the user**; a known deviation for ui-verifier | comparing rows by clicking is the main mouse flow; ✕ and Esc close the drawer |
| 15 | Next and Previous row moves wrap; page moves and First/Last clamp; dock tab moves wrap | keeps today's ↑/↓, which wrap: the kit default is `loop_selection: true` (gpui-component `table/state.rs:306`) and `configure` never changes it. Ctrl Tab wraps in every tabbed app |
| 16 | `[` `]` select the container and switch to the Containers tab | the change is visible from any tab (W4b note 2) |

## Row actions

| # | Decision | Rationale |
|---|---|---|
| 17 | A letter acts on the cursor row, drawer open or closed; no cursor → nothing | wireframe: "only while a resource is selected" |
| 18 | Extend `ResourceAction`; no new enum | menus and keys share `action_availability` |
| 19 | A disabled key shows a notice with the reason; a key the subject does not offer is silent | users learn why S does nothing on UAT; L on a Service is simply not a thing |
| 20 | `KindAction { label, action }` replaces the label list | no matching of menu strings |
| 21 | Menus show key hints through `PopupMenuItem::action`; no menu item is added or renamed | one source of truth; added items belong to their owner specs |
| 22 | Mutating keys are bound but always disabled in 0028. A is not bound: `Attach` and its key are a later item (no longer owned by 0036) | read-only rule; 0031–0036 enable them by flipping the gate |

## Sheet, dock, scope

| # | Decision | Rationale |
|---|---|---|
| 23 | `?` opens a kit dialog with the wireframe's key grid | the wireframe draws the grid, not a separate overlay; the kit dialog gives focus trap and Esc |
| 24 | The sheet reads keys from the live keymap | it cannot drift; `Kbd` formats per OS |
| 25 | User rebinding is a non-goal, with no reserved settings key. 0028 owns Settings › Keyboard Shortcuts as a read-only page at W2 position 4 | the wireframe shows only the nav item, with no page content; 0024 reserves no key; 0025 omits the page and leaves it to 0028 |
| 26 | Dock keys do nothing without tabs | the dock is not drawn without tabs (0004) |
| 27 | Add `--screen shortcuts` and `--screen pods-cursor` | ui-verifier cannot press keys; these show the two new visual states |
| 28 | 0029 reuses the 0028 actions and `shortcut_rows()`; nothing palette-specific is built | room without scaffolding |
| 29 | No region focus cycling, no back navigation key | the wireframe shows neither (0020 open item 3 stays open) |
