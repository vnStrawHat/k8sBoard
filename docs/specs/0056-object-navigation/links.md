# 0056 · Links, Prev / Next (H4 style, M9)

[Back to index](README.md)

## One link style

Rule: **text that opens another object is drawn in `theme.link` with an underline**, in detail rows and in list rows alike. Plain text never navigates.

- `drawer.rs`: extract `fn link_style(div: Stateful<Div>, cx) -> Stateful<Div>` (link color, underline, pointer, `Open {name}` tooltip); `link_text` uses it.
- List rows keep their row hover and their click target (the whole row), and draw the **object name** with `link_style`. Status labels, ages, and addresses stay as they are.

| Row (file) | Name drawn as link |
|---|---|
| Endpoints `endpoint_element` (`live_sections.rs`) | the pod name; the address stays muted mono before it: `10.0.0.5:8080 · ` + link `api-7d9f8c-x2k4q`; an endpoint without a pod has no link |
| Not-ready pods, Recent jobs, Revisions, Selected pods, Mounted by (`live_sections.rs`) | the pod / job / ReplicaSet name |
| Workload and node pod lists (`related_pods.rs`) | the pod name (the `namespace/` prefix stays muted) |
| WHY box `Open secret … →` / `Open pod … →` (`kind_drawer.rs` ~425) | already link-styled; its click goes through `open_link` |

Out of scope: Overview cards, the node heatmap, and the Issues table (lists of their own screens, not drawer links).

## Following a link

One chokepoint: B1 adds `drawer.rs::open_link(shell, target, window, cx)`, called by `link_text`, every list row above, and the kind-drawer WHY link (today they call `reveal`). A3 adds `AppShell::follow_link(target, window, cx)` and changes only the body of `open_link` to call it (so A3 depends on B1). Other `reveal` callers (palette, Issues, dialogs, Overview) are unchanged.

```rust
// navigation_history.rs, pure; reuses cluster_session::scope_includes (no second copy)
pub(crate) enum LinkStep { Reveal, Denied(SharedString), OutOfScope(SharedString) }
pub(crate) fn link_step(target: &ResourceKey, access: &AccessState, scope: &NamespaceScope) -> LinkStep;
```

| Case | Step | User sees |
|---|---|---|
| Target kind allowed and in scope (or cluster-scoped) | `Reveal` | target screen, row selected, drawer open; Back available |
| Access report known and denies listing the kind (`kind_availability` = Denied) | `Denied(reason)` | no navigation; notification with the reason, e.g. `Not permitted: list secrets`; the drawer stays |
| Namespaced target outside a `Named` / `Several` scope | `OutOfScope(text)` | no navigation; notification `payments-api is in team-b, outside the scope`; the drawer stays |
| Report still checking / failed | `Reveal` | the list's own error state explains a 403 (0005 rule) |

The scope is never changed by a link: the picker caps a scope at 5 namespaces (`namespace_picker.rs` `MAX_NAMESPACES`), `set_scope` relists every watch, and `scope_memory` would keep the widened scope. The user changes the scope in the picker.

## Prev / Next in the drawer header (M9)

The 75 % Pod drawer hides the list; the header gets the row cursor controls so the user can walk the list without closing it.

| Item | Detail |
|---|---|
| Where | right side of the header row, before `⋯`: `[ChevronUp] [ChevronDown] 12 of 40` |
| Shown | screens with a table cursor: Pods, Nodes, kind screens. Hidden on Topology, Overview, Port forwarding. |
| Click | the same as `K` / `J` (`step_cursor(RowStep::Previous / Next)`), so the drawer follows and the tab is kept (0028) |
| Ends | wrap, like `J` / `K` (0028 `step_row`): Next on the last row goes to the first. The position text makes the wrap visible. |
| Filter / sort | steps over the visible rows in display order; `12 of 40` counts visible rows (`40` is the filtered count) |
| Disabled | both buttons when the subject is not a visible row (pending reveal still loading) or the table has one row; position text hidden then |
| Tooltips | `Previous row (K)`, `Next row (J)` |
| Editor open | hidden (the cursor does not move under Edit YAML) |

Pure helper in `navigation_history.rs`: `fn row_position(selected_row: Option<usize>, visible_rows: usize) -> Option<(usize, usize)>` (1-based) for the text.

## `DrawerNavigation` (header input)

```rust
// drawer.rs
pub(crate) struct DrawerNavigation {
    pub(crate) back: Option<BackTarget>,          // label + tooltip text
    pub(crate) rows: Option<RowPosition>,         // None hides Prev/Next
}
pub(crate) struct DrawerHeader { /* existing fields */ pub(crate) navigation: DrawerNavigation }
```

`workspace.rs::render_drawer` builds it once and passes it to `pod_drawer`, `node_drawer`, `kind_drawer` (one new parameter each); the Port forwarding drawer passes `DrawerNavigation::default()` (nothing shown).
