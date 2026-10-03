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

## 3. No Secret value is fetched when the editor opens

The base read keeps key names, sizes, `is_binary`, type, and flags; the data is zeroized before `values_base` returns. Changing a value means typing a new one. "Load current value" fetches one key on explicit action via the 0016 `secret_values` GET; the other keys are dropped in the same call.

## 4. An edit exists only from a server-read base

`ValuesBase` has private fields and comes only from `ClusterConnection::values_base`, which refuses Helm release, service-account-token, and immutable objects. `ValuesEdit` comes only from `ValuesBase::edit`. Both carry the base `resourceVersion`; a server object of that version is the one that was checked, so no second GET is needed (fail-closed).

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

The kit's masked input already keeps its value off the clipboard; an unmasked one would not. Secret fields capture `Copy` and `Cut` and drop them; the row's Copy button uses the 0016 private write and the 30 s clear. Paste is allowed.

## 11. Unmask is per field and timed

The field's eye toggles the kit mask (`set_masked`). An unmasked field re-masks after `REVEAL_DURATION` (30 s, 0016), on Apply, and on close.

## 12. 409: reload and keep changes by key

"Reload and keep my changes" reads a new base and re-applies each change by key. A `Set`/`Remove` whose key is gone and an `Add` whose key now exists are dropped and listed. Each kept change to a key present on the server adds a confirm warning `{key}: the object changed on the server since you opened it` (values cannot be compared).

## 13. Size limits

A value over 1 MiB is refused locally (`TooLarge`). ConfigMap values over 128 KiB (`MAX_INLINE_VALUE`) are not put in an input (main-thread layout); they read `text, N KiB, edit with Edit YAML` and can only be removed.

## 14. Warnings (non-blocking)

- `app.kubernetes.io/managed-by: Helm` label: `Managed by Helm: the next upgrade replaces this change`.
- `ownerReferences` not empty: `Owned by {kind}/{name}: its controller may replace this change`.
- Always: `Pods that read these keys as environment variables keep the old values until they restart`.

## 15. `zeroize` is already a dependency

New values, loaded values, and every copy taken out of a kit input are `Zeroizing<String>`. The kit's `Rope`, its undo history, GPUI text caches, and serde/hyper buffers are freed, not wiped: the 0016 ceiling, stated in [secret-safety.md](secret-safety.md).
