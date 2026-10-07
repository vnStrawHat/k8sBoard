# 0031 · App: the editor view and flows

[Back to index](README.md) · Step 3 (entry, view, local checks, Format; no server write) and step 4 (shipped, preview, apply, conflicts, 422) · Modules: `yaml_edit.rs` (new) + `yaml_edit_tests.rs`, `yaml_diff.rs` (new, pure), `write_flow.rs` and `confirm_dialog.rs` (0030 step 4), `resource_actions.rs`, `resource_kind.rs`, `keyboard_navigation.rs`, `app_shell.rs`, `app_shell_view.rs`, `workspace.rs`, `keymap.rs`, `launch_options.rs`, `screenshot.rs`. Decisions 9, 14–22, 26, 28–30. Wireframe W10.

## Entity

```rust
/// The open edit, in `AppShell.edit`; it replaces the table and drawer in the workspace.
pub(crate) struct YamlEditView {
    target: ClusterObject,            // the cursor's cluster + key (0027); guard and connection resolved per request from this slot
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
| Header | the STG/PROD pill of the cluster (the one of the confirm dialogs), kind short name, `{namespace}/{name}` mono, muted `resourceVersion {rv}` and the cluster name; right: `Env values` (disabled while dirty, tooltip `Discard your changes to show env values`), `Format` |
| Tabs | kit `TabBar`: `Editor`, `Diff vs cluster` (`· {n}` after a passed preview) |
| Editor tab | `Editor::new(&editor).bordered(false).text_xs()` |
| Diff tab | gpui `list` (wrapping rows, `ListState` reset when the rows change) of `DiffRow`: old and new line numbers, sign, mono text; a long line wraps in its own column so continuation lines hang under the text, past the gutter. In a 1:1 `−`/`+` pair the changed span (after the common prefix and suffix) gets a stronger tint. Removed rows use the danger token tint, added rows the success token, folded rows muted `··· {n} unchanged lines`. Otherwise a spinner (`Running`), the failure, or `Nothing to check: the text is unchanged` |
| Side panel (280 px) | `{n} changes` (path mono with ellipsis and tooltip, `old → new`, `—` when absent, `and {more} more`). `Checks`: the dry-run line, then each `EditCheck` text (edit-preview.md, warning tone). `Changed on the server since you opened it`: the `server_changed` paths. `The change is invalid` + each field **verbatim** (danger) |
| Banner | Conflict: `The object changed since you opened it.` + `Reload and keep my changes` / `Discard my changes`. After a rebase with `unreachable`: one line `{path}: no longer exists on the server` each. Deleted / OutcomeUnknown: as in [write-path.md](write-path.md) |
| Footer | `Not checked yet`, `Server dry-run…`, `Dry-run OK · {ms} ms · unchanged since you opened it`, `Changed since the last check`, or the failure. Right: `Cancel`, `Apply…` primary with `Kbd` Ctrl S (disabled while clean: `No changes`; while `Running`: `Waiting for the dry-run…`) |

Theme tokens only. There is no Revision history tab, snapshot line, or managedFields toggle.

## `yaml_diff.rs` (pure, step 3)

```rust
pub(crate) enum DiffRowKind { Same, Removed, Added, Folded { lines: usize } }
pub(crate) struct DiffRow { pub(crate) kind: DiffRowKind, pub(crate) old_line: Option<usize>, pub(crate) new_line: Option<usize>, pub(crate) text: SharedString, pub(crate) changed: Option<Range<usize>> /* span of a 1:1 change */ }
pub(crate) fn diff_rows(before: &str, after: &str) -> Vec<DiffRow>; // similar::TextDiff::from_lines, grouped_ops(3); one Folded per gap
```

Runs on `cx.background_executor()`. Both inputs are masked and have no header.

## Flows

| Trigger | Step | Behavior |
|---|---|---|
| Open: menu `Edit YAML`, ConfigMaps `Edit`, E, palette | 3 wired, 4 enabled | all four dispatch the E key action; `run_row_key` gates on the cursor slot (`key_availability`), then the `EditYaml(_)` arm of `run_available_row_key` calls `AppShell::open_edit(subject: ClusterObject)`. Until step 4 the gate reads `Comes in a later version`. Another dirty edit → discard prompt first. Then `edit_base(object, Hidden)` on `slot_live(&subject.cluster)?.connection()`, `set_value`, focus |
| Env values / Format | 3 | env refetch only while clean; `format_yaml` → `set_value` or `Syntax` in the footer |
| Ctrl S / `Apply…`, preview missing or stale | 3: local only; 4: + server | `ObjectEdit::new`: `Err` → `Failed(Local)`, stay on the tab. Step 3 ends here with `Local checks passed`. Step 4: `WriteRequest::new`, `preview_write` → `Running`, tab = Diff |
| Ctrl S / `Apply…` while `Running` | 3–4 | nothing (decision 15) |
| Ctrl S / `Apply…`, `Passed` for this text | 4 | `run_guarded(GuardedIntent { action: EditYaml, label: "Apply changes", risk: Change, expected_name: None, kind: Write(request), warnings: check texts, on_commit: Some(..) })` |
| Preview `Conflict` / `NotFound` / `OutcomeUnknown` | 4 | banner; `Invalid` → `Failed(Invalid)`; else `Failed(Server(redacted))` |
| `on_commit(Ok)` | 4 | close the editor; 0030 notice `Apply changes: done` |
| `on_commit(Err(Conflict))` / `Invalid` | 4 | the dialog is closed; banner / side panel. Other errors: 0030 notification, the editor stays |
| `Reload and keep my changes` | 4 | `edit_base` again; `rebase(old, text, new)` → `set_value(rebased.text)`, `base = new`, `server_changed`, banner `unreachable` lines; rerun the preview |
| `Discard my changes` / `Reload` | 4 | `edit_base` again → `set_value(base.text())` |
| Cancel or navigating away (screen, reveal, palette Go to, namespace change) | 3 | when dirty: `Discard changes to {name}?` with `Discard` (danger) / `Keep editing` |
| Switch, view change, or `Remove from view` that releases the edited cluster | 3 | `leaving_work` (below) lists `Unsaved changes to {kind}/{name}`; confirm discards and releases; the slot release closes the editor |

## Write-flow additions (0031, step 4)

```rust
/// Dry-run only: gate (EditYaml enabled), then 0030 `checked_write` with `CommitMode::DryRun` (no audit).
pub(crate) fn preview_write(&self, cluster: &ClusterRef, request: WriteRequest, cx: &mut Context<AppShell>)
    -> Task<Result<WriteOutcome, CheckedWriteError>>;
