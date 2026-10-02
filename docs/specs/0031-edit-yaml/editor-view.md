# 0031 · App: the editor view and flows

[Back to index](README.md) · Step 3 (view, local checks, Format; no server write) and step 4 (shipped, preview, apply, conflicts, 422) · Modules: `yaml_edit.rs` (new) + `yaml_edit_tests.rs`, `yaml_diff.rs` (new, pure), `write_flow.rs`, `confirm_dialog.rs`, `resource_actions.rs`, `app_shell.rs`, `workspace.rs`, `keymap.rs`, `launch_options.rs`, `screenshot.rs`. Decisions 9, 14–22, 26. Wireframe W10.

## Entity

```rust
/// The open edit, in `AppShell.edit`; it replaces the table and drawer in the workspace.
pub(crate) struct YamlEditView {
    target: ClusterObject,            // row's cluster + key (0027); the connection is resolved per request via guard_for
    object: ObjectRef,
    base: Option<EditBase>,           // None while loading
    editor: Entity<EditorState>,      // language "yaml", line numbers, search, editable
    tab: EditTab,                     // Editor | Diff
    is_dirty: bool,                   // editor value != base.text()
    load: LoadState,                  // Loading { _task } | Ready | Failed(SharedString)
    preview: PreviewState,
    banner: Option<EditBanner>,       // Conflict { unreachable: Vec<FieldPath> } | Deleted | OutcomeUnknown
    server_changed: Vec<FieldPath>,   // from the last rebase; shown in the side panel
}
enum PreviewState { NotChecked, Running { _task: Task<()> }, Passed { for_text: SharedString, preview: EditPreview,
    rows: Vec<DiffRow>, elapsed: Duration }, Failed(PreviewFailure) }
enum PreviewFailure { Local(EditError), Invalid { message: SharedString, fields: Vec<SharedString> }, Server(SharedString) }
```

- There is no `Debug`, no `tracing::`, and no disk write. `InputEvent::Change` updates `is_dirty`. A `Passed` preview whose `for_text` differs from the current text is stale.
- `EditPreview` stays in this view: it is never passed to the audit or a notification (decision 9).

## Layout (W10)

| Region | Content |
|---|---|
| Header | kind short name, `{namespace}/{name}` mono, muted `resourceVersion {rv}` and the cluster name; right: `Env values` (disabled while dirty, tooltip `Discard your changes to show env values`), `Format` |
| Tabs | kit `TabBar`: `Editor`, `Diff vs cluster` (`· {n}` after a passed preview) |
| Editor tab | `Editor::new(&editor).bordered(false).text_xs()` |
| Diff tab | `uniform_list` of `DiffRow`: old and new line numbers, sign, mono text. Removed rows use the danger token tint, added rows the success token, folded rows muted `··· {n} unchanged lines`. Otherwise a spinner (`Running`), the failure, or `Press Ctrl S to check the change with the server` |
| Side panel (280 px) | `{n} changes` (path mono with ellipsis and tooltip, `old → new`, `—` when absent, `and {more} more`). `Checks`: the dry-run line, then each `EditCheck` text (edit-preview.md, warning tone). `Changed on the server since you opened it`: the `server_changed` paths. `The change is invalid` + each field **verbatim** (danger) |
| Banner | Conflict: `The object changed since you opened it.` + `Reload and keep my changes` / `Discard my changes`. After a rebase with `unreachable`: one line `{path}: no longer exists on the server` each. Deleted / OutcomeUnknown: as in [write-path.md](write-path.md) |
| Footer | `Not checked yet`, `Server dry-run…`, `Dry-run OK · {ms} ms · unchanged since you opened it`, `Changed since the last check`, or the failure. Right: `Cancel`, `Apply…` primary with `Kbd` Ctrl S (disabled while clean: `No changes`; while `Running`: `Waiting for the dry-run…`) |

Theme tokens only. There is no Revision history tab, snapshot line, or managedFields toggle.

## `yaml_diff.rs` (pure, step 3)

```rust
pub(crate) enum DiffRowKind { Same, Removed, Added, Folded { lines: usize } }
pub(crate) struct DiffRow { pub(crate) kind: DiffRowKind, pub(crate) old_line: Option<usize>, pub(crate) new_line: Option<usize>, pub(crate) text: SharedString }
pub(crate) fn diff_rows(before: &str, after: &str) -> Vec<DiffRow>; // similar::TextDiff::from_lines, grouped_ops(3); one Folded per gap
```

