# 0040 · Restart pod and Evict

[Back to index](README.md) · Step 3 · Decisions 1, 2, 10–16. Wireframe: W4 menu (`Restart pod` · `delete & recreate`, `Evict`). Modules: `object_delete.rs` (+ tests), `resource_actions.rs`, `keymap.rs`, `keyboard_navigation.rs`, `palette_search.rs`, `screenshot.rs`.

## Actions and gates (`resource_actions.rs`)

| Item | `RestartPod` | `EvictPod` |
|---|---|---|
| `RowAction`, key action | `RestartPod`, unbound unit action | `EvictPod`, unbound unit action |
| `subject_action` | Pod only | Pod only |
| `gate()` | `Mutating { checks: [Delete(ObjectKind::Pod)] }` (the lazy check Delete pod reads, so the two items agree) | `Mutating { checks: [CreatePodEviction] }` |
| `action_risk`, `action_label` | `Destructive`, `Restart pod` | `Destructive`, `Evict` |

```rust
/// Why this pod cannot take `action`, `None` when it can. Pure; menus, keys, palette, and the start read it.
pub(crate) fn pod_block(action: ResourceAction, pod: &PodSummary) -> Option<SharedString>;
```

| Pod | Restart pod | Evict |
|---|---|---|
| no controller | `Not managed by a controller; it would not come back. Use Delete pod…` | allowed, warning |
| controller kind `Node` (static) | `Static pod: the kubelet owns it` | `Static pod: the kubelet owns it` |
| terminating: row `PodStatus::Terminating`, or `identity.deletion_started` at the uid read | `Already terminating` | `Already terminating` |
| `pod.is_finished` (phase `Succeeded` / `Failed`, step 1; never the status reason, which reads `Error`, `Completed`, `OOMKilled` on running pods too) | `The pod has finished; its controller does not restart it` | allowed (the API deletes it without a budget check) |
| otherwise | allowed | allowed |

The uid read re-checks the one fact the row may lag on: `finish_delete_start` refuses Restart and Evict with `Already terminating` when `identity.deletion_started` is set (nothing sent, no dialog).

`key_availability_of`: for these two actions on a pod, the gate first, then `pod_block` (the reason the user can act on first comes first, like `row_availability`). The palette lists `> Restart pod` and `> Evict` with `needs confirm`; disabled entries never run.

## Menu (`pod_menu`, W4 order)

`View logs ▸` · `Open shell ▸` · [`Debug container…`] · `Port-forward ▸` · `Attach` (A) · separator · `Edit YAML` (E) · `View YAML` (Y) · `Restart pod` (muted `delete & recreate`) · `Evict` · separator · `Copy name` · `Copy kubectl command` · separator · `Delete pod…`. Restart and Evict items have no `on_click`: they dispatch their unit action, whose `run_available_row_key` arm calls `start_removal(Removal::{Restart, Evict}, vec![subject])` on the cursor pod. A disabled item shows the gate or block reason under its label.

## One start for three removals (`object_delete.rs`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Removal { Delete, Restart, Evict }
pub(crate) struct DeleteExtras { pub(crate) removal: Removal, /* propagation, kind, targets, already_gone as 0033 */ }
impl AppShell {
    /// `start_delete` renamed; Delete keeps its scope rule, Restart and Evict always get the cursor pod alone.
    pub(crate) fn start_removal(&mut self, removal: Removal, scope: Vec<ClusterObject>, window: &mut Window, cx: &mut Context<Self>);
}
```

- `delete_gate`, `delete_plan`, `finish_delete_start` take the `Removal`: the action they check is `Delete(kind)`, `RestartPod`, or `EvictPod`, plus `pod_block` for the last two (re-read in `finish_delete_start` together with the generation: `still_ready`).
- Everything else of the 0033 start is reused unchanged: the `delete_start` guard (a held key reads once), `Reading 1 object…`, `object_identity` (metadata GET), 404 → `{ns}/{name} not found (already deleted or not served)`, other read errors → `Could not read {name} to pin its uid (…); nothing was deleted` (true of all three).
- `DeleteTarget::request(removal, propagation)`: `Delete` / `Restart` → `DeleteObject { uid, propagation }` (always `Background` for a pod); `Evict` → `EvictPod { uid, grace: GracePeriod::PodDefault }`. `with_propagation` is unreachable for pods (`owns_dependents` is false) and keeps the removal.
- `TargetFacts::Pod { has_controller }` becomes `Pod { controller: Option<ControllerRef> }` (the warnings name the owner kind).

## Batch texts (`delete_batch` by removal)

| Field | Delete (0033, unchanged) | Restart | Evict |
|---|---|---|---|
| `action` | `Delete(kind)` | `RestartPod` | `EvictPod` |
| `label` (title) | `Delete pod` | `Restart pod` | `Evict pod` |
| `verb` (button) | `Delete` | `Restart` | `Evict` |
| `button` (audit action) | `Delete` | `Restart pod` | `Evict` |
| item `label` | `Delete pod {ns}/{name}` | `Restart pod {ns}/{name}` | `Evict pod {ns}/{name}` |
| `expected_name` | pod name | pod name | pod name |

## Warnings (warning tone, after the 0033 finalizer lines)

The 0033 `kind_warnings` run for `Removal::Delete` only, so the bare-pod line of an Evict appears once (from the table below). Restart and Evict use only these lines:

| Removal | Condition | Line |
|---|---|---|
| Restart | always | `Restart deletes the pod without checking PodDisruptionBudgets; Evict checks them` |
| Restart | owner `StatefulSet` | `The replacement keeps the name {name} and its volume claims` |
| Restart | owner `Job` | `A Job may count the deleted pod as failed toward its backoffLimit` |
| Evict | no controller | `Not managed by a controller; it will not come back` |
| Evict | owner `DaemonSet` | `A DaemonSet pod is recreated on the same node at once` |

## Dry-run and results

| Answer | Dry-run row | Commit notice |
|---|---|---|
| success | `passed · {ms}` | Restart: `Restart pod {ns}/{name}: terminating; its controller creates a replacement`. Evict: `Evict pod {ns}/{name}: accepted; the pod is terminating` |
| 429 (Evict: PDB) | `refused for now: {first cause}` (`Rejected`): Apply stays off; close and evict later | `Evict pod {ns}/{name} failed: refused for now: {cause}`; **not audited** (0030 decision 36) |
| 404 | already gone (skipped) | `{ns}/{name} was already deleted` (from `item.object`, not by trimming the label) |
| 409 (uid precondition) | `A new object with this name exists; nothing was deleted` | the same text |
| other | 0030 texts | 0030 texts; `OutcomeUnknown` → `the outcome is unknown…` |

No automatic retry of a 429 (decision 15). The Retry of the 0030 dialog, where offered, re-runs the dry-run.

## Audit

One line per commit through `checked_write`: `Restart pod` with `deleteOptions.propagationPolicy = Background`; `Evict` with `pods/eviction = grace pod default`. Note checkbox as 0030. Dry-runs and 429 refusals write none.

## Screens (screenshot feature, fixed data, dead buttons, as `delete-confirm`)

| Screen | Fixture |
|---|---|
| `restart-pod-confirm` | PROD TypeName tier, a StatefulSet pod `data/kafka-1`, row `passed · 112 ms`, the PDB and StatefulSet warnings, pod-name field empty |
| `evict-confirm` | STG Click tier, `payments/api-7d9f8c-m8n2p`, row `refused for now: The disruption budget api-pdb needs 2 healthy pods and has 2 currently`, Apply disabled |