pub(crate) struct GuardedIntent { /* 0030 + warnings */ pub(crate) on_commit: Option<CommitCallback> }
pub(crate) type CommitCallback = Box<dyn FnOnce(&Result<WriteOutcome, CheckedWriteError>, &mut Window, &mut App)>;
```

- `on_commit` runs after the audit append. When it is set, the dialog closes after the commit whatever the outcome (no 0030 `Retry`: a stale `resourceVersion` cannot pass).
- The dialog dry-run line for `ReplaceObject` adds `· unchanged since you opened it`. The object row reads `{namespace}/{name} · {n} fields changed`; change lines show paths only.
- `ClusterConnection::write` stays reachable only through `checked_write` (0030 decision 30). Signatures follow the merged 0030 step 4 code; where it differs from 0030 write-flow.md, follow the code and note the deviation.

## Entry, cursor, and release (0027/0028 baseline)

- `ResourceAction::EditYaml(ObjectKind)`: gate `Mutating { check: Update(kind) }` (lazy), **shipped in step 4**. Keys and palette name `RowAction::EditYaml`, resolved by `subject_action` from the subject's kind: Pod → `EditYaml(Pod)`; Node → not offered (non-goal); `Kind { kind }` via `kind.builtin_object()` when `is_editable`; Helm releases (their spec reads `Secret`) and custom kinds → not offered.
- Menus: `action_item(EditYaml(kind), guard)` with the E hint and **no `on_click`** (decision 29). Pod menu `Edit YAML` after Port-forward (W4); every editable kind menu `Edit YAML`, except ConfigMaps, whose W7 `Edit` `KindAction` becomes `KindAction::keyed("Edit", EditYaml(ConfigMap))`.
- The editor keeps `AppShell.selected` (the cursor) and the drawer flag; `workspace.rs` renders the editor in their place and restores them on close. While `AppShell.edit` is `Some`, `run_row_key` returns early: no row key acts on the hidden cursor.
- `leaving_work(leaving: &[ClusterRef], cx) -> Vec<SharedString>` (one function, owned by whichever of 0031 step 3 / 0036 step 4 lands first; the other adds its line): `switch_cluster`, `view_clusters`, and `remove_from_view` compute the clusters a call would release and, when the list is non-empty, show one kit `Dialog` (`Leave {cluster}?`, lines, `Leave` / `Stay`) before calling `release_all` / `release_slot`. Confirm re-enters with `ReleaseCheck::Confirmed`.
- `release_slot` / `release_all` close the editor of a released cluster (like `log_dock.close_tabs_of`).

## Keys, screenshot

- `keymap.rs`: `secondary-s` → `ApplyEdit` in context `YamlEdit` (step 3); `keymap_tests.rs` drops `secondary-s` from `RESERVED_KEYS`; one sheet row. The code editor is an `Input`, so 0028 `WORKSPACE` single keys stay inactive there.
- `--screen edit-yaml-diff` (step 3, screenshot feature): `YamlEditView::fixture(..)` with W10's diff (`spec.replicas` 3 → 5, `…limits.memory` 512Mi → 1Gi), `Rollout { RollingUpdate }`, `Passed { 412 ms }`, no base (Apply disabled), no connection call.

## Errors point at the line (UX fix H4)

`edit_error_line.rs` (pure): `local_error_line` gives the 1-based line of a local error (a syntax error's own line; a placeholder or identity error through its field path) and `line_of_field` looks a field path (`spec.template.spec.containers[0].image`) up in the text by walking keys and sequence indexes, falling back to the deepest part that exists; `[name]` selectors are not followed. `YamlEditView::sync_error_mark` tints that line in the editor (a `RangeDecoration` fill in the danger token; the editor has no gutter hook), and any edit clears it. The footer error and the 422 field rows of the side panel (`spec.replicas · line 13`) are clickable and call `go_to_line`, which opens the Editor tab and puts the cursor there.

## Diff opens with its check, and the confirm shows old and new (UX round 3, O16 and O17)

- Showing the **Diff** tab asks the server when the editor holds changes that no passed dry-run covers (`YamlEditView::show_tab`, the guard `can_check` that Ctrl S uses): no first Ctrl S is needed to see the diff. A text that already passed is not asked again. Ctrl S or `Apply…` for a text whose dry-run passed, from the Editor or the Diff, opens the confirm at once.
- The **confirm dialog** lists `path: old → new` for each changed scalar, from the masked preview (`edit_change_lines`: `spec.replicas: 3 → 5`, a hidden env value `<hidden> → <hidden, changed>`), the path alone for a map or a list and for the data of a Secret, at most 12 lines then `and N more`. The audit line keeps paths only. The Helm line (`HELM_MANAGED_WARNING`) comes first among its warnings, as in the Scale and Roll back confirms.
