# 0047 · Decisions

[Back to index](README.md)

## 1. Merge patch with the base `resourceVersion`, not a `PUT` replace

| | JSON merge patch + base `resourceVersion` (chosen) | `PUT` replace (0031 `ReplaceObject`) |
|---|---|---|
| Concurrency (C8) | same guarantee: the server compares `metadata.resourceVersion` and answers 409 | 409 on a stale `resourceVersion` |
| Plaintext in flight | only the changed keys' new values | every value of the Secret: a fresh GET of all keys at each dry-run and commit, and all of them in the body |
| Needs old values | no | yes (0031 restores them from the fresh GET) |
| Refusal check | the base read refuses; the version precondition proves the server object is the one checked (decision 4) | fresh GET each time |
| C8 wording | "JSON merge patch for single-field actions": a key edit changes one map, field by field | "YAML edits replace" |
| Precedent | 0034 `SetNodeTaints` (merge patch guarded by `resourceVersion`) | 0031 |

Smaller leak surface and no full-Secret read per request decide it. No SSA and no Force, as C8 says. 0031 stays unchanged: Edit YAML still refuses Secret `data`/`stringData` changes (0031 AC 4).

## 2. `data` only; text values are base64-encoded locally

`stringData` is a write-only alias: the server merges it into `data` and never returns it. The editor reads and writes `data` only, so the dry-run, the patch, and the next read use one form. A user types plain text (the `stringData` experience); the cluster crate encodes it. ConfigMap `data` is sent as typed.

## 3. Secret values are write-only in the editor (security review, coordinator 2026-10-03)

The base read keeps key names, sizes, `is_binary`, type, and flags; the data is zeroized before `values_base` returns. The editor never fetches or shows a current Secret value. The user types or pastes a new value, or leaves the field empty to keep the current one. Viewing and copying a current value stays in the W7 drawer (0016 Reveal and Copy value). There is no "Load current value" and no Copy in the editor.

## 4. An edit exists only from a server-read base

