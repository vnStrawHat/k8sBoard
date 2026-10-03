# 0032 · Actions in menus, drawer, keys, palette

[Back to index](README.md) · Steps 2a-i, 2a-ii, 2a-iii · Modules: `resource_actions.rs`, `resource_kind.rs`, `keyboard_navigation.rs`, `palette_search.rs`, `command_palette.rs`, `keymap.rs`, `workload_actions.rs` (new), `value_popover.rs` (new), `live_sections.rs`. Decisions 10–18, 30–31.

## Key layer: `RowAction` (2a-i, first commit; shared with 0031 and 0033)

On main the 0028 key layer passes `ResourceAction` values around: `on_row_key::<Scale>(root, ResourceAction::Scale, cx)`, palette `ROW_ACTIONS: [ResourceAction; 11]`, `PaletteTarget::RowAction(ResourceAction)`, `key_availability_of(action, subject, pod, guard)` (an `is_offered` match, then `subject_action`), and `ResourceAction::key_action()`. A kind-carrying variant (`Scale(ObjectKind)`) has no value before the subject is known, so the layer gets a kind-less name (decision 30):

```rust
/// A row action as a key, a menu hint, or the palette names it, before the subject is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowAction { ViewLogs, ViewYaml, CopyName, OpenShell, PortForward, Cordon, Drain, EditYaml,
    RestartRollout, Scale, Delete, PauseRollout, RollBack, SuspendCronJob, TriggerCronJob, RerunJob }
impl RowAction { pub(crate) fn key_action(self) -> Box<dyn Action>; }      // exhaustive (0029 decision 22 moves here)
impl ResourceAction { pub(crate) fn row_action(self) -> RowAction; }       // total: OpenNodeShell → OpenShell, Scale(_) → Scale
/// The subject's resolved action, `None` = not offered (replaces the `is_offered` match and today's `subject_action`).
pub(crate) fn subject_action(row: RowAction, subject: &ResourceKey) -> Option<ResourceAction>;
pub(crate) fn key_availability_of(row: RowAction, subject: &ResourceKey, pod: Option<&PodSummary>, guard: &ClusterGuard<'_>) -> KeyAvailability;
```

- `KeyAvailability::Run` carries the resolved `ResourceAction`; `run_row_key(row)` resolves, gates on the cursor slot (`slot_live` + `guard_for` of `self.selected.cluster`, as today), then calls the exhaustive `run_available_row_key(action: ResourceAction, subject: ClusterObject)`.
- Resolution for a `Kind { kind }` subject reads the kind table: the `KindAction` whose `action.row_action() == row` (so `KindAction::keyed("Scale…", ResourceAction::Scale(ObjectKind::Deployment))` is the one source); `Delete` and `EditYaml` resolve from `kind.builtin_object()` (0031, 0033 exclude Helm releases and custom kinds).
- Palette: `ROW_ACTIONS: [RowAction; 16]`, `PaletteTarget::RowAction(RowAction)`, `row_action_icon(RowAction)`; labels `action_label(resolved)`. Test `every_offered_row_action_maps` (0029) keeps its meaning.
- Menus: `action_item(action, guard)` uses `action.row_action().key_action()` for the hint and dispatch. Every later `ResourceAction` (0032b, 0034, 0037) adds a `RowAction` and a unit gpui action (unbound unless the wireframe binds a key), compiler-enforced.

## One entry point (decision 31)

A menu item without an argument is `action_item(action, guard)` with **no `on_click`**: the kit menu dispatches its key action through the trigger focus, a right click has already moved the cursor to that row (0027 `move_cursor_to_clicked_row`), and the drawer ⋯ menu's subject is the cursor. So the menu, the key, and the palette (which dispatches the same key action and never confirms a disabled entry) all end in the action's arm of `run_available_row_key`. A disabled element item dispatches too, and `run_row_key` shows the 0028 notice. Items with an argument (a revision, a replica count) keep an `on_click` that captures the `RowContext` of their slot.

## Actions and gate (2a-i)

`ResourceAction` changes `Scale` and `RestartRollout` (unit variants, `ActionGate::Planned` on main) into `Scale(ObjectKind)` and `RestartRollout(ObjectKind)`, and gains `PauseRollout`, `RollBack`, `SuspendCronJob`, `TriggerCronJob`, `RerunJob`. All are `ActionGate::Mutating { check, is_shipped }` (merged enum).

