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
- Intent: `SetNodeTaints { taints: <all rows in order>, resource_version: NodeEdit.resource_version, previous: NodeEdit.taints }`. `previous` is never sent: it lets the confirm and the audit line name each taint that changed, one line each: `conflict 4 → 3` (new value, same key and effect), `− workload=data:NoSchedule` (removed), `+ maintenance=true:NoSchedule` (added), changed first, then removed, then added (L2, L10). Label `Edit taints of node wk-04`.
- Risk `Destructive` when a taint with effect `NoExecute` is **added** (pods without a toleration are evicted at once); `warnings`: `NoExecute evicts pods that do not tolerate it`. Otherwise `Change`.
- 409 (on the dry-run or the commit) → no confirm step with the server's words: the confirm closes and the editor reopens on the node as it is now, with the notice (see Walk follow-ups below; L3). The node's managed taints are taken from the fresh read, so a stale copy cannot be sent again.

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
- **Conflict (G10, L2, L3).** A 409 on the taint change reopens the editor at once (`ConfirmDialog::reload_after_conflict`), no Retry step. Rows are merged against the taints the editor first read (`rows_after_conflict`, held in `AppShell::taint_base` from Review): a row the user did not touch takes the node's value (or goes if the node dropped it), an edited or added row is kept as typed, a removed row stays removed, a taint new on the node joins the rows, and the node's managed taints are as they are now. So the retry never puts back what someone else changed. The notice lists what changed on the node (`The node changed (by someone else): added …, removed …, a became b. Rows you did not touch follow the node; your edits are kept. Review before applying.`); the next Review shows the diff against the node as it is now.
- **Node shell (G11).** The confirm lists `metadata.namespace → {namespace}` before the pod's own fields (also in the audit line). The red warning shows in the confirm only; the options dialog no longer repeats it.
- **Add and closing (L8, L9).** `+ Add` puts the new row at the top of the taint or label list and the cursor in its key field, so typing goes into it. The label editor folds the kubelet labels into one `{n} kubelet labels ▸` row under the editable ones (a click opens them). A row with no key is named (`Enter a key for every label`, danger border on its key) only after `Review…` was pressed, not while typing; `Review…` stays pressable for that. An outside click never closes an editor (taint, label, bulk label). `Cancel` and Esc close at once when no row changed (rows equal the node as read; for the bulk editor, no typed change); otherwise `Discard changes?` opens with `Keep editing` and `Discard`.
- **Enter is Review… (Q4).** In a row's field, and with the focus nowhere in particular (the `✕` that was clicked goes with its row, so a removal gives the focus to the editor body: `focus_handle`), a fresh Enter presses `Review…`; a focused button or select keeps its own Enter. The editor never closes silently on Enter and drops its rows.
- **A bulk Review that cannot go stays in the editor (Q13).** `Review…` of the bulk label editor builds the batch before it closes: a key the kubelet owns, a label a DaemonSet selects by, nothing to change, or a selection that left the cluster is shown on the editor's problem line (red, under the rows) with the rows kept, instead of a toast after the editor closed.
- **Bulk label words (L17).** The bulk Edit labels confirm button reads `Apply to 2 nodes` (`Apply to 1 node`), and a batch that went through whole toasts `Set lab-batch on 2 nodes` and, for a removal, `Removed lab-batch from 2 nodes` (both, joined by `. `); a partial result keeps the generic counts. The DaemonSet warning names the DaemonSets (at most three, then `+N`) whose `nodeSelector` or required node affinity uses a key the batch removes from a node or changes there (`DaemonSet shop/logs selects nodes by team: its pods on these nodes are deleted`); a key only added to nodes lacking it, or a key no DaemonSet selects by, warns nothing. `DaemonSetSummary.node_affinity_keys` carries the affinity keys. When the cluster's DaemonSet feed is not loaded, or covers only one or two namespaces, a removal keeps the generic line `Removing a label can make DaemonSets that select nodes by it delete their pods on these nodes`.
