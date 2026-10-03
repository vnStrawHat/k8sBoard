# 0033 — Delete

Status: **steps 1–3 built 2026-10-03** (see [as-built.md](as-built.md)); amended after the advisor review (M1, S1–S5, N1–N3); refreshed 2026-10-03 against main `2c7dc08`. **Mutating.** Step 1 sends no write. **Steps 2 and 3 send real deletes. C3: Approved by the user on 2026-10-02 (one approval for all mutating specs).** Debug builds still block writes unless `K8SBOARD_ALLOW_WRITES=1` (agents never set it); UAT checks stay denied-path-only. Roadmap: gap plan 0033 (delete part); C1, C3, C8, C10; R1, R2.

**Prerequisites.** Merged: 0009 (ticks, selection bar), 0016, 0027, 0028 (Del offered on every subject), 0030 steps 1/2a/3. Before step 1: 0031 steps 0–1 (`get_object`, `ObjectKind::{ALL, resource}`, lazy `Update(kind)`, `review_access_for`). Before step 2: 0030 steps 2b + 4 (in flight: `run_guarded`, `checked_write`, `GuardedIntent.warnings`, `confirm_dialog.rs`), 0031 step 3 (session `kind_access`, `ClusterGuard.kind_access`), 0032 2a-i (`RowAction`) and 2b (`GuardedKind::Batch`, `checked_rows`). Lane W1, after 0031.

## Goal

- **Delete** for every kind: the red last menu item, Del (and ⌘⌫ on macOS), the palette, and a `Delete…` selection-bar button for **multi-select**.
- A **uid precondition** on every delete. The uid is read when the action starts, so a recreated object with the same name is never deleted.
- **Propagation** (Background default, Foreground, Orphan) for kinds that own dependents.
- **Finalizer hints** before the delete (finalizers present, or already terminating) and after it (waiting for finalizers, or terminating through the grace period).
- The 0030 core and the 0032 batch, used for single and bulk deletes alike:
  - the lazy SSAR `delete {resource}` and the lock;
  - the Destructive risk: always a dialog (as every 0030 tier); on PROD (`TypeName`), type the **object name** (single) or the **cluster name** (bulk); elsewhere click the danger button;
  - a dry-run of every object, `checked_write`, and one audit line per object.

## Non-goals

Restart pod (0032 or a follow-up), Evict (0034), force delete or a grace-period choice, removing finalizers, `deletecollection`, delete by selector, undo, optimistic row removal, cross-cluster bulk, custom resources (0018: no `ObjectKind`), and Helm releases (`Uninstall release…` is 0038, deferred).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Cluster: `WriteOperation::DeleteObject`, `DeletePropagation`, `WriteEffect::{Deleted, DeletionPending}`, `object_identity` (metadata GET), lazy `AccessCheck::Delete(kind)`, `ObjectKind::owns_dependents` | 1–4, 11 |
| 2 | App, single delete through a one-item `BatchPlan` with `BatchExtras::Delete`: menus, Del and ⌘⌫, palette, dialog extras, notices, audit, `--screen delete-confirm`. Approved by the user on 2026-10-02 (one approval for all mutating specs). | 1, 2, 5–10 |
| 3 | App, multi-select: scope rule, selection bar `Delete…`, checked-row menu, `--screen delete-bulk-confirm`. Approved by the user on 2026-10-02 (one approval for all mutating specs). | 1, 2, 5–10, 12 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions and rationale |
| [cluster-api.md](cluster-api.md) | step 1: operation, request shape, identity read, effects, errors, lazy RBAC |
| [delete-flow.md](delete-flow.md) | steps 2–3: entries, scope, plan, dialog, propagation, warnings, finalizer hints, batch use, notices, audit |
| [files-to-touch.md](files-to-touch.md) · [test-plan.md](test-plan.md) | files per step; tests and checks |
| [as-built.md](as-built.md) | where the code differs from the text, and why |

## Acceptance criteria

- [x] 1. Quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`; `Cargo.lock` unchanged.
- [x] 2. Every test of the step in [test-plan.md](test-plan.md) exists and passes offline. No test talks to a real cluster.
- [x] 3. Request shape: `DELETE {path}/{name}`, no query, JSON body `{"propagationPolicy":…,"preconditions":{"uid":…}}` (+ `"dryRun":["All"]`). No `fieldManager` and no `resourceVersion` precondition (0030 decision 14). The `propagationPolicy` string equals `DeletePropagation::as_str`.
- [x] 4. A uid mismatch (409) reads `A new object with this name exists; nothing was deleted`. `WriteRequest::new` refuses an empty uid. A refused identity read stops before the dialog.
- [x] 5. Delete is enabled only when shipped, the lazy `delete {resource}` check allows it, and the row's own cluster (`ClusterObject.cluster`, never the primary) is unlocked. Menus, Del/⌘⌫, and the palette run the one `Delete(_)` arm of `run_available_row_key`; the selection bar reads the same gate. Helm release and custom-kind rows keep their item disabled (`Comes in a later version`) and Del does nothing there.
- [x] 6. Every delete opens the dialog. On TypeName tiers, a single delete types the object name and a bulk delete the cluster name. On `Click` tiers the focused danger primary confirms by click or Enter; a held Enter never confirms.
- [x] 7. Owner kinds show the propagation radio. Changing it rebuilds the batch items and reruns the dry-run. Other kinds send `Background`.
- [x] 8. Finalizer hints before and after the commit. A pod reads `{label}: terminating (grace period)`.
- [x] 9. One audit line per committed object (through `checked_write`): action `Delete`, object, `deleteOptions.propagationPolicy` and its value. No Secret value or server message of a Secret target.
- [x] 10. (UAT part done 2026-10-03, see [as-built.md](as-built.md); the ui-verifier run is pending) UAT (debug build): Delete is disabled with `Not permitted: delete {resource}` (or the probe's real answer). A trace shows only GETs and SSAR POSTs. The ui-verifier finds no high-severity defect in `delete-confirm` and `delete-bulk-confirm`.
- [x] 11. `object_identity` uses `get_metadata` and keeps only `uid`, `finalizers`, and `deletionTimestamp`.
- [x] 12. Bulk uses the 0032 batch: one cluster, at most 50 objects, sequential dry-runs, and **all** items must pass (NotFound counts as "already gone" and is skipped). Commits go through `checked_write` and stop on `Blocked`. The notification is the source of truth even after the dialog closes.

## Open items

1. R2: no write-capable cluster. Commits are proven only by fake-transport tests.
2. 0030 open item 5 applies to the lazy checks: with scope All, namespace-only `delete` rights show as denied.
3. Roles that grant `delete` without `get` cannot delete through k8sBoard (no uid to pin). This is on purpose (decision 3).
4. Helm release rows: their kind spec reads `ObjectKind::Secret`, so a naive `Delete(builtin_object())` would target a Secret with the release's name. `subject_action(Delete, ..)` excludes `HelmReleases` and custom kinds explicitly (decision 26); `Uninstall release…` waits for 0038.
5. RBAC names with `:` follow 0031 decision 27 (path-segment rule for the four RBAC kinds), pending the user's answer to 0031 open item 6.
