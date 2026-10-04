# 0042 · App: New buttons, templates, and the create view

[Back to index](README.md) · Step 2 · Modules: `object_templates.rs` (new), `object_create_view.rs` (new) + `object_create_view_tests.rs`, `edit_yaml_flow.rs`, `write_flow.rs`, `resource_actions.rs`, `kind_access.rs`, `workspace.rs`, `leaving_work.rs`, `launch_options.rs`, `screenshot.rs`. Decisions 2, 3, 7, 8, 11, 12. Wireframes W7 (`top` buttons), W10 (checks, footer, confirm).

## Entry and gate

| Item | Rule |
|---|---|
| Action | `ResourceAction::CreateObject(ObjectKind)`; gate `ActionGate::Mutating { checks: [Create(kind)], is_shipped: true }`; risk `Change`; label `New {Kind}` |
| Lazy check | `kind_access::lazy_checks(kind)` appends `Create(kind)` when `kind.is_creatable()` (asked when the screen first shows, 0031 decision 24) |
| Header button | `render_header_actions` arms for `Screen::Kind(Namespaces | ConfigMaps | ResourceQuotas | PodDisruptionBudgets | RoleBindings)`: small button `New namespace` (Namespaces) or `New` (others); disabled with the gate reason as tooltip; click → `AppShell::open_create(kind)` |
| One edit at a time | `open_create` does nothing while `self.edit` is `Some` (as `open_edit`) |
| Connection, guard | the active session (0046); none → nothing opens |

## Templates (`object_templates.rs`, pure)

```rust
/// The starting text of a new object; `None` for a kind that is not creatable. `namespace` is
/// ignored for a Namespace.
pub(crate) fn template_text(kind: ObjectKind, namespace: &str) -> Option<String>;
```

Namespace = `scope.namespaces().first()` (`Named(ns)`, or the first of the sorted `Several`), else `default` (decision 8).

| Kind | Body (after `apiVersion`, `kind`, `metadata.name`, `metadata.namespace`) |
|---|---|
| Namespace | `name: new-namespace`, `labels: {}` |
| ConfigMap | `name: new-config`, `data: { KEY: value }` |
| ResourceQuota | `name: compute-quota`, `spec.hard`: `requests.cpu: "4"`, `requests.memory: 8Gi`, `limits.cpu: "8"`, `limits.memory: 16Gi`, `pods: "20"` |
| PodDisruptionBudget | `name: new-pdb`, `spec.minAvailable: 1`, `spec.selector.matchLabels.app: my-app` |
| RoleBinding | `name: new-binding`, `roleRef: { apiGroup: rbac.authorization.k8s.io, kind: ClusterRole, name: view }`, `subjects: [{ kind: ServiceAccount, name: default, namespace: {ns} }]` |

Each template passes `ObjectDraft::new` (test `every_template_is_a_valid_draft`).

## View (`ObjectCreateView`, in `OpenEdit::Create(Entity<ObjectCreateView>)`)

```rust
pub(crate) struct ObjectCreateView {
    shell: WeakEntity<AppShell>, cluster: ClusterRef, cluster_name: SharedString, kind: ObjectKind,
    template: SharedString,             // dirty = editor text != template
    editor: Entity<EditorState>,        // yaml, line numbers, search (as YamlEditView)
    check: CreateCheck, focus_handle: FocusHandle, _subscription: Subscription,
}
enum CreateCheck { NotChecked, Running { _task: Task<()> },
    Passed { for_text: SharedString, request: WriteRequest, warnings: Vec<SharedString>, elapsed: Duration },
    Failed(CreateFailure) }
enum CreateFailure { Local(SharedString), Invalid { message: SharedString, fields: Vec<SharedString> }, Server(SharedString) }
```

No `Debug`, no `tracing::`, no disk write. Key context `YAML_EDIT` (so Ctrl S is `ApplyEdit` and the workspace keys stay off).

| Region | Content |
|---|---|
| Header | kind badge, `New {Kind}`, muted cluster name; right `Format` (0031 `format_yaml`) |
| Body | editor (left), side panel 280 px (right): `Checks`: the dry-run line, each warning (warning tone), or `The object is invalid` + fields verbatim (danger) |
| Footer | `Not checked yet` / `Server dry-run…` / `Dry-run OK · {ms} ms` / `Changed since the last check` / the failure; right `Cancel`, `Create…` primary with `Kbd` Ctrl S |

