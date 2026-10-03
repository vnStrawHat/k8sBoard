# 0028 · Single-letter row actions

[Back to index](README.md) · Step 3 · Modules: `resource_actions.rs`, `resource_kind.rs`, `keyboard_navigation.rs`. Decisions 17–22.

Wireframe rule: "single-letter keys work only while a resource is selected and no text field has focus; destructive actions always open a confirmation". The subject is `self.selected` (the cursor), with the drawer open or closed. No selection → the key does nothing.

## One availability source

`ResourceAction` (existing) gains five variants: `ViewYaml`, `EditYaml`, `Delete`, `RestartRollout`, and `Scale`. `gate()` returns `None` for all five. In `action_availability`, the no-gate `match` changes its `Enabled` arm from `ResourceAction::CopyName` to `ResourceAction::CopyName | ResourceAction::ViewYaml`. The other four fall into the existing `_ => disabled(READ_ONLY_MODE_REASON)` arm, as `Cordon` and `Drain` do. There is no `Attach` variant and no A key: a later item (no longer owned by 0036) adds both with the Attach menu item.

```rust
pub(crate) enum KeyAvailability { Run, Disabled { reason: SharedString }, NotOffered }
/// What a key for `action` does on `subject`; menus and keys read the same gates.
pub(crate) fn key_availability(action: ResourceAction, subject: &ResourceKey, live: &LiveCluster) -> KeyAvailability;
```

| Key | `ResourceAction` | Offered for | Runs (when enabled) |
|---|---|---|---|
| L | `ViewLogs` | pods (0019 extends to workloads) | `LogDock::open` for the pod, like the menu; a pod with no containers → `Disabled` "The pod has no containers" |
| Y | `ViewYaml` | subjects with `object_ref` (every YAML tab) | `open_yaml(key)` |
| Ctrl C | `CopyName` | all | clipboard ← object name (pods: name only, like the menu) |
| S | `OpenShell` (pods), `OpenNodeShell` (nodes) | pods, nodes | — (gated) |
| F | `PortForward` | pods, kinds with `has_port_forward` | — (gated) |
| C / D | `Cordon` / `Drain` | nodes | — (gated, 0034) |
| E | `EditYaml` | subjects with `object_ref` | — (gated, 0031) |
| R / ⇧S | `RestartRollout` / `Scale` | kinds whose `read_only_actions` hold that action | — (gated, 0032) |
| Del | `Delete` | every subject. This goes **beyond the wireframe**, which shows Del only in the W4 pod menu; the kinds already list a disabled Delete item | — (gated, 0033) |

Not offered → silent (no notice), for example L on a Service. `key_availability` reuses `action_availability(action, &live.access)` for the gate, so menus, keys, and later specs agree.

## Updated by 0032 (workload actions)

- The key layer names a kind-less `RowAction` (the keys, menu hints, and palette); `ResourceAction` is what a subject resolves it to. `Scale(ObjectKind)` and `RestartRollout(ObjectKind)` carry the row's kind, so the gate reads that kind's permission; `PauseRollout`, `RollBack`, `SuspendCronJob`, `TriggerCronJob`, and `RerunJob` are new, each with a unit key action (unbound).
- `subject_action(row, subject)` resolves through the kind table: the `KindAction` whose action maps to `row`. Every W7 workload `KindAction` is now `keyed` with its action; Helm `Roll back…` stays `named`.
- A menu item of a kind row has no `on_click`: the kit dispatches its key action through the trigger focus, and a right click has moved the cursor to that row, so the menu, the key, and the palette end in one arm of `run_available_row_key`. `row_block` (a paused Deployment does not restart) is checked after the gate, in the menu, the palette entry, and the arm.

## Feedback for a disabled key

`Disabled { reason }` → `window.push_notification(Notification::warning(text).id::<RowKeyNotice>(), cx)` (`gpui_kit::component::WindowExt`), `text = "{label} is unavailable: {reason}"`, e.g. "Edit YAML is unavailable: Read-only mode". One id, so repeated presses replace the notice instead of stacking. `RowKeyNotice` is a private marker struct. Labels come from one `fn action_label(ResourceAction) -> &'static str` shared with the menus.

No row action in 0028 calls a mutating API: every mutating action stays disabled (project rule; the 0001 read-only grep must stay clean). Specs 0031–0036 flip their gate; the key starts working with no keymap change.

## Kind menu data

`KindSpec.read_only_actions: &'static [&'static str]` becomes `&'static [KindAction]`:

```rust
#[derive(Clone, Copy, Debug)]
pub(crate) struct KindAction { pub(crate) label: &'static str, pub(crate) action: Option<ResourceAction> }
```

`Scale…` → `Scale`, `Restart rollout` → `RestartRollout`, ConfigMaps `Edit` → `EditYaml`; other labels (`Roll back…`, `Pause rollout`, `Re-run job`, `Trigger now`, `Suspend`) → `None` (no key in the wireframe).

## Menu key hints

Kit `PopupMenuItem::action(Box<dyn Action>)` renders the bound key (`Kbd`) next to the item and, when the item also has `on_click`, still runs the click handler. Menus add `.action(...)` to: View logs (`ViewLogs`), Open shell / Open node shell (`OpenShell`), Port-forward (`PortForward`), View YAML (`ViewYaml`), Cordon (`Cordon`), Drain… (`Drain`), Copy name (`CopyName`), the `KindAction` items with an action, and the Delete item (`Delete`). Disabled items (`disabled_menu_item`, an element item) get the hint too. Items keep their `on_click`, so a right-clicked row (not the cursor) is still the item's target.

Hints resolve through the menu's trigger focus path, which has no `PopupMenu` context, so `WORKSPACE` bindings are found. Labels stay as today. The wireframe "Edit YAML" item on every kind and the "Attach" item on pods are not added; their owners are 0031 and a later item (Attach is no longer owned by 0036).