- `action_availability(action, guard)` stays **two-argument** (0030 decision 35). `gate()` reads the carried kind: `Scale(Deployment)` → `PatchDeploymentScale`, `Scale(StatefulSet)` → `PatchStatefulSetScale`, `RestartRollout(DaemonSet)` → `PatchDaemonSets`, …; the single-kind variants map directly (write-operations.md).
- Then `row_block(action, &KindObject) -> Option<SharedString>` (per row, after the gate is Enabled; `row_action_item(action, guard, row)` shows it, and the arm re-checks it on the cursor row read from `slot_live`):

| Action | `row_block` reason |
|---|---|
| `RestartRollout(Deployment)` | paused → `Resume the rollout first` (kubectl refuses) |
| `RollBack` | paused → `Resume the rollout first`; revisions not loaded → `Open the deployment to load its revisions`; no older revision → `No earlier revision` |

Labels follow state: `Pause rollout` / `Resume rollout` (`is_paused`), `Suspend` / `Resume` (`is_suspended`), palette `Roll back to rev {n}` (n = highest owned revision below the current one, 0012 `revision_rows`).

## Intent builders (pure, `workload_actions.rs`)

Every builder returns a 0030 `GuardedIntent { cluster, kind: Write(request), warnings, .. }` (0030 step 4); `cluster` = the subject's `ClusterObject.cluster`; expected typed name = the cluster display name (`guard.display_name()`); risk: Scale to 0 `Destructive`, the rest `Change`.

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

- On main `kind_menu` renders `kind.read_only_actions()` as disabled `NOT_SHIPPED_REASON` items; 2a-i builds every item with `action: Some(..)` through `row_action_item`. Every W7 workload `KindAction` gets an action (`KindAction::keyed` with the kind's `ObjectKind`).
- The arms: `Scale(_)` opens the popover for the cursor row; the others build their intent and call `run_guarded` (the 0030 confirm dialog always opens, decision 9).
- New unit actions without default bindings, registered with `on_row_key` so the menu and the palette can dispatch them: `PauseRollout`, `RollBack`, `SuspendCronJob`, `TriggerCronJob`, `RerunJob`, plus `ScaleCursorRow` (palette argument). Not on the shortcut sheet (`every_bound_action_is_on_the_sheet` covers bound ones only).

## Scale popover: `ValuePopover` with `ValueForm::Replicas` (2a-ii)

No replicas input in the drawer (W7 note 4). `ValuePopover { form: ValueForm, targets: ValueTargets, cluster: ClusterRef }`; `ValueForm::Replicas { input: Entity<InputState> }` (0032b adds forms); `ValueTargets::{One(KindRow), Ticked(Vec<CheckedRow>)}`.

| Entry point | Opens the popover | Submit |
|---|---|---|
| menu `Scale…`, ⇧S, palette ⏎ on `Scale` | the `Scale(_)` arm, for the cursor row | `Scale` or Enter → `run_guarded` (0030 dialog) |
| selection bar `Scale…` | for the ticked rows | → `Batch` (bulk-write.md) |

- Kit `Popover`, anchored to the selection-bar button for bulk, otherwise at the selection-bar position (bottom center). Content: title `Scale deployment/api` (or `Scale 3 deployments`), `NumberInput` (min 0; prefilled with `desired` for one row, empty for bulk), muted `Now 3 desired · 2 ready`, the warnings as you type, `Cancel` · `Scale` (disabled while empty, unchanged, or not a `u32`).

## Palette (W9) and Roll back (2a-iii)

- Row actions of the cursor row show the "needs confirm" pill. **`Ctrl ⏎`** (`secondary-enter` → `ScaleCursorRow` in `Command > Input`, the merged `PALETTE_INPUT` context; out of `RESERVED_KEYS`) on an enabled Scale entry turns the input into `Replicas for deployment/payments-api (now 3)`; Enter with a valid `u32` closes the palette and runs `run_guarded(scale intent)`; otherwise `Enter a whole number`; Esc returns to the list.
- `Roll back to rev {n}` only from loaded revisions (no new list calls).
- Drawer **Revisions** (0012 `revision_element`): the disabled `Roll back` button becomes gated on the drawer subject's slot (`drawer_subject()`, never the primary); click → `run_guarded(roll_back_intent(..))`. No button on the current revision. Menu `Roll back…` → open the cursor row's drawer scrolled to Revisions (`set_drawer_open(true)`).
- No optimistic UI: rows and drawers update from the existing watches.
