# 0029 · Decisions

[Back to index](README.md). Architect defaults; the user confirms.

## Search

| # | Decision | Rationale |
|---|---|---|
| 1 | Own scorer (`fuzzy_score`, ~60 lines), **no new dependency**; ranking properties and named tests are the contract, weights are the coder's | `Cargo.lock` has no fuzzy matcher (checked: no `nucleo`, `fuzzy-matcher`, `sublime_fuzzy`, `skim`, `strsim`); the candidate set is small (thousands), so a greedy subsequence scorer is fast enough. Supersedes the C6 row "Fuzzy matching: `nucleo`" |
| 2 | Ranking in the host (`filterable(false)`), not the kit's filter | the kit filter is a case-insensitive substring test in source order: no fuzzy, no scores, no multi-token |
| 3 | Multi-token queries: every token must match a field; scores add | W9 `rest pay` matches the action `Restart rollout` and the target `payments-api` |
| 4 | An exact kind alias ranks first in `:` mode | `:po` ⏎ must always open Pods, as in k9s |
| 5 | Prefixes `: @ # >` select one group; no prefix searches all | W9 note 1 |
| 6 | No-prefix empty query shows Actions and nothing else; Resources need at least one character | listing 1,000 pods unasked is noise |
| 7 | Caps per group (20 / 50 / 30) with a "+N more" hint | the kit list is virtual, but a short list is what a palette is for |

## Sources and safety

| # | Decision | Rationale |
|---|---|---|
| 8 | Entries come only from memory: `LiveCluster` lists, the cursor, 0026 switcher rows, `shortcut_rows()` | the palette must stay instant and add no API load |
| 9 | **No new list call**: Resources searches only the loaded lists (pods, nodes, the visible kind); other kinds are reached with `:kind`, which is ordinary navigation and starts that kind's existing explorer watch | a one-shot list per keystroke or per open would multiply API calls on large clusters; the hint under Resources says what was searched |
| 10 | Row actions target the **cursor row** only; the wireframe's action × search-hit pairs (`Restart rollout · deployment/payments-api` found by name) are not built | requested scope; to act on another object, Go to it first (it becomes the cursor), then `>` |
| 11 | Commands run by `FocusHandle::dispatch_action` on the `AppShell` root after the dialog closes; kit items carry no `.action` | the dialog lives outside `AppShell`, so the kit's own dispatch from the palette's focus reaches no shell handler; dispatching on the shell keeps one handler per command (0028) |
| 12 | Entries hold kind, namespace, name, and `StatusLabel` only; queries are never traced or stored | C1 and the "never log secrets" rule; a Secret's name is not secret, its values never reach a summary |
| 13 | Mutating actions are listed, disabled, with the 0028 reason; "needs confirm" appears only once an action is enabled by 0030+ | W9 note 3; read-only rule |
| 14 | Disabled screens show the sidebar reason (`kind_availability` made `pub(crate)`) | one source for "Not permitted" texts |

## UI

| # | Decision | Rationale |
|---|---|---|
| 15 | Kit `Dialog` hosting kit `Command`, not a `Popover` | W9 shows a scrim and a centered panel; `Command` already gives input, groups, virtual list, keyboard, disabled items |
| 16 | A new `CommandPalette` entity per open, observing the shell | live statuses (W9 note 4) without a global; nothing stale survives a close |
| 17 | "Tab preview" in its cheap, safe form only: Tab moves the table cursor to the highlighted resource when it is a visible row of the current screen (0028 `change_selection`, no drawer, no screen switch, no watch); the footer shows it only then | W9 names it without showing the preview; this form adds no API load and cannot surprise |
| 18 | Ctrl ⏎ is not bound; its reservation moves to 0032 | in W9 it appears only on Scale, a 0032 action with an input; no read-only entry has a secondary action |
| 19 | `@` rows reuse 0026 `cluster_switcher_rows` (`switcher_sections`, `search_text`) and `switch_cluster`; the palette binds none of the 0026 keys (`secondary-shift-c`, `secondary-1`…`9`); `@` is a single switch (0027 multi-view stays in the switcher via `view_clusters`); Go to a cluster does not carry the namespace across | W9 shows single Go to rows; "same namespace payments" conflicts with 0026 decision 3 (scope remembered per cluster); open item 3 |
| 20 | `--palette <query>` launch flag | ui-verifier cannot type; one flag reproduces W9 |
| 21 | Opened by `:`, the first Esc (or Backspace) on the untouched seed closes the palette (tracked through `on_query`) | k9s habit: `:` then Esc returns to the table in one key |
| 22 | Row actions dispatch `keymap::action_for(ResourceAction)`; both shell variants map to `OpenShell` | one key handler per action; the palette cannot drift from the keys |
