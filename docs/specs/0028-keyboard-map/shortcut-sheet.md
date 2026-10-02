# 0028 · Shortcut sheet, Settings page, dock keys, room for 0029

[Back to index](README.md) · Steps 1, 4 · Modules: `keymap.rs`, `shortcut_sheet.rs` (new), `settings_window.rs` (0025), `log_dock.rs`, `launch_options.rs`, `screenshot.rs`. Decisions 23–29.

## `?` sheet

The wireframe grid ends with "See all shortcuts `?`" and draws the key map as a two-column grid (label left, keys right) under the line "Single-letter keys work only while a resource is selected and no text field has focus. Destructive actions always open a confirmation." No separate overlay is drawn, so the sheet is that grid in a kit dialog.

```rust
pub(crate) enum ShortcutGroup { General, Tables, Drawer, SelectedResource, Dock }
pub(crate) struct ShortcutRow { pub(crate) group: ShortcutGroup, pub(crate) label: &'static str,
    pub(crate) action: Box<dyn Action> }
pub(crate) fn shortcut_rows() -> Vec<ShortcutRow>;               // keymap.rs, wireframe order
pub(crate) fn open_shortcut_sheet(window: &mut Window, cx: &mut App);   // shortcut_sheet.rs
/// The grid alone, shared by the dialog and Settings › Keyboard Shortcuts.
pub(crate) fn shortcut_sheet(cx: &App) -> impl IntoElement;
```

- `open_shortcut_sheet`: `window.open_dialog(cx, |dialog, _, cx| dialog.title("Keyboard shortcuts").width(px(640.)).child(shortcut_sheet(cx)))`. The kit dialog traps focus, closes on Esc and on an overlay click, and restores focus.
- Keys of a row: `cx.key_bindings().borrow().bindings_for_action(&*row.action)`, the first keystroke of each binding, deduplicated, in registration order, each as `Kbd::new(key.as_keystroke().clone())` (`key: &KeybindingKeystroke`) (so "Next row" shows `J` `↓`). The sheet shows the real bindings, formatted for the OS, and cannot drift from them. Rows are rebuilt per render (about 30 boxes; no cache).
- Groups render in enum order, a muted group title over a two-column grid, the wireframe note as the footer (English). Theme colors only.
- Rows (English labels): General — Show all shortcuts; Choose namespace; Open Settings; Import kubeconfig file (Settings window). Tables — Filter the table; Next row; Previous row; First row; Last row; Next page; Previous page; Open the drawer; Close the drawer, then clear the selection. Drawer — Next container; Previous container. Selected resource — View logs; View YAML; Copy name; Open shell (pod or node); Port-forward; Cordon or uncordon node; Drain node (opens a dialog); Edit YAML; Restart rollout; Scale; Delete. Dock — Toggle the dock; Zoom the dock in or out; Next dock tab; Previous dock tab; Close the dock tab.
- `LeaveInput` has no row (it is the Esc of a field). Reserved keys get rows in their owner specs.

## Settings › Keyboard Shortcuts (0028 owns the page)

0025 lands first and omits this page. 0028 adds it to the `Settings::new("settings").pages(..)` list in `settings_window.rs`, at W2 position 4: after Appearance and before the pages that come later (Safety and the rest). With 0025's pages the order becomes Clusters, Appearance, Keyboard Shortcuts, About.

- `SettingPage::new("Keyboard Shortcuts").resettable(false)` holds one group with one `SettingItem::render` that draws `shortcut_sheet(cx)`. The page is read-only: there are no edit controls (decision 25).
- `settings_window_tests::pages_follow_w2_order` gains the new page in position.
- The keymap is app-wide, so the page shows the same keys as `?`. `OpenSettings` has no context, so Ctrl , works in both windows.

## Dock keys (`log_dock.rs`)

```rust
pub(crate) fn toggled_visibility(mode: DockMode) -> DockMode;  // Normal|Zoomed → Minimized; Minimized → Normal
pub(crate) fn toggled_zoom(mode: DockMode) -> DockMode;        // Zoomed → Normal; Normal|Minimized → Zoomed
pub(crate) enum TabStep { Next, Previous }
pub(crate) fn step_tab(active: usize, len: usize, step: TabStep) -> usize; // wraps; len ≥ 1
impl LogDock { pub(crate) fn toggle_visibility(..); pub(crate) fn toggle_zoom(..);
    pub(crate) fn step_active_tab(&mut self, step: TabStep, cx: &mut Context<Self>); pub(crate) fn close_active_tab(..); }
```

Each method returns early when `tabs` is empty. `close_active_tab` reuses `close_tab(active)`. The shell handlers only forward to the dock entity.

## Launch screens for ui-verifier (`--screen`, listed in `USAGE`)

| `--screen` | Shows |
|---|---|
| `shortcuts` | Pods, with the `?` sheet open (opened in the first render with a live session, like `open_pending_logs`) |
| `pods-cursor` | Pods, first row selected, drawer closed |

## Room for the command palette (0029)

- 0029 binds `secondary-k` and `:` in `WINDOW`/`WORKSPACE` (reserved, tested), and its list keys (`up down enter secondary-enter tab escape`) in its own context (`CommandPalette`, or the kit's `Command` context if 0029 adopts `gpui_kit::component::command`). The palette input is an `Input`, so no single key leaks to the table behind it.
- Actions: the palette's "> action" items dispatch the same unit actions with `window.dispatch_action`, so a palette command and a key run one handler. `shortcut_rows()` plus `ResourceAction` give the palette its command labels and key hints; 0029 needs no new action for the 0028 commands.
- Mutating palette items show "needs confirm" and are disabled through the same `key_availability` (W9).
- Nothing palette-specific is built in 0028.
