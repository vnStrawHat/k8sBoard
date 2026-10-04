# 0040 · Test plan

[Back to index](README.md). Offline only: `FakeApi` (cluster), pure tests, window tests over two fake clusters (0046 fixtures). No test sends to a real cluster. Window tests use the existing debug `WritePolicy` injection, never `K8SBOARD_ALLOW_WRITES`.

## Step 1 · cluster

| Test | File | Asserts |
|---|---|---|
| `container_terminal_reads_stdin_stdin_once_and_tty` | `pod_tests.rs` | none / tty only / stdin only → `None`; both → `Interactive`; plus `stdinOnce` → `InteractiveOnce`; sidecar and init containers read too |
| `container_wait_reads_main_and_sidecar_statuses` | `debug_shell_tests.rs` | `Container` finds a running main container and a running native sidecar (init status list) |
| `container_wait_keeps_exit_127_as_an_ended_container` | same | no `NO_SHELL` text outside `NodeShellPod` |
| `container_attach_requests_the_attach_path_after_one_read` | same | one GET of the pod, then `GET …/pods/{p}/attach` with query `container=c`, `stdin=true`, `stdout=true`, `tty=true` (any order) |
| `the_debug_policy_blocks_an_attach` | same | `Blocked`: one `Failed` with the `WritesBlocked` text, zero requests |

## Step 2 · Attach

| Test | File | Asserts |
|---|---|---|
| `attach_gate_needs_both_attach_verbs` | `resource_actions_tests.rs` | checking → denied (`Not permitted: get and create pods/attach`) → lock → enabled |
| `attach_block_reasons` | same | not running, init, no terminal texts |
| `default_attach_container_prefers_running_main` | same | Main before Sidecar; none → the `Err` text |
| `pod_menu_follows_w4_order` | same | item order of [pod-removal.md](pod-removal.md) |
| `a_runs_attach_on_pods_only` | `keymap_tests.rs` | A → `Attach` on a pod; `NotOffered` on nodes and kinds |
| `shortcut_sheet_lists_attach` | `shortcut_sheet_tests.rs` | `A` row |
| `attach_open_takes_the_attach_permit` | `write_flow_tests.rs` | `granted` is `None` without both verbs; `create()` is `None` |
| `attach_follows_the_0030_gate_and_tier` | `shell_open_tests.rs` | PROD types the cluster name; the dialog shows both warnings and `Dry-run not supported` |
| `attach_audits_one_line_per_start` | same | `Attach`, `container`; applied / failed / abandoned |
| `reattach_reuses_the_tab` | same | Reconnect on an attach tab opens the connect dialog, not the debug options; same tab, separator note |
| `attach_after_a_switch_opens_nothing` | same | [safety.md](safety.md) |
| `container_attach_item_is_inert_after_a_switch` | `resource_actions_tests.rs` | [safety.md](safety.md) |
| `attach_tab_label_and_header` | `shell_tab_tests.rs` | `attach · {suffix}/{container}`, no shell picker, `AttachWait::Container` |

## Step 3 · Restart pod and Evict

| Test | File | Asserts |
|---|---|---|
| `restart_pod_refuses_bare_static_finished_and_terminating_pods` | `resource_actions_tests.rs` | the four texts; a ReplicaSet pod is allowed |
| `evict_refuses_static_and_terminating_pods_only` | same | bare and DaemonSet pods allowed |
| `restart_and_evict_gates_and_risks` | same | `Delete(Pod)` lazy / `CreatePodEviction`; `Destructive`; no single key |
| `removal_requests_are_uid_pinned` | `object_delete_tests.rs` | Restart → `DeleteObject { uid, Background }`; Evict → `EvictPod { uid, PodDefault }` |
| `removal_batch_texts` | same | title, verb, audit action, item label, typed pod name per removal |
| `removal_warnings_by_owner` | same | Restart PDB line, StatefulSet, Job; Evict bare, DaemonSet |
| `removal_notices` | same | success, gone (from `item.object`), 409, 429 texts |
| `restart_pod_dry_runs_then_deletes_with_uid` | `app_shell_delete_tests.rs` | metadata GET, dry-run body with `dryRun`, commit body with the uid; one audit line `Restart pod` |
| `evict_429_dry_run_blocks_the_commit` | same | both 0034 429 fixtures: row shows the cause, Apply off, nothing committed |
| `evict_commit_429_is_not_audited` | same | commit 429 → notice `refused for now`, zero audit lines |
| `restart_read_landing_after_a_switch_opens_nothing` | same | [safety.md](safety.md) |
| `evict_confirmed_after_switching_back_sends_nothing` | same | [safety.md](safety.md) |

