# 0040 · Edit labels of several nodes

[Back to index](README.md) · Step 5 · Decisions 21–24. Wireframe: W5 header `Edit labels`, note 4 (multi-select), note 6. Modules: `node_edits.rs` (+ tests), `node_editor.rs`, `screenshot.rs`. Closes 0034 open item 2 and decision 13.

## Header button (`node_editor.rs`)

| Ticked nodes (one cluster) | Button |
|---|---|
| 0 | disabled `Tick nodes first` |
| 1 | the 0034 single editor (`open_node_editor(Labels, ..)`), unchanged |
| 2–50 | the bulk editor; tooltip `Edit the labels of the ticked nodes` |
| > 50 | disabled with the `ticked_nodes` text `Select at most 50 rows` (one cluster at a time, 0046; its `Select rows of one cluster` guard stays as written) |

Then, in order: `action_availability(EditLabels, guard_for(&cluster))` (`patch nodes`, lock), `A batch is running` while `running_batches` holds the cluster. `edit_labels_target` returns:

```rust
pub(super) enum LabelTarget { One { cluster: ClusterRef, node: String }, Several { cluster: ClusterRef, nodes: Vec<TickedNode> } }
```

`TickedNode` (0034) gains `labels: Vec<String>` (the `key=value` terms of `NodeSummary.labels`), filled by `ticked_nodes`.

## Bulk editor (kit `Dialog`, width 560)

| Part | Content |
|---|---|
| Title | `Edit labels of {n} nodes` |
| Muted line | `Changes apply to every ticked node; other labels stay.` |
| Rows | Key `Input` · `Select` `Set` / `Remove` · Value `Input` (hidden for Remove) · `✕`; `+ Add` appends an empty Set row; one empty row at open |
| Row error (under the row) | from `label_batch` checks below |
| Footer | `Cancel` · `Review…` (primary; disabled `No changes` with no non-empty row) |

No node read: label edits carry no `resourceVersion` (0034 decision 6) and the patch is per key, so the summaries are enough. `Review…` closes the editor, re-reads the ticked nodes **now** (`ticked_nodes`), builds `label_batch`, and calls `start_batch`; an `Err` shows as a notice `Edit labels is unavailable: {reason}`.

## Batch (`node_edits.rs`, pure)

```rust
/// One bulk edit over `nodes`: per node only the changes that are not already true. `Err` is why it cannot go.
pub(crate) fn label_batch(scope: &NodeScope<'_>, nodes: &[TickedNode], changes: &[LabelChange]) -> Result<BatchIntent, SharedString>;
```

Checks, first failure wins (texts of the 0034 `label_intent` where they exist):

| Check | Text |
|---|---|
| no change | `No changes` |
| empty key | `Enter a key for every label` |
| a key twice | `{key} is listed twice` |
| `is_kubelet_label(key)` | `{key} is set by the kubelet` |
| `WriteRequest::new` refuses | `A key or value is not valid for Kubernetes (letters, digits, - _ .)` |
| every node skipped | `All selected nodes already have these labels` |

Per node: a `Set` whose value already equals the node's, and a `Remove` of a key the node lacks, are dropped; a node with nothing left goes to `skipped` (`already labelled`). The rest:

| `BatchIntent` field | Value |
|---|---|
| `action`, `risk` | `EditLabels`, `Change` |
| `label` | `Edit labels of {k} nodes` (k = items) |
| `verb`, `button` | `Edit labels`, `Edit labels` |
| `expected_name` | `None`: TypeName types the cluster name (a bulk names no single object) |
| `warnings` | when any change is a Remove: `Removing a label can make DaemonSets that select nodes by it delete their pods on these nodes` (decision 24) |
| item | `object` = node name, `label` = `Edit labels of node {name}`, request `SetNodeLabels { changes }` (that node's own list, sorted by key) |
| plan | `skipped`, `BatchExtras::None`, `BatchFailure::Continue` |

## Batch behavior (0032 / 0033 rules, unchanged)

- The 0030 confirm dialog in its list variant: one row per node with its dry-run state; skipped nodes muted with `already labelled`.
- Dry-runs one at a time, all must pass; commits one at a time through `checked_write`; a `Blocked` result stops the rest; a per-node error goes on; the notice is the 0032 `batch_notice` (`Edit labels 3 of 3`, `, 1 failed (…)`).
- One audit line per committed node: action `Edit labels`, field `metadata.labels` = `team=infra; -old-key` (0034 format).
- The nodes table updates from the watch; no optimistic UI.

## Screen

`--screen node-labels-bulk-editor`: three fixture nodes ticked, rows `team = infra` (Set) and `old-key` (Remove), `Review…` enabled.
