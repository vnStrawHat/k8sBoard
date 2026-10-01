# 0009 · App: table view, filter, sort

[Back to index](README.md) · Step 2 · Column layout is in [filter-bar.md](filter-bar.md) · Modules: `table_view.rs` (new), `table_filter.rs` (new), `table_sort.rs` (new), `pod_table.rs`, `node_table.rs`, `kind_table.rs`, `table_selection.rs`, `app_shell.rs`

"Logical column" below = an index into the table's full column list (Pods 0..6, Nodes 0..7, kinds: `Name` first when shown, then `kind.columns()`), before hiding.

## Rows (`table_view.rs`)

```rust
/// What the toolkit reads from a row. Implemented next to each delegate.
pub(crate) trait TableRow {
    fn namespace(&self) -> Option<&str>;
    fn name(&self) -> &str;
    fn labels(&self) -> impl Iterator<Item = &str>;   // `key=value` terms
    fn tone(&self) -> StatusTone;
    fn value(&self, column: usize) -> CellValue<'_>;
    fn in_preset(&self, preset: &FilterPreset) -> bool;
}
pub(crate) enum CellValue<'a> {
    Text(Cow<'a, str>),                                   // natural order
    Qualified { prefix: Option<&'a str>, text: &'a str }, // prefix, then text
    Number(i64),
    Status { tone: StatusTone, text: SharedString },
    Age(Option<jiff::Timestamp>),                          // ascending = youngest first
    Span { started: Option<jiff::Timestamp>, finished: Option<jiff::Timestamp> },
    Absent,
}
```

| Row | `value` per column | `tone` | `in_preset` |
|---|---|---|---|
| `PodSummary` | Name `Qualified{ns, name}`; Status `Status`; Ready `Text(owned "3/4")`; Restarts `Number`; Node `Text` or `Absent`; Age `Age` | `pod_status_label(pod).tone` | always true |
| `NodeSummary` | Name `Text`; Status `Status`; Roles `Text(joined)` or `Absent`; Taints `Text(first)` or `Absent`; Version `Text`; Internal IP `Text`/`Absent`; Age | `node_status_label(..).tone` | `node_in_group` ([screen-filters.md](screen-filters.md)) |
| `KindRow` | Name `Qualified{namespace, name}`; cells: `Text`/`Mono` → `Text`, `Qualified` → `Qualified`, `Toned` → `Status`, `Absent`, `Age` → `Age`, `Duration` → `Span` | `row.status.tone` | `HideInactive` → `tone != Done` |

## View

```rust
pub(crate) struct TableView {
    pub(crate) filter: TableFilter,
    pub(crate) sort: Option<TableSort>,
    pub(crate) hidden: BTreeSet<usize>,          // logical columns
    default_filter: TableFilter,                 // ReplicaSets: preset HideInactive (decision 26)
    rows: Vec<usize>,                            // item indices, display order
    total: usize,
}
impl TableView {
    pub(crate) fn new(default_filter: TableFilter) -> Self; // filter starts at the default
    pub(crate) fn rebuild<T: TableRow>(&mut self, items: &[T], column_count: usize, now: jiff::Timestamp);
    pub(crate) fn rows(&self) -> &[usize];
    pub(crate) fn item_index(&self, row: usize) -> Option<usize>;
    pub(crate) fn row_of(&self, item: usize) -> Option<usize>;   // linear
    pub(crate) fn total(&self) -> usize;
    pub(crate) fn is_filtering(&self) -> bool;   // text, chips, or preset
    pub(crate) fn clear_filter(&mut self);       // empty filter ("Clear filters"); keeps sort, hidden
    pub(crate) fn reset_filter(&mut self);       // the default filter (context switch)
    pub(crate) fn add_chips(&mut self, chips: Vec<FilterChip>); // skips chips already present
}
pub(crate) fn default_filter(screen: Screen) -> TableFilter; // HideInactive for ReplicaSets only
```

Step split (dead-code rule): `FilterPreset`, `in_preset`, `default_filter`, `TableView::new`, and `reset_filter` land in step 3 with their first users; in step 2 the view is `Default` and a context switch calls `clear_filter`.

`rebuild`: keep the items that `matches` (below), then, with a sort, compute each kept row's key once and stable-sort `(key, index)` pairs with `compare_values`.

