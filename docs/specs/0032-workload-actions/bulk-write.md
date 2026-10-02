# 0032 · Batch: the one bulk mechanism

[Back to index](README.md) · Step 2b · Modules: `write_flow.rs` (+ tests), `confirm_dialog.rs`, `row_selection.rs`, `value_popover.rs`. Decisions 19–25. Owned by 0032; used by 0033 (bulk delete, `BatchExtras::Delete`) and 0034 (bulk cordon). Builds on 0030 `run_guarded` and `checked_write` (0030 write-flow.md, decisions 30–36).

## Types

```rust
// 0030 GuardedKind gains: Batch(BatchPlan)
pub(crate) struct BatchPlan { pub(crate) items: Vec<BatchItem>, pub(crate) skipped: Vec<SkippedItem>, pub(crate) extras: BatchExtras,
    pub(crate) on_failure: BatchFailure }
/// What a failed commit does to the rest. `Continue`: 0032, 0033, 0034 (independent objects). `Stop`: 0032b Set default (ordered items).
pub(crate) enum BatchFailure { Continue, Stop }
pub(crate) struct BatchItem { pub(crate) object: SharedString /* "ns/name" */, pub(crate) label: SharedString, pub(crate) request: WriteRequest }
pub(crate) struct SkippedItem { pub(crate) object: SharedString, pub(crate) reason: SharedString }
pub(crate) enum BatchExtras {
    None,                                                     // 0032, 0034
    /// 0033 bulk delete (lands with 0033's step, so it is never dead code): a dry-run 404 moves the
    /// item to `already_gone` (shown as skipped), and a propagation change rebuilds the items.
    Delete { propagation: DeletePropagation, already_gone: Vec<SharedString>, warnings: Vec<SharedString> },
}
pub(crate) const MAX_BATCH_ITEMS: usize = 50;
/// A ticked row with its own cluster (0027 `ClusterObject`) and its `KindObject` or `NodeSummary`.
pub(crate) struct CheckedRow<'a> { pub(crate) cluster: &'a ClusterRef, pub(crate) object: CheckedObject<'a> }
/// Builds the `GuardedIntent { kind: Batch(plan), .. }` with `on_failure: Continue`; `Err` is the button's disabled reason.
pub(crate) fn batch_intent(rows: &[CheckedRow], action: ResourceAction, label: SharedString, risk: ActionRisk,
    build: impl Fn(&CheckedRow) -> Result<BatchItem, SkippedItem>) -> Result<GuardedIntent, SharedString>;
```

| `batch_intent` error | Reason |
|---|---|
| ticked rows of more than one cluster (0027) | `Select rows of one cluster` |
| more than 50 ticked rows | `Select at most 50 rows` |
| every row skipped | the first skip reason, e.g. `All selected cronjobs are already suspended` |

## Sequence (`run_guarded`, `Batch` branch)

1. Gate once with `guard_for(cluster)`; `confirm_step(guard.confirm, risk, expected)`. `expected` = `intent.expected_name` or the cluster display name; the list variant honours it (a single delete in 0033 types the object name). **A batch always opens a dialog**, like every 0030 tier (decision 9).
2. Dry-runs one item at a time, in list order, each `checked_write(WriteStep { mode: DryRun, .. })` (no audit). Rows show `…` / `passed` / the error.
3. **Every item must pass.** Apply is enabled only when all dry-runs passed and `commit_block` (aggregated state, typed name) is `None`; any failure keeps Apply disabled and the dry-run line names it. With `BatchExtras::Delete`, a 404 is not a failure: the item moves to `already_gone` and shows as skipped (0033).
4. An extras control change (0033: propagation) rebuilds `items` from the extras and restarts every dry-run; Apply waits again. `BatchExtras::None` has no control.
5. Commits one at a time, each `checked_write(WriteStep { intent: <item intent>, generation, mode: Commit { confirmed }, note })`, where the item intent is the batch's cluster, action, risk, and warnings with `label = item.label` and `kind: Write(item.request)`. So every commit runs `commit_block` and writes its own audit line.
   - `Write(err)` on one item → that item fails; with `BatchFailure::Continue` the next item continues, with `Stop` no further request is sent and the rest read `Not sent: an earlier step failed`.
   - `Blocked(text)` (lock, session switch, reconnect) → stop; the rest read `Not sent: {text}`.
6. One notification: `Restart: 4 done`, or `Restart: 3 done, 1 failed ({first error})`; `OutcomeUnknown` items count as `unknown`. With `Stop`: `{label}: stopped after {k} of {n}: {error}` plus `Retry`, which re-dispatches the same `ResourceAction` on the same row without `row_block` (the gate still applies), so the action's builder re-plans from current data and opens a new dialog (its warnings explain the state left behind).

- Closing the dialog during step 2 drops the dry-runs. During step 5 the commits run to the end (0030 decision 16).
- One request at a time: at most 50 short requests (about 5 s); gentle on the API server, no limiter code.

## Dialog (0030 confirm dialog, list variant)

| Part | Content |
|---|---|
| Title | env badge + `{label} on {cluster}?` (`Restart 4 deployments on prod-eu-1?`) |
| Object list | scrollable, max 240 px: `{namespace}/{name}` mono + dry-run state; skipped rows muted with their reason |
| Changes | the shared changed field once (`spec.replicas → 5`) |
| Warnings | `GuardedIntent.warnings` (0030) |
| Dry-run line | `Server dry-run passed for 4 of 4` (success) or `Dry-run failed for 1 of 4: {first error}` (danger) |
| Typed name, note, buttons | as 0030; primary = `{verb} {n}` (`Restart 4`); danger variant when `Destructive` |

## Selection bar (`row_selection.rs`)

`bulk_actions(screen)` returns `KindAction`s; each button is gated like the menu item (gate of the ticked rows' cluster, then `batch_intent`). Disabled → tooltip with the reason.

| Screen | Buttons → item per row |
|---|---|
| Deployments | `Scale…` (Scale popover, same N for all) · `Restart` · `Roll back…` (disabled: `Roll back one deployment at a time`) |
| StatefulSets | `Scale…` · `Restart` |
| DaemonSets | `Restart` |
| Jobs | `Re-run` |
| CronJobs | `Trigger now` · `Suspend` (reads `Resume` when every ticked row is suspended; otherwise suspends the rest, the suspended ones skipped) |

Rows refused by `row_block` (for example a paused Deployment on Restart) become `SkippedItem`s and are listed, not sent.
