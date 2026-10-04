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
| Checkbox | — | enabled when the session report allows `AccessCheck::DeletePods` (`delete pods`, 0037); else disabled with `Not permitted: delete pods`; while a dry-run or commit runs, disabled |
| Steps strip | `2 Evict {n} pods` | `2 Delete {n} pods` |
| Preview `Evict(Bypassed)` | — | `Deleted directly; PDB {name} not checked` (`{name} and {k} more`), warn tone, sorted with `Waits` |
| HEADS UP | budget waits (0034) | danger tone: `PodDisruptionBudgets are not checked. {k} pods protected by {pdb}, {pdb2} go down without waiting for replacements.` (only when k > 0) |
| Grace select | 0034 choices | fixed `Pod default`, disabled, muted `Deletes use each pod's own grace period` |
| Dry-run line | `… evictions accepted, … refused by PDB` | `Server dry-run: cordon passed · {a} of {n} deletes accepted` |
| Confirm tier | `confirm_step(mode, Destructive, expected)` | `confirm_step(mode, Privileged, expected)`: TypeName in **every** tier; node name for one node, cluster name for several |
| Buttons | `Cordon only` · `Drain wk-04` | same labels, danger primary; `Cordon only` shares the one typed field, so it also asks for the name |

- Toggling the option resets the eviction dry-run aggregate to `Running` and dry-runs **every** pod again (the request kind changed); the cordon dry-runs stand. A dry-run that answers after a toggle is dropped (keyed by the policy it was sent with).
- The dialog's own gate is unchanged (`create pods/eviction`, `patch nodes`): a user who may delete but not evict cannot open it.
- `live_tier` reads the policy: `Privileged` while ticked, the 0034 rule otherwise. Unticking returns to the open-time tier.
- The `Drain` button carries the policy into the run with the `Confirmed` of the current dry-run generation, so a toggle after the dry-run passed needs a new pass.

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
