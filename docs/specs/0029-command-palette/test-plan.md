# 0029 · Test plan

[Back to index](README.md). **S** is the step that adds the test. Offline and deterministic, one behavior per test. Pure functions get plain unit tests; keymap tests reuse the 0028 `resolve` helper (`TestAppContext`, no window); two dispatch tests use the headless shell window of `app_shell_tests.rs` (no session, no network).

## Scorer and query (`fuzzy_score.rs`, `palette_search_tests.rs`)

| S | Test | Checks |
|---|---|---|
| 1 | `fuzzy_score_rejects_a_non_subsequence` | `xyz` in `payments-api` → `None` |
| 1 | `fuzzy_score_ignores_ascii_case` | `PAY` = `pay` against `payments-api` |
| 1 | `fuzzy_score_prefers_word_starts` | `pa` scores `payments-api` above `deploy-pa`; `dp` scores `deploy-pa` above `adpx` (word start beats mid-word) |
| 1 | `fuzzy_score_counts_index_zero_once` | `fuzzy_score("p", "p-q") - fuzzy_score("q", "p-q")` equals the scorer's own prefix-bonus constant: index 0 earns the word-start bonus once, and only the prefix rule adds to it |
| 1 | `fuzzy_score_prefers_consecutive_matches` | `api` scores `payments-api` above `a-p-i` |
| 1 | `fuzzy_score_ranks_exact_then_prefix_highest` | `pods` vs `pods`, `podsx`, `prods` |
| 1 | `fuzzy_score_empty_needle_matches_with_zero` | `""` → `Some(0)` |
| 1 | `parse_query_reads_each_prefix` | `:po`, `@prod`, `#web`, `> rest pay`, `api` → mode and text |
| 1 | `parse_query_skips_leading_spaces` | `"  :po"` → `Kinds`, `po` |
| 1 | `entry_score_needs_every_token` | `rest pay` matches label + detail; `rest zzz` → `None` |
| 1 | `entry_score_lets_tokens_share_a_field` | `pay api` against one field `payments-api` → `Some` |
| 1 | `short_names_cover_every_kind` | every `ResourceKind::ALL` kind has a non-empty list; no alias repeats across kinds |

## Entries and ranking (`palette_search_tests.rs`, fixtures from existing `*_tests.rs` builders)

| S | Test | Checks |
|---|---|---|
| 2b | `empty_query_lists_actions_only` | `All` + `""` → only `Actions`; no `Resources` |
| 2b | `kind_mode_ranks_an_exact_alias_first` | `:deploy` → Deployments first; `:po` → Pods first |
| 2b | `kind_mode_disables_denied_kinds_with_the_sidebar_reason` | denied report → `Disabled { "Not permitted: list …" }` |
| 2b | `namespace_mode_lists_all_namespaces_first_then_the_live_list` | marks the current scope |
| 2b | `commands_come_from_general_and_dock_rows` | "Show all shortcuts" and "Open Settings" included; "Import kubeconfig file (Settings window)" and the two palette rows excluded (compared by `action.name()`); dock rows disabled "No dock tabs" without tabs |
| 2b | `no_session_lists_commands_and_screens_only` | `live: None` |
| 3 | `resources_search_pods_nodes_and_the_visible_kind_only` | a kind list not visible is never read (input has only the explorer kind) |
| 3 | `resource_entries_carry_name_and_status_only` | a ConfigMap row with cell text `secret-ish` and a label: query `secret-ish` → no match. Positive control: a row named `api-config` matches `api-conf`, and its label and detail are exactly `api-config` and `{namespace}/api-config`; `status` is the `StatusLabel` |
| 3 | `row_actions_follow_key_availability` | cursor pod: View logs enabled, Edit YAML disabled "Read-only mode"; no cursor → no row actions |
| 3 | `row_action_detail_names_the_cursor_object` | `deployment/payments-api` |
| 3 | `every_offered_row_action_maps` (`palette_search_tests.rs`) | each `ResourceAction` that `key_availability` can offer maps through `key_action` to its same-named action; `OpenShell` and `OpenNodeShell` → `OpenShell` (by `name()`) |
| 3 | `cluster_mode_lists_switcher_rows_in_order` | 0026 rows, active marked |
| 3 | `caps_cut_each_group_and_count_the_rest` | 60 matching pods → 50 kept, "+10 more" |
| 3 | `ranking_is_stable_for_equal_scores` | source order kept |

## Keys (`keymap_tests.rs`, extended)

| S | Test | Checks |
|---|---|---|
| 2a | `ctrl_k_opens_the_palette_inside_text_fields` | `secondary-k` under `AppShell > Input` → `OpenPalette` |
| 2a | `colon_opens_kind_mode_outside_text_fields_only` | `:` (typed `;`+shift, `key_char ":"`) under `AppShell` → `OpenKindPalette`; under `Input` → none |
| 2a | 0028 `bindings_avoid_reserved_keys`, `every_bound_action_is_on_the_sheet` | still pass; `RESERVED_KEYS` keeps `secondary-enter` (now 0032) |

## Headless dispatch (`app_shell_tests.rs`)

| S | Test | Checks |
|---|---|---|
| 2a | `ctrl_k_opens_the_palette_dialog` | `press("secondary-k")` → `has_active_dialog` and `focused_input` (the palette query input holds focus) |
| 2a | `escape_on_the_seeded_colon_closes` | `press(":")` → query `:`; Esc → dialog closed at once; `press(":")`, type `p`, Esc → still open with an empty query |
| 2a | `palette_entity_is_created_once` | two renders of the open dialog keep the same `CommandPalette` entity id |
| 2b | `tab_moves_the_cursor_only_for_a_visible_row` | highlighted pod on Pods: Tab → `selected` = that pod, drawer closed, dialog open, no screen change; highlighted node on Pods: Tab → nothing |
| 2a | `escape_clears_the_query_then_closes` | type `x`, Esc → dialog open, query empty; Esc → closed, shell root focused |

## Live checks (UAT, read-only) and ui-verifier

- Manual, release build: Ctrl K from the table and from the `/` filter; `:po` ⏎, `:deploy` ⏎; a pod name ⏎ opens its drawer; `#kube-system` ⏎ scopes; `>` on a selected pod lists View logs (runs) and Edit YAML (disabled, reason); mutating entries never confirm.
- Trace check (AC 6, sanity only; the proof is structural): `RUST_LOG=cluster=debug`, open the palette and type on Pods → no new `sending request` line. Confirming an entry may log requests (ordinary navigation).
- ui-verifier: `--screen deployments --select <name> --palette "> rest pay"` and `--screen pods --palette ":"` against W9. Known deviations: decisions 10, 19.
