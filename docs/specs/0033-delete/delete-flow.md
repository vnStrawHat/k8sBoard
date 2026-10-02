# 0033 · App: delete flow

[Back to index](README.md) · Steps 2 (single) and 3 (multi-select) · Modules: `object_delete.rs` (new) + `object_delete_tests.rs`, `write_flow.rs` (0032 batch: `BatchExtras::Delete` arm), `confirm_dialog.rs`, `resource_actions.rs`, `keyboard_navigation.rs`, `keymap.rs`, `row_selection.rs`, `launch_options.rs`, `screenshot.rs`. Decisions 9–25.

## Entry points (one gate)

| Entry | Targets (`delete_scope`) |
|---|---|
| Row and ⋯ menus: last item, danger, Del hint, `kind.delete_label()`; on a checked row with ≥ 2 checked: `Delete {n} {plural}…` | the row, or the checked set |
| Del, and ⌘⌫ on macOS (`cmd-backspace` → `Delete` in `WORKSPACE`, `#[cfg(target_os = "macos")]` binding list) | the cursor row, or the checked set when the cursor row is checked and ≥ 2 are checked |
| Selection bar (0009/0032): `Delete…` danger button, last before ✕, every screen (step 3) | the checked set |
| Palette (0029) `> Delete…` | the palette subject |

```rust
/// Pure. Which objects a delete covers (decision 21). `Err` is the disabled reason (0032 `bulk_intent` texts).
pub(crate) fn delete_scope(subject: &ClusterObject, checked: &[ClusterObject]) -> Result<Vec<ClusterObject>, SharedString>;
```

Errors reuse the 0032 texts: `Select rows of one cluster`, `Select at most 50 rows`. Gate: `action_availability(Delete, guard_for(cluster))` with the lazy `AccessCheck::Delete(kind)`, `mutates: true`, `ActionRisk::Destructive`. It is shipped in step 2 for single deletes; the bulk entries come in step 3. A disabled entry shows its reason; Del shows the 0028 notice.

## Building the batch

```rust
/// 0033's extras of the 0032 BatchPlan.
pub(crate) enum BatchExtras { /* 0032 … */ Delete { propagation: DeletePropagation, already_gone: Vec<SharedString>, warnings: Vec<SharedString> } }
pub(crate) struct DeleteTarget { pub(crate) object: ObjectRef, pub(crate) name: SharedString, pub(crate) identity: ObjectIdentity }
impl AppShell { pub(crate) fn start_delete(&mut self, scope: Vec<ClusterObject>, window: &mut Window, cx: &mut Context<Self>); }
fn delete_items(targets: &[DeleteTarget], propagation: DeletePropagation) -> Vec<BatchItem>;   // one DeleteObject each
fn delete_warnings(kind: ObjectKind, targets: &[DeleteTarget], live: &LiveCluster, now: Timestamp) -> Vec<SharedString>;
```

1. Gate first: a disabled gate sends no request.
2. `object_identity` for each target, **sequentially** on the runtime. 404 → `already_gone`. Any other error stops: `Could not read {name} to pin its uid ({error}); nothing was deleted` (decision 3).
3. Every target gone → notice `{name} was already deleted` / `All {n} objects were already deleted`. Otherwise `run_guarded(GuardedIntent { action: Delete, label, risk: Destructive, expected_name, kind: GuardedKind::Batch(BatchPlan { items, skipped: vec![], extras: BatchExtras::Delete { propagation: Background, already_gone, warnings }, on_failure: BatchFailure::Continue }), warnings, on_commit: None })`.
   - `label`: `Delete pod` (single) or `Delete 12 pods`.
   - `expected_name`: `Some(object name)` for a single delete, `None` (the cluster name) for bulk (decision 10).

## Batch behavior (0032 `run_guarded` Batch arm; delete specifics only)

