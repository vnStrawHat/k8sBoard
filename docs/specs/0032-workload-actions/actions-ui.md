# 0032 · Actions in menus, drawer, keys, palette

[Back to index](README.md) · Steps 2a-i, 2a-ii, 2a-iii · Modules: `resource_actions.rs`, `resource_kind.rs`, `workload_actions.rs` (new), `value_popover.rs` (new), `live_sections.rs`, `keymap.rs`, `keyboard_navigation.rs`, `command_palette.rs`. Decisions 10–18.

## Actions and gate (2a-i)

`ResourceAction` (0028) changes `Scale` and `RestartRollout` into `Scale(ObjectKind)` and `RestartRollout(ObjectKind)`, and gains `PauseRollout`, `RollBack`, `SuspendCronJob`, `TriggerCronJob`, `RerunJob`. All are `mutates: true`.

- `action_availability(action, guard)` stays **two-argument** (0030 decision 35). `gate()` reads the kind the variant carries: `Scale(Deployment)` → `PatchDeploymentScale`, `Scale(StatefulSet)` → `PatchStatefulSetScale`, `RestartRollout(DaemonSet)` → `PatchDaemonSets`, …; the single-kind variants map directly (write-operations.md).
- Ripple: 0028 `key_availability` builds `Scale(subject kind)`; 0028/0029 matches become `Scale(_)` / `RestartRollout(_)` (`action_for`, `action_label`). 0036 is unchanged.
- Then `row_block(action, &KindObject) -> Option<SharedString>` (per row, after the gate Enabled):

| Action | `row_block` reason |
|---|---|
| `RestartRollout(Deployment)` | paused → `Resume the rollout first` (kubectl refuses) |
| `RollBack` | paused → `Resume the rollout first`; revisions not loaded → `Open the deployment to load its revisions`; no older revision → `No earlier revision` |

Labels follow state: `Pause rollout` / `Resume rollout` (`is_paused`), `Suspend` / `Resume` (`is_suspended`), palette `Roll back to rev {n}` (n = highest owned revision below the current one, 0012 `revision_rows`).

## Intent builders (pure, `workload_actions.rs`)

Every builder returns a 0030 `GuardedIntent { kind: Write(request), warnings, .. }`; expected typed name = cluster display name; risk: Scale to 0 `Destructive`, the rest `Change`.

| Action | Label | `warnings` (each only when it applies) |
|---|---|---|
| Scale | `Scale deployment api from 3 to 5` | `Scaling down from 3 to 1`; `HPA {name} manages replicas ({min}–{max}); it will override this` (HPA list loaded and targets the row) |
| Restart | `Restart rollout of statefulset kafka` | `Update strategy OnDelete: pods restart only when deleted` |
| Pause / Resume | `Pause rollout of deployment api` | — |
| Roll back | `Roll back deployment api to rev 37 (2.13.4)` | — |
| Suspend / Resume | `Suspend cronjob reconcile` | — |
| Trigger now | `Run cronjob reconcile now` | `{n} job(s) of this CronJob are running; this run starts anyway`; Forbid → `While this run is active, scheduled runs are skipped (concurrency Forbid)`; Replace → `A scheduled run replaces this job if it is still running (concurrency Replace)` |
| Re-run | `Re-run job etl-nightly-29312400` | — |

After a create, the 0030 notice reads `Run cronjob reconcile now: created reconcile-manual-x7k2p` (`created_name`).

## Menus (W7 order) and keys

| Kind | Items (key) |
|---|---|
| Deployments | `Scale…` (⇧S), `Restart rollout` (R), `Roll back…`, `Pause rollout`/`Resume rollout` |
| StatefulSets | `Scale…` (⇧S), `Restart rollout` (R) |
| DaemonSets | `Restart rollout` (R) |
| Jobs | `Re-run job` |
| CronJobs | `Trigger now`, `Suspend`/`Resume` |

- `KindSpec.read_only_actions` (0028 `KindAction`) gets `action: Some(..)` for every item; items use `action_item` (gate + `row_block`), so menus, keys, and palette agree.
- Plain items → `run_guarded(intent, Trigger::Pointer)`; R → the same with `Trigger::Key` on the cursor row.
- New unit key actions without default bindings (so the palette can dispatch them): `PauseRollout`, `RollBack`, `SuspendCronJob`, `TriggerCronJob`, `RerunJob`, plus `ScaleCursorRow` (palette only). Not on the shortcut sheet.

## Scale popover: `ValuePopover` with `ValueForm::Replicas` (2a-ii; one surface for every Scale entry point)

There is **no replicas input in the drawer** (W7 note 4: no action buttons in the drawer; actions live in the context and ⋯ menus). The `Desired` field stays a plain value. The per-revision `Roll back` buttons stay: W7 draws them inside the Revisions section.

```rust
pub(crate) struct ValuePopover { form: ValueForm, targets: ValueTargets, cluster: ClusterRef }
/// Each form owns its inputs. 0032 ships `Replicas`; 0032b adds `ReplicaRange { min, max }` and `Storage { input }`.
pub(crate) enum ValueForm { Replicas { input: Entity<InputState> } }
pub(crate) enum ValueTargets { One(KindRow), Ticked(Vec<CheckedRow>) /* bulk, bulk-write.md */ }
```

| Entry point | Opens the popover | Commit trigger |
|---|---|---|
| menu `Scale…` | for that row | click `Scale` → Pointer; Enter in the input → Key |
| ⇧S | for the cursor row | same |
| palette ⏎ on the `Scale` entry (fallback) | dispatches ⇧S after the palette closes | same |
| selection bar `Scale…` | for the ticked rows | → `Batch` (bulk-write.md) |

- Kit `Popover`, anchored to the selection-bar button for bulk, otherwise floating at the selection-bar position (bottom center of the table). Content: title `Scale deployment/api` (or `Scale 3 deployments`), `NumberInput` (min 0, step 1; prefilled with `desired` for one row, empty for bulk), muted `Now 3 desired · 2 ready` (one row), the intent's warnings as you type, `Cancel` · `Scale` (disabled while empty, unchanged, or not a `u32`).
- `Scale` closes the popover and calls `run_guarded` (one row) or `run_guarded` with `Batch` (ticked rows).

## Palette (W9)

- Row actions of the cursor row show the "needs confirm" pill (keys always get a dialog, 0030 decision 9).
- **Inline argument (W9 `Ctrl ⏎`)**: `Ctrl ⏎` (`secondary-enter`, context `CommandPalette`, action `ScaleCursorRow`) while the cursor row offers Scale turns the input into `Replicas for deployment/payments-api (now 3)` with `3` selected and footer `⏎ scale · Esc back`. Enter with a valid `u32` closes the palette and runs `run_guarded(scale intent, Trigger::Key)`; otherwise `Enter a whole number`. Esc returns to the list. `secondary-enter` leaves 0028 `RESERVED_KEYS`.
- `Roll back to rev {n}` only from loaded revisions (0029: no new list calls); otherwise disabled with the `row_block` reason.

## Roll back (2a-iii)

- Drawer **Revisions** (0012 `revision_element`): the disabled `Roll back` button becomes gated (`action_item` rules); click → `run_guarded(roll_back_intent(deployment, set), Pointer)`. No button on the current revision, nor on a revision whose ReplicaSet is the current one.
- Menu `Roll back…` → reveal the row, open its drawer scrolled to Revisions. Palette entry → the previous revision.
- The crate also refuses a target with the current template (write-operations.md step 3), so a stale list cannot roll back to "itself".

No optimistic UI: rows and drawers update from the existing watches.
