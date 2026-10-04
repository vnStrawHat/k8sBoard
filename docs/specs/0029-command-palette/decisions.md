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
| 10 | **Superseded by 23 (step 5).** Row actions target the **cursor row** only; the wireframe's action × search-hit pairs (`Restart rollout · deployment/payments-api` found by name) are not built | requested scope; to act on another object, Go to it first (it becomes the cursor), then `>` |
| 11 | Commands run by `FocusHandle::dispatch_action` on the `AppShell` root after the dialog closes; kit items carry no `.action` | the dialog lives outside `AppShell`, so the kit's own dispatch from the palette's focus reaches no shell handler; dispatching on the shell keeps one handler per command (0028) |
| 12 | Entries hold kind, namespace, name, and `StatusLabel` only; queries are never traced or stored | C1 and the "never log secrets" rule; a Secret's name is not secret, its values never reach a summary |
| 13 | (Its "later" clause is settled by 28, step 5.) Mutating actions are listed, disabled, with the 0028 reason; "needs confirm" appears only once an action is enabled by 0030+ | W9 note 3; read-only rule |
| 14 | Disabled screens show the sidebar reason (`kind_availability` made `pub(crate)`) | one source for "Not permitted" texts |

## UI

| # | Decision | Rationale |
|---|---|---|
| 15 | Kit `Dialog` hosting kit `Command`, not a `Popover` | W9 shows a scrim and a centered panel; `Command` already gives input, groups, virtual list, keyboard, disabled items |
| 16 | A new `CommandPalette` entity per open, observing the shell | live statuses (W9 note 4) without a global; nothing stale survives a close |
| 17 | "Tab preview" in its cheap, safe form only: Tab moves the table cursor to the highlighted resource when it is a visible row of the current screen (0028 `change_selection`, no drawer, no screen switch, no watch); the footer shows it only then | W9 names it without showing the preview; this form adds no API load and cannot surprise |
| 18 | Ctrl ⏎ is not bound; its reservation moves to 0032 | in W9 it appears only on Scale, a 0032 action with an input; no read-only entry has a secondary action |
| 19 | `@` rows reuse 0026 `cluster_switcher_rows` (`switcher_sections`, `search_text`) and `switch_cluster`; the palette binds none of the 0026 keys (`secondary-shift-c`, `secondary-1`…`9`); `@` is a single switch (0027 multi-view stays in the switcher via `view_clusters`); Go to a cluster does not carry the namespace across (**superseded by 31, step 5**) | W9 shows single Go to rows; "same namespace payments" conflicts with 0026 decision 3 (scope remembered per cluster); open item 3 |
| 20 | `--palette <query>` launch flag | ui-verifier cannot type; one flag reproduces W9 |
| 21 | Opened by `:`, the first Esc (or Backspace) on the untouched seed closes the palette (tracked through `on_query`) | k9s habit: `:` then Esc returns to the table in one key |
| 22 | Row actions dispatch `ResourceAction::key_action()` (0028, `resource_actions.rs`); both shell variants map to `OpenShell` | one key handler per action; the palette cannot drift from the keys |

## Step 5 (local-only: no new Kubernetes calls)

| # | Decision | Rationale |
|---|---|---|
| 23 | Action × resource pairs for search hits; supersedes decision 10. The cursor row's actions stay as they are | W9 note 1: `> rest pay` finds the action and the object together |
| 24 | A pair needs two or more tokens: one matches the action label, a **different** one the object | `> rest` alone would pair Restart with every Deployment; one token matching both (`restic-backup`) is noise |
| 25 | Objects come only from the loaded lists (decision 9), the top 50 by score on the tokens that do not name an action; one scan of the loaded rows in both modes, which also scores the `All`-mode resource entries (stored on the entry); Ctrl ⏎ stays on the cursor Scale entry | adds no API load. The real cost is the scan, not the ≤ 50 × 19 pairs: tokens × loaded rows × 4 fields per rebuild, which runs on every keystroke and, throttled to one per 250 ms, on shell changes such as batched watch updates; one scan serves both the resource scores and the pair objects. It must fit the 4 ms budget, measured by the `palette ranked` trace |
| 26 | No pair for Roll back, Delete, or an action that has not shipped (`is_planned`: a kind without a check; Drain shipped with 0034 and pairs) | Roll back: revisions load only for the cursor Deployment's drawer, and the cursor entry `Roll back to rev N` covers W9. Delete: `delete_at_cursor` acts on the ticked set when the cursor is one of several ticked rows, so `Delete · pod/x` could open an N-object dialog. Unshipped: a pair that can never run is noise |
| 27 | Running a pair = reveal the object (`when_selected`), then, deferred, the key's own handler (`run_row_key`, or `copy_cursor_name` for Copy name) | one handler per action (decision 22); the gate is read again at run time; the confirm dialog, popover, or editor is the key's; the palette never builds an intent or writes |
| 28 | `needs confirm` = the action's gate is `Mutating`; shown on enabled entries only; supersedes the "later" clause of decision 13 | W9 note 3; every `Mutating` action reaches a 0030 confirm (dialog, popover, editor diff, or connect tier) |
| 29 | A pair's detail adds ` · {namespace}` when the object is namespaced and the scope is not a single namespace | two `api` Deployments in two namespaces must read differently; W9's `ns: all` example shows one |
| 30 | Underline only the (token, field) pairs that `entry_score` matched: each token underlines the field that gave its best score (ties: the first, in `score_of` order), and nothing when that field is a keyword; ranges are computed at render for shown rows only, with `color: None` | W9 `<u>`; the underline shows why the row matched, not every place a token could fit; ranking stays allocation-free; no color literal |
| 31 | `@` carries a single `Named` scope to a cluster other than the active one; for that switch it wins over the 0026 memory and the saved default (like `--namespace`); `All` and several namespaces carry nothing; supersedes the last clause of decision 19. Accepted by the user (2026-10-03) | W9 "same namespace payments"; explicit in the row text, so it cannot surprise; Ctrl 1–9 and the switcher keep 0026 decision 3 |
| 32 | No existence check of the carried namespace | local-only; a missing namespace shows empty lists, like a saved default namespace that does not exist |
