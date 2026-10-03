# 0032 · As built (steps 2a-i to 2b)

[Back to index](README.md). Where the code differs from the text of the other files, and why. Main had no `run_guarded` or `GuardedIntent` when these steps landed (0030 "As built": one `WriteIntent`, `start_write`), so the spec's names map as follows.

| Spec | Code |
|---|---|
| `run_guarded(GuardedIntent)` | `AppShell::start_write(WriteIntent)`; a batch is `start_batch(BatchIntent)` |
| `GuardedKind::Batch(BatchPlan)` | `DialogKind::Batch(Rc<BatchIntent>)`; `BatchIntent` holds the cluster, action, label, verb, risk, warnings, and the `BatchPlan` |
| `batch_intent(rows, action, label, risk, build)` | `batch_plan(rows, build) -> Result<BatchPlan, reason>`; the caller (`workload_actions::bulk_intent`, `bulk_scale_intent`) builds the `BatchIntent` once it knows the count |
| `CheckedRow { cluster, object: CheckedObject }` | `CheckedRow { cluster, object: &KindObject }`; 0034 adds the node case |
| `BatchExtras`, `BatchFailure`, `BatchPlan.on_failure` | not built: `BatchExtras::None` and `BatchFailure::Continue` are the only values 0032 needs, and a one-variant enum is dead code. 0033/0032b add them with their first use |
| `expected_name` honoured by the list variant | not built: a batch types the cluster name; 0033 adds the object name for a single delete |
| `batch_*` code in `write_flow.rs` | `batch_write.rs` (child of `app_shell`): types, `start_batch`, `commit_batch`, the selection-bar glue (`bulk_buttons`, `run_bulk`, the bulk Scale popover) |
| `value_popover.rs` in `main.rs` | same; the shell side (`open_scale_popover`, `submit_scale`, `start_roll_back`) is in `write_flow.rs` because it reads private shell state |

## Behavior notes

- **Menu items** of a kind row have no `on_click` (decision 31). `row_availability` is the gate, then `row_block`; the palette applies the same two and flips labels from the row (`Resume rollout`).
- **Roll back…** (menu) opens the drawer on the Overview and scrolls to the Revisions on the next paint (`DrawerState::reveal_revisions`, a `ScrollHandle` over the drawer sections). It is off until the drawer has loaded the ReplicaSets (`Open the deployment to load its revisions`). The palette entry `Roll back to rev {n}` is a `PaletteTarget::RollBack` that carries the revision and starts the dialog itself (an argument-bearing item, decision 31), only while the revisions are loaded.
- **Scale popover** is a floating card at the bottom centre (above the selection bar while rows are ticked), not the kit `Popover`, which needs a `Selectable` trigger. It closes on Escape, Cancel, a cursor move, a screen change, or the palette opening.
- **Palette `Ctrl ⏎`** turns the query into the replicas field (`PaletteArgument` key context). The kit `Dialog` confirms on Enter and would close the palette before the number is read, so `enter` is bound to `NoAction` there and handled once on a fresh press, like the confirm dialog.
- **Batch**: the dry-runs and commits run one item at a time through `checked_write`; every item must pass its dry-run; a failed commit does not stop the next item; a `Blocked` result stops the rest (`Not sent: {reason}`). One notice at the end (`Restart: 3 done, 1 failed (…)`). Warnings of a batch are summaries (`OnDelete` once, `Scaling down k of n`, `k of them are managed by an HPA`); Trigger now lists no concurrency lines in a batch.
- **Screens** (screenshot builds): `scale-popover`, `scale-confirm`, `restart-bulk-confirm`, and `<plural>-menu` (the drawer ⋯ menu open), all against the real first rows of the cluster.
- The palette reads `KindObject` flags (paused, suspended) of the cursor row for labels and reasons; it still reads no cell text.
