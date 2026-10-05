# 0003 · Overlay drawer (pod and node)

[Back to index](README.md) · Modules: `drawer.rs`, `pod_drawer.rs`, `node_drawer.rs`

## Why not the kit `Sheet`

`window.open_sheet(cx, ..)` (the kit `WindowExt`) is window-level. It starts below the title bar but covers the sidebar and the status bar, and it takes focus, so ↑ ↓ would stop moving the table selection. The wireframe wants an overlay on the **workspace only**, with the table keeping focus. So the drawer is a plain element:

```rust
// inside the workspace container, which is `.relative()`
div().absolute().top_0().right_0().bottom_0().w(width)
    .bg(theme.background).border_l_1().border_color(theme.border).shadow_lg()
    .occlude()             // clicks do not fall through to the table
```

- Width: see "As built (2026-10-05)" below (it was a fixed `px(420.)` / `px(640.)`). The table never resizes.
- Open = `AppShell.selected.is_some()`. Close:
  - ✕, which calls `table.clear_selection`;
  - Esc while the table is focused (the kit binds Esc to `Cancel`, which emits `ClearSelection`);
  - a click on the table's empty area, only if the kit already emits `ClearSelection` there. Do not build custom hit-testing.
- A **subject change** (the selected key differs from the previous one; see [tables.md](tables.md)) keeps the drawer open with the new subject. The tab and the expanded flag persist, and only then does `selected_container` reset. Snapshot re-syncs of the same key never reset it.
- No action buttons in the body. Actions live only in the header ⋯ menu and the row context menu ([actions.md](actions.md)).

```rust
pub(crate) struct DrawerState { tab: DrawerTab, is_expanded: bool, selected_container: Option<String> }
pub(crate) enum DrawerTab { Overview, Containers, Events } // spec 0007 replaces `PodDrawerTab`; every drawer has a tab bar
/// Shared frame: header, subtitle, optional tab bar, scrollable body.
pub(crate) fn drawer_frame(header: DrawerHeader, tabs: Option<AnyElement>, body: AnyElement,
    width: Pixels, cx: &App) -> impl IntoElement;
pub(crate) struct DrawerHeader { kind_badge: &'static str /* "Po" | "No" */, name: SharedString,
    subtitle: AnyElement, menu: AnyElement /* ⋯ DropdownMenu button */, is_expanded: bool, /* + toggle and close handlers */ }
```

## Pod drawer header (W4)

- Row 1: badge `Po`, pod name (mono, ellipsis), then the icon buttons ⋯ (`Ellipsis`), ⤢/⤡ (`Maximize2`/`Minimize2`), and ✕ (`X`). The icons are Lucide icons from `gpui_kit::assets::IconName`.
- Row 2: the status label in its tone, `· {namespace} · created {age} ago`, muted.
- Tabs: `TabBar` (underline) with `Overview` and `Containers {containers.len()}`.

## Pod Overview tab (pod-level only)

| Section | Rows |
|---|---|
| Pod | Node (`node_name` or "—"), Pod IP, QoS class, Service account, Controlled by (`{kind}/{name}`, plain text, no link yet) |
| Conditions | one chip per `PodCondition` in API order. `is_true` uses the `success` dot, otherwise a `muted_foreground` dot |
| Containers · {running} of {total} running | one row per container in lifecycle order: kind tag (`INIT`/`SIDECAR`/`MAIN`), name (mono), state label in its tone, restarts (right). Clicking a row sets `tab = Containers` and `selected_container = name` |

- The label/value rows use the kit `DescriptionList` if it fits, otherwise a two-column `h_flex` with a muted label.
- Spec 0008 adds the WHY box above the Pod section, Node and Controlled by links, and tooltips on conditions that are not true.

## Pod Containers tab (W4b, master-detail)

- Layout: when `is_expanded`, the list (240 px) sits left of the detail. Otherwise the list is on top and the detail below. This follows the W4b note "at default width the list stacks above the detail".
- List groups (headers muted, with counts):
  - `Init · ran in order {done}/{n}` (kind Init; done = terminated with exit 0)
  - `Sidecars {n}`
  - `Containers {ready}/{n}` (kind Main)

  Each item shows the name, the restarts, and the state label. Empty groups are hidden.
- Default selection: the first Main container that is not ready, else the first Main, else the first container.
- Detail: header `{name}`, the state label, and the kind tag. Then rows:

| Row | Value |
|---|---|
| State | `container_state_label` text, plus `· started {age} ago` when running |
| Last state | `{reason} · exit {code}` (+ `· signal {n}`) `· ended {age} ago`, or "—" |
| Restarts | `restart_count` |
| Image | `image` (mono, selectable if the kit text supports it) |

- Spec 0008 replaces these rows with Info · Env · Mounts sub-tabs (digest, pull policy, ports, resources, probes, env and mount names and sources). No container ⋯ menu.

## Node drawer

- Spec 0008 replaces the body rows below with Overview sections (Node, Conditions, Addresses, System, Resources, Pods, Labels) and adds the YAML and Events tabs.

- Header: badge `No`, the node name, the status label, and `· created {age} ago`. Only ⋯ and ✕, with no expand (one column is enough).
- No tab bar. Body rows: Status, Roles ("—" when empty), Taints (every taint, one per line, mono; "—"), Kubelet version, Internal IP, Created (timestamp, then the age).

## As built (2026-10-05): drawer polish

- Width is a share of the workspace (window width minus the 220 px sidebar), set on `DrawerState` by each paint of `render_workspace`: `drawer_width(is_expanded, workspace)` is 55 % by default and 90 % expanded, never below 480 px, never above the workspace (a window narrower than 480 px gets a full-width drawer), and expanded is never narrower than the default. At the default 1320 px window: 605 px, expanded 990 px. The selection bar and value popover still sit left of the drawer (`right(self.drawer.width())`); the Topology reveal and minimap use the default width for their canvas.
- The title shows the kind before the name: a muted, uppercase mono caption (the Topology card caption style) from `ResourceKind::display_name` (the Kubernetes `kind`, `Helm release` for a release), then the semibold mono name, which alone truncates. `DrawerHeader.kind_name` feeds it; the chip lost its tooltip.
- `section_title` (the one heading all drawers use, Overview and container detail) is `text_sm` semibold in `theme.foreground`, with a `theme.border` rule below and `mt_6` above.
