# 0034 · As built (steps 1, 2, 3a, 3b)

[Back to index](README.md). Where the code differs from the text of the other files, and why. Main had no `run_guarded`, `GuardedIntent`, or `BatchFailure` when this landed (0030, 0032, 0033 "As built"), so the spec's names map as follows.

| Spec | Code |
|---|---|
| `run_guarded(GuardedIntent)` | `AppShell::start_write(WriteIntent)`: confirm dialog, dry-run, `commit_write`, `checked_write` |
| `GuardedKind::Batch(BatchPlan { .. on_failure })` | `AppShell::start_batch(BatchIntent)`; bulk cordon builds `BatchExtras::None` with `on_failure: BatchFailure::Continue` |
| `taint_intent(..) -> Result<GuardedIntent, _>` | `taint_intent(&NodeScope, node, &NodeEdit, &[TaintRow]) -> Result<WriteIntent, SharedString>`; `label_intent` likewise; `NodeScope` is the node's `ClusterRef` and display name |
| `node_edits.rs` | `node_edits.rs` (pure: rows, intents, bulk plan) and `node_editor.rs` (the two dialogs, the header button, the Nodes bulk buttons) |
| `drain_run.rs` (state machine and driver) | `drain_run.rs` (pure) and `drain_driver.rs` (an `AppShell` child: start, driver loop, stop, Uncordon); `drain_writes.rs` builds the cordon and eviction `WriteIntent`s |
| `DockTab::Drain(Entity<DrainTab>)` | as specified (`drain_tab.rs`); `Dock::close_tab` refuses a running drain, `remove_tab` (release) does not |
| `AuditOutcome::{Drained, Stuck, Cancelled, Stopped}`, `drain_summary_entry` | as specified; `AuditIdentity` copies cluster, context, and user when the run starts, so a line written after the slot is gone still names them |
| `ActionGate::Mutating { checks, is_shipped }` | `Drain`: `[CreatePodEviction, PatchNodes]`; `EditTaints`, `EditLabels`, `Uncordon`: `[PatchNodes]` |

## Step 1 (cluster crate)

- `EvictPod`, `SetNodeTaints`, `SetNodeLabels` in `object_write.rs`; bodies and the validity rules in `node_maintenance_bodies.rs`. `WriteRequest::new` refuses an unsafe uid, an empty `resourceVersion`, a taint whose key is not a qualified name, whose value is not a label value, whose effect is not one of the three, or that repeats a key and effect, and a label edit with an invalid or repeated key or value. The three arms live in the exhaustive `checked_operation` (no catch-all), so a new operation must decide.
- The eviction is `create_subresource::<Value, Status>`; a body that is not `Success` is sorted by its `code` through `error_from_status` (a 201 `Failure` with code 500 is `Cluster`, with 429 `TooManyRequests`, with no code `Cluster`). 429 messages are the first cause for every operation (`first_cause_message`).
- Reads: `drain_pods(node)` (field selector, paged by the new `list_all_where`), `list_pod_disruption_budgets()`, `node_for_edit(name)`; node names are checked before they go in a path or selector. `NodeTaint.time_added` is new. `AccessCheck::CreatePodEviction` is last in `ALL` (54).

## Step 2 (bulk cordon, editors)

- Menu in W5 order; `Edit taints…` and `Edit labels…` are unbound key actions (`EditTaints`, `EditLabels`); `Uncordon` is a `ResourceAction` of the bulk button only (its key resolves to `Cordon`).
- The editor reads the node fresh, keeps `timeAdded`, shows managed rows (system taints, kubelet labels) disabled with the reason under the row, and enables `Review…` only with a change. Adding a `NoExecute` taint (a new key and effect) is `Destructive` with the warning. The typed name of an edit is the node name.
- A 409 on a taint edit offers Retry in the confirm dialog, which closes it and reopens the editor fresh with the notice (`ConfirmDialog::retry`). A label edit sends no `resourceVersion`.
- Bulk Cordon/Uncordon: `cordon_batch` over the ticked nodes of one cluster (at most 50); nodes already in the state go to `skipped`; all skipped reads `All selected nodes are already cordoned` (`schedulable`). The header `Edit labels` acts on one ticked node (`Tick one node` otherwise).
- Screens `node-taints-editor` and `node-labels-editor`.

## Step 3a (plan and dialog)

