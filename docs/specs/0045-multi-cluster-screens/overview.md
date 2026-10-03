# 0045 · Overview across clusters

[Back to index](README.md) · Step 2 · Modules: `overview.rs`, `overview_report.rs`, `node_heatmap.rs`, `workspace.rs`, `app_shell.rs`, `app_shell_view.rs`. Decisions 3, 5–7, 10.

## Input

```rust
/// One Live viewed cluster as Overview draws it. Single mode passes one.
pub(crate) struct OverviewSlot<'a> {
    pub(crate) row: RowContext,            // cluster, label, weak session (0027 decision 25)
    pub(crate) environment: Environment,   // for the chip
    pub(crate) live: &'a LiveCluster,
    pub(crate) board: &'a IssueBoard,
}
pub(crate) struct OverviewData<'a> {
    pub(crate) slots: &'a [OverviewSlot<'a>],   // was `live`, `board`, `row`; primary first, then display order
    pub(crate) window: ChangeWindow,
    pub(crate) dock: &'a WeakEntity<Dock>,
    pub(crate) is_multi: bool,
}
```

`render_list` (Overview arm) and `render_multi_body` build the slots from `view.slots()` whose phase is Live (`slot_row_context` + `profile.environment`). Primary first (`view.primary()`), the rest in slot order.

## Panels (W3)

| Panel | Single (today) | Multi |
|---|---|---|
| Needs attention (pin 1) | first `ATTENTION_ROWS` (6) of the board | first 6 of `merge_attention(slots)`; each row gets a cluster chip (`environment_badge` + label, muted) after the pill; count pill = `sum_summaries`; status = merged coverage (issues.md) |
| Capacity (pin 2) | `capacity_model(live)` rows | one block per slot: a heading row (`environment_badge` + label) then that slot's rows; the legend once |
| Nodes (pin 3) | one heat map; `{n} · colored by CPU`; `{k} NotReady` | one heat map per slot under a heading; subtitle and NotReady count summed |
| Recent changes (pin 4) | `recent_changes(ChangeInputs)` of the primary | entries of every slot merged newest first, each with the cluster chip; a slot whose feeds are loading or unavailable adds a muted `{label}: {text}` line |

```rust
/// Issues of every slot in board order: (severity, since, shown), ties by slot order. With the slot index.
pub(crate) fn merge_attention<'a>(slots: &[OverviewSlot<'a>]) -> Vec<(usize, &'a Issue)>;
/// Change entries of every slot, newest `at` first, ties by slot order.
pub(crate) fn merge_changes(per_slot: Vec<Vec<ChangeEntry>>) -> Vec<(usize, ChangeEntry)>;
```

The chip is drawn only when `is_multi`; single mode renders exactly as today (AC 11).

## Header and stats line

| Part | Single | Multi |
|---|---|---|
| Header count (`render_header`) | `headline_text(context, version, nodes)` | `{n} clusters` (the per-cluster headline goes into the export and the Capacity headings' tooltip) |
| Stats line (`render_overview_stats`) | `stats_line(live)` | `stats_line_of(&[&LiveCluster])`: ready/total nodes and pods summed over slots whose list is Ready (`—` while none is), namespaces summed |
| Interruption banner | primary pods/nodes | unchanged in single; in multi the 0027 per-slot banners cover it |

`stats_parts(live)` becomes `stats_parts(lives: &[&LiveCluster])`; single mode passes one, so `stats_text` keeps its output.

## Click-through and actions

Every handler takes the `ClusterObject` of its own slot (`slots[i].row.object(key)`) and calls `reveal_object`:

| Site | Today | Change |
|---|---|---|
| `attention_row` click, `attention_button` Open | `reveal_in_primary(key)` | `reveal_object(row.object(key))` |
| `attention_button` View logs | `data.row`, `data.live` (primary) | the issue's slot `row` and `live` (`logs_pod`, `logs_launch`, `LogOrigin::new(&row, connection)`) |
| `node_heatmap` cell | `reveal_in_primary(key)` | `node_heatmap(cells, &row, cx)` → `reveal_object(row.object(key))` |
| `change_row` target | primary | its slot's `row.object(target)` |
| `View all {n} issues →` | Issues screen | unchanged (Issues is merged) |

## Change feeds

`set_overview_visible(screen == Overview, cx)` on every slot (`show_screen`) and in `new_slot` (`is_overview` no longer checks `is_primary`). A slot that connects while Overview is shown starts its two feeds in `LiveCluster::start` as today. Cost in [budget.md](budget.md).

## Export (`export_overview_report`, `overview_report.rs`)

- Single: unchanged (`live_report`, file `overview-{context}`).
- Multi: `live_report` per Live slot (each starts with its own headline), joined by `\n\n---\n\n`; file name `export_file_name("overview-{n}-clusters", "md", now)`.
- The save is canceled when the viewed set changed while the dialog was open: compare the `entity_id`s of the slot sessions captured at start with the current ones (the single-mode check generalized).
- A slot that is not Live is listed as `{label}: not connected` at the top; the report holds no kubeconfig data (0021 rule).

## Empty and partial states

- Body shows when at least one slot is Live (`any_live`); otherwise the 0027 busy/error views.
- Panels list Live slots only; Connecting and Failed slots appear only in the per-slot banners.
- `Checking the cluster…` in Needs attention becomes `Checking the clusters…` in multi mode while no board summary is ready.
