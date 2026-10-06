# 0034 · Bulk cordon, Edit taints, Edit labels

[Back to index](README.md) · Step 2 · Modules: `node_edits.rs` (new) + `node_edits_tests.rs`, `resource_actions.rs`, `row_selection.rs`, `node_table.rs`. Decisions 9–14, 42. Wireframe: W5 menu, selection bar, header `Edit labels`.

## Bulk Cordon / Uncordon (W5 selection bar)

- Buttons `Cordon`, `Uncordon` build a 0032 `GuardedKind::Batch(BatchPlan { items, skipped, extras: BatchExtras::None, on_failure: BatchFailure::Continue })` of 0030 `SetNodeSchedulable` requests through `batch_intent`, one item per ticked node; action `Cordon`, risk `Change`.
- Nodes already in the target state go to `skipped` (`already cordoned` / `already schedulable`); all skipped → disabled `All selected nodes are already cordoned`.
- Every 0032 Batch rule applies: one cluster, ≤ 50, always a dialog, sequential dry-runs that must all pass, commits through `checked_write`, stop on `Blocked`, one audit line per node.
- The single-node menu item and key C stay exactly as 0030 shipped them.

## Menu (W5 order)

`Open node shell` (S) · `Cordon`/`Uncordon` (C) · `Drain…` (D) · separator · `Edit taints…` · `Edit labels…` · `View pods on node` · `View YAML` (Y) · separator · `Copy name`; the 0027 `Filter by this cluster` tail stays (multi mode). On main `node_menu` reads: Open node shell, View YAML, View pods on node, separator, Cordon, Drain, separator, Copy name. Mutating items are `action_item`s without `on_click`; their arms in `run_available_row_key` open the dialogs for the cursor node (0032 decision 31).

| Item | `ResourceAction` | Gate (`ObjectKind::Node`) |
|---|---|---|
| `Drain…` | `Drain` (exists) | `CreatePodEviction`; cordon right checked by the dialog dry-run |
| `Edit taints…` | `EditTaints` (new) | `PatchNodes` |
| `Edit labels…` | `EditLabels` (new) | `PatchNodes` |

The W5 header button `Edit labels` acts on the single ticked node; otherwise disabled `Tick one node` (bulk labels: open item 2).

## Shared editor dialog

Both editors are a kit `Dialog` (width 560) that first calls `node_for_edit(name)` on the runtime (spinner `Loading node…`; failure → the error and `Close`). Rows are editable inputs; `+ Add` appends an empty row; `✕` removes a row. Footer: `Cancel` · `Review…` (primary). `Review…` closes the editor and calls `run_guarded(intent)`, so the 0030 confirm dialog (tier, dry-run, typed name, note) follows. No change → `Review…` disabled `No changes`. A fresh Enter in a row's key or value field presses `Review…` when it is enabled (a held Enter never does; a focused button or select keeps its own Enter), in the taint, label, and bulk label editors. The Drain dialog does the same on its own primary button, typed-name gate included.

Client checks (everything else is the server dry-run's job, shown as 0030 `Invalid { fields }`): key not empty; no duplicate key (labels) or key + effect (taints).

## Edit taints

| Column | Control |
|---|---|
| Key | `Input` |
| Value | `Input` (optional) |
| Effect | `Select`: `NoSchedule`, `PreferNoSchedule`, `NoExecute` |

- **Read-only rows**: keys starting with `node.kubernetes.io/` or `node.cloudprovider.kubernetes.io/` (node lifecycle, cordon, cloud init). Shown muted with `Managed by Kubernetes`; never removable; sent back unchanged with their `timeAdded`.
- Intent: `SetNodeTaints { taints: <all rows in order>, resource_version: NodeEdit.resource_version }`. Label `Edit taints of node wk-04`.
- Risk `Destructive` when a taint with effect `NoExecute` is **added** (pods without a toleration are evicted at once); `warnings`: `NoExecute evicts pods that do not tolerate it`. Otherwise `Change`.
- 409 → 0030 Conflict surface; `Retry` re-reads the node and reopens the editor with the user's rows kept and a notice of what changed on the node (see Walk follow-ups below). The node's managed taints are taken from the fresh read, so a stale copy cannot be sent again.

## Edit labels

| Column | Control |
|---|---|
| Key | `Input` |
| Value | `Input` |

- **Read-only rows** (kubelet re-sets them): keys starting with `kubernetes.io/`, `beta.kubernetes.io/`, `topology.kubernetes.io/`, or `node.kubernetes.io/`. Muted with `Set by the kubelet`. `node-role.kubernetes.io/*` stays editable (W5 Roles column).
- Intent: `SetNodeLabels { changes }`: set for added or changed keys, `None` for removed keys; unchanged keys are not sent. No `resourceVersion` (per-key merge, 0030 decision 14). Label `Edit labels of node wk-04`; risk `Change`.

```rust
pub(crate) fn taint_intent(cluster: &ClusterRef, node: &str, edit: &NodeEdit, rows: &[TaintRow]) -> Result<GuardedIntent, SharedString>;
pub(crate) fn label_intent(cluster: &ClusterRef, node: &str, edit: &NodeEdit, rows: &[LabelRow]) -> Result<GuardedIntent, SharedString>;
pub(crate) fn is_system_taint(key: &str) -> bool;
pub(crate) fn is_kubelet_label(key: &str) -> bool;
```

The node row and drawer update from the nodes watch; no optimistic UI.

## Walk follow-ups (G9, G10, G11)

- **Row problems (G9).** A key or value the write path refuses is named by row: `Row 4: key 'bad key!' is not a valid Kubernetes key` (or `value '…' is not valid`), the offending input gets a danger border, and the hint `optional prefix/ then name: letters, digits, - _ ., max 63` shows under the line. Each row is judged on its own by the same `WriteRequest` check (`taint_row_problem`, `label_row_problem`). A managed key cell cut with an ellipsis shows its full text in a tooltip.
- **Conflict (G10).** Retry after a 409 on the taint change reopens the editor with the user's rows kept and the node's managed taints as they are now (`rows_after_conflict`). The notice lists what changed on the node since the editor read it (`The node changed (by someone else): added …, removed …, a became b`); the first read is held in `AppShell::taint_base` from Review until the retry.
- **Node shell (G11).** The confirm lists `metadata.namespace → {namespace}` before the pod's own fields (also in the audit line). The red warning shows in the confirm only; the options dialog no longer repeats it.
