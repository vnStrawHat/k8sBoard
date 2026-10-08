# 0051 · UI icons

User request (2026-10-04): add more icons so the UI looks less monotonous. Icons support labels; they never replace a label (keyboard-first app). No emoji.

## Goal

One kind → icon mapping and one row action → icon mapping, used in the screen header, sidebar, drawer headers, Topology cards, palette rows, ⋯ / context menus, toolbar buttons, and the Settings nav.

## Key decisions

- **Source: the Lucide catalog already bundled.** `main.rs` registers `gpui_kit::assets::AllAssets` (all 1,818 Lucide 1.43.0 icons, `gpui-kit-assets` 0.7.0, `LICENSE-LUCIDE` = ISC). Every chosen icon was checked against `assets/icons/*.svg`. No new SVG files, no new asset source, no new crate, no license to add.
- **No official Kubernetes icons** (kubernetes/community, CC-BY-4.0): they are filled blue hexagons with hard-coded colors (breaks the theme-token rule), unreadable at 12–16 px, and would need an extra asset source plus attribution.
- **Wireframes are text-only** (`ic:"Po"` badges, plain `menuHTML` rows, text sidebar). This spec departs on purpose, by user request; layout and labels stay as drawn.
- **Mapping lives with the domain data:** `KindSpec.icon` (per-kind data, like `badge`), `TopologyKind::icon`, `RowAction::icon` (moved from `command_palette.rs`), `SettingsPage::icon`, `screen_icon(Screen)` in `navigation.rs`. Matches are exhaustive, no `_` arm.
- **Sidebar: icons on top items and group headers only**; kind children stay text (30 icons in a 250 px column is noise; the chevron + indent already shows hierarchy).
- **Screen header:** `screen_icon` before every screen title (always visible; the biggest win).
- **Colors: inherited.** Icons take the text color of their row (kit sets foreground / muted when disabled / active tone). Only the Topology chip sets a color (`colors.kind_text(hue)`, as the badge text did). No new theme tokens. Delete keeps a neutral `Trash` (confirmed).

## Non-goals

- Table cells (Issues `Kind` column, Port Forwarding `Target` column): text stays; the `svc/` prefix already names the kind.
- Topology export (`topology_export.rs`): keeps the two-letter badge for now (confirmed).
- Topology toolbar chips and every 0050 file (`topology_view.rs`, `topology_route.rs`, …): see design.md § Lanes.
- Dialog footer buttons (Cancel, Confirm, Retry, Review…): text only.
- Icons on `.checked()` menu items (log, filter, clusters page menus): the kit icon would replace the check mark.

## Files

| File | What |
|---|---|
| [icon-map.md](icon-map.md) | kind, screen, group, action, menu-item, toolbar, Settings tables |
| [design.md](design.md) | placement, sizes, tones, code shape, steps, files to touch, tests, lanes |

## Acceptance criteria

- [ ] No new SVG, asset source, or dependency; `Cargo.lock` unchanged.
- [ ] Every `KindSpec` (26 statics + the custom spec in `custom_kind.rs`) sets `icon`; `TopologyKind::icon`, `RowAction::icon`, `SettingsPage::icon`, `screen_icon` have no `_ =>` arm (grep check).
- [ ] Unit tests in design.md § Tests pass; quality gate green after every step.
- [ ] Screen headers show the screen icon before the title.
- [ ] Sidebar: Overview, Issues, Topology, and the 8 group headers show icons; kind items do not.
- [ ] Drawer headers (pod, node, kind, port forward) show the kind icon in the muted chip with the kind name as tooltip; no two-letter text.
- [ ] Topology cards show the kind icon in the chip (zoomed-out cards too, checked in code review); ghosts keep `?`.
- [ ] Palette: every row leads with an icon (no `Po`, `·`, `#`, `@`, `⇄` text).
- [ ] Every menu item with a key hint, submenus included (View logs ▸, Open shell ▸, Port forward ▸), shows `RowAction::icon`; other items follow icon-map.md; no `.checked()` item has an icon.
- [ ] Toolbar buttons and Settings nav per icon-map.md.
- [ ] ui-verifier screenshots in light and dark (design.md § Verification) show aligned icons, no clipping at 1320 px, no hard-coded colors.

## Open items

- Danger-toned Delete icon: the kit draws a menu icon outside the item element, so it takes the row's foreground. Would need a theme color passed into `pod_menu`/`node_menu`/`kind_menu` (6 callers). Declined for now.
- Icons on sidebar kind children: one `.icon(...)` per item if the user asks.
- Topology export icons: inline the Lucide path data into `node_svg` after 0049/0050 merge.
