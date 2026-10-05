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
- A **subject change** (the selected key differs from the previous one; see [tables.md](tables.md)) keeps the drawer open with the new subject. The tab persists, and only then does `selected_container` reset. Snapshot re-syncs of the same key never reset it.
- No action buttons in the body. Actions live only in the header ⋯ menu and the row context menu ([actions.md](actions.md)).

```rust
pub(crate) struct DrawerState { tab: DrawerTab, selected_container: Option<String> }
pub(crate) enum DrawerTab { Overview, Containers, Events } // spec 0007 replaces `PodDrawerTab`; every drawer has a tab bar
/// Shared frame: header, subtitle, optional tab bar, scrollable body.
pub(crate) fn drawer_frame(header: DrawerHeader, tabs: Option<AnyElement>, body: AnyElement,
    width: Pixels, cx: &App) -> impl IntoElement;
pub(crate) struct DrawerHeader { kind_badge: &'static str /* "Po" | "No" */, name: SharedString,
    subtitle: AnyElement, menu: AnyElement /* ⋯ DropdownMenu button */, /* + close handler */ }
```

## Pod drawer header (W4)

- Row 1: badge `Po`, pod name (mono, ellipsis), then the icon buttons ⋯ (`Ellipsis`) and ✕ (`X`); there is no Expand button (removed 2026-10-05). The icons are Lucide icons from `gpui_kit::assets::IconName`.
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

- Layout: always master-detail: the list (240 px) is a left sidebar with a `theme.border` right edge, the detail fills the rest, and each side scrolls on its own (the body is `DrawerBody::Filling`).
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

- Header: badge `No`, the node name, the status label, and `· created {age} ago`. Only ⋯ and ✕.
- No tab bar. Body rows: Status, Roles ("—" when empty), Taints (every taint, one per line, mono; "—"), Kubelet version, Internal IP, Created (timestamp, then the age).

## As built (2026-10-05): drawer polish

- (Widths superseded by the next section.) Width is a share of the workspace (window width minus the 220 px sidebar), set on `DrawerState` by each paint of `render_workspace`: `drawer_width(is_expanded, workspace)` is 55 % by default and 90 % expanded, never below 480 px, never above the workspace (a window narrower than 480 px gets a full-width drawer), and expanded is never narrower than the default. At the default 1320 px window: 605 px, expanded 990 px. The selection bar and value popover still sit left of the drawer (`right(self.drawer.width())`); the Topology reveal and minimap use the default width for their canvas.
- The title shows the kind before the name: a muted, uppercase mono caption (the Topology card caption style) from `ResourceKind::display_name` (the Kubernetes `kind`, `Helm release` for a release), then the semibold mono name, which alone truncates. `DrawerHeader.kind_name` feeds it; the chip lost its tooltip.
- `section_title` (the one heading all drawers use, Overview and container detail) is `text_sm` semibold in `theme.foreground`, with a `theme.border` rule below and `mt_6` above.

## As built (2026-10-05): no Expand, 50 % and 75 % widths

- The Expand button, `DrawerState.is_expanded`, `toggle_drawer_expanded`, and the expanded launch-screen flag are gone. `drawer_width(DrawerSize, workspace)` is 50 % for every drawer (`DrawerSize::Standard`) and 75 % for the Pod drawer (`DrawerSize::Wide`, `DrawerSize::of(&ResourceKey)`), never below 480 px and never above the workspace. At a 1320 px window: 550 px, Pod 825 px. `AppShell::open_drawer_width` feeds the bars left of the drawer; the Topology reveal and minimap take the size from the selected key, so a Pod drawer there is 75 % too.
- The Monitor tab no longer has an expanded layout: its cards wrap (`flex_1`, 280 px minimum), so two sit side by side whenever the drawer is wide enough.
- The Overview body is scrolled by a box whose child carries the padding, and the first section heading (`first_section_title`) has no top margin, so nothing stacks above the first line. Padding on the scrolled box itself is left out of the scroll extent and cut the last lines.

## As built (2026-10-05): copy buttons

- `clipboard_copy.rs` holds the one helper set: `copy_button` (kit `Clipboard`: tooltip "Copy", the icon turns into a check for 2 s), `copyable_mono` (truncating mono text plus the button), and `copy_text`. They write the plain app clipboard; a Secret's values never use them and keep the masked `secret_clipboard` path.
- Copy buttons sit after: the drawer title name; the container Image and Digest; Pod IP; node addresses (InternalIP, Hostname, …); Service Cluster IP and External; Ingress Hosts (a bare-hosts row) and Address; workload container images (`DetailRow::CopyField`, built by `DetailRow::copyable_field`).
- `chips(id, terms, cx)` (selector, labels, TLS hosts): a click on a chip copies its text (`key=value`, tooltip "Click to copy"), and one button after the set copies all of them, one per line.
