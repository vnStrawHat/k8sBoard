# 0030 — Guardrails and write path

Status: draft, amended after the advisor review (M1–M5, S6–S12, N13–N18), HEAD `71cc6f7`; shared amendments from 0031–0034 (decisions 30–36: `checked_write`, `CommitMode::Commit { confirmed }`, `GuardedIntent.warnings`, `WriteEffect`, `created_name`, `Batch`, gate by kind, 429, delete exception, no SSA); user decisions of 2026-10-02 (C3 approved, two confirm tiers, 0038 deferred). Prerequisites merged: 0024, 0025, 0028 (0027 optional: the row-cluster contract works with one session). Crates: `crates/cluster` (the one write module) and `crates/app` (lock, gate, confirm, audit). Roadmap: gap plan 0030; C3, C5, C8, C10; risks R1, R2. Wireframes: W2 Safety, W10 confirm modal and notes 3–4, W6 typed name, keyboard map Ctrl Shift R, env token rows.

**User approval (C3):** Approved by the user on 2026-10-02 (one approval for all mutating specs). Steps 1–3 send no write to any cluster; step 4 adds the first real commit (Cordon). Safety mechanisms are unchanged: debug builds block writes unless `K8SBOARD_ALLOW_WRITES=1`, agents never set it, and UAT live checks stay denied-path-only.

## Goal

- **One write path**: every mutating request goes through `ClusterConnection::write` in `crates/cluster/src/object_write.rs`; the allow-list is the `WriteOperation` enum, enforced by clippy `disallowed-methods`; debug builds block writes unless `K8SBOARD_ALLOW_WRITES=1`.
- **One gate** for every mutating action: feature shipped, RBAC (SSAR per verb and resource), then the per-cluster read-only lock (PROD locked by default), always from the **row's own cluster**.
- **Confirm tiers** per cluster: PROD types the name; every other environment (STG, DEV, LOCAL, unknown → STG) gets a confirm dialog and a click on its confirm button. Every guarded action opens a dialog. Defaulted from the environment, set in Settings.
- **Server-side dry-run** before every commit; webhook rejection blocks; conflict and unknown-outcome surfaces.
- **Local audit log** (W10 note 4): JSON lines, no secrets, no bodies; commits and lock toggles.
- **First consumer**: Cordon / Uncordon node (one-field merge patch), taken from 0034.

## Non-goals

- The features of 0031–0038. Each adds its `WriteOperation`, `AccessCheck`, and allow-list row.
- Audit export (C9 save dialog), rotation, `SelfSubjectReview` identity; auto re-lock; bulk; undo; a dry-run escape.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Cluster crate: `object_write.rs` (`WriteRequest`, `SetNodeSchedulable`, `WriteMode`, `WritePolicy`, `WriteError`), `AccessCheck::PatchNodes`, `fake_api`, clippy `disallowed-methods` | 1–4, 12 |
| 2a | Pure `write_guard` (`ClusterGuard`, `WriteLock`, `ConfirmMode`, `confirm_step`), gate order, registry/profile `confirm`, Settings select and Safety page, `app_has_no_kube_dependency` | 1, 2, 3, 6, 7, 13 |
| 2b | Session lock (`WriteLock`, generation), badge, Ctrl Shift R, unlock dialog | 1, 2, 5, 8 |
| 3 | `audit_log.rs` (pure; sends nothing); lock and unlock lines | 1, 2, 10 |
| 4 | `write_flow.rs`, confirm dialog write variant, Cordon / Uncordon, `--screen cordon-confirm`; UAT denied path; ui-verifier. Approved by the user on 2026-10-02 (one approval for all mutating specs). | 1, 2, 9–14 |

Progress: steps 1, 2a, and 3 are implemented (branch `spec-0030`). Steps 2b and 4 touch `app_shell.rs` and the UI heavily and wait for 0027. Until they land, the audit module is marked with a `cfg_attr(not(test), expect(dead_code))` that the first production user turns into a compile error, and no action is shipped, so the lock never decides a reason yet.

**Entry condition for step 2b:** before any action ships, the session stores `lock: WriteLock` and `generation: u64` (it holds neither today: `ClusterSession::guard` derives the lock from the profile, and `ClusterGuard` has no `generation`), and `commit_block` and the gate read them from the session through `ClusterGuard`.

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale |
| [write-path.md](write-path.md) | `object_write.rs` API, kill switch, allow-list and clippy, dry-run, write errors, conflicts, fake transport |
| [guardrails.md](guardrails.md) | lock, gate, reasons, confirm tiers, settings, row's own cluster, badge and key |
| [write-flow.md](write-flow.md) | sequence and `commit_block`, confirm dialog and Enter handling, error surfaces, async, Cordon |
| [audit-log.md](audit-log.md) | file, atomicity, record, secret rules, failure handling |
| [files-to-touch.md](files-to-touch.md) | files per step, Cargo, clippy, doc updates |
| [test-plan.md](test-plan.md) | fake transport, unit and window tests, UAT denied path, ui-verifier |

