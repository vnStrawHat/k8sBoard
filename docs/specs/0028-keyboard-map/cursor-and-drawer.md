# 0028 · Row cursor and drawer

[Back to index](README.md) · Steps 2a, 2b · Modules: `drawer.rs`, `app_shell.rs`, `workspace.rs`, `screenshot.rs`, `keyboard_navigation.rs` (new), `pod_drawer.rs`. Decisions 10–16.

Today (0003) the drawer is open exactly while a row is selected. The wireframe separates the two: J/K move rows, ⏎ opens the drawer, ↑↓ keep moving rows while it is open, and Esc closes it.

## Step 2a: drawer split (no new keys)

```rust
pub(crate) struct DrawerState { /* existing fields */ pub(crate) is_open: bool } // new, false
impl AppShell {
    /// The object the drawer shows: the selection while the drawer is open.
    pub(crate) fn drawer_subject(&self) -> Option<&ResourceKey>;
    /// Sets the flag, then `follow_drawer_subjects` and `cx.notify()`.
    fn set_drawer_open(&mut self, is_open: bool, cx: &mut Context<Self>);
}
```

- `selected: Option<ResourceKey>` stays the one selection (the row cursor). Invariant: `drawer.is_open` implies `selected.is_some()`, and `change_selection(None)` sets `is_open = false`.
- Every **drawer** reader moves from `self.selected` to `drawer_subject()`:
  - `follow_drawer_subjects` and `selected_related_subject`;
  - `kubelet_demand`, `shows_monitor`, `refresh_monitor_cache`, `sync_yaml_view`, `is_yaml_loading`;
  - `render_drawer` and the selection-bar offset in `workspace.rs`;
  - the settle input in `screenshot.rs`.
- `set_drawer_open(false)` runs `follow_drawer_subjects` with no subject. That stops the running events and related watches and sets `pending_subjects = None`, which drops the pending 250 ms debounce task (`app_shell.rs` `follow_drawer_subjects`). On the next render, `sync_yaml_view` finds no subject and drops `drawer.yaml`. A closed drawer therefore runs no object-events watch, related watch, YAML GET, or kubelet demand.
- **Selection** readers keep `self.selected`: `sync_selection`, `apply_pending_reveal`, `open_yaml`, `reveal`, and the row actions ([row-actions.md](row-actions.md)).
- Renames:
  - Today's `close_drawer` body (selection set to `None` on every table) becomes `clear_selection`. Its three callers (`start_session`, `set_namespace`, `show_screen`) now call `clear_selection`.
  - The new `close_drawer` is `set_drawer_open(false)`. The three drawer ✕ buttons (`pod_drawer.rs`, `node_drawer.rs`, `kind_drawer.rs`) keep calling `close_drawer`, so the row stays highlighted.
- `reveal`, `open_yaml`, and the `--screen …-drawer` launch selection call `set_drawer_open(true)`.

### Telling a click from a shell move

The kit emits `TableEvent::SelectRow` both for clicks and for every `set_selected_row` call. The shell marks its own calls:

```rust
/// The row the shell itself just selected. Its `SelectRow` echo moves the cursor but never opens the drawer.
row_echo: Option<usize>,                                       // AppShell field
fn select_table_row<D: TableDelegate>(&mut self, table: &Entity<TableState<D>>, row: usize, cx: &mut Context<Self>);
/// In `table_selection.rs`, next to `selection_sync`. Whether a `SelectRow` for `row` is the echo of the shell's own move; consumes the mark.
pub(crate) fn take_row_echo(echo: &mut Option<usize>, row: usize) -> bool;
```

- `select_table_row` sets `row_echo = Some(row)` and then calls `set_selected_row`. Step 2a uses it in `apply_selection_sync`, so a snapshot reorder never reopens a closed drawer. Step 2b also uses it for row-move keys.
- `on_*_table_event` `SelectRow(row)`:
  1. `let is_echo = take_row_echo(&mut self.row_echo, *row)`.
  2. Compute the key as today, then call `change_selection(key)`.
  3. If `!is_echo`, call `set_drawer_open(true)`. This runs even when `change_selection` returned `false` (same key): clicking the highlighted row of a closed drawer opens it.
  4. Call `focus_table` in both cases, whether or not the key changed.
- In 2a the kit's own arrow keys still emit non-echo `SelectRow`s, so they open the drawer as they do today. Step 2b re-binds them.

## Step 2b: keys

| Trigger | Selection | `is_open` |
|---|---|---|
| `SelectNextRow` … `SelectNextPage` | moves (`select_table_row`) | unchanged |
| `OpenDrawer` (⏎) | kept. With no selection, `set_selected_row(0)` without a mark, so it acts as a click | true |
| `Dismiss` step `CloseDrawer` | kept | false |
| `Dismiss` step `ClearSelection` | none (`clear_selection`) | false |

Because `TABLE` re-binds every kit row-move key, a non-echo `SelectRow` from then on is always a pointer click. On Issues, ⏎ reveals the object of the cursor row, as a click does, and does nothing without a cursor. Enter stays with a focused button (the action propagates unless the shell root or a table has focus).

```rust
pub(crate) enum RowStep { Next, Previous, First, Last, NextPage, PreviousPage }
pub(crate) fn step_row(current: Option<usize>, row_count: usize, step: RowStep, page: usize) -> Option<usize>;
```

- `row_count == 0` → `None`. `current == None` → `Last` gives `row_count - 1`; every other step gives `0`.
- `Next` and `Previous` **wrap**: `Next` on the last row goes to 0, and `Previous` on row 0 goes to the last row. This keeps today's ↑/↓ behaviour: the kit default is `loop_selection: true` (gpui-component `table/state.rs:306`), and `configure` in `app_shell.rs` never changes it.
- Page steps and First/Last clamp. A stale `current >= row_count` is treated as the last row.
- `page = max(1, visible rows - 1)`, from `TableState::visible_range().rows()`.
- The handler reads the table of `self.screen`, calls `select_table_row` (the kit scrolls), and focuses the table.

### Containers `[` `]`

```rust
/// Containers-list order: Init, Sidecar, Main, stable inside each group (`container_list` uses it too).
pub(crate) fn container_display_order(containers: &[ContainerSummary]) -> Vec<usize>;
pub(crate) enum ContainerStep { Next, Previous }
pub(crate) fn step_container(order: &[usize], current: Option<usize>, step: ContainerStep) -> Option<usize>;
```

- Acts only when `drawer_subject()` is a pod.
- `current` is `selected_container_index(pod, &drawer)`; `None` gives the first in `order`. Steps clamp with no wrap. An empty `order` gives `None` (no-op).
- Effect: sets `drawer.tab = Containers` and calls `select_container(name)`, so the change shows from any tab.
