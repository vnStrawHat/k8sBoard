# 0038 · Files to touch

[Back to index](README.md). **S** is the step; each passes the gate on its own and every new item has a production user in its step (`ClusterConnection::helm` and `detect_helm` are `pub`, so step 1 has no dead code). Prerequisites merged: 0017, 0030, 0024.

## Cargo and lint config

| S | File | Change |
|---|---|---|
| 1 | `crates/cluster/Cargo.toml` | tokio feature `process`. `Cargo.lock`: no package change (AC 1); anything else, stop and report |
| 1 | `clippy.toml` (root) | `disallowed-methods`: `std::process::Command::new`, `tokio::process::Command::new`, reason "processes run only through helm_command.rs" |

## `crates/cluster`

| S | File | Change |
|---|---|---|
| 1 | `src/helm_command.rs` (new) + `helm_command_tests.rs` | `HelmCli`, `HelmVersion`, `HelmCliState`, `detect_helm`, `HelmAction`, `HelmRequest` (+ `new`, `access_checks`, `commit_args`), `HelmOutcome`, `HelmError` (manual `Debug`), `ClusterConnection::helm`; private `resolve_helm`, `helm_args`, `helm_env`, `HelmEnv`, `run_with`, `classify`, `error_line`, `parse_version`, inline `CREATE_NO_WINDOW` (windows); the one named `#[allow(clippy::disallowed_methods)]` on the spawn |
| 1 | `src/dns_name.rs` (new) | `is_dns_label`, `is_dns_subdomain` (moved from `kubelet_stats.rs`, `is_node_name` renamed) with their tests |
| 1 | `src/kubelet_stats.rs` (+ tests) | uses `dns_name`; its node-name tests move |
| 1 | `src/connection.rs` | fields `kubeconfig_files`, `has_proxy_url` set in `open`; `#[cfg(test)] from_client` takes them |
| 1 | `src/access_review.rs` (+ tests) | `CreateSecrets`, `UpdateSecrets`, `DeleteSecrets` in `ALL` |
| 1 | `src/lib.rs` | `mod dns_name; mod helm_command;` and the `helm_command` `pub use` |

## `crates/app`

| S | File | Change |
|---|---|---|
| 2 | `src/write_flow.rs` (0030, + tests) | `GuardedKind::Helm { cli, request }`; dry-run and commit branches call `connection.helm` |
| 2 | `src/confirm_dialog.rs` (0030) | Helm variant: object row, changes text, command line with copy, dry-run label, helm line |
| 2 | `src/audit_log.rs` (0030, + tests) | `HelmRelease` object, `revision` field, error class |
| 2 | `src/resource_actions.rs` (+ tests) | `ResourceAction::{HelmRollback, HelmUninstall}`; `release_action_availability`; `helm_preview` |
| 2 | `src/helm_rows.rs` (0017) | Releases menu items enabled through the gate; top `Rollback` button |
| 2 | `src/helm_release_view.rs` (0017) | History row `Roll back` buttons |
| 2 | `src/helm_dialogs.rs` (new) | Roll back options dialog |
| 2 | `src/app_shell.rs` | `helm_cli` state and lazy detection; rollback and uninstall handlers |
| 2 | `src/screenshot.rs`, `src/launch_options.rs` (+ tests) | `--screen helm-rollback-confirm` |
| 2 | `src/main.rs` | `mod helm_dialogs;` |

## Docs (with the step that ships them)

| S | File | Change |
|---|---|---|
| 1 | `docs/roadmap/cross-cutting.md` | C12 resolved: `helm` CLI for rollback and uninstall; no upgrade |
| 1 | `docs/specs/0030-guardrails-write-path/write-path.md`, `README.md` AC 1 | allow-list rows `helm rollback`, `helm uninstall` (external process); the process clippy entries; AC 1 names four exceptions (SSAR, `object_write.rs`, kubelet GETs, `helm_command.rs`) plus the 0035–0037 connect sites |
| 1 | `CLAUDE.md`, `docs/agents/code-style/project-rules.md` | one line: "processes run only through `helm_command.rs`" (the user edits or approves the wording) |
| 2 | `docs/specs/0017-helm-releases/decisions.md` | decision 24 superseded by 0038 |
| 2 | `docs/roadmap/inventory-kinds.md` | Releases: Roll back, Uninstall → Done |
| 2 | `docs/roadmap/gap-plan-local-and-mutating.md` | 0038 done; upgrade not planned (no wireframe) |
