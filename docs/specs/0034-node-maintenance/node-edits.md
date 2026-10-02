# 0034 · Bulk cordon, Edit taints, Edit labels

[Back to index](README.md) · Step 2 · Modules: `node_edits.rs` (new) + `node_edits_tests.rs`, `resource_actions.rs`, `row_selection.rs`, `node_table.rs`. Decisions 9–14, 42. Wireframe: W5 menu, selection bar, header `Edit labels`.

## Bulk Cordon / Uncordon (W5 selection bar)

- Buttons `Cordon`, `Uncordon` build a 0032 `GuardedKind::Batch(BatchPlan { items, skipped, extras: BatchExtras::None, on_failure: BatchFailure::Continue })` of 0030 `SetNodeSchedulable` requests through `batch_intent`, one item per ticked node; action `Cordon`, risk `Change`.
- Nodes already in the target state go to `skipped` (`already cordoned` / `already schedulable`); all skipped → disabled `All selected nodes are already cordoned`.
- Every 0032 Batch rule applies: one cluster, ≤ 50, always a dialog, sequential dry-runs that must all pass, commits through `checked_write`, stop on `Blocked`, one audit line per node.
- The single-node menu item and key C stay exactly as 0030 shipped them.

## Menu (W5 order)

`Open node shell` (S) · `Cordon`/`Uncordon` (C) · `Drain…` (D) · separator · `Edit taints…` · `Edit labels…` · `View pods on node` · `View YAML` (Y) · separator · `Copy name`.

| Item | `ResourceAction` | Gate (`ObjectKind::Node`) |
|---|---|---|
| `Drain…` | `Drain` (exists) | `CreatePodEviction`; cordon right checked by the dialog dry-run |
| `Edit taints…` | `EditTaints` (new) | `PatchNodes` |
| `Edit labels…` | `EditLabels` (new) | `PatchNodes` |

The W5 header button `Edit labels` acts on the single ticked node; otherwise disabled `Tick one node` (bulk labels: open item 2).

## Shared editor dialog

Both editors are a kit `Dialog` (width 560) that first calls `node_for_edit(name)` on the runtime (spinner `Loading node…`; failure → the error and `Close`). Rows are editable inputs; `+ Add` appends an empty row; `✕` removes a row. Footer: `Cancel` · `Review…` (primary). `Review…` closes the editor and calls `run_guarded(intent)`, so the 0030 confirm dialog (tier, dry-run, typed name, note) follows. No change → `Review…` disabled `No changes`.

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
- 409 → 0030 Conflict surface; `Retry` re-reads the node and reopens the editor **fresh** (the user's rows are dropped) with the notice `The node changed; review the current taints and edit again`. Re-applying old rows over a changed list could resurrect a removed taint.

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
