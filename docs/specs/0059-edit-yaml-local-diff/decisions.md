# 0059 · Decisions and rationale

[Back to index](README.md)

| # | Decision | Rationale |
|---|---|---|
| 31 | The Diff tab compares `without_header(base.text())` with `without_header(format_yaml(text)?)` through `diff_rows` (`yaml_diff.rs`). Equal bodies give no rows | `base.text()` is already serializer output (`object_edit_tests` asserts `format_yaml(base.text()) == base.text()`), so only the edited side needs formatting. `format_yaml` sorts keys and drops every comment except the leading block; dropping that block on both sides makes a deleted or changed header comment show nothing, since comments are never sent. **Changed default:** the orchestrator wrote `diff_rows(base.text(), after)`; the header strip is added for the "only the actual diff" request |
| 32 | `LocalDiff` is recomputed from `refresh_dirty` when `tab == Diff`, and from `show_tab(Diff)`. The render guard shows rows only for `for_text == text` | `refresh_dirty` already runs after every text change (the `InputEvent::Change` subscription, Format, the three `finish_load` paths, the test setter), so one call site covers Format, reload, rebase, Discard, and undo. The guard is the regression fix itself: no code path can show rows of another text again |
| 33 | `format_yaml` errors (`Syntax`, `NotAnObject`, `LeadingZero`, `TooLarge`) show as the Diff tab body, danger tone, the `EditError` display text (the text the footer shows for a local error) | One rule for Format and the Diff. A leading-zero number is refused here although the dry-run accepts it with a warning: the serializer would print it as decimal, so a diff would lie about it. The message says how to write it |
| 34 | `Dry-run` (kit `Button`, small, outline like `Cancel`, id `edit-dry-run`) between `Cancel` and `Apply…`. `run_preview` no longer sets `self.tab` | The user wants the check on demand. The result already has a place that does not depend on the tab: footer, side panel `Checks`, change list, quota line, 422 fields |
| 35 | `apply_press` opens the confirm only when `is_checked(text)`; otherwise it returns. `apply_block_reason(text)` adds `Run the dry-run first`. `KeyPress::Held` guard unchanged | Apply means apply; the first-press check was the hidden dry-run trigger the user removed. The confirm dialog keeps its own dry-run (0030), so a commit still never goes unchecked |
| 36 | The `Rebase` branch of `finish_load` ends with `preview = NotChecked` (no `self.apply(..)`) | A rebase is a new text: the user clicks Dry-run again. Keeps "runs only when the user clicks it" literal |
| 37 | Keep `Diff vs cluster` | The left side is still the cluster's object as opened (and moved by a rebase); "vs cluster" stays true. The count suffix moves to `is_checked(text)` so a stale count never shows |
| 38 | Side panel: a `Passed` preview whose `for_text != text` renders like `NotChecked` (`No changes checked yet`, no quota line); the `Checks` dry-run line keeps `Changed since the last check` | Same bug as the Diff tab, in the change list: after an undo it still listed `spec.replicas 3 → 5` |
| 39 | Delete `EditPreview::{before, after}`; `build_preview` loses its two `to_yaml_text` calls | Only `yaml_edit.rs::run_preview` read them (grep of `crates/app/src`, `crates/cluster/src`). If those were `build_preview`'s only fallible steps it returns `EditPreview` and `object_write.rs` drops the `map_err`; otherwise keep the `Result` |

## Secret and masking notes

- Both sides of the local diff are editor texts: masked (`<hidden>`), never a server value the editor does not already show. A value the user typed over `<hidden>` shows as typed (the old dry-run diff showed `<hidden, changed>`); it is already on screen in the editor, and nothing leaves the view. The side panel change list still reads `<hidden, changed>` / `<hidden, moved>` from the preview.
- No `tracing::`, no `Debug`, no disk write is added (0031 AC 13 stands).

## What of 0031 this supersedes

| 0031 text | Now |
|---|---|
| editor-view.md "Diff opens with its check", first bullet (O16) | Diff is local (31); Dry-run button (34) |
| editor-view.md Flows row "Ctrl S / Apply…, preview missing or stale … tab = Diff" | Apply does nothing without a passed check (35) |
| editor-view.md Flows row "Reload and keep my changes … rerun the preview" | no rerun (36) |
| editor-view.md Layout: Diff tab spinner and `Nothing to check: the text is unchanged`; footer Apply tooltips | [view-design.md](view-design.md) |
| AC 6 "The Diff tab shows the dry-run result" | the Diff tab shows the local diff; markers stay in the side panel |
| `PreviewState::Passed { rows }`, "As built" `PassedPreview` diff rows | `LocalDiff` (32), no rows in the preview (39) |
