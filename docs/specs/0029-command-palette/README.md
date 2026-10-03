# 0029 — Command palette (W9)

Status: steps 1–4 implemented (deviations at the end), amended after the advisor review, HEAD `0350adb`. **Step 5 (5a–5c): draft 2026-10-03 against main `a50264c`, local-only: no new Kubernetes calls** (wireframe-gap-audit gap 7). Lands after 0028 (keys, actions, `shortcut_rows`, `key_availability`) and 0026 (cluster rows, `switch_cluster`); step 5 after 0030–0033 and 0032b (shipped gates and confirm flows). Crate: `crates/app` only. The palette never writes: a mutating entry opens the same gated flow as its key. Wireframe: W9 and its notes 1–5, the title-bar search box (inventory T6), the keyboard map (Ctrl K, `:`).

## Goal

- Ctrl K, `:`, and the title-bar search box open a palette: a kit `Dialog` hosting the kit `Command` list.
- Prefixes `:` kind, `@` cluster, `#` namespace, `>` action; no prefix searches everything.
- Fuzzy, multi-token ranking with a small local scorer (no new dependency).
- Groups Actions, Resources, Go to, with live status pills, key hints, a scope header (cluster env badge, `ns:`), and a syntax footer.
- Actions run the same 0028 unit actions on the shell; row actions target the cursor row; disabled and mutating actions show their reason.
- Resources: objects of the already loaded lists only; selecting one reveals it and opens its drawer.
- Step 5: action × resource results (`> rest pay` → `Restart rollout · deployment/payments-api`) that reveal the hit and run its key's gated flow; a `needs confirm` pill; underlined matched characters; `@` keeps a named namespace scope.

## Non-goals

New list or watch calls for search (decision 9); pairs for kinds that are not loaded, for Roll back, Delete, or unshipped actions, or with Ctrl ⏎ (decisions 25, 26); a Tab preview that opens a drawer or switches screens (decision 17); a confirmation dialog of the palette's own (the 0030 dialogs are reused); query history or saved commands; several namespaces at once from `#`; anything multi-cluster (the app shows one cluster at a time, 0046); carrying the namespace from Ctrl 1–9 or the switcher.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | `fuzzy_score`, `parse_query`, `entry_score`, kind `short_names` | 1, 2, 3 |
| 2a | Host: `open_palette` (entity and `CommandState` created once), `OpenPalette` / `OpenKindPalette`, shell focus accessor, initial `:` Esc rule, headless tests | 1, 2, 4 |
| 2b | Command, screen, and namespace sources, header, footer, title-bar box, dispatch, Tab preview | 1, 2, 5, 8 |
| 3 | Resources from loaded lists, row actions with reasons, `@` clusters, caps | 1, 2, 6, 7, 9 |
| 4 | `--palette <query>`, roadmap docs, ui-verifier run | 1, 2, 10 |
| 5a | Highlight: `fuzzy_ranges`, `match_ranges`, underlined label and detail | 1, 2, 11, 15 |
| 5b | `@` namespace carry: `PaletteTarget::Cluster(row, scope)`, `switch_cluster_in_scope` | 1, 2, 11, 16 |
| 5c | Action × resource pairs, `needs confirm` pill, `run_row_action_on`, docs, ui-verifier | 1, 2, 11–14, 17 |

## Files

| File | Contents |
|---|---|
| [query-and-ranking.md](query-and-ranking.md) | modes, kind aliases, `fuzzy_score`, entry score, order, caps, budget |
| [entries.md](entries.md) | entry model, sources per group, running an entry, mutating actions, C1 and no-new-list rules |
| [palette-ui.md](palette-ui.md) | Dialog + Command host, rows, header, footer, keys, title-bar box, focus, launch flag |
| [step5-results.md](step5-results.md) | step 5c: pairs, `needs confirm`, running a pair through the key's flow |
| [step5-highlight-scope.md](step5-highlight-scope.md) | steps 5a, 5b: matched-character ranges, `@` namespace carry |
| [decisions.md](decisions.md) | numbered decisions (23–32 for step 5) |
| [files-to-touch.md](files-to-touch.md) | files per step, docs to update |
| [test-plan.md](test-plan.md) | scorer, entries, keys, headless dispatch, live and ui-verifier checks |

