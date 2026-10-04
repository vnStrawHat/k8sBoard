# 0040 · As built (steps 1 to 6)

[Back to index](README.md). Where the code differs from the text of the other files, and why. Nothing here adds a `WriteOperation`, a clippy entry, an `#[allow]`, a connect file, or a dependency; `object_write.rs`, `clippy.toml`, `Cargo.lock`, and `fresh_enter.rs` are untouched.

## Deviations

| Spec | Code | Why |
|---|---|---|
| `start_attach(open: ShellOpen, ..)` | `start_attach(open, terminal: ContainerTerminal, ..)` | the dialog needs the `stdinOnce` warning, and `ShellOpen` is shared with exec |
| `container_attach_item` private, wrapped by `guarded` in `container_menu` | `pub(crate)` and `guarded` inside it | the inert-after-a-switch test lives where the two-cluster window fixtures are (`shell_open_tests.rs`, not `resource_actions_tests.rs`) |
| Restart pod and Evict in palette pairs | cursor entries only (`is_pairable` is false for both) | `> rest pay` would list a refused `Restart pod` for every bare pod and break the 0046 pair tests; the entries (`> Restart pod`, `> Evict`, `needs confirm`) are as specified |
| `pod_menu` order | a `POD_MENU` array of `PodMenuEntry` drives it (as `CONTAINER_MENU`) | the kit's `PopupMenu` hides its items, so `pod_menu_follows_w4_order` reads the array |
| Gone text `{ns}/{name} was already deleted` | also for a plain Delete: the 0033 text read `pod {ns}/{name} …` (the label with `Delete ` trimmed) | one rule for the three removals, from `item.object` as specified |
| Row `passed · 112 ms` of `restart-pod-confirm` | the row reads `passed`; the 112 ms is the dry-run line's elapsed (set by the fixture) | the shared batch dialog (0032) has no per-row elapsed |
| Row error under the row (bulk editor) | one problem line under the rows, from `label_batch` over the nodes ticked now (live) | the checks are not tied to one row (`All selected nodes already have these labels`) |
| `LabelTarget::Several` header gate | `A batch is running` applies to the bulk only | the single editor is a plain write, as in 0034 |
| `held_enter_never_confirms_the_new_dialogs` (one test) | one held-Enter test per flow: `a_held_enter_never_confirms_an_attach`, `…_a_restart_or_an_evict`, `…_a_skip_pdbs_drain`, `…_the_bulk_labels` | each runs in the fixture of its own flow |
| Label of an Edit labels batch of one node | `Edit labels of 1 node` | grammar, as `Cordon 1 node` |

## Behavior notes

- **Attach**: `ConnectOpen::Attach` takes `attach_permit_of` at the confirm; `ShellKind::Attach` is opened by `Dock::open_attach` and attaches with `AttachWait::Container`. The tab has no Reconnect and no `Debug container…` offer; a second A opens a new tab. The audit action is `Attach` with the field `container`.
- **Restart / Evict**: `start_delete` became `start_removal(Removal, scope, ..)`. Both read the uid first, refuse a pod found terminating (`Already terminating`, nothing sent), re-check `pod_block` at the start, and take the cursor pod alone (`Select one pod` for anything else). Evict commits `EvictPod { uid, PodDefault }`; a 429 reads `refused for now: {cause}` in the row (Apply stays off) and in the notice, and `checked_write` writes no line for it. The finalizer lines come first, then the removal lines (`kind_warnings` runs for Delete only).
- **Skip PDBs**: `BudgetPolicy` is a `DrainOptions` field with a fresh default per open. The checkbox reads its own **cluster-wide** `delete pods` SelfSubjectAccessReview (no namespace, `review_access_for` with `NamespaceScope::All`), asked when the dialog opens: a drain deletes the pods of every namespace on the node, so the session's lazy `Delete(Pod)` review (its namespaces only) is not used. It is off with `Checking permissions…`, `Not permitted: delete pods`, or `Permissions could not be checked`, and with `{cluster} is read-only` when locked. The dialog no longer touches the session's kind access and observes nothing of the shell; the review's answer repaints it. Ticking sets the grace to `Pod default` and unticking puts the user's own choice back, clears every pod's dry-run and the elapsed time, and asks again. A 429 of a direct delete reads `Refused: …` (not `by PDB`) in the dialog and the tab.
- **Dialog layout**: the typed-name field, the note field, and the reason line moved out of the scrolling body (`BODY_MAX_HEIGHT` 590): ticking Skip adds a note and the field, which would otherwise sit below the fold.
- **Bulk labels**: `label_batch` takes the per-node `key=value` terms from `TickedNode.labels`; the editor reads no node. Review… rebuilds the batch from the nodes ticked at that moment and refuses a selection that moved to another cluster. `Edit labels is unavailable: {reason}` is the notice of an `Err`. The editor's problem line is worked out when a row changes (and when a row is added or removed), not on every draw; each row owns its subscriptions, which go with it.
- **Screens** (screenshot builds, fixed data, dead buttons): `attach-confirm` (Production, `stdinOnce`), `restart-pod-confirm` (Production, StatefulSet pod), `evict-confirm` (Staging, refused by `api-pdb`), `drain-dialog-skip-pdbs` (Staging, 24 deletes, empty typed field), `node-labels-bulk-editor`. Stored as `.tmp/ui-shots/v98-*-{light,dark}.png`.

## Tests

`pod_tests`, `debug_shell_tests` (cluster); `resource_actions_tests`, `keymap_tests`, `shortcut_sheet_tests`, `write_flow_tests`, `shell_open_tests`, `shell_tab_tests`, `object_delete_tests`, `audit_log_tests`, `drain_plan_tests`, `drain_writes_tests`, `drain_run_tests`, `node_edits_tests`, `palette_search_tests`, `launch_options_tests`; window tests over two fake clusters in `app_shell_delete_tests` (`DeleteServer` now answers evictions, a terminating pod, and the review), `app_shell_drain_tests` (`DrainServer` answers deletes and the review, and holds a dry-run or a delete with a gate), and `app_shell_node_edit_tests`.

## UAT (2026-10-04, read-only, debug build, no `K8SBOARD_ALLOW_WRITES`)

`--kubeconfig monitor-uat-readonly.yml --context readonly@Monitor`, `RUST_LOG=kube_client::client::builder=debug,cluster=debug`, four screen runs:

- Palette on the cursor pod: `Attach` reads `Not permitted: get and create pods/attach`, `Restart pod` reads `Not permitted: delete pods`, `Evict` reads `Not permitted: create pods/eviction` (`v98-uat-palette-attach-light`, `v98-uat-restart-light`, `v98-uat-evict-light`). Nodes with two ticks: the header `Edit labels` is disabled (`v98-uat-nodes-ticked-light`; the tooltip reason is the `patch nodes` check of Cordon, which reads denied on the same screen).
- Counted from the app's own debug log (`HTTP{http.method=… http.url=…}` request lines): **248 GET, 231 POST (all `selfsubjectaccessreviews`), 0 PUT, 0 PATCH, 0 DELETE, 0 POST to `/eviction`, 0 `/attach`, 0 `write finished` lines**.
- Not run live: a real attach, restart, eviction, skip-PDB drain, or bulk label (R2: no write-capable cluster). Commits are proven by fake-transport tests.
