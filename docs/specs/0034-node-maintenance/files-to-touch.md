# 0034 · Files to touch

[Back to index](README.md). **S** is the step. Each step passes the gate on its own; every new item has a production user in its step. Prerequisites: 0030, 0032, 0013, 0009, 0028 merged. C3: Approved by the user on 2026-10-02 (one approval for all mutating specs).

No Cargo change. No clippy change (0032 already forbids `Api::cordon` / `Api::uncordon`; `Api::evict` and `create_subresource` are on the 0030 list).

## `crates/cluster` (step 1)

| File | Change |
|---|---|
| `src/object_write.rs` (+ `object_write_tests.rs`) | `EvictPod`, `SetNodeTaints`, `SetNodeLabels`, `GracePeriod`, `LabelChange`; wire format, `access_check`, `changed_fields`, manual `Debug`; eviction response decoded as `Status` (non-`Success` body → error by its code); `effect: Created` for evictions, `Patched` for node patches; `TooManyRequests` mapping (`Status.details.retry_after_seconds`, first cause message) |
| `src/node.rs` (+ `node_tests.rs`) | `NodeTaint.time_added`; `node_for_edit`, `NodeEdit` |
| `src/pod.rs` (+ `pod_tests.rs`) | `drain_pods(node)` (field selector, all namespaces, paged), `DrainPod` (incl. `is_pending`), `drain_pod` mapper |
| `src/disruption_budget.rs` | `list_pod_disruption_budgets()` (all namespaces, one-shot) |
| `src/access_review.rs` | `CreatePodEviction` (`create "" pods/eviction`, namespaced); `ALL` + 1 |
| `src/lib.rs` | export `DrainPod`, `NodeEdit`, `GracePeriod`, `LabelChange` |

## `crates/app`

| S | File | Change |
|---|---|---|
| 2 | `src/resource_actions.rs` (+ tests) | `EditTaints`, `EditLabels` (gate `PatchNodes`); `Drain` gate `CreatePodEviction` (two-argument `action_availability`) + running-drain check; node menu in W5 order |
| 2 | `src/node_edits.rs` (new) + `node_edits_tests.rs` | taint and label editors, `taint_intent`, `label_intent`, `is_system_taint`, `is_kubelet_label`, Retry on 409 reopens fresh with a notice; builders return `GuardedIntent` |
| 2 | `src/row_selection.rs` (+ tests) | Nodes `Cordon` / `Uncordon` as a 0032 `Batch` via `batch_intent` (skip nodes already in the target state); header `Edit labels` for one ticked node |
| 2 | `src/node_table.rs` | header button wiring |
| 3a | `src/drain_plan.rs` (new) + `drain_plan_tests.rs` | `DrainOptions`, `PodVerdict`, `Budget`, `pod_verdict`, `node_plan`, `drain_blocker`, option counts, preview order and texts |
| 3a | `src/drain_dialog.rs` (new) | W6 dialog, loading, dry-runs, aggregate `DryRunState`, `confirmed()`, `Dry-run accepted` downgrade, Cordon only via `checked_write`, start of the run |
| 3b | `src/drain_run.rs` (new) + `drain_run_tests.rs` | `DrainRun`, `PodProgress`, `NextStep`, `retry_delay`, `on_write`, `on_poll`, `cancel`, `node_started`, driver (`write_step`, 3 s poll) |
| 3b | `src/drain_tab.rs` (new) | dock tab view: header, progress, rows, Cancel, Uncordon, Close |
| 3b | `src/dock.rs` (0036) or `src/log_dock.rs` | `DockTab::Drain`; close disabled while running (if 0036 is not merged: its `LogDock → Dock` rename first, as a separate mechanical commit) |
| 3b | `src/audit_log.rs` (0030) | `drain_summary_entry` (one line per node: counts and outcome); the 429 no-audit rule lives in `checked_write` (0030 decision 36) |
| 3a | `src/keyboard_navigation.rs` (0028) | D opens the drain dialog for the cursor node |
| 3a, 3b | `src/launch_options.rs` (+ tests), `src/screenshot.rs` | `--screen drain-dialog` (3a), `--screen drain-progress` (3b) (fixture plan and run; no connection call) |
| 2, 3a, 3b | `src/main.rs` | `mod node_edits;` (2); `mod drain_plan; mod drain_dialog;` (3a); `mod drain_run; mod drain_tab;` (3b) |

## 0030 amendments

Done in this revision by the 0032/0034 architect: allow-list rows, `TooManyRequests`, decision 36 (429 not audited, drain summary outcomes), audit-log.md outcomes.

| S | File | Change |
|---|---|---|
| — | `docs/specs/0030-guardrails-write-path/` | done (see above) |
| 3a | `docs/specs/0013-policy-kinds/network-policy-and-pdb.md` | note: `disruption_state()` also drives the drain preview |
| 3b | `docs/specs/0036-terminal-shell/shell-tab.md` | `DockTab::Drain` joins the enum |
| 3b | `docs/roadmap/gap-plan-local-and-mutating.md`, `inventory-*.md` | 0034 done; Skip PDBs → 0033 |