- `drain_plan.rs`: verdict table, rank per budget, blocker text, option counts and hints, preview order and texts, dry-run aggregate and line. A 429 dry-run is `PodCheck::Refused` (the server's words, not a failure); other errors fail the dry-run.
- `drain_dialog.rs`: reads on the node's own connection; cordon dry-runs start at once (they need no pod list), eviction dry-runs follow in list order, one request at a time; ticking an option dry-runs the pods it adds. `Cordon only` has its own aggregate (the cordons alone), so a refused eviction never holds it; it commits through `checked_write` under the `running_batches` guard. The body scrolls past 660 px; the reason a button is off sits outside the scroll area.
- Screen `drain-dialog` (24 pods to evict, 6 DaemonSet pods, a blocked budget, `Force` unticked so the reason shows).

## Step 3b (run, tab, audit)

- `DrainRun` takes `now` as a `Duration` since the start. `NextStep` has one step more than the spec, `Read(node)`: the re-read of the pods when a node starts (it also sets `node_started`). A pod the dialog already dry-ran (accepted or refused) is not dry-run again; a pod that arrived since gets one first. A pod that needs an option the user did not tick fails up front and makes the node stuck.
- The driver sends one request at a time (`checked_write`), sleeps at most 1 s so the countdown moves and Cancel is seen, and polls every 3 s. A blocked write (lock, switch, reconnect) stops the run with `{text}; drain stopped`.
- Audit: each cordon and each accepted or failed eviction commit writes its own line (action `Cordon` / `Evict`, the dialog note); 429 refusals write none. One `Drain` line per node reached: fields `evicted`, `refused`, `failed`, `skipped`, outcome `drained`, `stuck`, `cancelled`, or `stopped`. A run that ends while a node is current writes its line once, whoever ends it.
- Leaving: `LeavingWork.drains` lists `A drain on {cluster} will stop; its nodes stay cordoned`; a confirmed release stops the run (`stop_drains_of`, before the tab is removed) and writes the lines; a window close asks the same way (`main_window_may_close`) and, once confirmed, stops every run and writes the lines at once; `cleanup_for_quit` stops them without asking. The end notification is handed out once (`take_end_notice`).
- One drain per cluster (`Dock::has_running_drain`): the dialog, the key, and the bar button refuse a second. The Uncordon button of a finished tab opens the bulk Uncordon batch over the nodes the run cordoned.
- Screen `drain-progress` (the dock zoomed on a frozen-clock tab: 12 of 23 gone, three being deleted, one refused three times).

## Deviations

- **Drain gate has two checks.** `CreatePodEviction` first, then `PatchNodes`: a drain cordons first, and a user who cannot cordon would only learn it at the dry-run.
- **Eviction audit action is `Evict`**, so the per-commit lines differ from the `Drain` summary.
- **`skip_pdbs_is_disabled_with_reason`** has no window test (the kit exposes no text query): the disabled checkbox and its reason are checked on the `drain-dialog` screenshots.
- **Not built:** a resume after a restart (decision 36), bulk label editing (open item 2), `Skip PodDisruptionBudgets` (open item 1).

## Tests

`node_maintenance_bodies_tests`, `object_write_node_tests` (wire format, both 429 shapes, the 201 `Failure`, validation), additions to `node_tests`, `pod_drain_tests`, `access_review`, `disruption_budget` (cluster); `node_edits_tests`, `drain_plan_tests`, `drain_writes_tests`, `drain_run_tests` (simulated time), `drain_tab_tests`, `audit_log_tests` additions, `app_shell_node_edit_tests`, `app_shell_drain_tests` (the flow over two fake clusters, including a real run to `drained`, Cancel, release, quit, and held Enter), and additions to `resource_actions_tests`, `row_selection_tests`, `launch_options_tests`, `screenshot` (settle).

## UAT (2026-10-03, read-only, no `K8SBOARD_ALLOW_WRITES`)

- `--screen nodes-selected` (two nodes ticked): `Cordon`, `Uncordon`, `Drain…`, and `Delete…` of the selection bar and the header `Edit labels` are disabled (`v87-uat-nodes-selected-light`). The palette on a node reads `Drain · Not permitted: create pods/eviction` and `Edit taints · Not permitted: patch nodes` (`v87-uat-palette-drain-light`, `v87-uat-palette-taints-light`): the server denies both rights for the read-only account.
- Requests, counted from the app's own debug log over three screen runs (`tower::buffer` request lines and the `reviewing access` actions; nothing else was read from the log): **POST 159 (all SelfSubjectAccessReviews), GET 190 (lists and watches), PUT 0, PATCH 0, DELETE 0, POST to `/eviction` 0**, and no `write finished` line, which `ClusterConnection::write` logs for every write that is sent.
- Not run live: a drain, an eviction, a taint or label edit (R2: no write-capable cluster). Commits are proven by the fake-transport tests only.
