# 0038 · Test plan

[Back to index](README.md). Offline and deterministic. **No test spawns `helm`, and no test or agent run rolls back or uninstalls anything.** The process is replaced by the `run_with` spawner seam (scripted exit code, stdout, stderr, delay, spawn error); paused tokio time for the 6 min cap. There is no helm binary on this machine.

## Step 1 — cluster crate (`helm_command_tests.rs`, `access_review` tests)

| Test | Checks |
|---|---|
| `args_table_v3_and_v4` | the four rows × two majors of [helm-command.md](helm-command.md), exactly; v4 rollback dry-run is `--dry-run=client`; values in `--namespace=` / `--kube-context=` tokens |
| `args_never_carry_values_or_debug` | no `--debug`, `--set`, `--values`, `--kubeconfig`, `--kube-token` in any row |
| `request_rejects_bad_names_and_targets` | uppercase, `..`, 54-char release, namespace with a dot, `to >= from` → `None` |
| `release_name_with_a_dot_is_accepted` | `my.release` → `Some` (DNS-1123 subdomain) |
| `dns_name_rules_moved_unchanged` | the existing `kubelet_stats` node-name cases now run against `dns_name::is_dns_subdomain` |
| `access_checks_per_action` | rollback → create + update + delete secrets; uninstall → delete secrets |
| `detect_resolves_absolute_path_from_path_only` | fake `PATH` (relative entry, empty entry, dir with `helm.bat`/`helm.cmd` only, dir with `helm.exe` / `helm`) + an `is_file` closure → the absolute `helm.exe` (Windows) or `helm` (Unix) path; never the CWD or exe dir; none → `Missing` |
| `env_sets_kubeconfig_from_loaded_files` | two files → joined with the platform separator; `HELM_DRIVER=secret`; `HELM_NO_PLUGINS=1` |
| `env_applies_to_detection_too` | the detection invocation carries the same `set` / `remove` (without `KUBECONFIG`) |
| `env_removes_helm_overrides` | every `HELM_KUBE*`, `HELM_NAMESPACE`, `HELM_DEBUG` in `remove` |
| `env_drops_proxies_unless_kubeconfig_sets_one` | `has_proxy_url` false → proxy variables removed; true → kept |
| `parse_version_accepts_v3_and_v4` | `v3.15.2`, `v4.3.0` → Found; `v2.17.0` → Unsupported; garbage → Unsupported with the text |
| `detect_maps_spawn_not_found_to_missing` | seam `NotFound` → `Missing` |
| `blocked_policy_spawns_nothing` | `WritesBlocked` in both modes; the seam records zero spawns |
| `classify_table` | helm 3 and helm 4 (slog) fixtures: `forbidden` → `Denied`; `release: not found` → `NotFound`; `another operation … in progress` → `Busy`; exit 1 other → `Failed`; exit 0 → `Ok` |
| `kill_on_drop_per_invocation` | detect and DryRun `true`; Commit `false` |
| `commit_cap_does_not_kill_helm` | seam never finishes; advance past 15 min → `OutcomeUnknown`; the seam's child is not killed and its output future is still polled to completion |
| `dry_run_cap_kills_and_fails` | advance past 2 min on DryRun → `Failed`, child killed |
| `error_line_picks_the_last_error_and_caps` | helm 3 `Error: …` and helm 4 `time=… level=ERROR msg=… error="…"` fixtures; control characters stripped; 500 chars |
| `output_is_truncated_after_collection` | 1 MiB scripted stdout and stderr both collected (no deadlock), each truncated to 64 KiB; message from the kept part |
| `helm_error_debug_omits_the_message` | `{:?}` holds no stderr text |
| `commit_args_match_the_commit_row` | `helm` + the Commit row |
| `secret_checks_are_namespaced` | SSAR `create`/`update`/`delete`, `""`, `secrets` |

## Step 2 — app

| Test | Checks |
|---|---|
| `release_gate_order_table` | each row of the gate table in [release-actions.md](release-actions.md), exact texts, first failing wins |
| `pending_release_disables_both_actions` | `pending-upgrade`, `uninstalling` |
| `default_rollback_target_skips_failed` | rev 4 failed, 3 superseded, 2 superseded → 3; only failed earlier → `No earlier revision…` |
| `history_roll_back_skips_the_current_row` | no button on rev 4 |
| `uninstall_types_the_release_name` | TypeName tier → `expected = "strimzi"`; Enter tier → dialog, danger button |
| `helm_kind_runs_dry_run_before_commit` | scripted seam: dry-run args then commit args, in that order; a failed dry-run → no commit spawn |
| `lock_toggled_during_dialog_blocks_the_commit` | 0030 `commit_block` with the Helm kind |
| `audit_line_for_rollback` | `HelmRelease`, `revision` `4 → 3`, error class only |
| `preview_is_shell_quoted` | `helm_preview` over a raw argv `["helm", "rollback", "a b", "--namespace=x'y"]` → each part quoted by `shell_quote` |
| `helm_errors_are_never_traced` | source scan of `write_flow.rs` and `confirm_dialog.rs`: no `tracing::` macro whose arguments name a `HelmError`, `message`, or `stderr` (AC 8) |
| `helm_detection_runs_once_on_first_releases_open` | second open spawns nothing |
| `release_actions_use_the_rows_cluster` | 0027 fixture: release of cluster B → guard B |

## Live checks (coder-lite, UAT, denied path only, debug build)

Always `--kubeconfig monitor-uat-readonly.yml --context readonly@Monitor --config-dir .tmp/config-0038-<case>`; never `K8SBOARD_ALLOW_WRITES`; never install helm for the check.

1. Record the session SSAR answers for `create secrets`, `update secrets`, `delete secrets` (allowed true/false only). If any action's checks are all allowed, stop and report; never press Continue.
2. Releases (or the empty state when UAT has none): `Roll back…`, `Uninstall release…`, the top `Rollback`, and History `Roll back` show their reason (SSAR denial first; else `Needs the helm CLI (v3 or v4) on PATH`).
3. Trace: no process other than the detection `helm version` attempt (which fails with not-found here); `RUST_LOG=kube=trace` shows only GETs and SSAR POSTs.

## ui-verifier

`--screen helm-rollback-confirm` (light and dark): PROD badge title `Roll back strimzi to revision 3 on …?`, object row, `revision 4 (failed) → 3`, mono command line, dry-run passed line, helm version line, typed-name field, `Back` / `Roll back`. Report color literals and clipped text.

## Write-capable cluster (R2, later, user-run)

With helm installed and `K8SBOARD_ALLOW_WRITES=1` set by the user on a disposable cluster: install a test chart twice (two revisions), roll back from k8sBoard and compare `helm history` with a CLI rollback; uninstall and confirm the release Secrets are gone; repeat with Helm 4 if available (open item 3).
