# 0040 · Safety per action

[Back to index](README.md) · ACs 1, 3, 4, 10–12. Every row keeps the 0030 invariants and the 0046 single-cluster invariants I1–I10 ([write-safety](../0046-single-cluster/write-safety.md)); nothing here relaxes them.

## Matrix

| | Attach | Restart pod | Evict | Drain, Skip PDBs | Bulk labels |
|---|---|---|---|---|---|
| Request | `pods/attach` upgrade (GET) | `DeleteObject { uid, Background }` | `EvictPod { uid, PodDefault }` | `DeleteObject { uid, Background }` per pod; cordon unchanged | `SetNodeLabels { changes }` per node |
| Sender | `debug_shell::attach_process` | `checked_write` | `checked_write` | `checked_write` (dialog and driver) | `checked_write` |
| Clippy ban | existing named exception (`debug_shell.rs`, `attach`); `Api::attach` stays banned elsewhere | existing `send` match in `object_write.rs`; `ClusterConnection::write` stays banned outside the two named senders | same | same | same |
| Kill switch | `attach_shell` returns `Failed(WritesBlocked)` before any request | `write` returns `WritesBlocked` before any request | same | same | same |
| Gate (row's cluster) | `get` + `create pods/attach`, lock, container block | lazy `delete pods` (`Delete(Pod)`), lock, pod block | `create pods/eviction`, lock, static-pod block | 0034 drain gate; option needs the lazy `delete pods` (`Delete(Pod)`) and no dry-run in flight | `patch nodes`, lock, no running batch |
| Proof at confirm | `AttachPermit` from the cluster's own report (`ConnectOpen::granted`) | uid from `object_identity` | same | uid from `drain_pods` (0034) | — |
| Dry-run | not supported (`Dry-run not supported for this action`) | yes (body `dryRun`) | yes (`dryRun=All`; checks the PDB) | yes, every pod, rerun on toggle | yes, every node |
| Tier | cluster tier, `Change`; TypeName types the cluster name | `Destructive`; TypeName types the pod name | same | `Drain`: `Privileged` while ticked, typed in every tier (node, or cluster for several); `Cordon only` keeps its tier | `Change`; TypeName types the cluster name |
| Held Enter | `fresh_enter` (connect dialog) | `fresh_enter` (batch dialog) | same | the 0034 dialog Enter rule | same as Restart |
| Audit | one line per start: `Attach`, Pod, `container`; applied / failed / abandoned | `Restart pod`, `deleteOptions.propagationPolicy` | `Evict`, `pods/eviction`; 429 not audited | per pod `Delete`; summary `Drain` + `disable_eviction` | `Edit labels` per node, `metadata.labels` |
| After A → B | `guard_for(A)` is `None`: no permit, nothing opens | `still_ready` (guard + generation) drops the uid read | same | `commit_block` stops the run (`Blocked`) | `commit_block` stops the batch |

## Row's own cluster

- Menu items: no `on_click` (dispatch the key action, which runs on the cursor's `ClusterObject`), except the container ⋯ `Attach`, which captures `RowContext.cluster` and is wrapped in `guarded(row, item)` like `container_shell_item`.
- `start_attach`, `start_removal`, the drain dialog, and the bulk editor read `guard_for(&cluster)` and `slot_live(&cluster)` of the object's `ClusterObject.cluster`, never an implicit session.
- The bulk editor keeps the cluster of the ticked nodes (`ticked_nodes`, `Select rows of one cluster` guard kept).
- The connect dialog re-reads `ConnectIntent::gate_block` on every render and at commit (0037 review); the batch dialog re-reads `commit_block` per item.

## Safety tests (names in [test-plan.md](test-plan.md))

| Test | Asserts |
|---|---|
| `attach_after_a_switch_opens_nothing` | Attach dialog on A, switch to B, confirm: no tab, no permit taken, no request on either fake |
| `container_attach_item_is_inert_after_a_switch` | a container ⋯ Attach item built on A does nothing after A → B → A (`guarded`) |
| `restart_read_landing_after_a_switch_opens_nothing` | uid read on A lands after A → B: no dialog, nothing sent |
| `evict_confirmed_after_switching_back_sends_nothing` | A → B → A: generation differs, nothing sent |
| `skip_pdbs_run_stops_on_a_lock` | a lock mid-run stops the deletes (`{text}; drain stopped`), nothing more sent |
| `bulk_labels_batch_of_a_sends_nothing_to_b` | a bulk label batch on A, switch mid-batch: rest `Not sent`, no request on B |
| `held_enter_never_confirms_the_new_dialogs` | attach, restart, evict, skip-PDB drain, bulk labels: a held Enter confirms none |
| `the_debug_policy_blocks_an_attach` | `Blocked` policy: one `Failed` update, zero recorded requests |
