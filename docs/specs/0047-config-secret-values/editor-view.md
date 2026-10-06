# 0047 · Editor view and flows (step 2)

[Back to index](README.md) · App crate. Decisions 3, 5, 6, 8–16. New: `values_edit.rs` (+ tests), `values_edit_flow.rs` (child of `app_shell`). Changed: `resource_actions.rs`, `resource_kind.rs`, `kind_access.rs`, `edit_yaml_flow.rs`, `app_shell.rs`, `keymap.rs`, `launch_options.rs`.

## Entry and gate

| Item | Rule |
|---|---|
| Action | `ResourceAction::EditValues(ObjectKind)`, `RowAction::EditValues`; label `Edit values…`; risk `Change` |
| Key (decision 9) | E on the ConfigMaps and Secrets screens: key action `EditValues`, bound in `WORKSPACE && ValuesScreen`; `EditYaml` moves to `WORKSPACE && !ValuesScreen`. `AppShell` adds `ValuesScreen` to its key context while one of the two screens is visible. Menu hint `E` on `Edit values…`; `Edit YAML` on these kinds has no hint |
| Offered on | ConfigMaps and Secrets rows: drawer ⋯ menu, row context menu, palette. Never on Helm release rows (explicit in `subject_action`, like 0033 decision 26) |
| Gate | `ActionGate::Mutating { checks: [Patch(kind)], is_shipped: true }`; lock of the active cluster |
| Row disable (from the summary) | `Helm release records cannot be edited` (type `helm.sh/release.v1`, or label `owner=helm` on either kind) · `Service account tokens are managed by Kubernetes` · `Immutable {kind}` |
| Removed | Secrets `KindAction::named("Edit")` placeholder |

The summary check is for the menu only; `values_base` refuses again from the server's answer (fail-closed, decision 4).

## Opening (`values_edit_flow.rs`)

`open_values_edit(subject)`: re-read the gate; one edit at a time (`self.edit.is_some()` → return); create `ValuesEditView` with the active connection; the view spawns `values_base` on the cluster runtime (`cx.spawn`). A `ValuesBaseError` shows in the view (`Cannot edit: {reason}`) with Close only.

`AppShell.edit: Option<OpenEdit>`; `enum OpenEdit { Yaml(Entity<YamlEditView>), Values(Entity<ValuesEditView>) }` with `cluster()`, `is_dirty()`, `subject_text()` (`Secret/payments/api-db-credentials`). `cancel_edit`, `close_edit`, `close_edit_of`, `unsaved_edit_of`, `parks_for_discard`, `ask_discard`, `is_editing` take either arm.

## Layout (workspace, replaces the table like Edit YAML)

- Title `Edit values · {kind} {ns}/{name}`, cluster badge, type chip; decision 14 warnings as kit `Alert` lines.
- One row per key, sorted: name · state chip (`added` / `changed` / `removed`, theme tokens) · value cell · row buttons.
- `+ Add key`: a name input and a value field (for Secrets the same masked textarea field as below); `Add` validates the name locally.
- Footer: `{n} changes` · `Cancel` · `Apply…` (Ctrl S, `ApplyEdit` in key context `ValuesEdit`).

| Key content | Value cell | Row buttons |
|---|---|---|
| ConfigMap `Text` | kit textarea, `auto_grow(1, 12)`, the current text | Remove / Undo |
| Secret `Hidden` | one `TextareaState`, empty = keep. Masked: a placeholder `•••• N chars` (`unchanged` when empty) is rendered instead of the textarea | eye (own toggle), `Paste` (while masked), Remove / Undo |
| `Binary` | `binary, N bytes` | Remove / Undo |
| `TooLarge` | `text, N KiB · edit with Edit YAML` | Remove / Undo |

- An empty Secret field means "keep"; any text makes a `Set`, even one equal to the current value (decision 16). The editor never fetches or shows a current Secret value (decision 3).
- Mask (decision 11): the kit mask and `Input::mask_toggle()` are never used. The textarea entity lives as long as the row; masking only swaps the rendered element for the placeholder, so the text never moves between widgets and newlines survive.
- Unmask: the editor's eye button, per field; its own 1 s ticker re-masks after `REVEAL_DURATION`, and Apply and close re-mask too. Typing needs an unmasked field; `Paste` inserts the clipboard text into the hidden textarea without showing it.
- Dirtiness: `InputEvent::Change` plus `text().len()`; never `value()` on a Secret field (decision 15).
- Secret fields capture `Copy` and `Cut` and drop them (decision 10). There is no Copy button: the drawer's Copy value (0016) does that.
- Screenshot runs (`value_access` = `Blocked`): the eye is disabled with `Disabled in screenshot runs`.
- While the view is open the table keys are inert: the keymap `WORKSPACE` context adds `!ValuesEdit`, and the shell's early returns test `is_editing()`.

## Apply

1. The view copies each changed field once, from the rope chunks into `Zeroizing::new(String::with_capacity(len))`, wraps it in a `NewValue`, builds `Vec<KeyChange>`, calls `base.edit(..)`. A `ValuesEditError` shows under its key; nothing is sent.
2. `WriteIntent { action: EditValues(kind), label: "Edit values of {kind} {name}", button: "Apply changes", request: WriteRequest::new(target, SetDataValues(Box::new(edit))), risk: Change, expected_name: None, warnings }`; `warnings` = decision 14 lines + decision 12 lines.
3. `start_write(intent)`: the confirm dialog runs the dry-run; its change list is `changed_fields()` (`data[DB_PASSWORD] value changed`); the tier decides click or typed name.
4. Commit → audit (`Edit values`, paths only) → notice `Updated {n} keys of {kind} {name}` (count and object name only).
5. `values_commit_finished` (called from `write_flow.rs` next to `edit_commit_finished`): success closes the editor; `Conflict` → banner; `Invalid` → field paths under the matching keys and in the footer; `NotFound` → banner `The object was deleted. Your changes cannot be applied.`; others → footer text.

## Conflict (decision 12)

Banner `The object changed since you opened it.` with `Reload and keep my changes` and `Discard`. Reload spawns `values_base`, re-applies the pending changes by key, lists dropped ones in the banner (`DB_USER: removed on the server, your change was dropped`), and keeps the next Apply's warnings.

A 409 on the dry-run (the object changed between opening the editor and Apply) is the same conflict: the confirm dialog closes (no Retry, the base `resourceVersion` is stale) and the editor shows this banner (`values_commit_finished`, called from the dialog's dry-run task). Test `a_conflict_on_the_dry_run_closes_the_dialog_and_shows_the_banner`.

## Leaving and switching

- Cancel, Esc in the footer, another screen, a reveal, a namespace change: the 0031 discard prompt when dirty, else close.
- Cluster switch (0046): `leaving_work.unsaved_edit` lists the edit; after confirm the shell closes it (`close_edit_of`). Dropping the entity drops every buffer.
- Lock toggled on while open: Apply's gate shows the lock reason; the text stays.

## Fixture

`--screen values-edit` (screenshot builds): a Secret `payments/api-db-credentials` with keys `DB_HOST`, `DB_USER`, `DB_PASSWORD` (Hidden), `ca.der` (Binary), a pending `Add` of `DB_PORT` (masked) and a `Remove` of `DB_USER`. No cluster; all values fixture text, all masked.
