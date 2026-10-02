# 0030 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; every new item has a production user in its step (dead-code rule). Prerequisites: 0024, 0025, 0028 merged. Step 4 needs the user's approval (C3).

## Cargo and lint config

| File | Change |
|---|---|
| `crates/cluster/Cargo.toml` `[dev-dependencies]` | `tower = { version = "0.5", default-features = false, features = ["util"] }`, `http = "1"` (both already in `Cargo.lock`; no new package) |
| `crates/cluster/Cargo.toml` `description` | "read-only cluster access" → "cluster access with one allow-listed write path" |
| `clippy.toml` (root, exists) | `disallowed-methods` list from [write-path.md](write-path.md), each with a `reason` |

No runtime dependency change. kube features unchanged (no `ws`).

## `crates/cluster` (step 1)

| File | Change |
|---|---|
| `src/object_write.rs` (new) + `object_write_tests.rs` | `WriteOperation`, `WriteRequest` (manual `Debug` for both), `WriteMode`, `WriteOutcome` (with `effect: WriteEffect` = `Patched` and `created_name`), `ChangedField`, `WritePolicy`, `WriteError`, `FIELD_MANAGER`, `run_raw`, redaction, `ClusterConnection::write`; the named `#[allow(clippy::disallowed_methods)]` |
| `src/fake_api.rs` (new, `#[cfg(test)]`) | `FakeApi`, `RecordedRequest` |
| `src/connection.rs` | field `write_policy` set in `open`; `#[cfg(test)] from_client(client, context, policy)` |
| `src/object_yaml.rs` | `api_resource` and `ObjectRef` accessors → `pub(crate)` (production user: `object_write`) |
| `src/access_review.rs` | `AccessCheck::PatchNodes`; `ALL` + 1; tests; the named allow on `review_one` (SSAR) |
| `src/kubelet_stats.rs` | the named allow on `kubelet_text` / `kubelet_lines` (read-only GET allow-list, 0011) |
| `src/lib.rs` | `mod object_write; #[cfg(test)] mod fake_api;`; export the public `object_write` types |

`ClusterConnection::write` is `pub`, so step 1 has no dead code before the app uses it in step 4.

## `crates/app`

| S | File | Change |
|---|---|---|
| 2a | `src/write_guard.rs` (new) + `write_guard_tests.rs` | `ClusterGuard`, `WriteLock` (+ `at_open`), `ConfirmMode`, `ActionRisk`, `Trigger`, `ConfirmStep`, `DialogConfirm`, `confirm_step`; test `app_has_no_kube_dependency` (`include_str!("../Cargo.toml")`) |
| 2a | `src/cluster_registry.rs` (+ tests) | `ClusterEntry.confirm`, `ClusterProfile.confirm`; allow-list test gains `confirm` |
| 2a | `src/resource_actions.rs` (+ tests) | gate order on `ClusterGuard`; `mutates`; reason constants replaced; `action_risk` |
| 2a | `src/app_shell.rs` | `guard_for(&ClusterRef)` (lock = `WriteLock::at_open(profile)` and generation 0 until 2b adds the session fields) |
| 2a | `src/settings_window.rs` (0025, + tests) | "Confirm changes by" select; Safety page (tier table); `pages_follow_w2_order` |
| 2b | `src/cluster_session.rs` | `lock: WriteLock` set at session start; `generation: u64` bumped on reconnect |
| 2b | `src/app_shell.rs` | `toggle_write_lock` (lock at once; unlock via `confirm_step`) |
| 2b | `src/confirm_dialog.rs` (new) | unlock variant (title, typed name, Enter handling, click-only) |
| 2b | `src/keymap.rs` (0028, + tests) | `ToggleReadOnly` on `secondary-shift-r` (`WINDOW`); out of `RESERVED_KEYS`; sheet row; `enter` → `NoAction` in `WriteConfirm` and `WriteConfirm > Input` |
| 2b | `src/title_bar.rs` | badge toggle, dashed env border, two states; multi-mode menu when 0027 is merged |
| 3 | `src/audit_log.rs` (new) + `audit_log_tests.rs` | `AuditEntry`, `AuditObject`, `AuditField`, `AuditOutcome`, `lock_entry`, `append_audit`; `toggle_write_lock` appends lock lines; Safety page audit path and `Show in folder` |
| 4 | `src/write_flow.rs` (new) + `write_flow_tests.rs` | `WriteIntent`, `GuardedIntent`, `GuardedKind`, `run_guarded` (the one core), `start_write` (wrapper), `commit_block`, `DryRunState`, `TypedMatch`, `GuardedIntent.warnings`, app `CommitMode` + `Confirmed`, `WriteStep`, `checked_write`, `CheckedWriteError` (decisions 30–32; `Batch` arrives with 0032) |
| 4 | `src/audit_log.rs` | `audit_entry` (takes `GuardedIntent`) |
| 4 | `src/confirm_dialog.rs` | write variant: object row, changes, dry-run line, note, danger variant |
| 4 | `src/resource_actions.rs`, `src/keyboard_navigation.rs` (0028) | Cordon / Uncordon item and key C → `start_write`; `Cordon` shipped |
| 4 | `src/launch_options.rs` (+ tests), `src/screenshot.rs` | `--screen cordon-confirm`; settle waits for the dialog |
| 2a–4 | `src/main.rs` | `mod write_guard;` (2a), `mod confirm_dialog;` (2b), `mod audit_log;` (3), `mod write_flow;` (4) |

## Docs (with the step that ships them)

| S | File | Change |
|---|---|---|
| 1 | `docs/specs/0001-cluster-read-only/README.md` AC 6 | the grep points at the 0030 allow-list table and the clippy list |
| 1 | `CLAUDE.md`, `docs/agents/code-style/project-rules.md` | read-only rule: "mutating calls only through the `object_write.rs` allow-list, each approved by the user; debug builds block writes unless `K8SBOARD_ALLOW_WRITES=1`, which agents never set" (the user edits or approves the wording) |
| 2b | `docs/specs/0028-keyboard-map/keymap.md` | Ctrl Shift R from reserved to bound |
| 4 | `docs/specs/0034-*` (when written) | single-node Cordon / Uncordon already shipped by 0030 |
| 4 | `docs/roadmap/gap-plan-local-and-mutating.md`, `inventory-*.md` | 0030 done; W2 Safety partial (node shell 0037) |
