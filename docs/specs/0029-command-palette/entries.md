# 0029 · Entries: sources, actions, data rules

[Back to index](README.md) · Steps 2b–3 · Modules: `palette_search.rs`, `command_palette.rs`, `navigation.rs`, `keymap.rs`. Decisions 8–16.

```rust
pub(crate) enum PaletteGroup { Actions, Resources, GoTo }
pub(crate) enum PaletteTarget {
    Command(Box<dyn Action>),                        // a 0028 action, dispatched on the shell
    RowAction(ResourceAction),                       // on the cursor row, dispatched as its 0028 action
    Screen(Screen),
    Resource(ResourceKey),
    Namespace(NamespaceScope),
    Cluster(ClusterRef),                             // 0026
}
pub(crate) struct PaletteEntry { pub(crate) group: PaletteGroup, pub(crate) label: SharedString,   // no derives
    pub(crate) detail: Option<SharedString>, pub(crate) status: Option<StatusLabel>,
    pub(crate) state: EntryState, pub(crate) target: PaletteTarget }
pub(crate) enum EntryState { Enabled, Disabled { reason: SharedString } }
/// Everything the palette may show, in source order, from in-memory state only.
pub(crate) fn palette_entries(input: &PaletteInput<'_>) -> Vec<PaletteEntry>;
pub(crate) fn ranked(entries: Vec<PaletteEntry>, query: &PaletteQuery<'_>) -> Vec<PaletteEntry>;
```

`PaletteInput` borrows what the shell already holds: `Option<&LiveCluster>`, the cursor `Option<&ResourceKey>`, the current `Screen`, the 0026 switcher sections, and `shortcut_rows()`. Building it takes no lock, no I/O, no new watch.

`PaletteEntry` derives nothing (`Box<dyn Action>` has no `PartialEq`/`Debug` worth deriving); tests compare fields and `action.name()`.

## Sources

| Group | Entries | Default list (empty text) | State |
|---|---|---|---|
| Actions | the cursor row's offered 0028 row actions (`key_availability` ≠ `NotOffered`), label `action_label`, detail `{kind}/{name}` | yes | `Disabled` with the 0028 reason, else `Enabled` |
| Actions | 0028 `shortcut_rows()` of groups General and Dock, except: the palette's own rows ("Command palette", "Jump to a resource kind") and "Import kubeconfig file (Settings window)" (its handler is on the `SettingsWindow` root, so a dispatch on the shell would be a silent no-op). "Show all shortcuts" stays: it runs after the palette closes and opens the sheet. "Open Settings" works (app-level `cx.on_action`) | yes | `Enabled`; dock rows `Disabled` "No dock tabs" while the dock is empty |
| Resources | rows of the **loaded lists**: pods, nodes, and the visible explorer kind's rows (`LiveCluster::kind_list` of `screen.kind()`) | no | `Enabled`; `status` = `pod_status_label`, `node_status_label`, `KindRow.status` |
| Go to | screens: Pods, Nodes, every `ResourceKind::ALL` kind | `Kinds` mode only | `Disabled` with the sidebar reason (`navigation::kind_availability`, made `pub(crate)`) |
| Go to | namespaces of `live.namespaces` plus "All namespaces" | `Namespaces` mode only | `Enabled`; the current scope is marked |
| Go to | clusters: 0026 `cluster_switcher_rows` (`switcher_sections`; fields from `search_text`), env badge, health line, `Kbd` of 0026's `SwitchToClusterN` | `Clusters` mode only | `Enabled`; the active cluster is marked |

- `@` reuses 0026 and binds nothing of its own: the palette never binds `secondary-shift-c` or `secondary-1`…`9` (0026 owns them). Confirm runs 0026 `switch_cluster`.
- 0027 (draft): `@` is a **single** switch through 0026. Viewing several clusters (ticks) stays in the switcher via `view_clusters`; W9 shows only single "Go to" rows, so the palette offers no multi-select.

- No session (kubeconfig missing, connecting): only the commands and the screens are listed; Resources and namespaces show the group's empty text "Cluster not connected".
- The Resources group says, under its heading when the text matches nothing, "Searched: Pods, Nodes{, visible kind}. Type :kind to open another kind." (decision 9).

## Running an entry (on confirm)

The palette closes first (`window.close_dialog(cx)`; the kit restores the previous focus), then runs the target with the window it got from `on_confirm`:

| Target | Runs |
|---|---|
| `Command(action)`, `RowAction(a)` | `shell_focus.dispatch_action(&*action, window, cx)` on the `AppShell` root `FocusHandle` (gpui-pre `window.rs:628`): the same 0028 handler as the key, so the cursor, gates, and notices apply unchanged. `RowAction(a)` dispatches `keymap::action_for(a)` (below) |
| `Screen(s)` | `shell.show_screen(s, cx)` |
| `Resource(key)` | `shell.reveal(key, cx)` (0028: opens the drawer; clears a filter that hides the row) |
| `Namespace(scope)` | `shell.set_namespace(scope, cx)` |
| `Cluster(c)` | `shell.switch_cluster(&c, cx)` (0026) |

The kit items carry **no** `.action(..)`: the kit would dispatch from the dialog's focus path, which is outside `AppShell` and reaches no handler (decision 11).

## Row action → key action (`keymap.rs`)

```rust
/// The 0028 unit action bound to a row action's key; `None` for a row action without a key.
pub(crate) fn action_for(action: ResourceAction) -> Option<Box<dyn Action>>;
```

- Lives in `keymap.rs` next to the 0028 `actions!` list, an exhaustive `match`. Today every variant maps: `ViewLogs`, `ViewYaml`, `CopyName`, `PortForward`, `Cordon`, `Drain`, `EditYaml`, `RestartRollout`, `Scale`, `Delete` → their same-named actions.
- `OpenShell` **and** `OpenNodeShell` → `OpenShell` (the S key); the 0028 handler picks pod or node from the cursor subject.
- A row action with no `action_for` is not offered in the palette (later variants without a key, for example 0032 Roll back, add one first). Test `every_offered_row_action_maps`.

## Mutating actions (W9 note 3)

- Every mutating row action is disabled in this version, so it shows its 0028 reason pill (for example "Read-only mode"; 0030 renames the unshipped reason to "Comes in a later version") and cannot be confirmed (the kit skips disabled items).
- When 0030–0036 enable an action, its entry is `Enabled` and shows the pill "needs confirm" when the 0030 confirmation rule applies to it (the rule is 0030's, not 0029's). 0029 adds no confirmation dialog.

## Data rules (C1, read-only, no new list calls)

- An entry holds only kind, namespace, name, and a `StatusLabel`. It never reads `KindRow.cells`, `sections`, `labels`, `event`, `object`, Env values, or YAML.
- Note (not an AC): a Secret row (0016) will therefore show its name and status label only; C1 summaries hold no values anyway.
- The query is never traced, logged, or persisted; traces carry counts and durations only. No query history.
- **No new list calls, structurally**: `palette_entries(&PaletteInput)` is a pure function over borrowed data. It receives no session `Entity`, no `Context`/`App`, and no connection, so it cannot start a list, watch, or request. `CommandPalette` only builds a `PaletteInput` from shell reads and calls it.
- Live sanity check: `RUST_LOG=cluster=debug` while opening the palette and typing shows no new `sending request` line.
- **Confirming** an entry is ordinary navigation: `:kind` (`show_screen`), a resource (`reveal`), or a namespace may start that screen's existing watch and the throttled sidebar count lists (C11), exactly like a sidebar click (decision 9).
