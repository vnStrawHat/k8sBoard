# 0059 · As built

[Back to index](README.md) · Deviations from the spec only; everything else was built as written.

- `LocalDiff::Running` holds only the task (`_task`), not `for_text`: a field nothing reads would be dead code. The stale-write guard in `refresh_local_diff` compares against the view's current text, and replacing `local_diff` drops the older task.
- `local_diff_rows` returns `Ok(vec![])` when the two bodies are equal. `diff_rows` alone folds an unchanged text into one `Folded` row, which would have shown instead of `No changes`.
- `dry_run_block_reason` answers `Reading the object…` while `base` is `None`, so the fixture needs no early return (both buttons stay disabled there). `is_fixture` remains for `environment()`.
- The side panel hides the stale check's `Checks` warning lines too (rollout, last-applied, quota), not only the change list and the quota line: they describe a text that is no longer in the editor.
- `build_preview` has no fallible step left, so it returns `EditPreview` and `object_write.rs` lost its `map_err`.
- `object_write_replace_tests.rs` also read `EditPreview.before/after` (three tests); they now read the field changes. `a_secret_is_masked_by_its_target_even_when_the_answer_names_no_kind` can no longer assert `password: <hidden>`: the unchanged Secret data produces no change, so it asserts the label change is listed and that no change side holds the data.
- `finish_load` sets `tab = Editor` before `refresh_dirty` on a replace, so the Diff is not computed for a tab about to be left.
- `EDIT_FIXTURE_BEFORE` is serializer output already: `--screen edit-yaml-diff` shows exactly the two changed lines as rows.
