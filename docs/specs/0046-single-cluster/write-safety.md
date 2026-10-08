# 0046 · Write safety

[Back to index](README.md) · ACs 8–12. Rule: removing code must never let a write reach a cluster other than the one shown. I1–I10 hold today; this spec keeps them with less code. Tests marked (step N) need that step's production change; the rest land in step 1.

## Invariants (after step 4)

| # | Invariant | Where it is enforced |
|---|---|---|
| I1 | A write needs `guard_for(cluster)`, which is `Some` only for the active cluster while Live | `session_of` cluster filter + `guard.cluster == *cluster` in `guard_for` |
| I2 | Guard, lock, generation, and tier are read again at commit from the active session | `live_block` / `gone_block` (`write_flow.rs`), batch commit (`batch_write.rs`), drain (`drain_dialog.rs`, `drain_driver.rs`), node edits (`node_editor.rs`), unlock (`finish_unlock`), connect starts (`shell_open.rs`, `port_forward_open.rs`, `debug_open.rs`, `node_shell_open.rs`) — unchanged |
| I3 | A session has a run-unique `generation`; a reconnect or a switch back gets a new one | `next_generation()` in `cluster_session.rs` — unchanged |
| I4 | Every row in a batch, delete, drain, or node edit is of one cluster, and that cluster is the active one | the "Select rows of one cluster" checks + `guard_for(first.cluster)` — unchanged |
| I5 | Ticks never outlive a switch | primary: `release_all` → `clear_all_filters` → `reset_filter` → `clear_checked` (`app_shell.rs` ~976, ~4378; `table_view.rs` ~403), which **must stay**; backup: `set_session` clears ticks and anchor on any change of the session entity, including to and from `None` (so A → B → A clears too) |
| I8 | A create or start that lands after a switch has no owner | `has_owner` in `debug_open.rs` keeps `self.session_of(&plan.cluster).is_some()`: no tab, the pod is deleted on the held connection |
| I9 | A stale menu acts on nothing | a `RowContext` action runs only if `row.session.upgrade()` is `Some` (the old entity is released before the next connect) |
| I10 | Results that land after a switch are dropped | `still_ready` in `object_delete.rs` (guard + generation); `show_leftover_notice` drops a notice whose cluster is not active |
| I6 | Menus, dialogs, and key actions act on the cluster they captured, not on "whatever is active now" | `RowContext.cluster`, `ClusterObject.cluster`, `DialogInputs.generation` — unchanged |
| I7 | The write ban, allow-list, kill switch, Enter rule, and audit are untouched | `clippy.toml` `disallowed-methods`, `object_write.rs`, `WritePolicy`, `fresh_enter.rs`, `audit_log.rs` |

## Kept identifiers and why

| # | Kept | Cost to remove | Why it stays |
|---|---|---|---|
| K1 | `ClusterObject { cluster, key }` | 37 files, every write intent, dialog, pending reveal, forward target | Without the cluster, an intent captured on A is indistinguishable from the same key on B; I1 and I6 would then rest on the generation alone. Not backward compatibility: it is a live guard input |
| K2 | `guard_for(&ClusterRef)`, `slot_*(&ClusterRef)` | ~40 call sites, many in 0030–0037 write code | Callers keep naming the cluster they act on; the check is in one function |
| K3 | "Select rows of one cluster" (`batch_plan`, `bulk_state`, `delete_scope`, `delete_gate`, `ticked_nodes`) | 5 × 3 lines + 2 pure tests | Proves I4 without trusting I5; unreachable in normal use; zero runtime cost |
| K4 | `RowContext.session: WeakEntity` + `cluster` | — | a menu kept open over a switch must not keep the old session alive or act on the new one |
| K5 | `running_batches` keyed by cluster | — | decision 10 |

## What goes, and why it is safe to go

| Removed | Was guarding | Now covered by |
|---|---|---|
| per-slot `guard_for` lookups over several sessions | rows of B while A is primary | I1 (one session) |
| `RowName.cluster`, `Clustered` | same-named rows of two clusters | I5 + I4 |
| "n of m unlocked" badge, per-cluster lock menu | which cluster is open | one lock; notice still names the cluster (`read_only_notice`) |
| `remove_from_view`, `view_clusters` | release of some sessions | `switch_cluster` (asks `leaving_work` first) |
| `subject_cluster` | drawer watches across clusters | `release_all` drops the session with its watches |

## Safety tests (step 1 unless marked; all stay green through step 5)

| Test | File | Asserts |
|---|---|---|
| `guard_for_another_cluster_is_none` | `app_shell_write_tests.rs` | after A → B Live: `guard_for(A)` is `None`, `guard_for(B)` is B's |
| `a_dialog_confirmed_after_a_switch_sends_nothing` | `app_shell_write_tests.rs` | cordon dialog on A, switch to B, confirm: no request on either fixture, outcome "no longer open; nothing was changed" |
| `a_dialog_confirmed_after_switching_back_sends_nothing` | `app_shell_write_tests.rs` | A → B → A: generation differs, nothing sent |
| `a_menu_built_on_a_acts_on_nothing_after_a_switch` | `app_shell_write_tests.rs` | a `RowContext` captured on A: its action is off after A → B |
| `a_menu_built_on_a_does_nothing_after_switching_back` (step 3) | `app_shell_write_tests.rs` | (c) menu built on A, A → B → A: `row.session.upgrade()` is `None`, no dialog, no request |
| `a_delete_read_landing_after_a_switch_opens_nothing` | `app_shell_delete_tests.rs` | (a) `delete_start` reads on A, switch to B before they land: `still_ready` false, no dialog, nothing sent |
| `a_node_shell_create_landing_after_a_switch_opens_no_tab` | `node_shell_open_tests.rs` | (b) create in flight on A, confirmed switch to B: no tab in the dock, the pod delete runs on A's held connection, the audit says so |
| `a_leftover_notice_landing_after_a_switch_is_dropped` (step 2) | `node_shell_sweep_tests.rs` | (d) the sweep of A answers after A → B: `sweep_notices` gains nothing |
| `a_switch_clears_ticks_of_a_same_named_row` | `app_shell_switch_tests.rs` | tick `shop/api` on A, switch to B with `shop/api`: no tick, bulk bar hidden, `checked_objects` empty |
| `a_running_batch_of_a_sends_nothing_to_b` | `app_shell_workload_tests.rs` | new, modeled on `a_lock_that_comes_on_mid_batch_stops_the_rest`: a batch on A, confirmed switch to B mid-batch; the rest read `Not sent`, no request reaches B |
| `forwards_keep_running_after_a_switch` | `port_forward_open_tests.rs` | port of `switch_cluster_keeps_forwards`; also asserts the forward keeps its last control (still accepts new local connections, decision 17) |
| existing `reconnect_bumps_the_generation`, `commit_rechecks_the_row_cluster_lock`, `held_enter_does_not_confirm`, `the_debug_policy_blocks_a_write_at_the_dry_run` | as today | unchanged |

## Reviewer checks (step 4)

- `git diff <step-1 base> -- clippy.toml crates/cluster crates/app/src/fresh_enter.rs crates/app/src/audit_log.rs crates/app/src/write_guard.rs` is empty.
- `grep -n "Select rows of one cluster" crates/app/src` still lists the 5 production sites.
- `guard_for` body still ends with `.filter(|guard| guard.cluster == *cluster)`.
