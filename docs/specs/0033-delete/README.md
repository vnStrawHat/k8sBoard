# 0033 — Delete

Status: draft, amended after the advisor review (M1, S1–S5, N1–N3), HEAD `1c859ae`. **Mutating.** Step 1 sends no write. **Steps 2 and 3 send real deletes. Each needs the user's explicit approval before it merges (C3); step 3 needs its own because bulk delete widens the blast radius.** Requires: 0030 with the 0032 architect's amendments (`GuardedIntent.warnings`, `WriteOutcome.effect` / `WriteEffect`, `ObjectKind::{ALL, resource}`, the delete exception to the field manager); 0032's `checked_write` and `GuardedKind::Batch(BatchPlan)`; 0031 step 1 (lazy per-kind write checks); 0028 (Del); 0009 (row checks, selection bar). Secrets wait for 0016; 0027 is optional. Roadmap: gap plan 0033 (delete part); C1, C3, C8, C10; R1, R2.

## Goal

- **Delete** for every kind: the red last menu item, Del (and ⌘⌫ on macOS), the palette, and a `Delete…` selection-bar button for **multi-select**.
- A **uid precondition** on every delete. The uid is read when the action starts, so a recreated object with the same name is never deleted.
- **Propagation** (Background default, Foreground, Orphan) for kinds that own dependents.
- **Finalizer hints** before the delete (finalizers present, or already terminating) and after it (waiting for finalizers, or terminating through the grace period).
- The 0030 core and the 0032 batch, used for single and bulk deletes alike:
  - the lazy SSAR `delete {resource}` and the lock;
  - the Destructive tier: always a dialog; on PROD, type the **object name** (single) or the **cluster name** (bulk);
  - a dry-run of every object, `checked_write`, and one audit line per object.

## Non-goals

Restart pod (0032 or a follow-up), Evict (0034), force delete or a grace-period choice, removing finalizers, `deletecollection`, delete by selector, undo, optimistic row removal, and cross-cluster bulk.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Cluster: `WriteOperation::DeleteObject`, `DeletePropagation`, `WriteEffect::{Deleted, DeletionPending}`, `object_identity` (metadata GET), lazy `AccessCheck::Delete(kind)`, `ObjectKind::owns_dependents` | 1–4, 11 |
| 2 | App, single delete through a one-item `BatchPlan` with `BatchExtras::Delete`: menus, Del and ⌘⌫, palette, dialog extras, notices, audit, `--screen delete-confirm`. **Needs user approval** | 1, 2, 5–10 |
| 3 | App, multi-select: scope rule, selection bar `Delete…`, checked-row menu, `--screen delete-bulk-confirm`. **Needs user approval** | 1, 2, 5–10, 12 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions and rationale |
| [cluster-api.md](cluster-api.md) | step 1: operation, request shape, identity read, effects, errors, lazy RBAC |
| [delete-flow.md](delete-flow.md) | steps 2–3: entries, scope, plan, dialog, propagation, warnings, finalizer hints, batch use, notices, audit |
| [files-to-touch.md](files-to-touch.md) · [test-plan.md](test-plan.md) | files per step; tests and checks |

## Acceptance criteria

- [ ] 1. Quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`; `Cargo.lock` unchanged.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists and passes offline. No test talks to a real cluster.
- [ ] 3. Request shape: `DELETE {path}/{name}`, no query, JSON body `{"propagationPolicy":…,"preconditions":{"uid":…}}` (+ `"dryRun":["All"]`). No `fieldManager` and no `resourceVersion` precondition (0030 decision 14). The `propagationPolicy` string equals `DeletePropagation::as_str`.
- [ ] 4. A uid mismatch (409) reads `A new object with this name exists; nothing was deleted`. `WriteRequest::new` refuses an empty uid. A refused identity read stops before the dialog.
- [ ] 5. Delete is enabled only when shipped, the lazy `delete {resource}` check allows it, and the row's cluster is unlocked. Menus, Del/⌘⌫, the palette, and the selection bar agree.
- [ ] 6. Every delete opens the dialog. On TypeName tiers, a single delete types the object name and a bulk delete the cluster name. Danger primary; a held Enter never confirms.
- [ ] 7. Owner kinds show the propagation radio. Changing it rebuilds the batch items and reruns the dry-run. Other kinds send `Background`.
- [ ] 8. Finalizer hints before and after the commit. A pod reads `{label}: terminating (grace period)`.
- [ ] 9. One audit line per committed object (through `checked_write`): action `Delete`, object, `deleteOptions.propagationPolicy` and its value. No Secret value or server message of a Secret target.
- [ ] 10. UAT (debug build): Delete is disabled with `Not permitted: delete {resource}` (or the probe's real answer). A trace shows only GETs and SSAR POSTs. The ui-verifier finds no high-severity defect in `delete-confirm` and `delete-bulk-confirm`.
- [ ] 11. `object_identity` uses `get_metadata` and keeps only `uid`, `finalizers`, and `deletionTimestamp`.
- [ ] 12. Bulk uses the 0032 batch: one cluster, at most 50 objects, sequential dry-runs, and **all** items must pass (NotFound counts as "already gone" and is skipped). Commits go through `checked_write` and stop on `Blocked`. The notification is the source of truth even after the dialog closes.

## Open items

1. R2: no write-capable cluster. Commits are proven only by fake-transport tests.
2. 0030 open item 5 applies to the lazy checks: with scope All, namespace-only `delete` rights show as denied.
3. Roles that grant `delete` without `get` cannot delete through k8sBoard (no uid to pin). This is on purpose (decision 3).
