# 0031 · Cluster crate: paths, changes, preview, checks

[Back to index](README.md) · Steps 1 (`FieldPath`, `field_paths`) and 2 (the rest) · Module: `edit_preview.rs` (new) + `edit_preview_tests.rs`. Decisions 7, 9–10, 18, 21.

## API

```rust
/// One field location. Display: `.key` when the key matches [A-Za-z0-9_-]+, else ["key"]; items `[name]` or `[index]`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FieldPath(Vec<PathSegment>);
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PathSegment { Key(String), Name(String), Index(usize) }
pub(crate) fn field_paths(before: &Value, after: &Value) -> Vec<FieldPath>;               // step 1
/// Server effect of an edit. Manual Debug (change count); never audited, never in a notification.
#[derive(Clone, PartialEq, Eq)]
pub struct EditPreview { pub before: String, pub after: String, pub changes: Vec<FieldChange>, pub more_changes: usize,
    pub checks: Vec<EditCheck> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldChange { pub path: FieldPath, pub old: Option<String>, pub new: Option<String> } // None: absent
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditCheck { Rollout { strategy: String }, StaleLastApplied, Moved { path: FieldPath }, LeadingZero { line: usize } }
pub(crate) fn build_preview(edit: &ObjectEdit, fresh: Value, response: Value, restored: &Restored) -> EditPreview; // step 2
```

Examples: `spec.template.spec.containers[api].resources.limits.memory`, `metadata.labels["app.kubernetes.io/name"]`, `spec.template.spec.containers[api].env[3].value` (duplicate env names, index matched).

## `field_paths` / changes

- Recursive over objects. Lists use the location rule of `edit_placeholders` (by unique `name`, else by index). A named item that is added or removed is one path (`[name]`). An index list of a different length is one path (the list). A type change, or an added or removed subtree, is one path at its root.
- `FieldChange` values: scalars as text cut to 80 chars, `{…}` / `[…]` for containers, `None` when absent. At most 200 changes; the rest go into `more_changes`.
- Values come **only from masked trees**.

## `build_preview` (step 2, called by the write)

1. Strip both `fresh` and `response` (decision 12).
2. Mask copies of both with `edit.env`.
3. Mark the masked after-tree: a `<hidden>` whose raw counterparts differ → `<hidden, changed>`; each `restored.moved` path → `<hidden, moved>` (wins over `changed`).
4. `changes = field_changes(masked_before, masked_after)`.
5. `before` / `after` = `to_yaml_text(masked, None)`: **no header** (decision 10).
6. `checks`, in this order:
   - `Rollout { strategy }`: kind is Deployment, StatefulSet, or DaemonSet and a change path starts with `spec.template`. The strategy comes from `spec.strategy.type` or `spec.updateStrategy.type` of the masked after-tree, default `RollingUpdate`.
   - `StaleLastApplied`: the fresh object has `kubectl.kubernetes.io/last-applied-configuration`.
   - `Moved` per `restored.moved` path.
   - `LeadingZero` per `edit.leading_zero_lines()`.

The raw `fresh` and `response` are dropped at the end of the function.

## App texts (side panel Checks, dialog warnings)

| Check | Text |
|---|---|
| `Rollout` | `Pods will be replaced ({strategy})`, or `Pods change only when they are deleted (OnDelete)` |
| `StaleLastApplied` | `kubectl apply users: the last-applied annotation is not updated, so a later kubectl apply can revert this change` |
| `Moved` | `{path}: a hidden value was matched by position; check it belongs to this item` |
| `LeadingZero` | `Line {n}: 0755 is read as 755 (YAML 1.2); write 493 or 0o755` |

The dialog's `GuardedIntent.warnings` (0030) receives the same texts.
