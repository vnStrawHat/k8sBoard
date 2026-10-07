# 0033 · As built (steps 1 to 3)

[Back to index](README.md). Where the code differs from the text of the other files, and why. Main had no `run_guarded`, `GuardedIntent`, or `BatchExtras` when this landed (0030 and 0032 "As built"), so the spec's names map as follows.

| Spec | Code |
|---|---|
| `run_guarded(GuardedIntent)` | `AppShell::start_batch(BatchIntent)`; a single delete is a one-item batch |
| `GuardedKind::Batch(BatchPlan)` with `extras: BatchExtras::Delete { … }` | `BatchPlan.extras: BatchExtras::{None, Delete(DeleteExtras)}`; `DeleteExtras { propagation, kind, targets, already_gone }` (`object_delete.rs`) |
| `BatchPlan.on_failure: BatchFailure` | not built: a failed item always lets the next one go, as in 0032 |
| `expected_name` of the list variant | `BatchIntent.expected_name` (the object name of a single delete; `None` types the cluster name) and `BatchIntent::typed_hint` |
| `checked_write(DryRun / Commit)` | unchanged: the dialog and `commit_batch` call it per item |
| lazy `AccessCheck::Delete(kind)` | `AccessCheck::Delete`, asked with `Update` in one review per kind and scope (`kind_access::lazy_checks`); the gate arm of `permission_reason` reads both |
| `ResourceAction::Delete(ObjectKind)`, `subject_action` | as specified; `delete_kind(subject)` / `delete_kind_of(kind)` are the one source (Helm releases and custom kinds give `None`) |
| `object_delete.rs` in `main.rs` | a child of `app_shell` (`#[path]`), because `start_delete` reads the viewed slots and `running_batches` |

## Behavior notes

- **Start** (`start_delete`): gate (`delete_gate`: one cluster, at most 50 rows of one kind, permission, lock, no running batch), then the identity reads one after the other on the runtime (`read_identities` stops at the first failure that is not a 404), then `start_batch`. A second Del while a read runs (`delete_start`) or while a dialog is open starts nothing, so a held Del opens one dialog and reads once. A cluster that reconnected or locked during the reads is refused.
- **Single label**: the dialog title is `Delete pod` (kind only, as specified) and the typed name is the object name; the notices and audit label use the item label `Delete pod payments/api-x`, so a notice names its object.
- **Review fixes**: the start is refused when another dialog or an editor opened during the reads, and `commit_batch` refuses (the dialog shows `A batch is running on …`) when the cluster already runs a batch. RBAC names with `:` pass `WriteRequest::new` for Delete, as for Edit YAML. A 404 at the identity read reads `{name} not found (already deleted or not served)`. The start posts `Reading N objects…`, and its refusal notices share one id so a held Del leaves one.
- **Gone objects**: a 404 at the identity read goes to `already_gone` (`already gone: …` line). A 404 at a dry-run or at the commit is `ItemProgress::Gone`: skipped, not a failure, excluded from the button, which then reads `Delete {n} of {m}`. `ItemProgress::Pending(finalizers)` is a `DeletionPending` answer; a bulk summary counts it as accepted.
- **Propagation**: `choose_propagation` rebuilds the batch (`with_propagation`) and restarts the dry-runs; ignored while a commit runs. Kinds that own no dependents never show the radio and send `Background`.
- **Helm release Secrets** appear on the Secrets screen too. A Secret row whose type is `helm.sh/release.v1` (`cluster::HELM_RELEASE_SECRET_TYPE`) is refused (menu reason, Del notice, or a skipped line in a bulk); a Secret row that cannot be found counts as one (fail closed).
- **Edit YAML**: starting a delete is refused while the editor is open. A delete accepted for the object an open editor holds (the dialog outlives the editor opening) shows that editor `Deleted` (`object_deleted`); the text stays. The drawer closes by the existing selection sync once the watch drops the row.
- **Menus**: the Delete item is an `element` item without `on_click` that dispatches the Del key action; its label counts what Del would delete when the menu draws (`delete_scope_size`, `Delete 12 pods…`). `kind_menu`, `pod_menu`, and `node_menu` all end with it.
- **Selection bar**: `Delete…` (danger, last before ✕) on Pods, Nodes, and every kind screen; not on Issues, whose rows are findings. `bulk_buttons` appends it to the 0032 buttons; `BulkButton.is_danger` draws it.
- **Cluster crate extras**: `ObjectKind::resource` is public (the title's plural noun); `RELEASE_TYPE` is exported as `HELM_RELEASE_SECRET_TYPE`; `FakeApi` records the `Accept` header.
- **Renames**: `Screen::edit_kind` became `access_kind` (every screen with rows asks both rights), `request_edit_access` became `request_row_access`.
- **Screens** (screenshot builds, fixed data, no cluster, buttons dead): `delete-confirm` (a Production Deployment with a finalizer, Dependents radio, object-name field) and `delete-bulk-confirm` (twelve Staging pods, two without a controller).

## Tests

`object_write_delete_tests.rs` (cluster: request shape, effects, identity), `object_delete_tests.rs` (scope, items, warnings, finalizer lines, notices, progress), `app_shell_delete_tests.rs` (the flow over two fake clusters), plus additions to `resource_actions_tests`, `keymap_tests`, `audit_log_tests`, `launch_options_tests`, `kind_access_tests`, and `app_shell_edit_tests` (`kind_access_includes_delete`). A lock that comes on mid-batch is held with a gate in the fake server (`a_lock_that_comes_on_mid_batch_stops_the_rest`). Not built: a test of the shared audit note (the 0032 machinery copies it).

## UAT (2026-10-03, read-only, no `K8SBOARD_ALLOW_WRITES`)

- The lazy `delete` review answered **denied for all 27 kinds** (one screen run per kind): every Delete item and the `Delete…` button read `Not permitted: delete {resource}` (screens `v83-pods-selected-*`, `v83-deployments-menu-*`).
- A local counting proxy (token injected by the proxy, `DELETE`/`PUT`/`PATCH` refused with a 403) saw **1924 GET, 1381 POST (all SelfSubjectAccessReviews), 0 DELETE, 0 PUT, 0 PATCH** over 29 screen runs. The gate stops before the identity read.
- Not run live: a real delete (R2: no write-capable cluster). Commits are proven by the fake-transport tests only.

## UX round 3 (P7, P8, P23, P27)

- **Bindings:** the confirm of a RoleBinding or ClusterRoleBinding delete says what it takes away, `Removes cluster-admin from ServiceAccount lab-batch/default` (at most three subjects, then `and N more`; a bulk delete names three bindings, then `and N more bindings`). The audit line keeps `roleRef` (`ClusterRole/cluster-admin`) and `subjects` beside the propagation policy (`WriteIntent.audit_fields`, from `TargetFacts::Binding`).
- **Claims:** the PVC confirm names the bound PersistentVolume and its reclaim policy: `The PersistentVolume pvc-… is deleted too (reclaim policy Delete)` or `… is kept (reclaim policy Retain)`. When the PV list is not loaded (it is a companion of the StorageClasses screen) the start reads that one volume (`ClusterConnection::volume_reclaim_policy`, a GET); only a failed read keeps the conditional line.
- **ReplicaSets:** deleting an old revision (a ReplicaSet with no pods that a Deployment owns) warns `Deployment web can no longer roll back to rev 21`; a ReplicaSet with no pods shows no Dependents choice, because nothing depends on it (`has_dependents`).
