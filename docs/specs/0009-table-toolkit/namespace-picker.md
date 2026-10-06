# 0009 · App: multi-namespace picker

[Back to index](README.md) · Step 4 · Modules: `namespace_picker.rs` (new), `title_bar.rs`, `filter_bar.rs`, `app_shell.rs`, `cluster_session.rs`, `launch_options.rs`. Crate support is step 1 ([cluster-scope.md](cluster-scope.md)).

## Popover (replaces the namespace `DropdownMenu`)

```text
[ns: payments, web ▾]
┌───────────────────────────────┐
│ All namespaces                │  click → scope All, close
│ ───────────────────────────── │
│ [☑] payments                  │  checkbox → toggles the draft
│ [☑] web                       │  name (ghost button) → only this namespace, close
│ [☐] kube-system               │  scrollable, max height 360
│ ───────────────────────────── │
│ 2 selected (max 5)  Clear  [Apply] │
└───────────────────────────────┘
```

- `gpui_kit::component::popover::Popover` with `.open(state.anchor == Some(anchor))` and `.on_open_change(..)`. Two triggers share one state: today's title bar ghost button (`PickerAnchor::TitleBar`) and the W7 filter bar chip `Namespace: all ▾` (`PickerAnchor::FilterBar`); only the clicked one opens.
- Rows: `Checkbox::new(("ns-check", ix)).checked(..)` (no label) plus a ghost `Button` with the name. A checkbox that would exceed `MAX_NAMESPACES` is disabled with tooltip "At most 5 namespaces; pick All namespaces for more".
- Footer: `{n} selected (max 5)`; `Clear` empties the draft; `Apply` (primary) is disabled when the draft is empty or equals the current scope. Apply → `AppShell::set_namespace(NamespaceScope::of_namespaces(draft))`, close.
- Loading and failure rows stay as today: "Loading namespaces…" (disabled), or "Could not list namespaces" plus the context default namespace.

```rust
// namespace_picker.rs
pub(crate) const MAX_NAMESPACES: usize = 5;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PickerAnchor { TitleBar, FilterBar }
#[derive(Default)]
pub(crate) struct NamespacePickerState { pub(crate) anchor: Option<PickerAnchor>, draft: BTreeSet<String> }
impl NamespacePickerState {
    /// Opening copies the current scope's namespaces into the draft.
    pub(crate) fn open(&mut self, anchor: PickerAnchor, scope: &NamespaceScope);
    pub(crate) fn close(&mut self);
    /// Returns false (and changes nothing) when adding would pass `MAX_NAMESPACES`.
    pub(crate) fn toggle(&mut self, name: &str) -> bool;
    pub(crate) fn clear(&mut self);
    pub(crate) fn can_add(&self) -> bool;
    pub(crate) fn applied_scope(&self, current: &NamespaceScope) -> Option<NamespaceScope>; // None: Apply disabled
}
pub(crate) fn namespace_picker(anchor: PickerAnchor, trigger: Button, shell: &AppShell,
    cx: &Context<AppShell>) -> AnyElement;
```

`AppShell.namespace_picker: NamespacePickerState`; a context switch resets it.

## Namespace chips (filter bar, namespaced screens only)

| Scope | Chips |
|---|---|
| `All` | `Namespace: all ▾`: a second trigger of the same popover (`PickerAnchor::FilterBar`) |
| `Named(a)` | `Namespace: a ×` |
| `Several([a, b])` | `Namespace: a ×`, `Namespace: b ×` |

A click on `×` sets the scope to the remaining namespaces through `of_namespaces` (none left → `All`). Nodes and Namespaces (cluster-scoped) show no Namespace chip.

## Launch flag

`--namespace a,b` splits on commas; more than `MAX_NAMESPACES` is a usage error "at most 5 namespaces". `RequestedStart.namespace` and `ConnectInputs.requested_namespace` become `Option<NamespaceScope>`; `initial_scope` returns the requested scope when given (unchanged rule otherwise). Screenshot: `pods` with `--namespace kube-system,default`.

## Watch and RBAC effects

| Item | `All` | `Named` | `Several(N)` |
|---|---|---|---|
| Pods watches | 1 | 1 | N |
| Explorer watches (namespaced kind) | 1 | 1 | N |
| Namespaces, Nodes, object events | 1 each | 1 each | 1 each |
| Session total (max) | 5 | 5 | 2N + 3 ≤ 13 |
| Access review on scope change | 19 SSARs | 19 | 16N + 3, one namespace at a time, AND-combined (gating only) |
| Needs cluster-wide `list` | yes | no | no: only `list` in each picked namespace |
| One namespace denied | — | list error | the other namespaces show; an interruption banner names the denied one |

- `set_scope` already drops and restarts pods and the explorer; nothing else changes. The status bar keeps counting resource types, not HTTP watches.
- `kind_availability` uses the AND-combined report; its `Several` reason text is in [cluster-scope.md](cluster-scope.md).
- `Copy kubectl command` (0008) and logs use the row's own namespace, so they are unaffected.

## Filter and row note (UX batch 5c)

- A `Filter namespaces…` input sits at the top of the popover and gets the focus on open (`Popover::track_focus`); it is cleared on every open. It keeps the names that contain the text, ignoring case; the list says `No namespace matches` when none does.
- A muted line under the list says what the two targets of a row do: `Click a name to switch · tick boxes to combine`. The footer keeps `N selected (max 5)`.
- `--screen namespace-picker` opens the title-bar picker once the session is live (screenshot builds).