Runs on `cx.background_executor()`. Both inputs are masked and have no header.

## Flows

| Trigger | Step | Behavior |
|---|---|---|
| Open: menu `Edit YAML`, `Edit` `KindAction`, E, palette | 3 wired, 4 enabled | `AppShell::open_edit(ClusterObject)`: `action_availability(EditYaml, guard)` must be `Enabled`; it reads `Comes in a later version` until step 4. Another dirty edit → discard prompt first. Then `edit_base(object, Hidden)`, `set_value`, focus |
| Env values / Format | 3 | env refetch only while clean; `format_yaml` → `set_value` or `Syntax` in the footer |
| Ctrl S / `Apply…`, preview missing or stale | 3: local only; 4: + server | `ObjectEdit::new`: `Err` → `Failed(Local)`, stay on the tab. Step 3 ends here with `Local checks passed`. Step 4: `WriteRequest::new`, `preview_write` → `Running`, tab = Diff |
| Ctrl S / `Apply…` while `Running` | 3–4 | nothing (decision 15) |
| Ctrl S / `Apply…`, `Passed` for this text | 4 | `run_guarded(GuardedIntent { action: EditYaml, label: "Apply changes", risk: Change, expected_name: None, kind: Write(request), warnings: check texts, on_commit: Some(..) })` |
| Preview `Conflict` / `NotFound` / `OutcomeUnknown` | 4 | banner; `Invalid` → `Failed(Invalid)`; else `Failed(Server(redacted))` |
| `on_commit(Ok)` | 4 | close the editor; 0030 notice `Apply changes: done` |
| `on_commit(Err(Conflict))` / `Invalid` | 4 | the dialog is closed; banner / side panel. Other errors: 0030 notification, the editor stays |
| `Reload and keep my changes` | 4 | `edit_base` again; `rebase(old, text, new)` → `set_value(rebased.text)`, `base = new`, `server_changed`, banner `unreachable` lines; rerun the preview |
| `Discard my changes` / `Reload` | 4 | `edit_base` again → `set_value(base.text())` |
| Cancel or navigating away (screen, reveal, palette Go to, namespace or cluster switch) | 3 | when dirty: `Discard changes to {name}?` with `Discard` (danger) / `Keep editing` |

## Write-flow additions (0031, step 4)

```rust
/// Dry-run only: gate (EditYaml enabled), then 0032 `checked_write` with `WriteMode::DryRun` (no audit).
pub(crate) fn preview_write(&self, cluster: &ClusterRef, request: WriteRequest, cx: &mut Context<AppShell>)
    -> Task<Result<WriteOutcome, WriteError>>;
pub(crate) struct GuardedIntent { /* 0030 + warnings */ pub(crate) on_commit: Option<CommitCallback> }
pub(crate) type CommitCallback = Box<dyn FnOnce(&Result<WriteOutcome, WriteError>, &mut Window, &mut App)>;
```

- `on_commit` runs after the audit append. When it is set, the dialog closes after the commit whatever the outcome (no 0030 `Retry`: a stale `resourceVersion` cannot pass).
- The dialog dry-run line for `ReplaceObject` adds `· unchanged since you opened it`. The object row reads `{namespace}/{name} · {n} fields changed`; change lines show paths only.
- `ClusterConnection::write` stays reachable only through `checked_write` (0032).

## Keys, menus, screenshot

- `keymap.rs`: `secondary-s` → `ApplyEdit` in `YamlEdit` (step 3). 0028 single-letter keys stay inactive in the editor.
- `ResourceAction::EditYaml`: gate `Update(kind)` (lazy), `mutates: true`, **shipped in step 4**. Pod menu `Edit YAML` (E) after Port-forward (W4); kind menus `Edit YAML` on editable kinds without an `Edit` `KindAction`.
- `--screen edit-yaml-diff` (step 3, screenshot feature): `YamlEditView::fixture(..)` with W10's diff (`spec.replicas` 3 → 5, `…limits.memory` 512Mi → 1Gi), `Rollout { RollingUpdate }`, `Passed { 412 ms }`, no base (Apply disabled), no connection call.
