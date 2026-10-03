# 0027 · Switcher ticks

[Back to index](README.md) · Step 2 · Modules: `cluster_switcher.rs`, `cluster_switcher_rows.rs` (0026), `keymap.rs` (0028) or switcher `bind_keys`. Decisions 3, 5, 9, 10. Wireframe: W1 `.dd-i .cb`, `.dd-f`.

## State and rows

```rust
// ClusterSwitcherState (0026) gains:
ticked: Vec<ClusterRef>,           // draft; applied with view_clusters
tick_notice: Option<SharedString>, // "View at most 5 clusters at once."
// SwitcherRow (0026) gains:
pub(crate) is_ticked: bool,
// cluster_switcher_rows.rs:
pub(crate) fn toggle_tick(ticked: &mut Vec<ClusterRef>, cluster: &ClusterRef) -> Result<(), TooManyClusters>;
pub(crate) fn ticks_differ(ticked: &[ClusterRef], viewed: &[ClusterRef]) -> bool; // as sets
pub(crate) fn normalize_query(text: &str) -> String;   // lowercase, all whitespace removed (decision 9)
```

- Open: `ticked = viewed set` in both modes; single mode starts at `[current]` (decision 10).
- `search_text` (0026) is built without whitespace, so `prod eu` and `prodeu` match a label `prod eu 1`.

## Row layout (adds to 0026)

| Part | Content |
|---|---|
| Checkbox | kit `Checkbox` left of the badge (W1 `.cb`); click toggles the tick only (stops propagation, so the row click does not switch) |
| Current mark | every viewed slot gets the 0026 check-mark treatment; the primary is also bold |

## Footer (`.dd-f`)

Shown only when `ticks_differ(ticked, viewed)`. `{n} selected` (bold) left; right: ghost `Clear` (sets `ticked = []`), primary `View {n} clusters` + `Kbd` ⏎ (`View 1 cluster` for one; disabled for zero). The 0026 "Manage clusters…" row stays below (`.dd-m`). A refused sixth tick shows `tick_notice` in theme warning above the footer until the next tick change.

## Keys

| Key | Binding | Context | Effect |
|---|---|---|---|
| Space | `space` → `ToggleClusterTick` | `ClusterSwitcher` and `ClusterSwitcher > Input` | ticks or unticks the highlighted row |
| ⏎ | `enter` → `SwitcherConfirm` (0026) | same | `ticks_differ` and non-empty → `view_clusters(ticked)`; else 0026 switch to the highlight |
| Esc | 0026 | same | closes; the draft is discarded |

Collision with the kit: the kit `Popover` binds `space` → `Confirm` in context `Popover` (gpui-base 0.7 `popover.rs:21`). `ClusterSwitcher` and `ClusterSwitcher > Input` both match deeper than `Popover`, so depth decides and the 0027 binding runs; the handler never propagates, so the kit `Confirm` (which would close the popover) never runs and the filter never receives a space (decision 9).

0028 follow-up (done): `RESERVED_KEYS` dropped `space`; `keymap.md` Space row moves to "Bound by 0027"; no sheet row (switcher-local, like the 0026 popover keys).

## Applying

- `View {n} clusters` / Enter (when ticks differ) → close the popover, `view_clusters(&ticked)`.
- A name click or Ctrl 1–9 → 0026 single switch; the draft is discarded.
- Ticks equal to the viewed set: no footer; Enter switches to the highlight (0026).

## Health and probes

Unchanged from 0026. Viewed slots show their session health (0026 decision 11 per slot). Ticking never probes or connects.
