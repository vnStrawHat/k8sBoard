# 0007 · App: View YAML menus and screenshot screens

[Back to index](README.md) · Step 3 · Modules: `resource_actions.rs`, `pod_table.rs`, `node_table.rs`, `pod_drawer.rs`, `node_drawer.rs`, `app_shell.rs`, `launch_options.rs`, `screenshot.rs`

## Gating (`resource_actions.rs`)

- `ResourceAction::ViewYaml` is new. `gate()` → `None`; `action_availability` → `Enabled` for `CopyName | ViewYaml`, whatever the access state (decision 14).
- Delete `YAML_DEFERRED_REASON` and every use (0005 kinds, 0006 event menu).

## Menus

All three builders take `shell: &WeakEntity<AppShell>` (0006 already added it to `kind_menu`).

| Menu | View group after this spec |
|---|---|
| `pod_menu(menu, pod, live, dock, shell)` | View logs, Open shell, Port-forward, **View YAML** |
| `node_menu(menu, node, access, shell)` | Open node shell, **View YAML** |
| `kind_menu(menu, kind, row, access, shell)` | **View YAML** (enabled), Port-forward when `has_port_forward` |
| event group (0006) | unchanged |

- Item: `fn view_yaml_item(key: ResourceKey, shell: &WeakEntity<AppShell>) -> PopupMenuItem` = `PopupMenuItem::new("View YAML").on_click(..)` → `shell.update(cx, |shell, cx| shell.open_yaml(key.clone(), cx))`.
- The row context menu and the drawer ⋯ menu still call the same builder, so they agree (0003).
- Callers: `PodTableDelegate::new(dock, shell)` and `NodeTableDelegate::new(shell)` keep a `WeakEntity<AppShell>` (built with `cx.weak_entity()` in `AppShell::new`, like 0006's `KindTableDelegate`); `pod_menu_button` and `node_menu_button` pass `cx.weak_entity()`.
- No `Y` key binding (0028).

## Launch screens (`launch_options.rs`)

Extends step 2's parse with the YAML tab:

| `--screen` | Variant |
|---|---|
| `pod-yaml` | `PodDrawer(Yaml)` |
| `node-yaml` | `NodeDrawer(Yaml)` |
| `<plural>-yaml` | `KindDrawer(kind, Yaml)` |

- `USAGE` adds `pod-yaml|node-yaml|<kind>-yaml`.
- Default width (not expanded): the screenshot shows what users see first.

## Settle (`screenshot.rs`, `app_shell.rs`)

- 0006's `is_drawer_ready(has_selection, is_launch_pending, is_object_events_pending)` renames its third parameter to `is_content_pending`.
- `settle_input` passes 0006's events value (`event_subject_task.is_some() || live.is_object_events_loading()`) `|| self.is_yaml_loading(cx)`.
- `fn is_yaml_loading(&self, cx: &App) -> bool` = `yaml_subject(self.selected.as_ref(), self.drawer.tab).is_some()` and (`drawer.yaml` is `None` or `is_loading()`). A failed fetch is settled: the alert is the screen to capture.

## Screenshot safety (C1)

- YAML screenshots: `pod-yaml`, `node-yaml`, `deployments-yaml`, `events-yaml`. Never `configmaps-yaml` (config values in a PNG) and never a Secret.
- No screenshot run toggles Env values. Pod and workload YAML therefore show `<hidden>` for env literals.
- PNGs stay in `.tmp/ui-shots/` (git-ignored). Reports describe layout only and never quote YAML values.