## Step 4 · Skip PodDisruptionBudgets

| Test | File | Asserts |
|---|---|---|
| `skip_pdbs_turns_blocked_and_refused_pods_into_deletes` | `drain_plan_tests.rs` | `Blocked`, `Waits`, two-budget `Refused` → `Evict(Bypassed)`; rows 1–6 unchanged |
| `skip_pdbs_preview_names_bypassed_budgets` | same | text, tone, order, steps strip `Delete {n} pods` |
| `removal_write_follows_the_budget_policy` | `drain_writes_tests.rs` | `EvictPod` with grace / `DeleteObject` with uid; labels and audit actions |
| `delete_mode_results_map_like_evictions` | `drain_run_tests.rs` | Ok, 404, 409, unknown, 429 transitions identical |
| `skip_pdbs_summary_records_disable_eviction` | `audit_log_tests.rs` | field only for `Skip` |
| `skip_pdbs_is_off_without_delete_pods` | `app_shell_drain_tests.rs` | replaces 0034 `skip_pdbs_is_disabled_with_reason` (kit has no text query: state read from the dialog entity) |
| `skip_pdbs_starts_off_each_time` | same | tick, cancel, reopen: off |
| `toggling_skip_pdbs_reruns_every_dry_run` | same | eviction dry-runs then delete dry-runs; a late answer of the old policy is dropped |
| `skip_pdbs_types_the_name_in_every_tier` | same | a Click cluster asks for the node name while ticked; unticked → click |
| `skip_pdbs_run_deletes_with_uid` | same | the run sends `DELETE` with `preconditions.uid`, never `/eviction` |
| `skip_pdbs_run_stops_on_a_lock` | same | [safety.md](safety.md) |

## Step 5 · Bulk labels

| Test | File | Asserts |
|---|---|---|
| `label_batch_skips_nodes_that_already_match` | `node_edits_tests.rs` | per-node change lists; `already labelled` |
| `label_batch_checks_in_order` | same | the six texts of [bulk-labels.md](bulk-labels.md) |
| `header_edit_labels_by_tick_count` | `app_shell_node_edit_tests.rs` | 0 / 1 / 2–50 / 51 / two clusters |
| `bulk_labels_dry_run_every_node_then_commit` | same | one PATCH per node per mode; one audit line per node |
| `bulk_labels_batch_of_a_sends_nothing_to_b` | same | [safety.md](safety.md) |

## All steps

- `held_enter_never_confirms_the_new_dialogs` (`app_shell_write_tests.rs`), `launch_options_tests` for the five screens.

## Screens and live checks (step 6)

- ui-verifier, light and dark: `attach-confirm` (W10 connect dialog), `restart-pod-confirm`, `evict-confirm` (W4 actions in the 0033 dialog), `drain-dialog-skip-pdbs` (W6), `node-labels-bulk-editor` (W5).
- UAT, debug build, `readonly@Monitor`, no `K8SBOARD_ALLOW_WRITES`: Pods menu and drawer ⋯ show Attach, Restart pod, Evict disabled with the probe's reasons; A shows the notice; the Nodes header with two ticks shows `Edit labels` disabled; a request trace (0034 method: debug log lines) shows only GETs and SSAR POSTs, zero PATCH/PUT/DELETE, zero `/eviction`, zero attach upgrades, no `write finished`.
