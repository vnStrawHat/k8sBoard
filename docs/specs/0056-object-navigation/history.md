# 0056 · Back and forward (H3)

[Back to index](README.md)

## Decision: a link still opens the target's screen

| Option | Verdict |
|---|---|
| Swap the drawer over the current screen (Topology's model, made general) | **Rejected.** A kind drawer reads `LiveCluster::row_of`: the explorer list of that kind or the Topology feeds. Only one explorer runs. Opening a Service drawer over the Pods screen would need a second single-object watch per kind plus its companion (endpoint slices) and joins: a copy of the explorer, more watches. It also splits the 0028 rule "the drawer shows the cursor row of the visible table" (j/k would move a cursor that is not the drawer's subject). |
| Keep "a link opens the target's screen" and make it reversible | **Chosen.** No new watch; one model (cursor = drawer subject); Back restores the origin exactly enough. |

Topology keeps its own rule (clicks on graph nodes select over the graph, no screen change, no history entry). A link inside a drawer opened over Topology leaves Topology like any link, and Back returns to Topology with that drawer.

## History entry

```rust
// crates/app/src/navigation_history.rs (new, pure, with tests)
pub(crate) struct Place {
    pub(crate) screen: Screen,
    pub(crate) selection: Option<ClusterObject>,
    pub(crate) is_drawer_open: bool,
    pub(crate) tab: DrawerTab,
    pub(crate) container: Option<String>,     // Pod drawer's selected container
    pub(crate) container_tab: ContainerTab,   // show_screen resets it to Info
    pub(crate) filter: Option<TableFilter>,   // the origin table's filter; None without a table
}
pub(crate) struct NavigationHistory { back: Vec<Place>, forward: Vec<Place> }
impl NavigationHistory {
    pub(crate) fn record(&mut self, from: Place);            // push back, clear forward, cap
    pub(crate) fn back(&mut self, current: Place) -> Option<Place>;
    pub(crate) fn forward(&mut self, current: Place) -> Option<Place>;
    pub(crate) fn previous(&self) -> Option<&Place>;         // the Back button label
    pub(crate) fn clear(&mut self);
}
```

- Not stored: scroll offset (restoring the selection scrolls its row into view, `TableView::reveal`), sort and hidden columns (they live in the per-screen `TableView` and survive), and the Monitor range / scope (`show_screen` resets `drawer.monitor`; Back shows the default range, as any screen change does).
- `filter` is stored because a same-screen link (Pod -> endpoint Pod, Service -> Service) may clear the filter that hid the target; Back puts it back.
- Cap: **50** entries in `back`; the oldest is dropped. `forward` is bounded by the same cap.

## What records an entry

- Recorded in **`reveal_object` only**; `reveal` and `follow_link` (links.md) reach it. Callers: drawer links, palette Go to a resource, Issues rows, Overview cards, dialog links (Who can, Check permissions, Test traffic), port-forward rows, drawer-menu "Go to" items (`resource_actions.rs` CornerDownRight items are user navigation too).
- Not recorded: direct `reveal_then` / `when_selected` callers (palette action pairs, key actions on another row), j/k and Prev/Next moves, table row clicks, sidebar screens, Topology node clicks, drawer close. Back means "back to where the last link was followed from".
- **Self-reveal is not recorded**: when the target equals the current place (same screen and selection), `record` is skipped.

## Restore (Back = `Alt+Left`, Forward = `Alt+Right`)

1. Unsaved Edit YAML / values text: ask first (the `ask_discard` path `reveal_then` uses).
2. `show_screen(place.screen)` (unzooms the dock, as any screen change does).
3. Write `place.filter` into that screen's `TableView` (rebuild).
4. Deferred, like `reveal_then`: `change_selection(place.selection)`, `set_drawer_open(place.is_drawer_open)`, then `set_drawer_tab(place.tab)` (never assign `drawer.tab`: the setter drops revealed Secret values), `select_container(name)` when `place.container` is some, `set_container_tab(place.container_tab)`. Table screens set `pending_reveal`; Topology does not.
5. **Topology**: no `pending_reveal`; the drawer reads `row_of`, which needs the Topology feeds, restarted by `show_screen(Topology)`; the drawer appears once they deliver the row. Needs a shell test.
6. A place **without a selection** restores the screen and filter only (no drawer).
7. The current place goes to the other stack before the restore.

## Resets

| Event | History |
|---|---|
| Cluster switch (0046) | cleared (entries hold another cluster's objects) |
| Scope changed (picker, palette `#`, `Ctrl+N`) | cleared |
| Session reconnect, same cluster and scope | kept |

## Edge cases

- **Object deleted since**: the screen is restored, the loaded list lacks the row, the reveal drops it (existing behaviour): no drawer, no error. The entry is consumed.
- **Custom kind no longer served** (CRD removed): the entry is skipped and the next one is tried.
- **List still loading**: `pending_reveal` resolves it on the first snapshot, as for any reveal.
- **Back with an empty stack**: the key does nothing; the button is hidden.

## Header UI (all drawers: Pod, Node, kind; not the Port forwarding drawer)

`[← api] [icon] SERVICE api-gateway [copy] ........ [^] [v] 12 of 40 [⋯] [x]`

| Control | Shown when | Text |
|---|---|---|
| Back button (ghost, small, `IconName::ArrowLeft` or `ChevronLeft`) + previous name, max 20 chars with `…` | `history.previous()` is some | label `api`; tooltip `Back to Service api (Alt+Left)`; a place without a selection shows the screen title, e.g. `Overview` |
| Prev / Next, position | see links.md | |

The Back label is the "from X" hint; no breadcrumb trail (a trail of 50 would not fit and adds nothing over repeated Back).

## Keys (0028 additions)

| Key | Action | Context | Sheet group / text |
|---|---|---|---|
| `Alt+Left` | `GoBack` | Workspace | Drawer · `Back` |
| `Alt+Right` | `GoForward` | Workspace | Drawer · `Forward` |

Text fields, the YAML / values editors and the terminal keep the keys. Add both to the terminal `NoAction` list. The coder must check which key context the kit `Input` and the editors expose and whether they bind `alt-left` / `alt-right` on Windows; where they do not, add explicit `NoAction` bindings for those contexts (the `FIELDS` list, `YAML_EDIT`, `VALUES_EDIT`), so Alt+Left in a focused field never navigates (AC 7). Both actions appear in the palette through `shortcut_rows` as `Back` and `Forward`.
