# 0037 · Files to touch

[Back to index](README.md). **S** is the step; each passes the gate on its own and every new item has a production user in its step. Prerequisites merged: 0030, 0036, 0035 (`ConnectOpen`), 0025.

No Cargo change (kube `ws`, `kube::runtime`, tokio features are already on).

## `crates/cluster`

| S | File | Change |
|---|---|---|
| 1 | `src/object_write.rs` (+ tests) | `WriteOperation::{AddDebugContainer, CreateNodeShellPod, DeleteNodeShellPod}` with bodies from [pod-specs.md](pod-specs.md); `WriteRequest::new` kind and name rules; `WriteOutcome.uid`; `supports_dry_run` false for `DeleteNodeShellPod`; the `k8sboard.io/instance` label; `changed_fields`; manual `Debug` arms; `random_suffix`, `node_shell_pod_name`, `DEFAULT_DEBUG_IMAGE` |
| 1 | `src/debug_shell.rs` (new) + `debug_shell_tests.rs` | `AttachPermit` (+ `#[cfg(test)] for_tests`), `AttachWait`, `AttachRequest`, `ClusterConnection::attach_shell`, kill-switch check, private 1 s GET poll `wait_running` (seam), `readiness` (incl. exit 126/127), `Readiness`, 120 s cap; the named `#[allow(clippy::disallowed_methods)]` on the `attach` call |
| 1 | `src/pod_shell.rs` (0036) | `drive` and `AUTO_SHELL_SCRIPT` → `pub(crate)`; `upgrade_error` takes the verb name |
| 1 | `src/access_review.rs` (+ tests) | `CreatePods`, `DeletePods`, `PatchPodEphemeralContainers`, `GetPodAttach`, `CreatePodAttach` in `ALL`; `AccessReport::attach_permit` |
| 1 | `src/node.rs` | `NodeSummary.operating_system` from `status.nodeInfo.operatingSystem`, if not present |
| 1 | `src/lib.rs` | `mod debug_shell;` and its `pub use`; `DEFAULT_DEBUG_IMAGE` |

## `crates/app`

| S | File | Change |
|---|---|---|
| 2 | `src/write_guard.rs` (0030, + tests) | `ActionRisk::Privileged`; `confirm_step` → `TypeName { expected }` for every mode and trigger |
| 2 | `src/write_flow.rs` (0030, + tests) | `GuardedKind::CreateThenAttach`; permit before commit; `run_cleanup` |
| 2 | `src/cluster_registry.rs` (0024, + tests) | `allow_node_shell`, `debug_image`, `node_shell_namespace` on `ClusterEntry` and `ClusterProfile`; allow-list test |
| 2 | `src/settings_window.rs` (0025, + tests) | W2 Safety `Allow node shell` toggle with hint |
| 2 | `src/resource_actions.rs` (+ tests) | `ResourceAction::DebugContainer`; gate rows ([ui.md](ui.md)); `Debug container…` in the Open shell submenu; `OpenNodeShell` checks replaced; shipped flags |
| 2 | `src/debug_dialogs.rs` (new) | Debug container options dialog (step 2); node shell options dialog (step 3) |
| 3 | `src/node_shell_sweep.rs` (new) + tests | run id, session-start leftover list, notice, Review dialog, deletes via `run_cleanup` |
| 2 | `src/shell_tab.rs`, `src/dock.rs` (0036, + tests) | `ShellSource::Attach`, labels, headers, hidden picker, `NoShell` → `Debug container…` button, prompt note |
| 2 | `src/app_shell.rs` | `open_debug_session`; `DebugContainer` handler |
| 3 | `src/shell_tab.rs` | `NodeShellCleanup` field; cleanup on `Exited`/`Failed`/release |
| 3 | `src/app_shell.rs` | `OpenNodeShell` handler; main window `on_window_should_close` waits for pending deletes; `cx.on_app_quit` best-effort hook (GPUI 200 ms); sweep on session Live |
| 3 | `src/main.rs` | `mod node_shell_sweep;` |
| 3 | `src/keyboard_navigation.rs` (0028) | S on nodes → `OpenNodeShell` |
| 3 | `src/screenshot.rs`, `src/launch_options.rs` (+ tests) | `--screen node-shell-confirm` |
| 2 | `src/main.rs` | `mod debug_dialogs;` |

## Docs (with the step that ships them)

| S | File | Change |
|---|---|---|
| 1 | `docs/specs/0030-guardrails-write-path/write-path.md` | allow-list rows for the three operations (delete: dry-run no) and `pods/attach`; decision 5 and the `supports_dry_run` note already list `DeleteNodeShellPod` (done in this amendment); grep list gains `debug_shell.rs`; `WriteOutcome.uid` |
| 2 | `docs/specs/0030-guardrails-write-path/guardrails.md` | `ActionRisk::Privileged` row in the tier table |
| 2 | `docs/specs/0024-settings-store/persisted-prefs.md` | `debug_image`, `node_shell_namespace` keys |
| 3 | `docs/roadmap/inventory-screens.md` | W4-4 Debug container, W5-5 node shell, W5-7 node shell tab → Done |
| 3 | `docs/roadmap/gap-plan-local-and-mutating.md` | 0037 done; W2 Safety complete |