`ValuesBase` has private fields and comes only from `ClusterConnection::values_base`, which refuses Helm release records (a Secret of type `helm.sh/release.v1`, and any ConfigMap or Secret labelled `owner=helm`: Helm's ConfigMap storage driver), service-account-token Secrets, and immutable objects. `ValuesEdit` comes only from `ValuesBase::edit`. Both carry the base `resourceVersion`; a server object of that version is the one that was checked, so no second GET is needed (fail-closed).

## 5. Binary values: remove only

A non-UTF-8 Secret value or a ConfigMap `binaryData` key reads `binary, N bytes`. No replace (no file picker), no text view.

## 6. Change markers live in the path text

`changed_fields()` returns `data[KEY] added`, `data[KEY] value changed`, `data[KEY] removed`, `value: None`. The confirm dialog and the audit already show a path with no value, and `recordable_fields` keeps dropping values for both kinds. No audit or dialog code changes.

## 7. Lazy `AccessCheck::Patch(ObjectKind)`

A merge patch needs RBAC `patch`, not `update`. `kind_access::lazy_checks` adds it for ConfigMap and Secret only; `fitting_access_check` accepts `SetDataValues` on exactly these two kinds.

## 8. One edit slot shared with Edit YAML

`AppShell.edit` becomes `Option<OpenEdit>` with `Yaml(Entity<YamlEditView>)` and `Values(Entity<ValuesEditView>)`. One edit at a time; the discard prompt, `leaving_work.unsaved_edit`, inert table keys, and `close_edit_of` work for both through `OpenEdit::{cluster, is_dirty, subject_text}`.

## 9. E opens Edit values on ConfigMaps and Secrets (amends 0031 decision 25)

Coordinator decision 2026-10-03: follow W7 (`Edit  E` on both drawers). Amends 0031 decision 25 / README open item 5 for these two kinds only.

| Screen | E | `Edit values…` item | `Edit YAML` item |
|---|---|---|---|
| ConfigMaps, Secrets | Edit values | menu + palette, hint `E` | menu + palette, no hint |
| Releases (Helm) | nothing (`NotOffered`, as today) | not offered | not offered |
| every other editable kind | Edit YAML (unchanged) | not offered | hint `E` |

- Two key actions, `EditYaml` (existing) and `EditValues` (new), both bound to `e`, split by a key-context identifier. The `AppShell` root adds `ValuesScreen` to its key context while the visible screen is ConfigMaps or Secrets.
- `keymap.rs`: `e` → `EditYaml` in `WORKSPACE && !ValuesScreen`; `e` → `EditValues` in `WORKSPACE && ValuesScreen`. The kit's hint lookup follows the same predicates, so the menu shows `E` only on the item that E runs.
- `RowAction::EditValues.key_action()` is `EditValues`; `keyboard_navigation.rs` adds `on_row_key::<EditValues>(root, RowAction::EditValues, cx)`. Both actions go through `run_row_key` → `run_available_row_key`, the one entry for key, menu, and palette.
- The palette and menu dispatch either action directly (no binding needed), so Edit YAML still works on these kinds.
- `subject_action(RowAction::EditValues, key)` resolves only ConfigMaps and Secrets kind rows. Anything else is `NotOffered`, so E can never open the wrong editor.
- The shortcut rows in `keymap.rs` (0028 sheet) add `row(SelectedResource, "Edit values (ConfigMaps, Secrets)", EditValues)`; the Edit YAML row reads `"Edit YAML (other kinds)"`.
- The Secrets placeholder `KindAction::named("Edit")` is removed.

## 10. Clipboard inside Secret fields

A kit textarea copies to the normal clipboard (history, cloud). Secret fields capture `Copy` and `Cut` and drop them; the editor has no Copy button (decision 3). Paste is allowed. ConfigMap fields copy normally.

## 11. One textarea per Secret field; the editor's own mask

A single-line kit `InputState` strips `\n` and `\r` on every insert and paste (gpui-base 0.7.0 `state.rs:2733`, `:3498-3499`), and the kit mask is single-line only. So:

- Each Secret value field is one `TextareaState` (`auto_grow(1, 8)`). The kit mask (`masked`, `set_masked`) and `Input::mask_toggle()` are never used on it.
- While masked, the view renders a placeholder `•••• N chars` (`N` = `text().chars().count()`; `unchanged` when empty) instead of the textarea element. The `TextareaState` entity stays alive, so the text never moves between widgets.
- The editor's own eye button unmasks one field; its own timer re-masks after `REVEAL_DURATION` (30 s, 0016), on Apply, and on close. A paste goes into the textarea; a field that is masked shows the eye and a `Paste` button only (paste writes into the hidden textarea).

## 12. 409: reload and keep changes by key

"Reload and keep my changes" reads a new base and re-applies each change by key. A `Set`/`Remove` whose key is gone and an `Add` whose key now exists are dropped and listed. Each kept change to a key present on the server adds a confirm warning `{key}: the object changed on the server since you opened it` (values cannot be compared).

## 13. Size limits

A value over 1 MiB is refused locally (`TooLarge`). So is an edit whose estimated object size exceeds 1 MiB (`ObjectTooLarge`): the base's key sizes, minus removed and replaced keys, plus the new values (base64 length for Secrets). This is an early hint only; the API server's limit decides. ConfigMap values over 128 KiB (`MAX_INLINE_VALUE`) are not put in an input (main-thread layout); they read `text, N KiB, edit with Edit YAML` and can only be removed.

## 14. Warnings (non-blocking)

- `app.kubernetes.io/managed-by: Helm` label: `Managed by Helm: the next upgrade replaces this change`.
- `ownerReferences` not empty: `Owned by {kind}/{name}: its controller may replace this change`.
- Always: `Pods that read these keys as environment variables keep the old values until they restart`.

## 15. `zeroize` covers only k8sBoard's own short-lived copies

- Dirtiness comes from `InputEvent::Change` plus `text().len()` (the borrowed `&Rope`). The view never calls `value()` (an `Arc<str>` copy) on a Secret field, per render or otherwise.
- At Apply the view copies each changed field once, from the rope's chunks into `String::with_capacity(len)` wrapped in `Zeroizing`, and builds the `NewValue` from it.
- Everything else is outside zeroize: the kit `Rope` and undo history, any `value()` copy the kit makes itself, `text_for_range` answers to the OS IME, grown or reallocated buffers, GPUI text caches, serde/hyper buffers. See the table in [secret-safety.md](secret-safety.md).

## 16. Empty keeps; an identical value is still a change

An empty Secret field means "keep", so a value cannot be set to `""` here (Edit YAML cannot either: Secret data is locked there; use `kubectl`). The editor cannot compare with a value it never loads, so a typed value equal to the current one is sent and audited as `value changed`.
