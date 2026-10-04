# 0040 · Files to touch

[Back to index](README.md). No new crate, dependency, connect file, `WriteOperation`, clippy entry, or `#[allow]`. Unchanged and to stay unchanged (AC 1): `clippy.toml`, `Cargo.lock`, `crates/cluster/src/object_write.rs`, `crates/app/src/fresh_enter.rs`, `write_guard.rs` logic (doc comment only).

## Step 1 · cluster crate

| File | Change |
|---|---|
| `crates/cluster/src/pod.rs` (+ `pod_tests.rs`) | `ContainerTerminal`, `ContainerSummary.terminal` from `stdin`, `stdinOnce`, `tty` |
| `crates/cluster/src/lib.rs` | `pub use` `ContainerTerminal` |
| `crates/cluster/src/debug_shell.rs` (+ `debug_shell_tests.rs`) | `AttachWait::Container`; `readiness` reads main then init statuses; action words per wait; module comment |
| every `ContainerSummary { .. }` literal (≈ 72, cluster and app fixtures) | `terminal: ContainerTerminal::None` (mechanical; coder-lite may do it) |

## Step 2 · Attach

| File | Change |
|---|---|
| `crates/app/src/resource_actions.rs` (+ tests) | `ResourceAction::Attach`, `RowAction::Attach`, gate, risk, label, `attach_block`, `default_attach_container`, `key_availability_of` arm, pod menu item, container menu entry `container_attach_item` (`guarded`) |
| `crates/app/src/keymap.rs` (+ tests) | `Attach` action, `a` in `WORKSPACE` |
| `crates/app/src/keyboard_navigation.rs` | `Attach` arm of `run_available_row_key` → `attach_default` |
| `crates/app/src/palette_search.rs`, `command_palette.rs` | `RowAction::Attach` in the action lists and its icon |
| `crates/app/src/shortcut_sheet.rs` (+ tests) | `A · Attach` in the resource group |
| `crates/app/src/write_flow.rs` (+ tests) | `ConnectOpen::Attach`, `ContainerAttachOpen`, `GrantedOpen::Attach`, `granted`, `create()` |
| `crates/app/src/shell_open.rs` (+ tests) | `start_attach`, `attach_default`, `reconnect_attach`, `start_audit` arm, fixture screen |
| `crates/app/src/shell_tab.rs` (+ tests) | `ShellKind::Attach`, wait mapping, label, header, starting text, Reconnect routing |
| `crates/app/src/debug_open.rs` | `is_debug_tab` for `Debug` and `NodeShell` only; `reopen_debug_options` arm for `Attach` unreachable → route in `shell_tab.rs` |
| `crates/app/src/launch_options.rs` (+ tests), `screenshot.rs` | `attach-confirm` |

## Step 3 · Restart pod and Evict

| File | Change |
|---|---|
| `crates/app/src/resource_actions.rs` (+ tests) | `RestartPod`, `EvictPod` (actions, row actions, gates, risk, labels), `pod_block`, pod menu in W4 order |
| `crates/app/src/keymap.rs` | unbound unit actions `RestartPod`, `EvictPod` |
| `crates/app/src/keyboard_navigation.rs` | two arms → `start_removal` |
| `crates/app/src/object_delete.rs` (+ tests) | `Removal`, `DeleteExtras.removal`, `start_delete` → `start_removal`, gate and `still_ready` by removal, `DeleteTarget::request`, `TargetFacts::Pod { controller }`, texts, warnings, notices (gone text from `item.object`) |
| `crates/app/src/app_shell_delete_tests.rs` | window flow tests (two fake clusters) |
| `crates/app/src/palette_search.rs`, `command_palette.rs` | the two row actions and icons |
| `crates/app/src/launch_options.rs` (+ tests), `screenshot.rs` | `restart-pod-confirm`, `evict-confirm` |

## Step 4 · Skip PodDisruptionBudgets

| File | Change |
|---|---|
| `crates/app/src/drain_plan.rs` (+ tests) | `BudgetPolicy`, `DrainOptions.budgets`, `Budget::Bypassed`, verdict row 7, preview text and order, steps-strip and dry-run line words |
| `crates/app/src/drain_writes.rs` (+ tests) | `evict_write` → `removal_write` |
| `crates/app/src/drain_dialog.rs` | enabled checkbox and its gate, toggle → rerun dry-runs, HEADS UP, grace select, `live_tier` by policy |
| `crates/app/src/drain_driver.rs`, `drain_run.rs` (+ tests), `drain_tab.rs` (+ tests) | `removal_write` calls, `Deleting…` texts |
| `crates/app/src/audit_log.rs` (+ tests) | `disable_eviction` field of `drain_summary_entry` |
| `crates/app/src/write_guard.rs` | `ActionRisk::Privileged` doc: also a drain that skips budgets |
| `crates/app/src/app_shell_drain_tests.rs` | dialog and run tests |
| `crates/app/src/launch_options.rs` (+ tests), `screenshot.rs` | `drain-dialog-skip-pdbs` |

## Step 5 · Bulk labels

| File | Change |
|---|---|
| `crates/app/src/node_edits.rs` (+ tests) | `label_batch`; `TickedNode.labels` |
| `crates/app/src/node_editor.rs` | `LabelTarget`, header button by tick count, the bulk editor view |
| `crates/app/src/app_shell_node_edit_tests.rs` | window tests |
| `crates/app/src/launch_options.rs` (+ tests), `screenshot.rs` | `node-labels-bulk-editor` |

## Docs after merge (orchestrator; not in this spec's steps)

| Doc | Update |
|---|---|
| `docs/specs/0030-guardrails-write-path/write-path.md` | allow-list row `pods/attach`: spec `0037 step 1, 0040 (attach to a running container)`; grep note unchanged |
| `docs/specs/0034-node-maintenance/README.md` | open items 1 and 2: done in 0040 |
| `docs/specs/0036-terminal-shell/README.md` | non-goal "Attach … a later item" → 0040 |
| `docs/roadmap/wireframe-gap-audit.md` | rows W4 n1, W4b n3, W6 n2, W5 header → Done (0040) |
| `CLAUDE.md`, `project-rules.md` | none: no new connect file |
| `docs/specs/0040-pod-actions/as-built.md` | the coder's deviations |
