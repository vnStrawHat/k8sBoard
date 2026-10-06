# 0009 · App: row checkboxes, Ctrl/Shift click, selection bar

[Back to index](README.md) · Step 5 (can move to 0032) · Modules: `row_selection.rs` (new), `table_view.rs`, `table_layout.rs`, `pod_table.rs`, `node_table.rs`, `kind_table.rs`, `workspace.rs`, `app_shell.rs`, `launch_options.rs`

## Model

```rust
// table_view.rs
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RowName { pub(crate) namespace: Option<String>, pub(crate) name: String }
pub(crate) struct TableView { /* step 2 fields */ checked: BTreeSet<RowName>, anchor: Option<RowName> }
impl TableView {
    pub(crate) fn is_checked<T: TableRow>(&self, row: &T) -> bool;
    /// Toggles one row and makes it the range anchor.
    pub(crate) fn toggle_checked<T: TableRow>(&mut self, row: &T);
    /// Checks every visible row between the anchor and `row` (inclusive, view order); without
    /// an anchor it acts like `toggle_checked`.
    pub(crate) fn check_range<T: TableRow>(&mut self, items: &[T], row: usize);
    pub(crate) fn set_all_checked<T: TableRow>(&mut self, items: &[T], checked: bool); // visible only
    pub(crate) fn all_checked<T: TableRow>(&self, items: &[T]) -> bool;                // visible, ≥ 1
    pub(crate) fn checked_count(&self) -> usize;
}
```

- `rebuild` prunes `checked` (and the anchor) to the names of the visible rows (decision 24): a filter, a deleted object, or a scope change unchecks.
- `clear_filter`, `reset_filter`, and a context switch clear `checked`; a kind switch keeps each kind's own set.
- Identity is `(namespace, name)`, unique within one table.

## Input

| Gesture | Effect |
|---|---|
| Click a row checkbox | `toggle_checked`; the row is **not** selected (the cell stops propagation of the left mouse down and the click) |
| Click the header checkbox | `set_all_checked(!all_checked)` |
| Ctrl+click a row (`secondary` modifier, Cmd on macOS) | `toggle_checked` |
| Shift+click a row | `check_range` |
| Plain click a row | unchanged: selects the row, opens the drawer |
| Space | ticks or unticks the cursor row (`RowCheck::Toggle`) |
| Shift+J / Shift+K | moves the cursor one row and ticks the range from the anchor to it (`RowCheck::Extend`; a step that wraps ticks nothing) |
| Ctrl+A (Cmd on macOS) | ticks every shown row, or unticks them when all are ticked (`RowCheck::ToggleAll`) |

The keys act only where Enter opens the row (the table or the shell root has focus), never in a text field; Issues has no ticks. The shortcut sheet lists them under Tables with a note for the mouse gestures.

- Modifiers: each delegate's `render_tr(row_ix, ..)` returns `div().id(("row", row_ix)).on_click(..)`; the handler reads `event.modifiers()` and calls `AppShell::toggle_row_checked` / `check_row_range` only when Ctrl or Shift is held. The kit adds its own row click after ours, so a modified click also selects the row and the drawer follows it (known ceiling; no workaround).
- Handlers are plain closures on `WeakEntity<AppShell>` (like `sortable_header`), going through `update_view`. If the checkbox cell still lets the kit select the row, the coder reports it instead of working around it (AC10).

## Checkbox column

- `layout_columns` prepends `Column::new("select", "").width(px(32.)).resizable(false)`; `TableColumns::logical` returns `None` for it, and Columns ▾ never lists it. `fit_width` counts its 32 px as fixed.
- Header cell `Checkbox::new("select-all").checked(all_checked)`; row cell `Checkbox::new(("select", row_ix)).checked(is_checked)`.

## Selection bar (W5)

```text
        ┌ 2 nodes selected   [Cordon] [Uncordon] [Drain…]   [✕] ┐
```

- Shown when the visible table's `checked_count() > 0`: absolute, bottom center of the table area (left of an open drawer), above the dock and status bar; `bg(theme.popover)`, `border_1` `theme.border`, `rounded_lg`, `shadow_md`, `px_3 py_2 gap_2`. No hardcoded colors.
- Text `{n} {singular|plural} selected` via `count_label`. `✕` clears the set.
- Bulk actions follow the W5 bar and the W7 list-level buttons that act on selected rows; every one is a disabled small `Button` with tooltip `READ_ONLY_MODE_REASON` ("Read-only mode"):

```rust
// row_selection.rs (pure)
pub(crate) fn bulk_actions(screen: Screen) -> &'static [&'static str];
```

| Screen | Actions |
|---|---|
| Nodes | Cordon, Uncordon, Drain… |
| Deployments | Scale…, Restart, Roll back… |
| StatefulSets | Scale…, Restart |
| DaemonSets | Restart |
| Jobs | Re-run |
| CronJobs | Trigger now, Suspend |
| every other screen | none (count and ✕ only) |

- 0032–0034 enable them. No Copy names: it is not in the wireframe.
- 0033 adds a `Delete…` danger button, last before ✕, on Pods, Nodes, and every kind screen (not Issues, whose rows are findings): it deletes the ticked rows through the same gate as the menu and Del.

## Launch screen

`--screen pods-selected` and `--screen nodes-selected`: that screen with its first two visible rows checked once the list loads (`apply_pending_launch_screen`), no drawer. Usage text updated.
