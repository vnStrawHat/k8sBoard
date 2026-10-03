# 0032b · As built (steps 1 to 4)

[Back to index](README.md). Where the code differs from the text of the other files, and why. Main had no `run_guarded`, `GuardedIntent`, `ValueForm`, or `BatchFailure` when this landed (0030, 0032, and 0033 "As built"), so the spec's names map as follows.

| Spec | Code |
|---|---|
| `run_guarded(GuardedIntent)` | `AppShell::start_write(WriteIntent)`; the Set default plan is `start_batch(BatchIntent)` |
| `ValueForm::ReplicaRange`, `Storage` | the built `value_popover.rs`: `ValueForm::{ReplicaRange, Storage}` beside `Replicas`; `ValueTargets` gained the `Hpa*` and `Claim*` cases |
| `hpa_range_intent`, `expand_intent`, `default_class_intent` in `resource_edits.rs` | the same names; they take a `WorkloadScope` (cluster and display name) and return `WriteIntent` (HPA, Expand) or `BatchIntent` (Set default) |
| `BatchPlan.on_failure`, `BatchFailure` | built here (0032 and 0033 left them out): `BatchFailure::{Continue, Stop}`; every older plan is `Continue` |
| `BatchExtras::None` for Set default | `BatchExtras::DefaultClass(DefaultClassExtras)`: the target class (the Retry subject) and the text of the state a partial run leaves behind |
| "generic 0032 Stop notice plus Retry" | not built by 0032, so built here: `stop_notice`, a notification action `Retry` (`notify_with_retry`), `AppShell::retry_batch` |
| `ActionGate::Mutating { checks, is_shipped }` | as written; three new `ResourceAction`s (`EditHpaRange`, `ExpandClaim`, `SetDefaultStorageClass`) with unbound unit key actions |
| shell flow in `write_flow.rs` | `resource_edit_flow.rs` (child of `app_shell`), so 0037 and 0032b do not edit the same file |

## Behavior notes

- **Wire format**: merge patches as specified (decision 1); no `resourceVersion` or `uid` precondition and no JSON Patch `test` op, because each patch sets whole fields and touches nothing else. A concurrent change to another field is never overwritten; a concurrent change to the same field is last-write-wins, and the dry-run plus the re-read of the row at submit narrow that window.
- **Validation**: `WriteRequest::new` refuses `min = 0`, `min > max`, either above `i32::MAX`, and an Expand quantity that `ByteAmount::parse` rejects or that is zero; the stored text is trimmed. The popover checks the same first (`range_input`, `storage_input`), so the button is off and the reason shows under the field.
- **Expand class check**: `claim_block` reads the class list only through `LiveCluster::loaded_storage_classes`, which is filled only while the explorer shows StorageClasses. A PVC row is shown on the PVCs screen, so in practice the list is never loaded there, the menu never reads `Storage class {name} does not allow expansion`, and the dry-run's admission refusal (`the change is invalid: only dynamically provisioned pvc …`) is the real backstop (open item 1). The function and its tests stay for the day a class GET or a companion list feeds it.
- **`KindObject::StorageClass`**: StorageClasses rows were `Plain`; they now carry their summary (`is_default`, `allows_expansion`, `created_at`), which `row_block`, the plan, and the Retry read.
- **Set default**: items are `[target true, other defaults false…]` in name order, `BatchFailure::Stop`. A failed commit sets `stopped = an earlier step failed`, so the rest read `Not sent: an earlier step failed`; a `Blocked` result keeps its own reason. The dialog lists each item (`gp3 · becomes the default`, `io2 · stops being the default`) and, for an ordered plan, every item's changed fields (a removed beta key reads `… → removed`).
- **Partial failure**: the notice reads `{label}: stopped after {k} of {n}: {error}`, plus the two-defaults text when the target is the default by then, and a `Retry` button. Retry (`retry_batch`) calls `start_set_default` with `RowCheck::Bypassed`: the gate still applies, `row_block` does not, the set is omitted, and the plan's warnings carry `two_defaults_text` (newest by `creationTimestamp`).
- **Audit**: one line per object. Both halves of Set default carry the action `Set default`; the fields (`…is-default-class` = `true` or `false`) tell them apart. HPA edits are `Set limits`, Expand is `Expand`.
- **Bulk**: `BulkValue` (`Nothing`, `Replicas`, `Range`, `Storage`) replaced `Option<u32>` in `bulk_batch`. Edit limits and Expand open a popover first (like Scale); Set default builds its plan at once and needs exactly one ticked class (`Tick one storage class`). All three reuse 0032's `running_batches` guard through `start_batch`.
- **Screens** (screenshot builds, fixed data, no cluster call, buttons dead): `hpa-range-popover` (fixture HPA 3 to 20, 9 replicas, max typed as 5 so the scale-down warning shows), `expand-confirm` (Production claim 100Gi to 150Gi), `default-class-confirm` (Staging, `gp3` set and `io2` unset).

