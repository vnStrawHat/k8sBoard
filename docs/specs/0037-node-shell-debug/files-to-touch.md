# 0037 · Files to touch

[Back to index](README.md). **S** is the step; each passes the gate on its own and every new item has a production user in its step. Baseline main `2c7dc08` (0024, 0025, 0027, 0028, 0030 steps 1/2a/3 merged). Step 1 needs 0036 step 1 (`ws`, `drive`); steps 2–3 need 0030 steps 2b + 4 and 0036 step 4. 0035 is not a prerequisite.

No Cargo change (kube `ws`, `kube::runtime`, tokio features are already on).

## `crates/cluster`

| S | File | Change |
|---|---|---|
| 1 | `src/object_write.rs` (+ tests) | `WriteOperation::{AddDebugContainer, CreateNodeShellPod, DeleteNodeShellPod}` with bodies from [pod-specs.md](pod-specs.md); `WriteRequest::new` kind and name rules; `WriteOutcome.uid`; `supports_dry_run` false for `DeleteNodeShellPod`; the `k8sboard.io/instance` label; `changed_fields`; manual `Debug` arms; `random_suffix`, `node_shell_pod_name`, `DEFAULT_DEBUG_IMAGE` |
| 1 | `src/debug_shell.rs` (new) + `debug_shell_tests.rs` | `AttachPermit` (+ `#[cfg(test)] for_tests`), `AttachWait`, `AttachRequest`, `ClusterConnection::attach_shell`, kill-switch check, private 1 s GET poll `wait_running` (seam), `readiness` (incl. exit 126/127), `Readiness`, 120 s cap; the named `#[allow(clippy::disallowed_methods)]` on the `attach` call |
| 1 | `src/pod_shell.rs` (0036) | `drive` and `AUTO_SHELL_SCRIPT` → `pub(crate)`; `upgrade_error` takes the verb name |
| 1 | `src/access_review.rs` (+ tests) | `CreatePods`, `DeletePods`, `PatchPodEphemeralContainers`, `GetPodAttach`, `CreatePodAttach` in `ALL`; `AccessReport::attach_permit` |
| 1 | `src/connection.rs` | the merged private `run_raw` moves here as `pub(crate)` unless 0035 already moved it |
| 1 | `src/lib.rs` | `mod debug_shell;` and its `pub use`; `DEFAULT_DEBUG_IMAGE` |

## `crates/app`

| S | File | Change |
|---|---|---|
| 2 | `src/write_guard.rs` (merged, + tests) | `ActionRisk::Privileged`; a new arm of `confirm_step` → `TypeName { expected }` for every mode |
| 2 | `src/write_flow.rs` (0030, + tests) | `GuardedKind::CreateThenAttach`; permit before commit; `run_cleanup` |
| 2 | `src/cluster_registry.rs` (0024, + tests) | `allow_node_shell`, `debug_image`, `node_shell_namespace` on `ClusterEntry` and `ClusterProfile`; allow-list test |
| 2 | `src/settings_window.rs` (0025, + tests) | W2 Safety `Allow node shell` toggle with hint |
| 2 | `src/resource_actions.rs` (+ tests), `src/keymap.rs` | `ResourceAction::DebugContainer` + `RowAction` + unbound unit action; gate rows ([ui.md](ui.md)) on the multi-check gate, `allow_node_shell` from `guard.profile`, subject `row_block` (`node.system.operating_system`); `Debug container…` in the Open shell submenu (`on_click` with `RowContext`); `OpenNodeShell` placeholder check replaced; shipped flags |
| 2 | `src/debug_dialogs.rs` (new) | Debug container options dialog (step 2); node shell options dialog (step 3) |
| 3 | `src/node_shell_sweep.rs` (new) + tests | run id, session-start leftover list, notice, Review dialog, deletes via `run_cleanup` |
| 2 | `src/shell_tab.rs`, `src/dock.rs` (0036, + tests) | `ShellSource::Attach`, labels, headers, hidden picker, `NoShell` → `Debug container…` button, prompt note |
| 2 | `src/app_shell.rs` | `open_debug_session`; `DebugContainer` handler |
| 3 | `src/shell_tab.rs` | `NodeShellCleanup` field; cleanup on `Exited`/`Failed`/release |
| 3 | `src/app_shell.rs`, `src/app_shell_view.rs` | main window `on_window_should_close` waits for pending deletes; `cx.on_app_quit` best-effort hook (GPUI 200 ms); sweep from `on_first_live(cluster)`; `leaving_work` line for node shells |
| 3 | `src/main.rs` | `mod node_shell_sweep;` |
| 3 | `src/keyboard_navigation.rs` (0028) | the `OpenNodeShell` arm (empty on main; S on a node already resolves to it) opens the node shell options dialog for the cursor node |
| 3 | `src/screenshot.rs`, `src/launch_options.rs` (+ tests) | `--screen node-shell-confirm` |
| 2 | `src/main.rs` | `mod debug_dialogs;` |

## Docs (with the step that ships them)

| S | File | Change |
|---|---|---|
| 1 | `docs/specs/0030-guardrails-write-path/write-path.md` | allow-list rows for the three operations (delete: dry-run no) and `pods/attach`; decision 5 and the `supports_dry_run` note already list `DeleteNodeShellPod` (done in this amendment); grep list gains `debug_shell.rs`; `WriteOutcome.uid` |
| 2 | `docs/specs/0030-guardrails-write-path/guardrails.md`, `README.md` | `ActionRisk::Privileged` row in the tier table; AC 9 names `run_cleanup` as the second caller of `write` (README open item 9) |
| 2 | `docs/specs/0024-settings-store/persisted-prefs.md` | `debug_image`, `node_shell_namespace` keys |
| 3 | `docs/roadmap/inventory-screens.md` | W4-4 Debug container, W5-5 node shell, W5-7 node shell tab → Done |
| 3 | `docs/roadmap/gap-plan-local-and-mutating.md` | 0037 done; W2 Safety complete |

## Parallel work (lane W2: 0036 → 0035 → 0037 → 0034)

Disjoint from lane W1: `debug_shell.rs`, `debug_dialogs.rs`, `node_shell_sweep.rs`, `shell_tab.rs`, `dock.rs`. Shared, append-only: `object_write.rs` (three variants; W1 adds 0031/0033/0032b ones), `access_review.rs` (+5 checks), `connection.rs` (`run_raw`, with 0035), `write_flow.rs` (`CreateThenAttach`, `run_cleanup`), `write_guard.rs`, `cluster_registry.rs`, `settings_window.rs`, `resource_actions.rs` (`node_menu`, also 0034), `keyboard_navigation.rs`, `launch_options.rs`, `screenshot.rs`.
