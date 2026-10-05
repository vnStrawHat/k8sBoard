# 0003 · Actions: context menu, ⋯ menu, RBAC gating

[Back to index](README.md) · Module: `resource_actions.rs`

## One builder, two menus

The row context menu (`TableDelegate::context_menu`) and the drawer header ⋯ (`Button` + `DropdownMenu`) call the same function, so their content is identical (W4 note 3):

```rust
pub(crate) fn pod_menu(menu: PopupMenu, pod: &PodSummary, access: &AccessState, /* handlers */) -> PopupMenu;
pub(crate) fn node_menu(menu: PopupMenu, node: &NodeSummary, access: &AccessState, /* handlers */) -> PopupMenu;
```

## Pod menu (read-only subset of W4)

| Group | Item | Availability |
|---|---|---|
| view | View logs | disabled: RBAC reason if `GetPodLogs` is denied, else "Logs open in the dock, coming in a later version" |
| view | Open shell | disabled: RBAC reason if `CreatePodExec` is denied, else "Not available in read-only mode" |
| view | Port-forward | disabled: RBAC reason if `CreatePodPortForward` is denied, else "Not available in read-only mode" |
| separator | | |
| copy | Copy name | enabled: writes `name` to the clipboard (`cx.write_to_clipboard(ClipboardItem::new_string(..))`) |
| copy | Copy kubectl command | enabled (spec 0008): writes `kubectl --context C -n NS describe pod NAME` |

- No container submenus: they would only lead to disabled items.
- No Edit/Restart/Evict/Delete/Attach (non-goal).
- View YAML (spec 0007) opens the drawer on its YAML tab and is always enabled.

## Node menu (read-only subset of W5)

| Group | Item | Availability |
|---|---|---|
| view | Open node shell | disabled: RBAC reason if `CreatePodExec` is denied (the node shell is a debug pod, W5 note 7), else "Not available in read-only mode" |
| separator | | |
| change | Cordon | disabled: "Read-only mode" |
| change | Drain… | disabled: "Read-only mode" |
| separator | | |
| copy | Copy name | enabled |

## Gating (pure)

```rust
pub(crate) enum ResourceAction { ViewLogs, OpenShell, PortForward, OpenNodeShell, Cordon, Drain, CopyName }
pub(crate) enum ActionAvailability { Enabled, Disabled { reason: SharedString } }
pub(crate) fn action_availability(action: ResourceAction, access: &AccessState) -> ActionAvailability;
```

| `AccessState` | RBAC-gated actions (logs, shell, port-forward, node shell) |
|---|---|
| `Checking` | Disabled "Checking permissions…" |
| `Unknown { .. }` | Disabled "Permissions could not be checked" |
| `Known(report)`, check denied | Disabled `"Not permitted: {AccessCheck}"`, e.g. "Not permitted: create pods/exec" |
| `Known(report)`, check allowed | the feature reason from the tables above |

- `Cordon` and `Drain` are always disabled with "Read-only mode", whatever the access state. `CopyName` is always `Enabled`.
- The decision is "disabled with a reason", never hidden, so users learn what exists.
- Menu items carry no description lines. An enabled item is the label (and its key, on the right) in the normal foreground colour, icon included. A disabled item (`disabled_menu_item`) is the label with a short reason on the right where the key would be (`short_reason`: `No permission`, `Read-only`, `Not running`, …; a reason it does not know shows as it is), all muted and faded (`DISABLED_ITEM_OPACITY`, label and icon), the reason cut at 120 px; the full reason is the tooltip of the row.
- The access review uses the current `NamespaceScope`, and is re-run on scope change ([bootstrap.md](bootstrap.md)).
