# 0045 · Issues across clusters

[Back to index](README.md) · Step 1 · Modules: `issue_table.rs`, `app_shell.rs`, `app_shell_view.rs`, `workspace.rs`, `title_bar.rs`, `navigation.rs`, `issue_board.rs`, `cluster_session.rs` (test seam). Decisions 1–4, 7, 10, 11.

## Data: nothing new to fetch

Each `ClusterSession` already owns an `IssueBoard` fed by its own pods, nodes, and `IssueFeeds` (`LiveCluster::start` starts them on every slot). Only two flags were primary-only:

| Today (`AppShell::show_screen`) | Change |
|---|---|
| `set_issues_visible(slot.is_primary && screen == Issues)` | `set_issues_visible(screen == Issues)` on every slot |
| `set_overview_visible(slot.is_primary && screen == Overview, cx)` (and `new_slot` in `app_shell_view.rs`) | every slot (step 2, [overview.md](overview.md)) |

## Table (`IssueTableDelegate`)

```rust
pub(crate) struct IssueTableDelegate {
    sessions: Vec<SlotSession>,          // was `session: Option<SlotSession>` (primary)
    addresses: Vec<RowAddress>,          // was `&[]`; FilteredTable::addresses returns it
    dock: WeakEntity<Dock>, shell: WeakEntity<AppShell>, layout: TableLayout, view: TableView,
}
impl IssueTableDelegate {
    pub(crate) fn set_sessions(&mut self, sessions: Vec<SlotSession>); // pod_table pattern: drop_column + replace_plan
    fn slot_issues<'a>(&self, cx: &'a App) -> Vec<&'a [Issue]>;        // empty slice for a slot that is not live
    fn issue_at<'a>(&self, row_ix: usize, cx: &'a App) -> Option<(&SlotSession, &'a Issue)>;
}
fn issue_plan(is_multi: bool) -> ColumnPlan;   // ISSUE_COLUMNS, `.with_cluster_column()` in multi mode
/// Stable sort of the merged rows (and their addresses, in step) into board order:
/// (severity, since, shown), ties by slot order.
fn sort_in_board_order(merged: &mut Vec<Clustered<'_, Issue>>, addresses: &mut Vec<RowAddress>);
```

- `rebuild_view`: `merge_rows(&[SlotRows { cluster, label, items: board.issues() }], ISSUE_COLUMNS.len())`, then `sort_in_board_order`, then `TableView::rebuild(&merged, ..)`; keep `addresses`. A user sort on a column still wins (TableView sorts after).
- `render_td`: `session_column == Some(logical)` → `cluster_cell(slot.environment, &slot.label, cx)` (`table_layout.rs`).
- `context_menu`: menu items use the row's own `slot.row_context(cx)` and `live` (View logs reads that slot's pods and connection); add `with_cluster_filter(menu, &row, shell)` (`resource_actions.rs`) so multi mode gets `Filter by this cluster`.
- `sync_view_sessions` (`app_shell_view.rs`): `issue_table.set_sessions(sessions.clone())` like the other tables; `table.refresh(cx)`.

## Click-through

```rust
impl AppShell {
    /// The object the issue at table row `row` opens, in the cluster its row came from.
    fn issue_object(&self, row: usize, cx: &App) -> Option<ClusterObject>;
}
```

- `reveal_issue(row)` → `issue_object` → `reveal_object`. `open_item` (row menu `Open {kind}`) captures `slot.row_context(cx).object(target)` and calls `reveal_object`.
- `reveal_in_primary` is deleted. Its callers: `issue_table.rs` (`open_item`), `overview.rs` (3 sites, step 2), `node_heatmap.rs` (step 2), `reveal_first_issue` (`app_shell.rs`, launch only: `primary_object(Some(target))` + `reveal_object`). Step 1 keeps the Overview and heat-map call sites compiling by passing the primary `ClusterObject` until step 2 replaces them.

## Counts

| Place | Single (today) | Multi |
|---|---|---|
| Title-bar flag (`title_bar.rs::issues_button`) | primary `summary()` | `sum_summaries` over slots whose board summary is `Some`; `None` (spinner tooltip) while none is ready |
| Flag tooltip (`issues_tooltip`) | `4 issues (1 critical)` | first line the sum, then one line `{label}: {n} issues ({c} critical)` per ready slot; a slot still checking reads `{label}: checking…` |
| Sidebar (`navigation_counts`) | `issue_counts(primary board)` | `issue_total` summed; per-screen `issue_counts` merged by `Screen` (sum, worst severity); `SlotCounts` gains `issues: Option<usize>` for the tooltip |
| Issues header (`render_header`) | `{total} issues` | `multi_header_count` gets an `Issues` arm: `{n} clusters · {total} issues` |
| Coverage status (`render_issues_status`) | primary `coverage()` | `Partial coverage` when any slot is partial; tooltip lines `{label}: {note}` |

```rust
// issue_board.rs
pub(crate) fn sum_summaries(summaries: impl Iterator<Item = Option<IssueSummary>>) -> Option<IssueSummary>;
// navigation.rs
pub(crate) fn merge_issue_counts(per_slot: impl Iterator<Item = Vec<ScreenIssues>>) -> Vec<ScreenIssues>;
```

## Body and slot states

- `render_multi_body`: the Overview/Issues branch that drew the primary alone is removed; Issues uses `render_issues` with merged summaries. Busy (`Checking the clusters…`) until some Live slot has a summary; the empty text names partial coverage as today.
- Failed, Connecting, and interrupted slots keep the 0027 banners (`render_slot_notices`); the `Showing {label} only` notice is removed for Issues and Overview.
- `issue_summary` and `render_issues_status` read all slots, not `self.session()`.

## Keys and cursor

Issues has no cursor row (arrow keys only move the highlight, 0020). After a click-through the cursor is a `ClusterObject` on the target screen, so every `RowAction` key runs through `run_row_key` → `slot_live(&subject.cluster)`. No key handler changes.
