# 0059 · Files to touch

[Back to index](README.md) · One lane, one coder: the files overlap.

## `crates/app/src/yaml_edit.rs`

| Item | Change |
|---|---|
| module doc | "a Diff tab from the server's dry-run" → a local Diff tab and a Dry-run button |
| `LocalDiff` (new), `YamlEditView.local_diff` | as [view-design.md](view-design.md); `empty` starts `Idle` |
| `local_diff_rows`, `without_header` (new, private) | pure; `format_yaml` + `diff_rows`; equal bodies → `Ok(Vec::new())` |
| `refresh_local_diff` (new) | background task, stale-write guard, `diff_list.reset` |
| `refresh_dirty` | `&gpui_kit::App` → `&mut Context<Self>`; ends with `refresh_local_diff` on the Diff tab, `Idle` elsewhere. Callers: the `InputEvent::Change` subscription, `format`, `finish_load` (×2), `set_text_for_test` |
| `show_tab` | drops the `run_preview` call; on `Diff` calls `refresh_local_diff` |
| `can_check` | replaced by `dry_run_block_reason` (keeps the fixture early return) |
| `apply_block_reason` | takes `text: &str`; adds `Run the dry-run first`, the Recreated reason |
| `dry_run` (new, `pub(crate)`) | the button's handler |
| `apply_press` | no `run_preview`; confirm only when checked and `KeyPress::Fresh` |
| `run_preview` | no `self.tab = Diff`; the task no longer runs `diff_rows`; passes no rows |
| `finish_preview` | loses the `rows` parameter, the `diff_list.reset`, and the `(Ok, None)` arm pairing (match on `result` only) |
| `finish_load` | `Rebase` arm: `preview = NotChecked` without `self.apply(window, cx)` |
| `PassedPreview` | delete `rows`; doc of `request`: "the request Apply hands to the confirm dialog" |
| `fixture` | `local_diff = Ready { for_text: after, rows: local_diff_rows(EDIT_FIXTURE_BEFORE, &after).map_err(..) }`; `diff_list.reset`; the `PassedPreview` loses `rows`. If `EDIT_FIXTURE_BEFORE` is not serializer output, rows other than the two changes appear: then make the constant match `format_yaml`'s output (`screenshot.rs`) |
| test accessors | add `#[cfg(test)] local_diff_for_test(&self, cx: &App) -> Option<&Result<Vec<DiffRow>, SharedString>>` (rows only when `for_text` is the current text, i.e. what the tab draws); keep the others |

## `crates/app/src/yaml_edit_panels.rs`

| Item | Change |
|---|---|
| `render_tabs` | count only when `is_checked(text)`; needs `text` (pass it from `render`, like `render_side`) |
| `render_diff` | reads `local_diff` + `text` per the Render table; `Nothing to check: the text is unchanged` is gone |
| `diff_row_at` | reads `LocalDiff::Ready { rows: Ok(rows), .. }` |
| `render_side` | change list and quota only when `is_checked(text)` |
| `render_footer` | new `Dry-run` button (id `edit-dry-run`) before `Apply…`; `Apply…` tooltip from `apply_block_reason(text)`, enabled tooltip `Apply the checked change` |

## `crates/cluster`

| File | Change |
|---|---|
| `edit_preview.rs` | delete `EditPreview::{before, after}` and their two `to_yaml_text` lines in `build_preview`; manual `Debug` unchanged; `build_preview` returns `EditPreview` when nothing else in it fails |
| `object_write.rs` | the `build_preview` call (~line 1171) drops `.map_err(..)?` if `build_preview` became infallible |
| `edit_preview_tests.rs` | see [test-plan.md](test-plan.md) |

## Docs

| File | Change |
|---|---|
| `docs/specs/0031-edit-yaml/editor-view.md` | "Superseded" note under "Diff opens with its check" (done with this spec) |
| `docs/k8sboard-wireframes.html` | no change; W10 notes "diff vs live + server dry-run + confirm", which still holds. The ui-verifier checks the new footer button |
