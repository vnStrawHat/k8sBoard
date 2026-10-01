# 0005 · Navigation, selection, menus, screenshots

[Back to index](README.md) · Modules: `navigation.rs`, `app_shell.rs`, `workspace.rs`, `kind_table.rs`, `table_selection.rs`, `resource_actions.rs`, `launch_options.rs`, `screenshot.rs`

## Screens and sidebar

```rust
pub(crate) enum Screen { Pods, Nodes, Kind(ResourceKind) }
impl Screen { pub(crate) fn kind(self) -> Option<ResourceKind>; }
```

- `screen_of(item)` covers `Pods`, `Nodes`, and `ResourceKind::from_label(item).map(Screen::Kind)`. Secrets and every other item stay disabled, as before.
- `NavigationCounts` gains `explorer: Option<(ResourceKind, usize)>`, which is the explorer list's `ready_count` for its kind. Other kinds show no count, because lazy watches have no data for them.
- `sidebar(active, counts, live: Option<&LiveCluster>, cx)`. For a kind item, the pure `fn kind_availability(kind, access: &AccessState, scope: &NamespaceScope) -> KindAvailability { Enabled, Denied { reason: SharedString } }` decides:

| `AccessState` | Result |
|---|---|
| `Known(report)` and `!report.is_allowed(kind.access_check())` | `Denied`. Named scope or cluster-scoped kind: "Not permitted: list deployments". Scope `All` and a namespaced kind: "Not permitted: list deployments in all namespaces" (the review asked cluster-wide) |
| `Known` allowed, `Checking`, `Unknown`, or not live | `Enabled` |

- A denied item uses `.disable(true)` with `.suffix(..)`: a `Lock` icon (`text_xs`, muted) in a `div().id(..)` with `.tooltip(reason)`. The kit renders the suffix for disabled items (checked in `sidebar/menu.rs`). Pods and Nodes are not gated here; their behavior is unchanged.

## AppShell

| Member | Change |
|---|---|
| `kind_table: Entity<TableState<KindTableDelegate>>` | created with the others (`configure`), subscribed with `on_kind_table_event` |
| `show_screen(screen)` | set `screen`; `session.set_explorer_kind(screen.kind())`; `kind_table`: `set_kind(screen.kind())`, then `refresh`; `close_drawer`; 0004's `log_dock.unzoom` stays |
| `start_session` | passes `self.screen.kind()` to `ClusterSession::new`, and sets the session on `kind_table` |
| `fit_table_widths`, `close_drawer` | also cover `kind_table` |
| `reveal_pod(key, cx)` (new) | `show_screen(Pods)`. If the pod is in `live.pods`: `change_selection(Some(key))`, then `pod_table.set_selected_row(ix)` (the same pattern as `apply_pending_launch_screen`). If it is not found: Pods with no selection |
| `sync_selection` | new `ResourceKey::Kind` arm: skip unless `explorer` has the same kind and is `Ready`; else `row_index` plus `apply_selection_sync(&kind_table, ..)` |

`KindTableDelegate` (`kind_table.rs`):

- fields: `session`, `kind: Option<ResourceKind>`, `columns: Vec<Column>`;
- `set_kind(kind)` is a no-op for the same kind. Otherwise it rebuilds `columns` with the Name minimum width, which invalidates the cached Name width, so the next `fit_width` refits (it compares against the current Name column). `show_screen` then resets the scroll on the `TableState`: `scroll_to_col(0)`, and the vertical scroll handle offset is set to zero. It does not call `scroll_to_row`, because the list is empty while it switches;
- rows are `live.explorer.list.items()` when `explorer.kind == self.kind`, else empty;
- `loading()` is true while the list is `Loading`, or while the explorer kind differs (it is switching);
- `render_td(row, 0)` is the Name cell. Column `c > 0` renders `row.cells.get(c - 1)`, and an empty `div()` when the index is missing. It never indexes directly ([kind-columns.md](kind-columns.md));
- `render_empty` uses the pure `fn empty_text(kind, scope_label: &str) -> String`: `No {plural}` for cluster-scoped kinds; otherwise `No {plural} in {scope_label}` (e.g. "No deployments in team-a", "No deployments in all namespaces");
- `context_menu` calls `kind_menu`.

## Workspace (`workspace.rs`, the post-0004 `upper` region)

- Header: `kind.label()`, plus `count_label(n, singular, plural)`. For namespaced kinds, ` · {scope_label}` follows. `count_label` gains a `plural` parameter, so the existing callers pass `"pod", "pods"` and `"node", "nodes"`.
- Interruption banner and failure view: from `explorer.list`, with the title "{label} are unavailable".
- Body: `DataTable::new(&self.kind_table)`. Drawer: `ResourceKey::Kind` gives `kind_drawer(kind, row, ..)` ([kind-drawers.md](kind-drawers.md)).

## Selection key (`table_selection.rs`)

```rust
ResourceKey::Kind { kind: ResourceKind, namespace: Option<String>, name: String }
impl ResourceKey { pub(crate) fn of_row(kind: ResourceKind, row: &KindRow) -> Self; pub(crate) fn is_row(&self, kind: ResourceKind, row: &KindRow) -> bool; }
```

## Menus (`resource_actions.rs`)

`pub(crate) fn kind_menu(menu: PopupMenu, kind: ResourceKind, row: &KindRow, access: &AccessState) -> PopupMenu`. The row menu and the drawer ⋯ menu use the same builder.

| Group | Items |
|---|---|
| view | `View YAML` on every kind, disabled with "YAML view comes in a later version" (constant `YAML_DEFERRED_REASON`); `Port-forward` via `action_item(ResourceAction::PortForward, ..)`, only if `kind.has_port_forward()` |
| change | each of `kind.read_only_actions()` as `disabled_menu_item(label, "Read-only mode")` |
| copy | `copy_name_item(&row.name, access)` |
| danger | `kind.delete_label()`, disabled with "Read-only mode" |

Separators go between non-empty groups only. `read_only_actions` (W7 mutating items; "Edit YAML" is dropped because View YAML covers YAML and every edit is disabled anyway):

| Kind | Items |
|---|---|
| Deployments | Scale…, Restart rollout, Roll back…, Pause rollout |
| StatefulSets | Scale…, Restart rollout |
| DaemonSets | Restart rollout |
| ReplicaSets | (none: Scale is owned by the Deployment) |
| Jobs | Re-run job |
| CronJobs | Trigger now, Suspend |
| Services, Ingresses | (none) |
| ConfigMaps | Edit |
| Namespaces | (none) |

## Screenshot screens (extends the 0003 and 0004 hook)

- `LaunchScreen` gains `Kind(ResourceKind)` (`--screen <plural>`, e.g. `deployments`) and `KindDrawer(ResourceKind)` (`--screen <plural>-drawer`). `screen()` maps both to `Screen::Kind(k)`, and `has_drawer()` is true for `KindDrawer`. Update `USAGE`.
- `apply_pending_launch_screen`: for `KindDrawer`, once `explorer` (same kind) is not `Loading`, select row 0 (`change_selection`, then `kind_table.set_selected_row(0)`). An empty list leaves no drawer, and the screen still settles.
- `settle_input`: for `Screen::Kind`, `is_loading` and `has_failed` come from `explorer`. A missing explorer counts as Loading.
- Step 2 wires `namespaces`, `namespaces-drawer`, `deployments`, `deployments-drawer`; step 3 adds the rest. ui-verifier set (step 3): all 20 screens in light; `deployments`, `deployments-drawer`, `services-drawer`, and `namespaces` in dark. Files: `.tmp/ui-shots/0005-<screen>-<theme>.png`.