## Tests

`object_write_resource_edit_tests.rs` (cluster: request shape per operation, validation, errors, dry-run, debug policy), `resource_edits_tests.rs` (inputs, intents, warnings, bulk plans, Set default plans), `batch_write_tests.rs` (`Stop` semantics and notices), `app_shell_resource_edit_tests.rs` (the flows over two fake clusters: popovers, dialogs, held Enter, tiers, audit lines, partial failure and Retry, lock mid-batch), and additions to `resource_actions_tests`, `launch_options_tests`, and `access_review` tests. The Retry button is checked by drawing the notification (`batch-retry`); its click path is `retry_batch`, called directly.

## UAT (2026-10-03, read-only, no `K8SBOARD_ALLOW_WRITES`, screenshot build = debug build with writes blocked)

- The menus of HPAs, PVCs, and StorageClasses show `Edit min / max…`, `Expand…`, and `Set as default` disabled with `Not permitted: patch horizontalpodautoscalers`, `…persistentvolumeclaims`, and `…storageclasses` (screens `v86-uat-*-menu-light`). The selection-bar states are covered by window tests (no `--screen` ticks kind rows).
- Counts come from the app's own debug log (`RUST_LOG=cluster=debug`, target `cluster::connection`, counted and never printed): per screen run about 32 GET reads and 50 to 51 SelfSubjectAccessReview POSTs, and **0 write attempts** (no PATCH, PUT, DELETE, or create). Watches do not pass through that log, so they are not in the counts.

## Notes on the acceptance criteria

- AC 3 is met by one test per operation (`hpa_range_patches_both_fields`, `expand_patches_the_storage_request`, `set_default_true_sets_the_ga_annotation`, `set_default_false_clears_the_beta_annotation`) that pins method, path, query, content type, and body, instead of extending `allow_list_matches_the_operations`.
- AC 2: a few tests are named differently from the plan (`expand_warnings_follow_the_claim_state`, `a_partial_default_change_offers_retry_that_replans_only_the_unsets` covers both Retry cases, `a_lock_that_comes_on_after_the_first_item_stops_the_rest`); every case of the plan has a test.
- AC 10: the screens were captured in light and dark (`v86-hpa-range-popover-*`, `v86-expand-confirm-*`, `v86-default-class-confirm-*`) and read by the coder; no ui-verifier agent run was made.

## Review fixes

- `checked_operation` lists every `WriteOperation` instead of a catch-all, so a new operation (0034) does not compile until it gets a validation decision.
- The state a stopped Set default leaves behind names the first unset that did not go through (`DefaultClassExtras.two_defaults` has one text per unset), not always the first class.
- When a Set default ends clean, the shell re-reads the loaded StorageClasses list with the batch's own changes laid over it (`defaults_after`). If more than one class is marked default (one was made the default while the dialog was open), the end notice is a warning, not `2 done`. Without a loaded list there is nothing to check.
- A Retry off the StorageClasses screen says `Open StorageClasses first`.
- **Not done: uid pin on Expand.** Adding `metadata.uid` to the merge body would make the server refuse a replaced claim, but no vendored source or doc in `.cargo-home` states how the server answers a uid mismatch on a merge patch, and `PersistentVolumeClaimSummary` has no uid. Skipped; the dry-run and the re-read of the row at submit are the only guards against a claim replaced under the same name.
