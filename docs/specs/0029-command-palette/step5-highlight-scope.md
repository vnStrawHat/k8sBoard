# 0029 · Steps 5a, 5b: matched characters and `@` namespace

[Back to index](README.md) · Modules: `fuzzy_score.rs`, `palette_search.rs`, `command_palette.rs`, `app_shell.rs`. Decisions 30–32. **Local-only: no new Kubernetes calls.**

## 5a · Underlined matches (W9 `<u>Rest</u>art rollout`, `deployment/<u>pay</u>ments-api`)

```rust
/// Byte ranges of `haystack` that `needle` matched, ascending and merged; `None` exactly when
/// `fuzzy_score(needle, haystack)` is `None`.
pub(crate) fn fuzzy_ranges(needle: &str, haystack: &str) -> Option<Vec<Range<usize>>>;  // fuzzy_score.rs
/// Ranges to underline in the label and the detail of `entry`, ascending and merged. Each token
/// underlines only the field that gave its best score in `score_of` order (label, detail, then
/// keywords; ties: the first); a token whose best field is a keyword underlines nothing.
pub(crate) fn entry_match_ranges(entry: &PaletteEntry, query_text: &str) -> EntryRanges;  // palette_search.rs
pub(crate) struct EntryRanges { pub(crate) label: Vec<Range<usize>>, pub(crate) detail: Vec<Range<usize>> }
```

- `fuzzy_ranges` follows `score`: the preferring alignment, else the leftmost one; when only the contiguous-run test (`windows`) made the match plausible, the ranges are that first run. Positions become byte ranges on char boundaries (ASCII: the index; other text: `char_indices`), consecutive positions merged.
- One alignment walk is shared by `fuzzy_score` and `fuzzy_ranges` (for example an `align` that can record positions); `fuzzy_score` stays allocation-free on its hot path.
- Computed in `render` for the shown entries only (≤ 100 rows), never in `ranked`.
- Only the (token, field) pairs `entry_score` matched are underlined (decision 30): `rest pay` on `Restart rollout` · `deployment/payments-api` underlines `Rest` in the label and `pay` in the detail, and not a `pay` that might also fit the label. `:po` on Pods: `po` scores best on the keyword `po` (exact), so the label shows no underline.
- `command_item(entry, query_text)` with `parse_query(&self.query).text`; `RowContent::of(entry, entry_match_ranges(entry, query_text))` keeps the label and detail ranges and draws both texts as `StyledText::new(text).with_highlights(..)` with
  `HighlightStyle { underline: Some(UnderlineStyle { thickness: px(1.), color: None, wavy: false }), ..Default::default() }`.
  `color: None` takes the text color (muted for the detail, muted again on a disabled row), so there is no color literal (same pattern as `log_rows.rs`).
- Empty query text → no ranges. Truncation of the detail (`truncate()`) is unchanged.

## 5b · `@` keeps the namespace (W9 Go to row `@ prod-us-1 · same namespace payments`)

```rust
pub(crate) enum PaletteTarget { /* … */
    /// A switcher row and the scope the switch carries; `None` keeps 0026's start scope.
    Cluster(SwitcherRow, Option<NamespaceScope>) }
/// `switch_cluster`, starting the target in `scope` when given: for this switch it wins over the
/// remembered and the saved default scope (0026 decision 3), as `--namespace` does.
pub(crate) fn switch_cluster_in_scope(&mut self, target: &ClusterRef,
    scope: Option<NamespaceScope>, cx: &mut Context<Self>);   // app_shell.rs
```

- `cluster_entries`: the carried scope is the live scope when it is a single `Named` namespace and the row is not the active cluster (`!row.is_active`); otherwise `None` (decision 31, accepted by the user).
- Detail when carried: none; `same namespace payments` is the entry's `note`, shown where the detail would be and never matched by the query.
- `switch_cluster(target, cx)` becomes `switch_cluster_in_scope(target, None, cx)`. The scope rides through `confirm_leaving` (shells, node shells, batches, unsaved edit) into `switch_to(target, scope, cx)`, whose `requested` argument already wins over `start_scope`; its doc comment names both callers.
- `CommandPalette::confirm`: `Cluster(row, scope)` → `shell.switch_cluster_in_scope(&row.cluster, scope, cx)`.
- Not carried: `All` and several namespaces (the target's memory is the better start), the active cluster (a switch to it does nothing), Ctrl 1–9 and the cluster switcher (0026 unchanged).
- A namespace missing in the target: no list call checks it; the lists are empty and the header reads `ns: payments`, as for a saved default namespace that does not exist. The namespace picker changes it.
- When the user later leaves the target, the carried scope is remembered for it like any scope (`record_leaving`).
- Existing matches on `PaletteTarget::Cluster(row)` in tests gain the second field.
