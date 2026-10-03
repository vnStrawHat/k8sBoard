# 0037 · Test plan

[Back to index](README.md). Offline and deterministic. **No test, probe, or agent run creates, patches, deletes, or attaches on any cluster; a privileged pod is never tried on UAT.** Requests go through the 0030 `FakeApi`; the attach body reuses 0036's duplex `drive` tests; `readiness` is pure; paused tokio time for the 120 s wait.

## Step 1 — cluster crate

`object_write_tests.rs` (extends 0030):

| Test | Checks |
|---|---|
| `allow_list_matches_the_operations` | the three new rows: method, path, query, content type, body byte for byte ([pod-specs.md](pod-specs.md)) |
| `debug_patch_is_strategic_merge` | `application/strategic-merge-patch+json`, path ends `/ephemeralcontainers` |
| `node_shell_pod_is_privileged_on_the_node` | `nodeName`, `hostPID: true`, `privileged: true`, `stdinOnce: true`, `activeDeadlineSeconds: 14400`, toleration `Exists`, labels incl. `k8sboard.io/instance`, namespace default `kube-system`, digest-pinned default image, no service account token |
| `node_shell_command_enters_pid_1` | `nsenter -t 1 -m -u -i -n -p -- sh -c` + `AUTO_SHELL_SCRIPT` |
| `delete_carries_uid_and_zero_grace_without_dry_run` | `preconditions.uid`, `gracePeriodSeconds: 0`; `supports_dry_run()` is false; the recorded request has no `dryRun` |
| `commit_returns_the_created_uid` | 201 body `metadata.uid` → `WriteOutcome.uid` |
| `node_shell_requests_reject_foreign_pods` | `DeleteNodeShellPod` on a pod not named `k8sboard-node-shell-…` → `None` |
| `node_shell_pod_name_is_valid` | uppercase, dots, 63-char nodes → valid DNS label ≤ 63, no trailing `-` |
| `random_suffix_is_five_base36_chars` | |
| `blocked_policy_sends_nothing_for_new_operations` | each new operation → `WritesBlocked`, zero requests |
| `changed_fields_name_privilege_and_image` | paths and values listed in pod-specs.md |
| `manual_debug_hides_image_and_command` | `{:?}` shows variant, kind, namespace, name only |

`debug_shell_tests.rs`: `attach_permit_needs_get_and_create`, `readiness_table` (exit 126/127 → `the node has no shell; node shell needs sh on the host`, running, each waiting reason, terminated, pod phase Failed, pod gone, ephemeral vs regular status list), `waiting_messages_are_dropped`, `wait_polls_get_every_second` (scripted `FakeApi` GETs, paused time; no watch request), `wait_times_out_after_120_s_with_the_last_reason`, `blocked_policy_sends_no_attach_or_get`, `attach_upgrade_403_names_pods_attach`, `blocked_policy_fails_attach_before_any_request`. `access_review` tests: `all_checks_cover_distinct_permissions` (new count), `attach_checks_use_get_and_create`, `ephemeral_check_targets_the_subresource`.

## Step 2 — guard, gate, debug container

| Test | Checks |
|---|---|
| `privileged_risk_always_types_the_name` | every mode → `TypeName { expected }` |
| `debug_container_gate_table` | each missing check → its reason; no running container; locked |
| `create_then_attach_takes_the_permit_before_commit` | no attach permit → no commit request recorded, no audit line |
| `create_then_attach_opens_after_commit` | `open` gets the permit and the outcome uid; commit error → `open` not called |
| `cleanup_needs_no_gate_and_audits_one_line` | locked cluster, no viewed session → one DELETE (no dry-run) sent (fake), one `Delete node shell pod` line |
| `cleanup_runs_once` | `Exited` then release → one delete |
| `allow_node_shell_defaults_by_environment` | PROD off; LOCAL (set or guessed) on; DEV and STG on only when set in the entry; guessed DEV, guessed STG and unknown off; an explicit `allow_node_shell` wins |
| `registry_keys_are_the_allow_list` | the three new keys |
| `safety_toggle_stores_allow_node_shell` | settings window test |
| `debug_image_validation` | empty, whitespace, 256 chars rejected |
| `debug_container_item_is_last_in_the_shell_submenu` | separator then `Debug container…` |
| `no_shell_tab_offers_debug_container` | `ShellEnd::NoShell` header button prefilled with the container |
| `debug_tab_label_and_hidden_picker` | `›_ debug · m8n2p/api`; no `Shell:` select |

