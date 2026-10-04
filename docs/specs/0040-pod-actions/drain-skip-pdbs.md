# 0040 · Drain: Skip PodDisruptionBudgets

[Back to index](README.md) · Step 4 · Decisions 17–20. Wireframe: W6 option `Skip PodDisruptionBudgets` (danger) and note 2 (`--disable-eviction`). Modules: `drain_plan.rs`, `drain_dialog.rs`, `drain_writes.rs`, `drain_run.rs`, `drain_driver.rs`, `drain_tab.rs`, `audit_log.rs`, `write_guard.rs` (doc comment), `screenshot.rs`. Closes 0034 open item 1 and decision 22.

## Option (`drain_plan.rs`)

```rust
/// Whether the drain asks the eviction API (which checks budgets) or deletes pods directly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum BudgetPolicy { #[default] Respect, /** kubectl --disable-eviction */ Skip }
pub(crate) struct DrainOptions { /* 0034 fields */ pub(crate) budgets: BudgetPolicy }
pub(crate) enum Budget { /* 0034 variants */ /** Skip: the budgets that would have been checked */ Bypassed { names: Vec<String> } }
```

- `DrainOptions::default()` keeps `Respect`; the dialog builds a fresh default on every open, so the option is **never remembered**.
- `pod_verdict` with `Skip`: row 7 (budgets) gives `Evict(Budget::Bypassed { names })` for a pod matched by one or more budgets (the two-budget `Refused` row is gone in this mode), `Evict(Budget::None)` for the rest. Rows 1–6 (mirror, terminating, finished, DaemonSet, unmanaged, emptyDir) are unchanged: the other options still apply.
- `run_verdict` is unchanged (it never reads budgets).

## Dialog (`drain_dialog.rs`)

| Part | `Respect` (0034) | `Skip` |
|---|---|---|
| Checkbox | — | enabled when the lazy `AccessCheck::Delete(ObjectKind::Pod)` allows it (the check Delete pod and Restart pod read; the dialog calls `request_kind_access(Pod)` on open, since the Nodes screen does not); `Checking permissions…` until it answers; else `Not permitted: delete pods`. Disabled while `is_checking` (a dry-run runs) and while a commit runs |
| Steps strip | `2 Evict {n} pods` | `2 Delete {n} pods` |
| Preview `Evict(Bypassed)` | — | `Deleted directly; PDB {name} not checked` (`{name} and {k} more`), warn tone, sorted with `Waits` |
| HEADS UP | budget waits (0034) | danger tone: `PodDisruptionBudgets are not checked. {k} pods protected by {pdb}, {pdb2} go down without waiting for replacements.` (only when k > 0) |
| Grace select | 0034 choices | fixed `Pod default`, disabled, muted `Deletes use each pod's own grace period` |
| Dry-run line | `… evictions accepted, … refused by PDB` | `Server dry-run: cordon passed · {a} of {n} deletes accepted` |
| Confirm tier, `Drain` | `confirm_step(mode, Destructive, expected)` | `confirm_step(mode, Privileged, expected)`: TypeName in **every** tier; node name for one node, cluster name for several |
| Confirm tier, `Cordon only` | 0034 | unchanged: a cordon never touches budgets. On a Click tier the typed field shows for `Drain` only, and `Cordon only` does not wait for it |
| Buttons | `Cordon only` · `Drain wk-04` | same labels, danger primary |

- Toggling (possible only while `is_checking` is false, so no dry-run is in flight) clears `checks.pods` and `checks.elapsed` and dry-runs **every** pod again (the request kind changed); the cordon dry-runs stand. With nothing in flight there is no late answer to drop.
- The dialog's own gate is unchanged (`create pods/eviction`, `patch nodes`): a user who may delete but not evict cannot open it.
- `live_tier` takes the button: `Drain` reads the policy (`Privileged` while ticked, the 0034 rule otherwise; unticking returns to the open-time tier); `Cordon only` always reads the 0034 rule.
- `Confirmed` holds the session generation, not a per-policy one; what keeps a toggle honest is the cleared `checks.pods`: the aggregate is `Running` again until every pod has a dry-run of the current policy, so `Drain` stays disabled until then. The run skips its own dry-run for uids the dialog already recorded as accepted or refused (0034 as built), and after a toggle those records are all of the current policy.

## Writes (`drain_writes.rs`)

```rust
/// The eviction or the direct delete of `pod`, pinned to its uid. Replaces `evict_write`.
pub(crate) fn removal_write(scope: DrainScope<'_>, pod: &PodKey, options: &DrainOptions) -> Option<WriteIntent>;
```

| Policy | Request | `label` | `button` (audit action) |
|---|---|---|---|
| `Respect` | `EvictPod { uid, grace: options.grace }` | `Evict pod {ns}/{name}` | `Evict` |
| `Skip` | `DeleteObject { uid, propagation: Background }` | `Delete pod {ns}/{name}` | `Delete` |

`action` stays `Drain` and risk `Destructive` for both (the gate of the run is the drain's). The three call sites (dialog dry-run, driver dry-run, driver commit) call `removal_write` with the run's options.

## Run (`drain_run.rs`, `drain_driver.rs`, `drain_tab.rs`)

- The state machine is unchanged: a delete answers like an eviction (`Ok` → `Evicted`, awaited by the 3 s poll; `NotFound` / `Conflict` → `Gone`; `OutcomeUnknown` → retried with the uid pin; a 429 from API priority and fairness → `Refused` with the 0034 backoff).
- Tab texts by policy: `Evicting…` → `Deleting…`; header `Evicting 12/23` → `Deleting 12/23`; end notice unchanged.
- A lock, switch, or reconnect stops the run as in 0034 (`commit_block` returns `Blocked`).

## Audit

- Per pod commit: action `Delete`, object the Pod, field `deleteOptions.propagationPolicy`; the dialog note.
- `drain_summary_entry` gains a field `disable_eviction` = `true`, written only for `Skip`; the counts keep their keys (`evicted` counts pods removed either way).

## Ceiling

- ponytail: deletes use each pod's own `terminationGracePeriodSeconds`; the W6 grace choice applies to evictions only. Upgrade path: a `grace` field on `DeleteObject` (a 0033 wire change and its tests) if users need a shorter grace with Skip.

## Screen

`--screen drain-dialog-skip-pdbs`: the 0034 `drain-dialog` fixture with the option ticked: `Deleted directly; PDB api-pdb not checked` rows, the danger HEADS UP, the grace select fixed, the steps strip `Delete 24 pods`, and the typed node name field on a Staging cluster (typed in every tier).