## Filter (`table_filter.rs`, pure)

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TableFilter { pub(crate) text: String, pub(crate) chips: Vec<FilterChip>,
    pub(crate) preset: Option<FilterPreset> }
pub(crate) enum FilterChip { Unhealthy, Equals { column: usize, title: &'static str, value: SharedString },
    Label(LabelQuery) }
pub(crate) enum FilterPreset { HideInactive, Nodes(NodeGroup) }
pub(crate) struct LabelQuery { pub(crate) key: String, pub(crate) test: LabelTest }
pub(crate) enum LabelTest { Exists, Equals(String), NotEquals(String) }
pub(crate) fn parse_label_queries(text: &str) -> Option<Vec<LabelQuery>>;
pub(crate) fn matches<T: TableRow>(row: &T, filter: &TableFilter, column_count: usize) -> bool;
pub(crate) fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool;
```

| Part | Passes when |
|---|---|
| `text` (trimmed; empty passes) | found in namespace, name, `{namespace}/{name}`, any column's `Text`/`Qualified`/`Status` text, or any label term |
| `Unhealthy` | tone is Warn, Bad, or Info |
| `Equals` | `value(column)` is `Text` or `Qualified.text` equal to `value` |
| `Label` | `Exists`: a term `key=…`; `Equals(v)`: term `key=v`; `NotEquals(v)`: no term `key=v` (kubectl `!=` semantics) |
| `preset` | `row.in_preset(preset)` |

`parse_label_queries`: requires the `label:` prefix; splits on `,`; each part `k=v`, `k!=v`, or `k`; trims; an empty key or empty part → `None`. Chips label: `label:{k}={v}`, `label:{k}!={v}`, `label:{k}`. `add_chips` skips duplicates.

## Sort (`table_sort.rs`, pure)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SortDirection { Ascending, Descending }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TableSort { pub(crate) column: usize, pub(crate) direction: SortDirection }
pub(crate) fn next_sort(current: Option<TableSort>, column: usize) -> Option<TableSort>;
pub(crate) fn compare_values(a: &CellValue, b: &CellValue, direction: SortDirection,
    now: jiff::Timestamp) -> Ordering;
pub(crate) fn natural_cmp(a: &str, b: &str) -> Ordering;
```

- `next_sort`: another column → Ascending; same Ascending → Descending; Descending → `None` (source order).
- `compare_values`: `Absent` (and `Age(None)`, `Span{started: None}`) after everything, in both directions; otherwise compare, then reverse for Descending. Status: rank Ok 0, Done 1, Info 2, Warn 3, Bad 4, then natural text. Age: later timestamp first. Span: elapsed `finished.unwrap_or(now) - started`. Qualified: prefix (None first), then text, both natural. Mixed variants: by variant order.
- `natural_cmp`: digit runs by numeric value (leading zeros ignored, then length), other runs by ASCII-lowercase, then plain byte order as the final tiebreak.

## Delegate wiring

```rust
/// What AppShell needs from any toolkit table.
pub(crate) trait FilteredTable: TableDelegate {
    fn view(&self) -> Option<&TableView>;          // None: kind table without a kind
    fn view_mut(&mut self) -> Option<&mut TableView>;
    /// Reads the session items, calls `rebuild`, and re-runs `layout_columns` with the last
    /// width; returns whether the columns changed (the caller then calls `refresh`).
    fn rebuild_view(&mut self, cx: &App) -> bool;
}
```

- `rows_count` = `view.rows().len()`; every row index (render, `SelectRow`, `context_menu`) goes through `item_index`. Kind table: `views: HashMap<ResourceKind, TableView>`, entry `TableView::new(default_filter(..))` created on `set_kind`.
- `AppShell::update_view(cx, f: impl FnOnce(&mut TableView))` on the visible table: `f`, `rebuild_view` (+ `refresh` when columns changed), then `sync_selection`, `cx.notify()`. Every toolkit action goes through it.
- `on_session_changed` and `show_screen` call `rebuild_view` on the visible table before `sync_selection`.
- `table_selection::list_row_index(list, view, is_selected)` searches `view.rows()`; a hidden subject yields `Some(None)` → `Clear` (decision 12). `apply_pending_launch_screen` maps its item to a row with `row_of`.
- `start_session` calls `reset_filter` on every view (decision 11) and sets `quick_filter_screen = None`.
