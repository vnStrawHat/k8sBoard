# 0026 · Switcher popover

[Back to index](README.md) · Step 2 · Modules: `cluster_switcher_rows.rs` (new, pure row model), `cluster_switcher.rs` (new, state + render), `title_bar.rs`, `app_shell.rs`. Decisions 6–8, 12–14. Wireframe: W1 `.dd`, `.dd-in`, `.dd-g`, `.dd-i`, `.dd-m`.

## Trigger and focus

`title_bar::cluster_switcher` keeps the 0024 trigger (env badge + label + caret) as the `trigger` of a controlled `Popover::new("cluster-switcher").open(state.is_open).on_open_change(..).track_focus(filter.read(cx).focus_handle())` (gpui-base `InputState::focus_handle(&self) -> &FocusHandle`, `input/base/state.rs:501`), default anchor, like `namespace_picker.rs`. Verified in gpui-base 0.7 `popover.rs`:

- On open the kit stores the previously focused handle and **focuses the tracked handle** (open branch, lines ~99–112), so the filter gets focus from the popover itself; focusing it earlier would be overwritten.
- On close it **restores the previous focus** when focus was inside the popover (close branch, lines ~116–123). The shell's existing `on_focus_lost` restore stays as the fallback.

## State (`cluster_switcher.rs`)

```rust
pub(crate) struct ClusterSwitcherState {
    pub(crate) is_open: bool,
    filter: Entity<InputState>,            // created once in AppShell::new, placeholder "Filter clusters…"
    segment: SwitcherSegment,              // reset to All on open
    highlight: Option<ClusterRef>,         // first visible row on open and on every edit
    health: HealthBoard,                   // health-probes.md
    probes: Vec<WatchSubscription>,        // dropped on close
}
```

Open (`open_cluster_switcher`): `is_open = true`, clear the filter text, start probes; focus is the kit's job (above). Close: `is_open = false`, `probes.clear()`, `health.clear_running()` (health-probes.md).

## Row model (`cluster_switcher_rows.rs`, pure)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SwitcherSegment { All, Connected }
pub(crate) struct SwitcherRow { pub(crate) cluster: ClusterRef, pub(crate) label: String,
    pub(crate) environment: Environment, pub(crate) health: RowHealth, pub(crate) shortcut: Option<u8>,
    pub(crate) is_active: bool, pub(crate) search_text: String /* lowercased label, context, badge, file name */ }
pub(crate) struct SwitcherSection { pub(crate) title: &'static str, pub(crate) rows: Vec<SwitcherRow> }
pub(crate) fn switcher_sections(groups: &[ClusterGroup], health: &HealthBoard,
    active: Option<(&ClusterRef, ActiveHealth)>) -> Vec<SwitcherSection>; // shortcuts 1..=9 in order
pub(crate) fn visible_sections(sections: &[SwitcherSection], filter: &str, segment: SwitcherSegment)
    -> Vec<SwitcherSection>;                                               // empty sections dropped
pub(crate) fn nth_cluster(sections: &[SwitcherSection], shortcut: u8) -> Option<&ClusterRef>;
pub(crate) enum HighlightStep { Next, Previous }
pub(crate) fn move_highlight(visible: &[SwitcherSection], current: Option<&ClusterRef>, step: HighlightStep) -> Option<ClusterRef>;
```

`groups` come from `cluster_groups` (0025 step 3, or its fallback location, see files-to-touch.md). `ActiveHealth` maps the session phase (decision 11).

## Layout (width 380, list max height 420 with vertical scroll)

| Part | Content | Kit parts |
|---|---|---|
| Header | filter `Input` (small, search icon prefix); segment `All {n}` / `Connected {m}` (counts over the unfiltered rows) | `Input`, two ghost `Button`s with `.selected()` |
| Section header | `{title}` left, `{count}` right, muted, small | `div` |
| Row | env badge (0024 `environment_badge`), mono label (flex, ellipsis), health line, right: a Retry or Check small button (Unreachable, Not checked) and the `Kbd` for `Ctrl n`, together | `Kbd::binding_for_action(&SwitchToClusterN, Some("AppShell"), window)` (kbd.rs:58) |
| Current mark | the active row: `theme.accent` background and `Icon::new(IconName::Check)` before the badge (W1 `.dd-i.on.cur`; `icons/check.svg` is in the kit default set) | `Icon` |
| Highlight | keyboard highlight: `theme.list_hover` background | — |
| Footer | `Manage clusters…` left, muted `opens Settings` + `Kbd::binding_for_action(&OpenSettings, Some("AppShell"), window)` right; click closes the popover and dispatches `ManageClusters` (0025: it switches an already open Settings window to the Clusters page; the hint stays on `OpenSettings`) | `Kbd` |

Health colors: Live/Reachable → `tone_color(Ok)`; Interrupted/Checking/Connecting → `Info`; Unreachable and Not checked → muted (W1 `.st.mute`). An Unreachable row (including the active Failed one) is dimmed (opacity 0.55 on the row body, not on Retry) and shows Retry next to its `Kbd` (W1 minikube row). The segment sits inline right of the filter; the chosen one is filled (`primary`).

## Interaction

- Row click or Enter on the highlight → close, `switch_cluster` (switch-lifecycle.md).
- Retry/Check click: health-probes.md "When" (does not switch).
- Typing → `visible_sections`; highlight = first visible row. Segment click → filter by Connected; highlight resets.
- Outside click → `on_open_change(false)` → close.

## Empty states

| Case | Text |
|---|---|
| Catalog loading | `Loading kubeconfigs…` |
| No clusters | `No clusters. Add one in Settings.` + the footer link |
| Filter has no match | `No cluster matches '{text}'.` |
| Connected segment empty | `No connected cluster yet. Open the list with All to check them.` |
