# 0009 · App: header, filter bar, `/`, Columns ▾

[Back to index](README.md) · Step 2 · Modules: `filter_bar.rs` (new), `workspace.rs`, `table_layout.rs`, `app_shell.rs`, `main.rs`, `launch_options.rs`

## Layout

```text
┌ header ─────────────────────────────────────────────────────────────────────┐
│ Pods   38 of 1,284 match                          [Warnings only] [Pause …] │
├ filter bar ─────────────────────────────────────────────────────────────────┤
│ [Status: unhealthy ×] [label:app=api ×] [+ Filter ▾]   [⌕ Filter  /] [Columns ▾] │
└─────────────────────────────────────────────────────────────────────────────┘
```

- The filter bar is one `h_flex` row under the header (and above the interruption banner) on every table screen, `px_4 py_1 gap_2`, bottom border `theme.border`.
- Left: Namespace chips (step 4; `Namespace: all ▾` opens the picker), Nodes summary chips (step 3), the active chips, then `+ Filter`. Right (`ml_auto`): the quick filter input, then Columns ▾.
- Header right side: the per-screen toggles (Warnings only today; Pause stream and Hide inactive in step 3).

## Header count (`workspace.rs`)

| State | Text |
|---|---|
| `view.is_filtering()` | `{shown} of {total} match`, with `group_digits` for both (ReplicaSets show it by default: Hide inactive is on) |
| otherwise | today's text (`104 pods · all namespaces`, `newest 2,000` suffix for Events) |

## Chips

- Each active chip is `Button::new(("filter-chip", ix)).small().outline().label(text).child(Icon::new(IconName::X))` (the × follows the label) with tooltip "Remove filter"; a click removes it (`update_view`). No hardcoded colors.
- Texts: `Status: unhealthy` everywhere (it matches Warn, Bad, and Info rows, so finished pods are hidden: UX walk H15); `{title}: {value}` for `Equals` (`Node: wk-03`, `Reason: BackOff`); `label:…` for a label chip.

## + Filter (`DropdownMenu` on a ghost small button "+ Filter")

| Screen | Items |
|---|---|
| Pods | `Status: unhealthy`, `Status: Completed` (checked when on), `Label…` |
| Nodes | `Label…` (the summary chips cover status and version) |
| Jobs | `Status: unhealthy`, `Status: Complete`, `Label…` |
| Kinds except Events | `Status: unhealthy`, `Label…`; ReplicaSets add a checkable `Hide inactive` |
| Events | no + Filter (Warnings only and Filter similar cover it) |

`Label…` focuses the quick filter and sets its value to `label:` (the menu handler has the window).

## Quick filter (`/`)

