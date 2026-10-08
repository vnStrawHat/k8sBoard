# 0059 · View design (`yaml_edit.rs`, `yaml_edit_panels.rs`)

[Back to index](README.md)

## State

```rust
/// The Diff tab: the editor text against the opened object, both in serializer form. Local only.
enum LocalDiff {
    /// Not computed since the text or the base last changed (the Diff tab was hidden).
    Idle,
    Running { for_text: SharedString, _task: Task<()> },
    /// `Ok(rows)`, empty when nothing changed; `Err` holds the `EditError` text.
    Ready { for_text: SharedString, rows: Result<Vec<DiffRow>, SharedString> },
}
// YamlEditView: + `local_diff: LocalDiff` (starts `Idle`); `diff_list: ListState` stays and is
// reset when a `Ready(Ok(rows))` arrives. `PassedPreview.rows` is deleted.

/// Pure; tested directly. `base_text` is already serializer output.
fn local_diff_rows(base_text: &str, text: &str) -> Result<Vec<DiffRow>, EditError>;
/// `text` after its leading `#` lines (the edit header).
fn without_header(text: &str) -> &str;
```

`refresh_local_diff(&mut self, cx: &mut Context<Self>)`: returns when `base` is `None`; else clones `base.text()` and the current text, sets `Running { for_text, _task }`, and spawns `local_diff_rows` on `cx.background_executor()`. The task writes `Ready` only if `for_text` still equals the view's text, resets `diff_list`, and notifies. `refresh_dirty` takes `&mut Context<Self>` and calls it when `tab == Diff`; on another tab it sets `Idle` (dropping a running task).

## Buttons

| Reason (first match) | `Dry-run` | `Apply…` |
|---|---|---|
| `base` is `None` (loading, fixture) | disabled, `Reading the object…` | disabled (as today) |
| `preview` is `Running` (a check or a reload) | `Dry-run running…` | `Waiting for the dry-run…` |
| `!is_dirty` | `No changes` | `No changes` |
| Secret value refused | `SECRET_VALUES_REASON` | `SECRET_VALUES_REASON` |
| banner `Recreated` | `The object was deleted and created again` | same |
| not `is_checked(text)` | enabled: `Ask the server to check the change; nothing is saved` | `Run the dry-run first` |
| checked | enabled (asks again) | enabled: `Apply the checked change` |

- `fn dry_run_block_reason(&self) -> Option<&'static str>` (rows 1–5) replaces `can_check`. `fn apply_block_reason(&self, text: &str) -> Option<&'static str>`: the Running text, else `dry_run_block_reason()`, else `Run the dry-run first` unless `is_checked(text)`.
- The fixture keeps `is_fixture`: both buttons disabled (no base).
- `pub(crate) fn dry_run(&mut self, cx: &mut Context<Self>)`: returns on a block reason, else `run_preview(text, cx)`. `run_preview` no longer sets `self.tab`.
- `apply_press`: returns unless `apply_block_reason(&text).is_none()`; then `confirm` on `KeyPress::Fresh` only.

## Render

| Region | Rule |
|---|---|
| Diff tab | `Ready { for_text == text, Ok(rows) }`: empty → `muted_center("No changes")`, else the `list` of rows. `Ready { for_text == text, Err(message) }` → `muted_center(message, danger)`. Anything else → `busy("Comparing…")`. Never reads `preview` |
| Tab title | `Diff vs cluster · {n}` when `is_checked(text)` (n = `passed.changes.len()`), else `Diff vs cluster` |
| Side panel | change list and quota line only when `is_checked(text)`; otherwise `No changes checked yet`. The `Checks` line stays `dry_run_line` |
| Footer | status text as today (`footer_text`); right: `Cancel`, `Dry-run`, `Apply…` + `Kbd` Ctrl S |

## Flows (replace the matching rows of 0031 editor-view.md)

| Trigger | Behavior |
|---|---|
| Show the Diff tab | `tab = Diff`, `refresh_local_diff`; no request |
| Text change (typing, undo, Format, reload, rebase, Discard) on the Diff tab | `refresh_dirty` → `refresh_local_diff`; old rows never render (guard) |
| Text change on another tab | `local_diff = Idle` |
| `Dry-run`, enabled | `run_preview`: local `ObjectEdit::new` (error → `Failed(Local)` + error mark), else one dry-run `PUT`; footer `Server dry-run…`; tab unchanged |
| `Apply…` / fresh Ctrl S, checked | `confirm` → `start_write` (unchanged) |
| `Apply…` / Ctrl S not checked, or a held Ctrl S | nothing |
| `Reload and keep my changes` | rebase as today, then `preview = NotChecked`; no request after the GET |
| A dry-run passes | `PreviewState::Passed` without rows; tab title and side panel follow |
