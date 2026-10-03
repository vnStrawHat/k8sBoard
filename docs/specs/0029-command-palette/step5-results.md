# 0029 · Step 5c: action × resource results

[Back to index](README.md) · Step 5c · Modules: `palette_search.rs`, `command_palette.rs`, `resource_actions.rs`, `keyboard_navigation.rs`, `app_shell.rs`. Decisions 23–29. **Local-only: no new Kubernetes calls.**

The app shows one cluster at a time (0046): every pair belongs to the live session.

W9: `> rest pay` → Actions `↻ Restart rollout · deployment/payments-api`, pill `needs confirm`, `⏎`. The object is a search hit, not the cursor (note 1); a dangerous action goes through the confirm dialog (note 3).

## Model (`palette_search.rs`)

```rust
pub(crate) enum PaletteTarget { /* … */
    /// A row action on a search hit: the object becomes the cursor, then the action's key runs on it.
    ObjectAction(ClusterObject, RowAction) }
pub(crate) struct PaletteEntry { /* … */
    /// Enabled and its action's gate is `Mutating`: the row shows `needs confirm`.
    pub(crate) needs_confirm: bool,
    /// The query score `palette_entries` already computed (`All`-mode resource entries); `ranked`
    /// uses it instead of scoring again. `None`: not scored yet.
    pub(crate) score: Option<u32> }
pub(crate) struct PaletteInput<'a> { /* … */
    /// The query text when pairs apply; `None` builds none.
    pub(crate) pair_text: Option<&'a str> }
/// Pairs apply to `All` or `Actions` mode with two or more tokens.
pub(crate) fn lists_pairs(query: &PaletteQuery<'_>) -> bool;
fn pair_entries(input: &PaletteInput<'_>, text: &str) -> Vec<PaletteEntry>;
```

- `palette_entries` adds `pair_entries` right after `row_action_entries` when `pair_text` is `Some`.
- `AppShell::palette_snapshot(wants_resources: bool, cx)` becomes `palette_snapshot(query: &PaletteQuery<'_>, cx)`: `include_resources = lists_resources(query)`, `pair_text = lists_pairs(query).then_some(query.text)`, and `None` while `is_editing()` (the rule of the cursor row actions). `CommandPalette::refresh` passes `&parse_query(&self.query)`.
- `Clone for PaletteTarget`, `row_icon` (`row_action_icon(row)`), and `RowContent::of` (`key_action = row.key_action()`) gain the `ObjectAction` arm.

## Which pairs (pure)

1. Tokens: `text.split_whitespace()`. Fewer than two → no pairs.
2. Objects (decision 25), over the fields of a resource entry (name, `namespace/name`, kind words):
   - `All` mode: the resource entries are built anyway. `palette_entries` scores each once with `score_of`, drops the ones that miss, and stores the score on the entry (`PaletteEntry.score: Option<u32>`; `ranked` uses a stored score instead of scoring again). The pair candidates are the top `RESOURCES_CAP` (50) of these by score. No second scan.
   - `Actions` mode: no resource entry is built, so `pair_entries` makes the one scan of the loaded rows over the same fields and keeps the top 50.
   - Cost of a rebuild: tokens × loaded rows × 3 fields `fuzzy_score` calls (about 45,000 for 5,000 rows and 3 tokens; byte compares, no allocation). A rebuild runs on every query change and on every shell notify, which includes each batched watch update while the palette is open. It must stay inside the 4 ms budget of [query-and-ranking.md](query-and-ranking.md); the existing `palette ranked` trace (counts and duration) measures it. `// ponytail: top-50 objects; an index per token if traces exceed the budget.`
3. Actions: `ROW_ACTIONS` where `subject_action(row, &key)` is `Some`, minus `RollBack`, minus `Delete`, and minus every action that has not shipped (`is_planned`: gate `Planned` or `Mutating { is_shipped: false }`, for example Drain) (decision 26). Label: `state_label(action, action_label(action), object)` for a kind row (`Resume rollout`), else `action_label(action)`.
4. Listed when every token matches the label or an object field, some token matches the label, and a different token matches the object (decision 24).
5. Skip the object equal to `input.cursor`: its actions are listed already, with the state of the loaded row and the revision of Roll back.
6. State: `key_availability_of(row, &key, pod, session.guard)` with the session's guard: `NotOffered` → skip; `Disabled { reason }` → disabled; `Run(action)` → `row_block(action, object, None)` for a kind row (a paused Deployment's Restart), else enabled.
7. Detail: `subject_text(&key)`, then ` · {namespace}` when namespaced and the scope is not `Named` (decision 29).
8. Group `Actions`; `ranked` scores them with the cursor actions over label and detail, under the same cap (20).

## `needs confirm` (W9 note 3)

```rust
/// Whether the action reaches a 0030 confirm: its gate is `Mutating`.
pub(crate) fn needs_confirm(action: ResourceAction) -> bool;   // resource_actions.rs
/// Whether the action has not shipped: gate `Planned`, or `Mutating` with `is_shipped: false`.
pub(crate) fn is_planned(action: ResourceAction) -> bool;       // resource_actions.rs
```

- Set on enabled cursor entries and enabled pairs from the resolved `ResourceAction`. Commands, screens, resources, namespaces, and clusters never set it.
- `RowContent` draws it as an outlined pill `needs confirm` in `tone_color(StatusTone::Warn)`, the `reason_pill` style, before the `Kbd`. A disabled entry shows its reason pill instead.

## Running a pair

`CommandPalette::confirm` closes the dialog, then `ObjectAction(object, row)` → `self.update_shell(cx, |shell, cx| shell.run_row_action_on(object, row, window, cx))`.

```rust
/// A palette action on a search hit: `object` becomes the cursor (its screen, the row, the
/// drawer), then `row` runs on it exactly as its key would. Nothing runs while Edit YAML is open.
pub(crate) fn run_row_action_on(&mut self, object: ClusterObject, row: RowAction,
    window: &mut Window, cx: &mut Context<Self>);   // keyboard_navigation.rs
```

- Body: `is_editing()` → return. `self.when_selected(object.clone(), cx, |_, _| {})` (at once when already selected, else `reveal_then`). Then `cx.defer_in(window, ..)`, queued after the reveal's own deferred selection: when `selected == Some(object)`, `CopyName` → `copy_cursor_name(window, cx)`, any other row → `run_row_key(row, window, cx)`. A row that vanished leaves the selection empty and nothing runs.
- `run_row_key` reads the gate again (`key_availability` with the session's guard), then `run_available_row_key` opens the key's own flow: the confirm dialog (Restart, Pause, Suspend, Trigger, Re-run, Cordon, Set default), the popover (Scale, Edit min / max, Expand), the editor (Edit YAML), or the connect tier (shell, port-forward). The palette builds no intent and calls no write.
- The drawer opens on the object behind the dialog (v0.6 overlay drawer), so the user sees what the action targets.

## Limits

- Only loaded lists: Deployment pairs need the Deployments screen open (the snapshot holds the rows of the visible kind only); pod and node pairs work from any screen.
- Pairs are rebuilt with the ranking (query change, shell notify); never per frame.
