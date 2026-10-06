# 0032b · Actions, popover forms, the two-object plan

[Back to index](README.md) · Steps 2–4 · Modules: `resource_actions.rs`, `resource_kind.rs`, `value_popover.rs` (0032), `resource_edits.rs` (new, pure intent builders), `row_selection.rs`. Decisions 7–15.

## Actions

`ResourceAction` gains `EditHpaRange`, `ExpandClaim`, `SetDefaultStorageClass` (single-kind, no carried kind; two-argument `action_availability`, 0030 decision 35), each with its `RowAction` and an unbound unit key action (0032 decision 30). `KindAction.action` is set for the W7 items `Edit min / max…`, `Expand…`, `Set as default` (on main they are `KindAction::named`, rendered disabled `Comes in a later version`). The items are `row_action_item`s without `on_click`; their `run_available_row_key` arms open the popover or build the intent for the cursor row (0032 decision 31), so the palette lists them too.

| Action | `row_block` reason (after the gate) |
|---|---|
| `EditHpaRange` | — |
| `ExpandClaim` | phase ≠ `Bound` → `Only a bound claim can be expanded`; terminating → `The claim is being deleted`; the class (StorageClasses list loaded) has `allows_expansion == false` → `Storage class {name} does not allow expansion` |
| `SetDefaultStorageClass` | `is_default` → `Already the default` |

## `ValuePopover` forms (0032 popover)

0032 ships `ValuePopover` with `ValueForm::Replicas { input }`; 0032b adds two forms, each owning its inputs. No rename. Anchoring, triggers, and the Pointer/Key rule stay 0032's.

```rust
pub(crate) enum ValueForm { Replicas { input: Entity<InputState> } /* 0032 */,
    ReplicaRange { min: Entity<InputState>, max: Entity<InputState> }, Storage { input: Entity<InputState> } }
```

| Form | Fields (prefill for one row; empty for ticked rows) | Enabled when |
|---|---|---|
| `ReplicaRange` | `Min` and `Max` `NumberInput`s (min 1), prefilled `3` / `20`; muted `Now 9 replicas` | both parse, `1 ≤ min ≤ max`, and the pair differs from the current one |
| `Storage` | one `Input` prefilled with the current request (`100Gi`); muted `Now 100Gi · class gp3` | `ByteAmount::parse(text.trim())` succeeds and the value is larger than `max(requested, capacity)` (0014 summary); the trimmed text is what the intent sends |

Invalid input shows the reason under the field: `Min must be at least 1`, `Min must not exceed max`, `Enter a size such as 150Gi`, `Must be larger than 100Gi`.

## Intents and warnings (`resource_edits.rs`, pure)

```rust
pub(crate) fn hpa_range_intent(cluster: &ClusterRef, hpa: &HorizontalPodAutoscalerSummary, min: u32, max: u32) -> Option<GuardedIntent>;
pub(crate) fn expand_intent(cluster: &ClusterRef, claim: &PersistentVolumeClaimSummary, storage: &str) -> Option<GuardedIntent>;
pub(crate) fn default_class_intent(cluster: &ClusterRef, target: &StorageClassSummary, classes: &[StorageClassSummary]) -> Option<GuardedIntent>;
```

| Action | Label | `warnings` (each only when it applies) |
|---|---|---|
| HPA | `Set replicas of hpa frontend-hpa to 3–20` | `The HPA will scale deployment/frontend down from 9 to {max}` (max < current); `… up from 2 to {min}` (min > current) |
| Expand | `Expand claim data-kafka-0 from 100Gi to 150Gi` | `A volume cannot shrink; this cannot be undone`; `A resize to {requested} is already in progress` (requested > capacity); `The file system grows when a pod mounts the claim` (condition `FileSystemResizePending`) |
| Set default | `Make gp3 the default storage class` | `New claims without a class will use gp3; existing claims keep their class`; `io2 stops being the default` (per old default) |

Expected typed name: the cluster display name (0030 default).

## Bulk (0032 `Batch`)

| Screen | Selection-bar button | Items |
|---|---|---|
| HPAs | `Edit min / max` (popover `ReplicaRange`, empty) | one `SetHpaReplicaRange` per ticked HPA; HPAs already at that range → skipped `already 3–20` |
| PVCs | `Expand` (popover `Storage`, empty) | one `ExpandClaim` per ticked claim; claims with `row_block` or a request ≥ the new size → skipped with the reason |
| StorageClasses | `Set default` | enabled with exactly one ticked class (`Tick one storage class`); same plan as the menu item |

All 0032 Batch rules hold: one cluster, ≤ 50, always a dialog, every dry-run must pass, sequential commits through `checked_write`, stop on `Blocked`.

## Set default: the two-object write

`default_class_intent` builds `GuardedIntent { action: SetDefaultStorageClass, risk: Change, kind: Batch(BatchPlan { items, skipped: [], extras: BatchExtras::None, on_failure: BatchFailure::Stop }) }`:

1. **First item**: the target, `SetDefaultStorageClass { is_default: true }`, **omitted when `target.is_default`** (a Retry, see below).
2. **Then** one item per other class with `is_default` (normally one; 0014 open item 2 allows several), `{ is_default: false }`, in name order.

- Set before unset, so there is never a moment without a default. Two defaults for a moment are allowed: the API server uses the **newest by `creationTimestamp`** (Kubernetes 1.26+).
- `BatchFailure::Stop`: if item 1 fails, no unset is sent and the old default stays the default (nothing changed). If an unset fails, the rest are not sent.
- The plan comes from the StorageClasses list on screen (loaded); no extra read. No previous default → one item.
- **Partial failure** (item 1 applied, an unset failed or was `Blocked`): the generic 0032 `Stop` notice plus `Retry`. The retry plan's `warnings` carry the state, from `resource_edits.rs` (`two_defaults_text`, not `write_flow.rs`): `Both {target} and {old} are marked default; the cluster uses the newer one ({name}) until {old} is unset`, with `{name}` the class with the later `created_at`.
- **Retry** calls `default_class_intent` again from the current list and **bypasses `row_block`** (the target is now `is_default`, so the normal path would read `Already the default`); item 1 is omitted, only the unsets remain. If the watch has not yet shown the first commit, item 1 is planned again: a repeated `true` patch is harmless.
- Audit: one line per class (0032 Batch rule), so the log shows both halves.
