# 0043 · Clusters page: search, order, colour

[Back to index](README.md) · Step 3 · Modules: `clusters_page.rs`, `cluster_form.rs`, `cluster_registry.rs`, `environment.rs`, `title_bar.rs`, `cluster_switcher_rows.rs`, `keymap.rs`, `shortcut_sheet.rs`. Wireframe: W2 header (`⌕ Search`), notes 3 and 4, the General `Color` row (`.sws`). One cluster at a time (0046): order only changes Ctrl 1–9 and the list order.

## Search (W2 header)

- `ClustersPage` owns `search: Entity<InputState>` (placeholder `Search clusters`), drawn in the page `title_suffix` before `Add cluster` (W2 position), width 200 px. Not persisted.
- Matching reuses the switcher filter: `cluster_switcher_rows::search_text` becomes `pub(crate)` and is fed the `ClusterRow` parts (label, context, environment badge, file name); the needle is `normalize_query(text)`. One rule for both lists.
- Filtered groups without a match are hidden; no match at all → muted `No clusters match '{text}'.`; the header count stays the full count.
- The selection does not change while typing; a selected row filtered out keeps its form.

## Order (W2 note 3)

Order already lives in `registry.clusters`: `cluster_groups` sorts rows by entry position, unregistered rows last in load order, and the switcher numbers Ctrl 1–9 from the same groups. 0043 only writes that order.

```rust
// cluster_form.rs, pure, tested without GPUI
/// Moves `from` to the place of `to` inside `group` (display order). Every row of `group` gets an
/// entry (`entry_mut`), then the group's entries move to the end of `registry.clusters` in the new
/// order; other groups keep their relative order. No-op when `from == to` or either is outside `group`.
pub(crate) fn move_cluster(registry: &mut ClusterRegistry, group: &ClusterGroup, from: &ClusterRef, to: &ClusterRef);
pub(crate) enum MoveStep { Up, Down }
/// One place up or down inside the group; no-op at the ends.
pub(crate) fn step_cluster(registry: &mut ClusterRegistry, group: &ClusterGroup, cluster: &ClusterRef, step: MoveStep);
```

| Interaction | Rule |
|---|---|
| Drag | `render_row` adds `.on_drag(DraggedCluster { cluster, group_title, label }, ..)` (the `DraggedTab` pattern of `dock.rs`); `drag_over::<DraggedCluster>` tints `theme.drop_target` only when the dragged row's `group_title` equals the target's; `on_drop` → `AppSettings::update(|s| move_cluster(..))` |
| Across groups | no tint, drop ignored: the environment decides the group (0024) |
| While searching | no `on_drag` (hidden rows make "drop here" ambiguous); hint text says so |
| Keyboard | `alt-up` → `MoveClusterUp`, `alt-down` → `MoveClusterDown` in `SettingsWindow && !Input`, on the selected row (`step_cluster`); bound in `keymap.rs`, listed in the shortcut sheet as `Move cluster up / down (Settings)` |
| Hint | muted under the list: `Drag to reorder inside a group; the order sets Ctrl 1–9.` |

- Registering on move is intended (0024 decision 20 registers on edit; a move is an edit). `Reset to defaults` still drops the entry, so that row returns to the end of its group (known, documented in the Reset tooltip: `Clears the overrides and the place in the list.`).
- Ctrl 1–9 in the switcher follow at once (`observe_global::<AppSettings>`).

## Colour (W2 note 4, `Color` row) — removed by [0053](../0053-custom-environments/README.md): no per-cluster colour; colours belong to environments

```rust
// environment.rs, next to environment_color (one token table)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "lowercase")]
pub(crate) enum ClusterColor { Red, Amber, Blue, Purple, Teal, Gray }
impl ClusterColor {
    pub(crate) const ALL: [Self; 6];                         // W2 swatch order
    pub(crate) fn of(environment: Environment) -> Self;      // PROD Red, STG Amber, DEV Blue, LOCAL Gray
    pub(crate) fn name(self) -> &'static str;                // tooltip: "Red", …
}
pub(crate) fn cluster_color(color: ClusterColor, cx: &App) -> Hsla;
// Red danger · Amber warning · Blue info · Purple magenta · Teal cyan · Gray muted_foreground
```

- `environment_color(env, cx)` becomes `cluster_color(ClusterColor::of(env), cx)`; no colour literal (0003 grep).
- `ClusterEntry.color: Option<ClusterColor>`; `ClusterProfile.color: ClusterColor` = entry value, else `ClusterColor::of(environment)`.
- **Reads**: the title-bar top border (`title_bar.rs`, 0024 decision 26) uses `cluster_color(profile.color)`. The environment badge keeps the environment colour everywhere (switcher, Clusters list, title bar): the badge is the risk signal, the border is the user's cue. **Removed on user request on 2026-10-04: the title bar has no coloured top or bottom border; it uses the kit's normal `title_bar_border`.**
- **Form**: row `Color` between Environment and Default namespace (W2 order): six 18 px round swatches, the current one ringed (`border_2`, `theme.foreground`), tooltip = name. Click → store `None` when the colour equals `ClusterColor::of(profile.environment)`, else `Some(color)`; so a cluster on its environment colour keeps following the environment.
- Changing the environment with `color: None` moves the border with it; with `Some` it stays.
- No new key for "labels in every table" (W2 note 4): after 0046 no table shows a cluster label.