## Step 3 — node shell

| Test | Checks |
|---|---|
| `node_shell_gate_table` | missing checks (first denied; attach pair text), setting off (`Node shell is off for …`, read from `guard.profile`), Windows node (`system.operating_system`), locked |
| `s_on_a_node_menu_and_palette_share_the_arm` | S, the node menu item, and the palette reach the `OpenNodeShell` arm for the cursor node |
| `node_shell_tab_label_matches_w5` | `›_ node shell · wk-03 (debug pod)` |
| `cleanup_on_exit_close_switch_and_failure` | exit, tab close, switch (`release_all`), slot release (`release_slot`, 0027), start failure → one delete each with the pod uid, on the held connection, after the `leaving_work` confirm where one applies |
| `sweep_runs_on_each_slots_first_live` | two viewed clusters → one leftover list per slot (`on_first_live`), each on its own connection |
| `cleanup_404_counts_as_done` | no error notice |
| `window_close_waits_for_pending_deletes` | `on_window_should_close` returns false while a delete is pending, then closes after it finishes |
| `quit_hook_is_best_effort` | `on_app_quit` starts pending deletes; nothing waits past GPUI's timeout |
| `sweep_lists_other_instances_in_any_phase` | selector has `k8sboard.io/instance!={id}`; Running and Succeeded both listed; this run's pods excluded; list only (no delete) at session start |
| `sweep_notice_never_deletes_by_itself` | notice shown, zero DELETE requests until `Delete selected` |
| `sweep_deletes_selected_leftovers_through_cleanup` | running rows unchecked by default; one delete and one audit line per checked row; locked cluster → button disabled with the reason |
| `options_persist_image_and_namespace_after_start` | written only after success and only when changed |
| `node_shell_uses_the_rows_cluster` | 0027 fixture: node of cluster B → guard B |

## Live checks (coder-lite, UAT, denied path only, debug build)

Always `--kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --config-dir .tmp/config-0037-<case>`; never `K8SBOARD_ALLOW_WRITES`.

1. Record the session SSAR answers of `create pods`, `delete pods`, `patch pods/ephemeralcontainers`, `get pods/attach`, `create pods/attach` (allowed true/false only). If every check of one action is allowed, stop and report; never press Continue.
2. Pods: `Open shell ▸` ends with `Debug container…` disabled and its reason. Nodes: `Open node shell` disabled with its reason; S shows the notice.
3. `RUST_LOG=kube=trace`: no POST to `/pods`, no PATCH, no DELETE, no `/attach`; only GETs and SSAR POSTs. Counts only.
4. Settings › Clusters › Safety: the toggle shows `off` for a seeded `environment: production` entry and `on` for a clean one.

## ui-verifier

`--screen node-shell-confirm` (light and dark): PROD badge title, danger primary, typed-name field for the node, `spec.hostPID → true`, dry-run passed line. Settings › Clusters › Safety: toggle with the W2 hint. Report color literals and clipped text.

## Write-capable cluster (R2, later, user-run)

With `K8SBOARD_ALLOW_WRITES=1` set by the user, on a disposable cluster: debug a distroless pod (`ps` shows the target), close the tab and confirm the ephemeral container terminated; open a node shell, run `hostname`, close the tab and confirm the pod is gone; kill the app mid-session and confirm the process ended (`stdinOnce`, open item 2) and the next session's notice offers the leftover; close the main window with a node shell open and confirm the pod is gone.