## Acceptance criteria

- [ ] 1. (steps 1, 2a, 3 pass; 2b and 4 pending) The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No `unsafe`. The only new `#[allow]`s are the three 0030 rows of the canonical `clippy::disallowed_methods` exception table (later specs add their own rows) in [write-path.md](write-path.md). `Cargo.lock` gains no package.
- [ ] 2. (steps 1, 2a, 3 done; the window tests of 2b and the write-flow tests of 4 pending) Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline. No test talks to a real cluster.
- [x] 3. A call to a disallowed kube method outside the canonical exception table fails clippy; the documentation grep lists only `access_review.rs` and `object_write.rs`; `app_has_no_kube_dependency` passes.
- [x] 4. A dry-run carries `dryRun=All` and `fieldManager=k8sboard`; a commit carries `fieldManager=k8sboard` and no `dryRun`; uncordon sends `false` (fake transport).
- [ ] 5. A PROD cluster opens locked; others unlocked unless the entry's `read_only` says otherwise. Ctrl Shift R and a badge click toggle the session lock; unlocking asks the cluster's tier.
- [x] 6. Disabled reasons follow the gate order: not shipped → permission checking/unknown → `Not permitted: {check}` → `{cluster} is read-only`.
- [x] 7. `confirm` defaults: PROD `type-name`; STG, DEV, LOCAL (and unknown → STG) `click`; every tier opens a dialog (`non_prod_tiers_always_show_a_dialog`). Clusters › Safety edits it; `settings.json` stores the kebab-case value.
- [ ] 8. The badge reads `Read-only` (lock) or `Unlocked` (open lock) with the env-colored dashed border; theme tokens only.
- [ ] 9. `ClusterConnection::write` is called only from `checked_write` in `write_flow.rs`; `commit_block` runs before every commit on the dialog and Batch paths (lock, switched or reconnected session, dry-run state, typed name).
- [ ] 10. Every commit and every lock toggle appends one audit line with the keys in [audit-log.md](audit-log.md); `OutcomeUnknown` records `unknown`; a Secret target records no values and a redacted error; no line holds a request body.
- [ ] 11. A failed or webhook-rejected dry-run blocks the commit and names the reason; 409 shows a Retry; a commit error after sending says the outcome is unknown; Enter activates the focused confirm button, and a held or repeated Enter never confirms.
- [ ] 12. (step 1 part done: `WritesBlocked` before any request; the UAT checks wait for step 4) Debug builds return `WritesBlocked` before building any request unless `K8SBOARD_ALLOW_WRITES=1` (zero recorded requests); agent runs never set it. UAT: Cordon is disabled with `Not permitted: patch nodes`; pressing C shows the notice; a trace shows only GETs and SSAR POSTs.
- [ ] 13. (the gate and the confirm tier are tested for two guards; the dialog badge and audit cluster wait for step 4) The gate, lock, tier, typed name, dialog badge, and audit cluster always come from `WriteIntent.cluster` / the row's cluster, never the primary (`gate_and_confirm_use_the_rows_cluster`).
- [ ] 14. ui-verifier: `--screen cordon-confirm` (PROD type-name tier, fixture state) and the badge in both states match W10/W1 with no high-severity defect.

## Open items

1. R2: no write-capable cluster yet. The commit path is proven by fake-transport tests only; a disposable kind cluster run follows once the user provides one (with `K8SBOARD_ALLOW_WRITES=1` set by the user).
2. Closed (user, 2026-10-02): two tiers, decision 9. The W10 text "dev one click, staging Enter" and LOCAL none are superseded; the wireframe is not edited, so ui-verifier treats it as a known deviation.
3. `SelfSubjectReview` identity for the audit log: a later item (decision 21 kept).
4. Audit export (C9) and rotation: later; Settings › Safety offers `Show in folder`.
5. With namespace scope All, the session SSAR is cluster-wide, so namespace-only rights show as denied in menus (namespaced writes from 0031 on). The dry-run is the precise per-object check.
6. The clippy exception list is one table in write-path.md (0030: 3 rows; 0035–0037 add `port_forward.rs`, `pod_shell.rs`, `debug_shell.rs`). `helm_command.rs` (0038) is deferred with 0038 and is not in the active table.
