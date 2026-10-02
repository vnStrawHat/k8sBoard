# 0026 · Keys

[Back to index](README.md) · Step 3 · Modules: `keymap.rs` (0028) or `app_shell.rs` `bind_keys`, `cluster_switcher.rs`. Decisions 8, 15, 16, 18. Read with 0028 [keymap.md](../0028-keyboard-map/keymap.md) and [contexts-and-focus.md](../0028-keyboard-map/contexts-and-focus.md) (not edited here).

## Bindings

| Key | Binding string | Action | Context | Effect |
|---|---|---|---|---|
| Ctrl Shift C | `secondary-shift-c` | `OpenClusterSwitcher` | `WINDOW` (`AppShell`) | toggles the popover; on open the kit focuses the filter |
| Ctrl 1 … 9 | `secondary-1` … `secondary-9` | `SwitchToCluster1` … `SwitchToCluster9` | `WINDOW` | `nth_cluster(sections, n)` → `switch_cluster`; no row → nothing; closes the popover if open |
| ↓ / ↑ | `down` / `up` | `SwitcherNext` / `SwitcherPrevious` | `ClusterSwitcher` and `ClusterSwitcher > Input` | moves the highlight over visible rows, wraps |
| ⏎ | `enter` | `SwitcherConfirm` | `ClusterSwitcher` and `ClusterSwitcher > Input` | switches to the highlighted row |
| Esc | `escape` | `CloseClusterSwitcher` | `ClusterSwitcher` and `ClusterSwitcher > Input` | closes; the kit restores the previous focus |
| Space | — | — | — | stays reserved for 0027 (tick) |

- `"ClusterSwitcher"` is a `key_context` on the popover content root, below the kit's `Popover` context. The kit popover binds `escape` → `Cancel`, `enter` and **`space` → `Confirm`** in `Popover` (gpui-base 0.7 `popover.rs:19-21`). The `ClusterSwitcher` bindings sit one level deeper, so they win when focus is on a row button; the `ClusterSwitcher > Input` bindings match at the `Input` depth and, registered after `gpui_kit::init`, win over the kit `Input` keys at equal depth (0028 "How GPUI picks a binding").
- The four handlers never call `cx.propagate()`.
- **Note for 0027**: Space under the switcher reaches the kit `Popover` `space → Confirm` unless 0027 binds `space` on `ClusterSwitcher` (row focus) and decides what Space does inside the filter input (it must still type a space there).
- Ctrl Shift C is a toggle, so the same chord closes the popover from inside the filter (`WINDOW` is an ancestor of the popover content).
- Ctrl 1–9 work inside text fields (0028 rule 2). Checked: gpui-base and gpui-component 0.7 bind no `secondary-<digit>` and no `secondary-shift-c`.
- Platform: `secondary` = ⌘ on macOS (⌘⇧C, ⌘1…9), Ctrl elsewhere. On non-US layouts (French AZERTY) the digits need Shift; GPUI matches the key, so `Ctrl 1` is the physical `&/1` key there; see README open item 5.

## With and without 0028

| 0028 state | Where 0026 binds | Reserved list |
|---|---|---|
| merged (committed spec; code may lag) | `keymap.rs`, in its `WINDOW` group; actions declared there | remove `secondary-shift-c` and `secondary-1`…`9` from `RESERVED_KEYS`; keep `space`; add sheet rows "Open cluster switcher" and "Switch to cluster 1–9" |
| code not merged | `app_shell::bind_keys` with context `"AppShell"`; actions in `cluster_switcher.rs` | 0028, when it lands, moves these bindings into `keymap.rs` and drops them from its reserved table |

In both cases the in-popover keys stay in `cluster_switcher.rs` (`bind_keys`): they belong to the switcher's own context.

## Focus

- Open: `Popover::track_focus(filter focus handle)`; the kit focuses it during the open transition (switcher-ui.md).
- Close by Esc, Enter, click, Ctrl n, or outside click: the kit restores the focus held before opening; when that element is gone, the shell's `on_focus_lost` restore puts focus on the shell root.
- The title-bar trigger stays clickable; clicking it while open closes it.

## Menu hints

Each row shows `Kbd::binding_for_action(&SwitchToClusterN, Some("AppShell"), window)` (gpui-component 0.7 `kbd.rs:58`): with the `AppShell` context the lookup finds the `WINDOW` binding even while focus is inside the popover. Labels read `Ctrl+1` or `⌘1` per platform. The footer shows `OpenSettings` the same way (0025).