- Dry-runs: one at a time, in order, through `checked_write(DryRun)`. **All** must pass. NotFound → moved to `already_gone`, not a failure. Any other error → that row shows it and Apply stays disabled.
- Propagation radio change → `items = delete_items(targets, new)`, `DryRunState::Running`, and the dry-runs restart.
- Commits: sequential through `checked_write(Commit)` (commit_block, send, audit per item). `Blocked` stops the loop; a per-item error goes on.
- **Progress is best-effort**: rows update while the dialog is open. Closing it never stops the loop (0030 decision 16), and the final notification is the source of truth.

## Dialog (0030 confirm dialog, 0032 list variant)

| Part | Content |
|---|---|
| Title | env badge + `{label} on {cluster}?` |
| Objects | the 0032 scrollable list with per-row dry-run state; `already gone: {names}` muted |
| Propagation (`kind.owns_dependents()`) | kit `RadioGroup` `Dependents`: `Delete in the background (default)` "{dependents} are deleted after the {kind}"; `Delete them first (foreground)` "the {kind} stays until {dependents} are gone"; `Keep them (orphan)` "{dependents} keep running without an owner". Dependents: Deployment `its ReplicaSets and pods`; StatefulSet, DaemonSet, ReplicaSet, Job `its pods`; CronJob `its Jobs and their pods` |
| Warnings (warning tone) | the table below, then the finalizer lines |
| Typed name | single: `Type the object name to confirm` (`api-x`); bulk: the cluster name |
| Buttons | `Back`, danger primary `Delete` / `Delete {n} of {m}` |

| Kind | Warning |
|---|---|
| Namespace | `Deletes every object in {name}` |
| Node | `Removes the node object; its pods are not drained first` |
| PersistentVolume | from `reclaim_policy`: `Delete` → `Reclaim policy Delete: the storage asset is deleted too`; `Retain` → none |
| PersistentVolumeClaim | its bound PV in the session's PV list (0014): `Delete` → `The bound volume {pv} has reclaim policy Delete: its data is deleted too`; PV not loaded → `If the bound volume's reclaim policy is Delete, its data is deleted too` |
| StatefulSet | `Volume claims stay unless the retention policy deletes them` |
| Pod without `controller` | `Not managed by a controller; it will not come back` (bulk: `{k} pods are not managed by a controller`) |

## Finalizer hints (no polling)

| When | Source | Text (names cut to 3 + `+{n}`) |
|---|---|---|
| dialog, finalizers, not terminating | identity | `Has finalizers: {names}. Deletion waits until their controllers remove them` |
| dialog, already terminating | `deletion_started` | `Already being deleted for {age}; waiting for finalizers: {names}. Deleting again does not remove them`, or `Already terminating for {age}` |
| after commit | `DeletionPending { finalizers }` non-empty | `{label}: marked for deletion; waiting for finalizers: {names}` |
| after commit | `DeletionPending` empty | pod: `{label}: terminating (grace period)`; other kinds: `{label}: terminating` |
| after commit | `Deleted` | `{label}: done` |

Bulk lines: `{k} objects have finalizers`, `{k} are already being deleted`. Bulk summary (0032 format plus delete parts): `Deleted {a} of {n}`, `, {p} waiting for finalizers`, `, {f} failed ({first})`, `, stopped: {reason}`. Single failures: Conflict → `A new object with this name exists; nothing was deleted`; NotFound → `{name} was already deleted`.

## Audit (through `checked_write`)

One line per committed object: `{"action":"Delete","object":{…},"fields":[{"path":"deleteOptions.propagationPolicy","value":"Background"}],"outcome":"applied"}`. `DeletionPending` counts as `applied`. The note is copied to each line. Dry-runs and skipped objects are not recorded.

## Screenshots (screenshot feature; no connection call)

| Screen | Fixture |
|---|---|
| `delete-confirm` | first Deployment, PROD TypeName tier (object name), propagation radio, one row `passed · 98 ms`, `Has finalizers: foregroundDeletion` |
| `delete-bulk-confirm` | Pods, 12 checked, STG Click tier, the list with `passed` rows, `2 pods are not managed by a controller`, danger `Delete 12 of 12` |