## Acceptance criteria

- [x] 1. The quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`. `Cargo.lock` unchanged.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline; none opens a real window. Partly met: see deviations 4 and 5.
- [x] 3. `:po` and `:deploy` rank Pods and Deployments first; `rest pay` matches `Restart rollout` on a `payments-api` cursor row.
- [x] 4. Ctrl K opens the palette from the table, the root, and text fields, with the query input focused; `:` only outside text fields; the title-bar box opens it on click. Esc clears the query, then closes; on an untouched `:` the first Esc closes; focus returns where it was.
- [x] 5. A command entry runs the same 0028 handler as its key (same notice for a disabled one, same dock effect).
- [ ] 6. Resources lists only pods, nodes, and the visible kind's rows; choosing one shows its screen with the row selected and the drawer open. Opening and typing start no list, watch, or request: `palette_entries(&PaletteInput)` is pure over borrowed data (no session entity, no `cx`; review), and a `RUST_LOG=cluster=debug` run shows no new `sending request` line while typing. Confirming an entry is ordinary navigation (it may start that screen's watch and throttled count lists).
- [x] 7. No entry shows or matches cell text, labels, sections, Env values, or YAML; a row whose name contains the query matches, and its label and detail are namespace/name only; queries are not traced (tests plus review).
- [x] 8. The header shows the active cluster with its env badge color (theme token) and the `ns:` label; the footer shows the four prefixes and `↑↓ select · Esc close`.
- [x] 9. Disabled row actions show their gate reason (for example `Not permitted: …`, `Read-only mode`) and can never be confirmed; enabled mutating ones go through their 0030 confirm (AC 13, 14); `@` rows show env badge, health, and the Ctrl 1–9 hint.
- [x] 10. (checked by the coder from screenshots `.tmp/ui-shots/v65-palette-*`; the ui-verifier run is still to do) ui-verifier: `--palette "> rest pay"` on Deployments and `--palette ":"` on Pods match W9 apart from decisions 10, 19 (both superseded by step 5); the 0003 color-literal grep is clean.

Step 5 (local-only: no new Kubernetes calls):

- [ ] 11. Steps 5a–5c meet AC 1 and AC 2 for their tests ([test-plan.md](test-plan.md) step 5). `crates/cluster` has no diff; typing starts no list, watch, or request: `pair_entries` is pure over `PaletteInput` like `palette_entries` (W9 note 1; decision 9).
- [ ] 12. `> rest pay` lists `Restart rollout · deployment/payments-api` when `payments-api` is a loaded Deployments row and not the cursor; `> rest` alone lists no pair; the cursor object gets no duplicate; Roll back, Delete, and unshipped actions such as Drain have no pair (W9 Actions rows, note 1; decisions 24–26).
- [ ] 13. Confirming an enabled pair reveals the object (its screen, the row, the drawer over the workspace, v0.6) and then runs the handler of its key on it: Restart rollout opens the 0030 confirm dialog with the cluster's tier, and nothing is sent before Confirm. A disabled pair shows its gate reason (`Not permitted: patch deployments`) and cannot be confirmed (W9 note 3; decision 27).
- [ ] 14. Every enabled entry whose action is mutating (cursor row or pair) shows the `needs confirm` pill in the warning tone; View logs, View YAML, and Copy name never do (W9 note 3; decision 28).
- [ ] 15. Matched characters are underlined in every group, only where `entry_score` matched each token (`> rest pay`: `Rest` of `Restart rollout`, `pay` of `deployment/payments-api`; a keyword-only match underlines nothing); ranges are built for shown rows only; the underline takes the text color (0003 grep clean) (W9 rows; decision 30).
- [ ] 16. With scope `payments` (one namespace), `@` rows of clusters other than the active one read `same namespace payments`, and confirming one starts that cluster in `payments` over its remembered scope (after the `leaving_work` dialog when shells are open). With All or several namespaces there is no note and 0026 behavior holds; Ctrl 1–9 and the switcher are unchanged (W9 Go to row; decisions 31, 32).
- [ ] 17. ui-verifier: `--screen deployments --palette "> rest <name prefix>"` without a cursor row, and `--namespace <ns> --palette "@"`, against W9: the pair row with underlines and its UAT reason pill, the `same namespace …` note. The `needs confirm` pill is proven by tests only (UAT is read-only).

## Open items

1. Settled: Tab preview is the cheap form only (decision 17). A richer preview (drawer behind the scrim) would need watches; revisit on user request.
2. Settled by step 5 (decisions 23–28): action × search-hit pairs.
3. Settled by the user (2026-10-03, decision 31): `@` carries a single named namespace only.
4. Searching kinds that are not loaded (for example all Deployments while on Pods) would need list calls; decision 9 refuses it. Revisit with the 0020 always-on watches (C13): any list those keep loaded is searchable for free through `PaletteInput`.
5. Settled by step 5 (decision 30): matched-character underlines.

## Implementation notes and deviations

1. `action_for` is not added: `ResourceAction::key_action()` (0028) already is that exhaustive map, and `RowAction` dispatches it. `every_offered_row_action_maps` lives in `palette_search_tests.rs`.
2. `PaletteTarget::Cluster` carries the 0026 `SwitcherRow` (environment, health, `Ctrl n`), not only the `ClusterRef`; `PaletteEntry` gains `keywords` (extra search words) and `is_current`; `ranked` returns `Ranked { entries, more }` so the footer can show "+N more".
3. `PaletteInput` holds borrowed slices (`PaletteSession`: scope, access, namespaces, pods, nodes, visible kind rows), not `&LiveCluster`, which no test can build. `key_availability_of` is `pub(crate)` for it. `open_palette` takes a snapshot built by `AppShell::palette_snapshot`, because the shell is mid-update inside its key handler.
4. The Tab-preview tests live in `app_shell_switch_tests.rs` (they use the 0026 `go_live_for_test` seam plus a `set_pods_for_test` seam): `tab_moves_the_cursor_without_opening_the_drawer` and its siblings, not `tab_moves_the_cursor_only_for_a_visible_row`. `palette_entity_is_created_once` is `the_palette_survives_renders_with_its_query_and_focus`.
5. The Commands list leaves out "Switch to cluster 1–9" too (its dispatch would switch to cluster 1; clusters are under `@`).
6. `ResourceKind::short_names` gives every kind one extra word (`job`, `role`, `secret`, `cr`, `rb`, `crb`, `helm`, `crd` where kubectl has none).
7. `--palette` with a cursor row uses `--screen deployments-drawer --select <name>` (the drawer is open behind the scrim): `--select` only applies to drawer screens.
8. The screenshot hook closes all dialogs after the capture: a focused palette query input is a handle the debug leak check reports at exit.
9. The reason pill is an outlined span in the theme's warning tone (`tone_color`), not a kit `Tag`, whose colors wash out on a disabled row in dark.
10. `PaletteInput.include_resources` (set by `lists_resources(query)`) keeps the resource entries from being built unless the query is text in `All` mode. The visible kind's rows come before pods and nodes.
11. The palette keeps `shown`, the list the last render gave the kit, and confirms and previews against it. A shell notify or a query change only marks the ranking stale; `render` ranks once. The footer reads the kit highlight itself.
12. Tab preview is a no-op while a drawer is open, and the hint is then hidden. After a preview the highlight is set again on the previewed row once the new ranking (with the row actions of the new cursor) has placed it.
13. A token must be a contiguous run of the field or start on a word start (`rest` no longer matches `previous dock tab`). ASCII is scored on bytes.
14. Actions have icons (kit set); cluster rows show no `Ctrl n` hint. `open_palette` closes the cluster switcher and the namespace picker first. Not done: scope chips on the input row (the kit `Command` has no slot there; they stay in the header above it).
