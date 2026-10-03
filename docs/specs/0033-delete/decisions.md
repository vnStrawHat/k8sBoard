# 0033 · Decisions

[Back to index](README.md). Architect defaults, amended after the advisor review; "(decided)" marks coordinator decisions. 0030 and 0032 decisions apply unless replaced here.

## Request

| # | Decision | Rationale |
|---|---|---|
| 1 | One `DeleteObject { uid, propagation }` per object, also for bulk; no `deletecollection` | per-object uid, dry-run, audit; a collection delete takes a selector, not the ticked rows |
| 2 | The **uid comes from a metadata GET (`get_metadata`) when the action starts**, not from summaries | summaries carry no uid (adding it touches about 20 modules). The read happens when the user decides, so the precondition covers the time until Apply; it also gives finalizers and `deletionTimestamp` |
| 3 | **No delete without that GET.** A refused or failed identity read stops before the dialog: `Could not read {name} to pin its uid ({error}); nothing was deleted`. The precondition is `uid` only, never `resourceVersion` (0030 decision 14) | without a uid, a delete could hit a recreated object. A `resourceVersion` precondition would fail on every status update of a busy pod; `uid` pins the identity, which is what a delete means |
| 4 | `get_metadata`, never a full GET | a Secret's `data` is never fetched |
| 5 | Propagation is **always sent**, default `Background` | kubectl's default; `batch/v1` Jobs default to orphan through the API |
| 6 | No grace-period choice, no force delete | the wireframe draws none; force hides kubelet state |
| 7 | No `fieldManager`; `dryRun: ["All"]` goes in the body (kube-core `Request::delete`) | `DeleteOptions` has no field manager. The 0030 exception is written by the 0032 architect |
| 8 | Response: `Left(object)` with `deletionTimestamp` → `DeletionPending { finalizers }`; otherwise `Deleted` | kube-client: `Left` means "your delete has started" |

## Confirmation and UI

| # | Decision | Rationale |
|---|---|---|
| 9 | `ActionRisk::Destructive`: always a dialog (0030 decision 9) | "destructive actions always open a confirmation" |
| 10 | (decided) TypeName tier: a **single** delete types the **object name** (`expected_name: Some(name)`, 0030 decision 10); a **bulk** delete types the **cluster display name** | typing the object proves the right target; a bulk delete has no single name |
| 11 | The 0030 confirm dialog in its 0032 list variant (W10 small modal) with a danger primary | the wireframe draws no delete dialog |
| 12 | A propagation radio only for Deployment, StatefulSet, DaemonSet, ReplicaSet, Job, CronJob. A change rebuilds the batch items and reruns the dry-run | only owners have dependents |
| 13 | Warnings per kind (Namespace, Node, StatefulSet static; PV and PVC from the **reclaim policy** in the summaries); pods without a `controller` | data loss and non-returning pods are the costly surprises |
| 14 | **Finalizer hints without polling**: identity before, the response effect after. A pod with no finalizers reads `terminating (grace period)` | the response says whether the object stays |
| 15 | An already-terminating object can still be deleted (a no-op); the dialog says so | blocking adds no safety |
| 16 | Delete is offered for **every** kind, Events included | today's menus list it; 0028 offers Del on every subject |
| 17 | No optimistic UI | the watches are the source of truth |
| 18 | (decided) ⌘⌫ (`cmd-backspace`) → `Delete` in `WORKSPACE` on macOS, as well as Del | the Mac keyboard has no forward Delete key |

## Bulk (the 0032 batch, decided)

| # | Decision | Rationale |
|---|---|---|
| 19 | Every delete is a `GuardedKind::Batch(BatchPlan { items, skipped, extras: BatchExtras::Delete { propagation, already_gone, warnings } })`; a single delete is one item | one mechanism, owned by 0032; `checked_write` makes every commit go through `commit_block` and the audit |
| 20 | 0032 rules: one cluster, at most **50** items, dry-runs one at a time in order, **all** must pass. Delete-specific: NotFound in the identity read or the dry-run → `already_gone`, skipped | one predictable bulk behavior across specs |
| 21 | Scope rule: a menu on a **checked** row with ≥ 2 checked, Del or ⌘⌫ on such a cursor row, and the selection bar act on the checked set; anything else acts on one row | W4 note 1 |
| 22 | Identity reads are sequential, like the 0032 dry-runs | ≤ 50 short GETs; no limiter to write |
| 23 | Commits are sequential through `checked_write`; `Blocked` stops the loop; a per-object error goes on. **Progress in the dialog is best-effort**; the loop survives the dialog closing, and the final notification is the source of truth | 0030 decision 16, 0032 sequence |
| 24 | One audit line per object with `deleteOptions.propagationPolicy`; the note is shared | C10 |
| 25 | (decided) RBAC: `AccessCheck::Delete(kind)` is **lazy per kind** (0031 decision 24): reviewed when a screen of the kind is first shown, cached in the session report | no session-start burst; the dry-run stays the precise check |

## Baseline (refresh 2026-10-03, main `2c7dc08`)

| # | Decision | Rationale |
|---|---|---|
| 26 | `ResourceAction::Delete(ObjectKind)` carries its kind (0030 decision 35). `subject_action(RowAction::Delete, subject)`: Pod → `Delete(Pod)`, Node → `Delete(Node)`, `Kind { kind }` → `kind.builtin_object()`, **except `HelmReleases` (its spec reads `Secret`) and custom kinds → `None`**. Their menu item stays `disabled_menu_item(kind.delete_label(), NOT_SHIPPED_REASON)` | a release row's name is not a Secret name; deleting "the Secret" would hit the wrong object or none; `Uninstall` is 0038 |
| 27 | One entry: the `Delete(_)` arm of `run_available_row_key` calls `start_delete(delete_scope(subject, checked))`. Menu items are `row_action_item`s without `on_click`, labelled `kind.delete_label()` (or `Delete {n} {plural}…` on a checked row); a right click has moved the cursor to that row (0027) | menus, Del, ⌘⌫, and the palette cannot diverge (0032 decision 31) |
| 28 | `WriteRequest::new` keeps the merged DNS-1123 rule, with 0031 decision 27 for the RBAC kinds | the name is a path segment that kube does not encode |
| 29 | The audit `fields` value of `deleteOptions.propagationPolicy` is dropped for Secret and ConfigMap targets by the merged `recordable_fields` (`PATH_ONLY_KINDS`); the path stays | neither kind owns dependents, so the value is always `Background`; no special case in `audit_log.rs` |
| 30 | A slot release or switch during a bulk delete needs no extra hook: the next item's `checked_write` re-resolves `guard_for` and returns `Blocked` (`{cluster} is no longer open; nothing was changed`), which stops the loop | 0030 `commit_block` already covers it |