## Flow

| Trigger | Behavior |
|---|---|
| Ctrl S / `Create…`, no `Passed` for this text | `ObjectDraft::new` → `Err` → `Failed(Local(error text))`. Ok → `WriteRequest::new(draft.target(), CreateObject(draft))` (`None` → `Local("this object cannot be created here")`) → `begin_preview` + `checked_write(DryRun)` → `Running` |
| Ctrl S while `Running` | nothing |
| Dry-run Ok | `Passed` with the draft warning texts (decision 7) and one `{path} is not a known field; the server dropped it` per `outcome.dropped_fields` (decision 14) |
| Dry-run Err | `Invalid` (including the mapped 409 and 404, decision 6) → `Failed(Invalid)`; others → `Failed(Server(error text))` |
| Ctrl S / `Create…`, `Passed` for this text | `start_write(WriteIntent { action: CreateObject(kind), label: "Create {Kind} {ns/}{name}", button: "Create", request, risk, expected_name, warnings })`. `risk` / `expected_name`: `ActionRisk::Privileged` and `Some(binding name)` when any draft warning `needs_typed_name()` (a typed binding name on every environment, decision 15); else `ActionRisk::Change` and `None` (the cluster's tier) |
| Commit Ok | the dialog's success notice comes from a new `CreateObject` arm in `commit_write`'s notice match (next to `EditValues`): `Created {Kind} {ns/}{name}`. `create_commit_finished` pushes nothing: it closes the view and calls `show_screen(Screen::Kind(kind))`; the watch adds the row; no cursor move (decision 11) |
| Commit `Invalid` / `Denied` | the dialog closes; the view shows the failure; Apply needs a new dry-run |
| Commit `OutcomeUnknown` | the view stays; footer `No answer in time; the object may have been created. Check the list before trying again.`; no Retry (decision 12) |
| Cancel, screen or namespace change | dirty → `Discard the new {Kind}?` (`Discard` danger / `Keep editing`, `FreshEnter`); clean → close |
| Cluster switch | `leaving_work` line `Unsaved new {Kind}` |

## Audit and confirm

- `audit_action` returns `Create` for `CreateObject(_)` (from `intent.button`). Fields: `changed_fields()`; ConfigMap values are never recorded (`PATH_ONLY_KINDS`; the field values are `None` anyway).
- The confirm dialog renders `changed_fields()` (existing) and `intent.warnings`; the object row reads `{Kind} {ns/}{name} · new`. RoleBinding subjects show one per line `{Kind} {ns/}{name}`; subject and ConfigMap key lists stop at 10 with `… and {n} more` (decision 16, built in `changed_fields`).
- `commit_write`: `CreateObject(_)` joins `EditYaml(_) | EditValues(_)` in the Retry exclusion, so the dialog never offers Retry (decision 12).

## Screenshot

`--screen new-config-map` (screenshot feature): the view with the ConfigMap template for `payments`, `Passed { 212 ms }`, no request; opened from `open_pending_dialog` like `edit-yaml-diff`.

## Shared edit slot (`OpenEdit`)

| Method or caller | Change |
|---|---|
| `OpenEdit::object(cx)` | returns `Option<&ObjectRef>`; `Create` → `None`. `object_delete.rs` compares `edit.object(cx) == Some(object)` (a create is never the deleted object) |
| `OpenEdit::discard_title(cx)` (new) | `Yaml` / `Values`: `Discard changes to {name}?` (today's text in `edit_yaml_flow.rs`); `Create`: `Discard the new {Kind}?` |
| `OpenEdit::leaving_line(cx)` (new) | `Yaml` / `Values`: `Unsaved changes to {subject}`; `Create`: `Unsaved new {Kind}`. `LeavingWork.unsaved_edit` holds this whole line; `leaving_work.rs` pushes it as is |
| `cluster`, `is_dirty`, `subject_text`, `commit_failed` | a `Create` arm each (`subject_text`: `new {Kind}`; `commit_failed`: the view's failure state) |
