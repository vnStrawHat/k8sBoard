# 0051 · Design, steps, files, tests

[Back to index](README.md) · Icon names: [icon-map.md](icon-map.md). All files are in `crates/app/src/`.

## Placement, size, tone

| Place | Kit hook | Size | Color |
|---|---|---|---|
| Screen header (`workspace.rs` `render_header`) | `Icon::new(screen_icon(screen)).size_4()` + title in an inner `h_flex().items_center().gap_2()` (the outer row is `items_baseline`; an svg has no baseline) | 16 px | `theme.muted_foreground` |
| Sidebar top items, group headers | `SidebarMenuItem::icon` | 14 px (kit sizes it from the row's `text_sm`) | inherited (active / muted by kit) |
| Popup / context / ⋯ menus | `PopupMenuItem::icon` | kit `xsmall` (12 px); the kit reserves one aligned left column per menu | inherited; disabled rows muted by kit |
| Drawer header chip | `Icon::new(..).size_4()` in today's chip (`bg(theme.muted)`, `px_1p5`, radius) with `py_0p5`; tooltip = kind name | 16 px | `theme.muted_foreground` |
| Topology chip (`card_body`) | `Icon::new(..)` sized `px(CHIP_SIZE * 0.55 * zoom)` | 16.5 px at zoom 1 | `colors.kind_text(hue)` (same as the badge text) |
| Topology zoomed-out (`badge_only_body`) | icon sized `px(NODE_HEIGHT * zoom * 0.4)`; ghost keeps `?` text | scales | `colors.text_on(fill)` |
| Palette rows | `Icon::new(row_icon(..)).size_4()` | 16 px | inherited |
| Buttons | `Button::icon` | kit per button size | inherited |
| Settings nav | `SettingPage::icon` | 14 px (kit, `text_sm` row) | inherited |

- Row density (28/36 px) does not apply: no icon goes into a table row. Menus, sidebar, and headers do not change with density.
- Icons sit before the label, vertically centered by the kit's `items_center`; no manual baseline offsets.
- Both themes come free: every icon but the Topology chip inherits a theme text color; no `rgb`/`hsla` literal is added.
- Labels never change except the four `+ X` buttons and the Port Forwarding `EMPTY_TEXT` in icon-map.md (the `+` glyph becomes the `Plus`/`ListFilterPlus` icon).
- **Never set `.icon()` on a `.checked()` menu item**: the kit's icon replaces the check mark. The log, filter, and clusters page menus stay icon-less.

## Code shape

```rust
// resource_kind.rs
pub(crate) const POD_ICON: IconName = IconName::Box;
pub(crate) const NODE_ICON: IconName = IconName::Server;
pub(crate) struct KindSpec { .., pub(crate) icon: IconName, .. }   // next to `badge`; 26 statics
impl ResourceKind { pub(crate) fn icon(self) -> IconName }         // self.spec().icon
// custom_kind.rs: the leaked KindSpec sets icon: IconName::Puzzle
// topology_graph.rs (step 2)
impl TopologyKind { pub(crate) fn icon(self) -> IconName }         // resource_kind().map_or(POD_ICON, ResourceKind::icon)
// navigation.rs
pub(crate) fn screen_icon(screen: Screen) -> IconName              // exhaustive over Screen
struct NavigationSection { name, icon: IconName, items, is_open_by_default }
// resource_actions.rs
impl RowAction { pub(crate) fn icon(self) -> IconName }            // moved from command_palette::row_action_icon
pub(crate) fn row_keyed(item: PopupMenuItem, row: RowAction) -> PopupMenuItem  // .action(row.key_action()).icon(row.icon())
// drawer.rs: DrawerHeader { kind_icon: IconName, kind_name: SharedString, .. }  // replaces kind_badge
// settings_window.rs
impl SettingsPage { fn icon(self) -> IconName }                    // exhaustive
// command_palette.rs: fn row_icon(target: &PaletteTarget) -> IconName  // RowIcon enum deleted
```

- `keyed(item, ResourceAction)` delegates to `row_keyed`. Every `.action(RowAction::X.key_action())` on a `PopupMenuItem` (resource_actions.rs ~13 sites, `port_forward_menu.rs` `item`) goes through `row_keyed`, and so do the three submenus (View logs ▸, Open shell ▸, Port forward ▸): `.action()` is a no-op on a Submenu, `.icon()` applies.
- `ResourceKind::badge` stays (only `TopologyKind::badge` → `topology_export.rs` uses it); fix its doc comment ("Two letters in the Topology export"). `kind_badge` in `custom_kind.rs` stays for the same reason.
- `command_palette.rs`: delete `row_action_icon`, `screen_badge`, and `RowIcon`; `row_icon` returns `IconName`.
- Drawer kind names: pod `"Pod"`, node `"Node"`, kind `kind.singular()`, port forward `"Port forward"` (icon `ArrowLeftRight`).

## Steps (each passes the gate alone; no item without a production caller)

| S | Files | Change |
|---|---|---|
| 1 | `resource_kind.rs`, `custom_kind.rs`, `navigation.rs`, `resource_actions.rs` (`RowAction::icon` only), `command_palette.rs`, `workspace.rs` (`render_header` only) (+ tests) | `KindSpec.icon`, `POD_ICON`/`NODE_ICON`, `screen_icon`, `RowAction::icon` move; palette rows; sidebar top items + groups; screen header icon |
| 2 | `topology_graph.rs`, `drawer.rs`, `pod_drawer.rs`, `node_drawer.rs`, `kind_drawer.rs`, `port_forward_page.rs` (header only), `topology_card.rs` | `TopologyKind::icon` + its test; header chip icon + tooltip; Topology chip and zoomed-out icon |
| 3 | `resource_actions.rs`, `port_forward_menu.rs`, `port_forward_page.rs` (`forward_menu`), `issue_table.rs` (`open_item`, `copy_name_item`) | `row_keyed` incl. submenus; unkeyed item icons |
| 4 | `filter_bar.rs`, `workspace.rs` (header buttons), `port_forward_page.rs` (buttons, `EMPTY_TEXT`), `node_editor.rs`, `helm_release_view.rs`, `permissions_view.rs`, `who_can_view.rs`, `secret_values.rs`, `settings_window.rs` | button icons, `+` labels; Settings nav icons |

## Tests (unit, same-module tests or `_tests.rs`)

| File | Test |
|---|---|
| `resource_kind` tests | `built_in_kind_icons_are_distinct`: `ResourceKind::ALL` icons + `POD_ICON` + `NODE_ICON` pairwise distinct |
| same | `custom_kinds_share_the_puzzle_icon` |
| `topology_graph` tests (step 2) | `pod_card_uses_the_pod_icon` |
| `resource_actions_tests.rs` (step 3) | `row_keyed_sets_key_and_icon`: match `PopupMenuItem::Item { icon: Some(_), action: Some(_), .. }` |
| `command_palette` tests | screen, pod, namespace, cluster rows lead with `screen_icon(..)`, `POD_ICON`, `Folder`, `Building2` |
| `settings_window_tests.rs` (step 4) | `settings_page_icons_are_distinct` over `PAGES` |
| existing tests | update label asserts for `Filter`, `New forward`, `Add`, `EMPTY_TEXT`; drawer header tests that read `kind_badge` |

Exhaustiveness is compile-time; review greps `screen_icon`, `RowAction::icon`, `SettingsPage::icon` for `_ =>`.

## Verification (ui-verifier, `--theme light` and `--theme dark`, `--window-width 1320`)

- Offline: `settings-general`, `settings-appearance` (nav icons); `port-forward-new-fixture` (Port Forwarding header); `shell-picker-fixture` (Open shell ▸ submenu icon).
- UAT read-only (`readonly@Monitor`): `overview` (sidebar, screen header), `pods` (filter bar), `pod-drawer`, `node-drawer`, `deployments-drawer`, `deployments-menu`, `secrets-menu`, `topology-rbac`, `port-forwards-list`, `who-can`, `--palette ":"` and `--palette ">"`.
- Check: icons aligned in one column per menu; disabled rows muted; checked items keep their check mark; no clipped header title at 1320 px; Topology chip legible at zoom 1. The zoomed-out card is checked in code review (no launch option sets zoom).

## Lanes

- Not touched: every 0050 file (`topology_view.rs`, `topology_route.rs`, `topology_layout.rs`, `topology_viewport.rs`, `topology_export.rs`, `settings.rs`, `launch_options.rs`, `screenshot.rs`, `app_shell.rs`, and their tests), plus `topology_canvas.rs` (0049). 0050 adds `--screen topology-curves`, `EdgeShape`, and `topology.edges`; 0051 uses none of them.
- Shared with 0049: `topology_graph.rs` (0049 step 2 adds `Relation::Calls`; 0051 step 2 adds `TopologyKind::icon`) and `topology_card.rs` (0049 step 3 edits node text/tooltip; 0051 edits only the chip `.child(..)` in `card_body` and the label in `badge_only_body`).
- Shared with 0048: `drawer.rs` (header vs `MonitorRange`), `settings_window.rs` (whoever lands second adds the `Metrics => ChartLine` arm).