- `AppShell.quick_filter: Entity<InputState>`, placeholder `Filter  /`, rendered `Input::new(..).small().cleanable(true)`, width 220.
- `InputEvent::Change` → `update_view(|view| view.filter.text = quick_filter_text(value))`. `quick_filter_text` returns an empty text while the trimmed input starts with `label:`: a label query waits for Enter and filters nothing by its literal text. `+ Filter` > `Label…` also clears `view.filter.text` (through `update_view`) before it sets the input to `label:`.
- `InputEvent::PressEnter { .. }`: if `parse_label_queries(value)` is `Some`, add those chips, clear `text`, and `set_value("")`. Otherwise nothing.
- Per screen: the text lives in the screen's `TableView`. `AppShell.quick_filter_screen: Option<Screen>` records which screen the input shows (`None` after `start_session`); `render` (which has the window) calls `set_value(view.filter.text)` when it is not `Some(self.screen)`, then stores it. `set_value` emits no `Change`.
- Key: `actions!(k8sboard, [FocusQuickFilter])` and `KeyBinding::new("/", FocusQuickFilter, Some("AppShell && !Input"))` bound in `app_shell::bind_keys(cx)`, which `run` calls. The root `v_flex` in `AppShell::render` gets `.key_context("AppShell")` and `.on_action(..)` that focuses the input. `!Input` (the kit input's context) keeps `/` typable in every input, the YAML editor included (decision 13). Live check: `/` right after start, before any click. The shell also registers `cx.on_focus_lost` and focuses `window.focus_lost_restore_target(cx)` (else its own root handle), so `/` keeps matching after a focused input or editor leaves the tree; `app_shell_tests.rs` covers it in a headless window.

## Sort header (`table_layout.rs`)

```rust
/// The header label plus a sort arrow; the whole cell is the click target.
pub(crate) fn sortable_header(id: usize, column: &Column, sort: Option<SortDirection>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static, cx: &App) -> AnyElement;
```

- Replaces `header_cell` in the three delegates. Arrow: `IconName::SortAscending` / `SortDescending` (`size_3`, `muted_foreground`), only on the sorted column.
- The click handler is a plain closure holding `WeakEntity<AppShell>` (not `cx.listener`): it runs outside the table update, so `shell.update(.., |shell, cx| shell.cycle_sort(logical, cx))` may update the table again. All three delegates already hold `shell: WeakEntity<AppShell>`.
- `configure` keeps `sortable(false)`, so the kit draws no second icon.

## Column layout (`table_layout.rs`)

```rust
pub(crate) struct TableColumns { pub(crate) columns: Vec<Column>, logical: Vec<usize> }
impl TableColumns { pub(crate) fn logical(&self, col_ix: usize) -> Option<usize>; }
/// The visible columns; `flexible` can never be hidden. Spare width is shared by column weight.
pub(crate) fn layout_columns(specs: &[KindColumn], flexible: usize,
    table_width: Pixels, hidden: &BTreeSet<usize>) -> TableColumns;
```

Widths (as built): a `KindColumn` has a `width` it always keeps, a `weight` (0 = short fixed values: counts, IPs, ages) and an optional `max_width`. The width left over is shared among the weighted columns in proportion to their weights; a column stops at its max and the others take its share; width nobody can take stays empty. Name is weight 3, capped at 640; text-heavy columns (Message, Cause, Object, selectors, subjects) carry weights.

Cell text (`cell_truncation.rs`): text cells carry a tooltip with the full value. Object names and `namespace/name` paths are cut in the middle (tail kept, namespace head kept) to the characters the column holds (`mono_capacity`), with the end ellipsis as the fallback. When two rows of a column would read the same after that cut (`tls-assets-vmagent-…` / `tls-assets-vmalert-…` share a tail), the cut keeps the start instead (3/4 head, 1/4 tail); the delegate passes the other shown rows of the column as siblings. Name takes the spare width: Secrets Type is sized to `kubernetes.io/tls`, Pods Status to `CrashLoopBackOff`, and "Used by" is capped with `up_to`. The Nodes Name and Pods Node columns use the same cut (`plain_text`), with the other shown rows as siblings. With exactly one namespace in the scope (`NameScope::OneNamespace`), the Pods and kind Name cells drop the `namespace/` prefix (kept under `all` and several namespaces); the tooltip keeps the whole `namespace/name`, and Copy name is unchanged.

Pods and Nodes get `const POD_COLUMNS` / `NODE_COLUMNS: [KindColumn; _]` (today's widths); the `FIXED_WIDTH` consts go. Kinds: `kind_columns(kind) -> Vec<KindColumn>` (a `Name` spec of width 200 first for `NameColumn::Flexible`). `KindColumn` and `Align` derive `Clone, Copy`. `render_td` maps `col_ix` through `logical` and matches on the logical index.

## Columns ▾

- `Button::new("columns").small().ghost().label("Columns").dropdown_caret(true)` + `DropdownMenu`: one `PopupMenuItem::new(name).checked(!hidden)` per logical column except the flexible one; a click toggles it in `view.hidden` through `update_view`.
- Hiding the sorted column keeps the sort (rows stay ordered by it); the arrow is simply not shown.
- Placement is deliberate: right of the filter bar on every screen (W7), also on Nodes where W5 draws it in the header (decision 10).

## Empty states (`render_empty` of the three delegates)

| Case | Text |
|---|---|
| `view.total() == 0` | today's text (`No pods in {scope}`) |
| `total > 0`, no row passes | `No {plural} match the filters` + a small `Clear filters` button → `update_view(TableView::clear_filter)` and `set_value("")` |

## AppShell API (all go through `update_view`)

```rust
pub(crate) fn cycle_sort(&mut self, column: usize, cx: &mut Context<Self>);
pub(crate) fn add_chips(&mut self, chips: Vec<FilterChip>, cx: &mut Context<Self>);
pub(crate) fn remove_chip(&mut self, index: usize, cx: &mut Context<Self>);
pub(crate) fn toggle_unhealthy(&mut self, cx: &mut Context<Self>);
pub(crate) fn toggle_column(&mut self, column: usize, cx: &mut Context<Self>);
pub(crate) fn set_preset(&mut self, preset: Option<FilterPreset>, cx: &mut Context<Self>);
pub(crate) fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>);
```

## Launch flag

`--filter <text>` sets the start screen's quick filter (a `label:` text becomes chips, as on Enter). Usage text updated. Screenshots: `pods` with `--filter kube` and `--filter label:k8s-app=kube-dns`.
