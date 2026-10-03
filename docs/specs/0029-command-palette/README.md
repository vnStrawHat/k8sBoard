# 0029 — Command palette (W9)

Status: implemented (deviations at the end), amended after the advisor review, HEAD `0350adb`. Lands after 0028 (keys, actions, `shortcut_rows`, `key_availability`) and 0026 (cluster rows, `switch_cluster`). Crate: `crates/app` only. Read-only and local: no cluster write, no new list or watch, no file written. Wireframe: W9 and its notes 1–5, the title-bar search box (inventory T6), the keyboard map (Ctrl K, `:`).

## Goal

- Ctrl K, `:`, and the title-bar search box open a palette: a kit `Dialog` hosting the kit `Command` list.
- Prefixes `:` kind, `@` cluster, `#` namespace, `>` action; no prefix searches everything.
- Fuzzy, multi-token ranking with a small local scorer (no new dependency).
- Groups Actions, Resources, Go to, with live status pills, key hints, a scope header (cluster env badge, `ns:`), and a syntax footer.
- Actions run the same 0028 unit actions on the shell; row actions target the cursor row; disabled and mutating actions show their reason.
- Resources: objects of the already loaded lists only; selecting one reveals it and opens its drawer.

## Non-goals

New list or watch calls for search (decision 9); action × search-hit pairs (decision 10); a Tab preview that opens a drawer or switches screens (decision 17); Ctrl ⏎ (0032); multi-cluster ticks from `@` (0027 switcher); any enabled mutating action or confirmation dialog (0030+); query history or saved commands; matched-character highlighting; several namespaces at once from `#`; multi-cluster search (0027).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | `fuzzy_score`, `parse_query`, `entry_score`, kind `short_names` | 1, 2, 3 |
| 2a | Host: `open_palette` (entity and `CommandState` created once), `OpenPalette` / `OpenKindPalette`, shell focus accessor, initial `:` Esc rule, headless tests | 1, 2, 4 |
| 2b | Command, screen, and namespace sources, header, footer, title-bar box, dispatch, Tab preview | 1, 2, 5, 8 |
| 3 | Resources from loaded lists, row actions with reasons, `@` clusters, caps | 1, 2, 6, 7, 9 |
| 4 | `--palette <query>`, roadmap docs, ui-verifier run | 1, 2, 10 |

## Files

| File | Contents |
|---|---|
| [query-and-ranking.md](query-and-ranking.md) | modes, kind aliases, `fuzzy_score`, entry score, order, caps, budget |
| [entries.md](entries.md) | entry model, sources per group, running an entry, mutating actions, C1 and no-new-list rules |
| [palette-ui.md](palette-ui.md) | Dialog + Command host, rows, header, footer, keys, title-bar box, focus, launch flag |
| [decisions.md](decisions.md) | numbered decisions |
| [files-to-touch.md] | files per step, docs to update |
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
- [x] 9. Mutating row actions appear disabled with their 0028 reason and can never be confirmed; `@` rows show env badge, health, and the Ctrl 1–9 hint.
- [x] 10. (checked by the coder from screenshots `.tmp/ui-shots/v65-palette-*`; the ui-verifier run is still to do) ui-verifier: `--palette "> rest pay"` on Deployments and `--palette ":"` on Pods match W9 apart from decisions 10, 19; the 0003 color-literal grep is clean.

## Open items

1. Settled: Tab preview is the cheap form only (decision 17). A richer preview (drawer behind the scrim) would need watches; revisit on user request.
2. Action × search-hit pairs (W9 `Restart rollout · deployment/payments-api` while that object is not the cursor) need a target picker per action; revisit with 0032 when actions are enabled.
3. W9 "Go to prod-us-1 · same namespace payments": carrying the namespace across clusters conflicts with 0026's per-cluster memory. Decide in 0027 or a follow-up.
4. Searching kinds that are not loaded (for example all Deployments while on Pods) would need list calls; decision 9 refuses it. Revisit with the 0020 always-on watches (C13): any list those keep loaded is searchable for free through `PaletteInput`.
5. Matched-character highlighting (W9 underlines) is skipped; the kit `CommandItem::child` allows it later with `StyledText` highlights.

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
