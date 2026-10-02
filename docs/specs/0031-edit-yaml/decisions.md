# 0031 · Decisions

[Back to index](README.md). Architect defaults, amended after the advisor review. 0007, 0016, and 0030 decisions apply unless replaced here.

## Write style

| # | Decision | Rationale |
|---|---|---|
| 1 | **Decided by the user on 2026-10-02 (option 1 of 3, after research).** **Replace** (`PUT`) the edited object with the **base `resourceVersion`** and `fieldManager=k8sboard`. No server-side apply, and **no Force path**. C8 and 0030 decision 4 say so (amended in 0030 by the 0032 architect; `cross-cutting.md` C8 by 0031). **Known effect:** a `PUT` does not update `kubectl.kubernetes.io/last-applied-configuration`, so a later `kubectl apply` diffs against the stale annotation and can revert or remove the edited fields. The Checks panel warns when the object carries it | (a) W10 pins the edit to a `resourceVersion`, which is optimistic concurrency. (b) A whole-object editor must delete a field when a line is deleted; SSA keeps fields owned by other managers. (c) SSA of a full GET makes `k8sboard` co-owner of every field, so GitOps drifts. (d) SSA conflicts would push users to Force. (e) Research: Headlamp and Rancher `PUT` with `resourceVersion` (Rancher also rebases on 409, as decision 21 does); kubectl edit, Lens, and Argo CD patch with no RV lock; no GUI tool uses SSA (Aptakube #580: SSA breaks Argo pruning). Fallback if the placeholder restore proves fragile: a JSON merge patch that carries `resourceVersion` |
| 2 | W10's "no field manager conflicts" becomes `unchanged since you opened it` | the honest statement for replace; `Conflict.managers` is always empty (0030) |
| 3 | The body omits `managedFields`, `status`, and server-owned metadata **except `uid`**. The base `uid` stays as an implicit precondition | the server keeps omitted `managedFields` and ignores `status` on the main resource. `uid` pins the object identity: a recreated object answers 409 |
| 4 | Every dry-run and commit first does a **fresh GET** inside `write`. A fresh `resourceVersion` that differs from the base → `Conflict`, and no `PUT` is sent | the restore needs server values; the same answer as the server's 409, one request sooner |

## Masked values (C1)

| # | Decision | Rationale |
|---|---|---|
| 5 | The editor text is the 0007 masked YAML (Secret data, manifest annotations, StorageClass secret parameters, env literals unless Env values is on) with the edit header | the app never holds raw values |
| 6 | **`<hidden>` means "keep the server's value".** Each `<hidden>` in the edited tree gets the value at the same logical location of the fresh object. Objects match by key. A list matches **by `name`** only when every item is an object with a string `name` and the names are unique; **otherwise by index** (this covers duplicate `env[].name`) | one generic rule for every mask rule; one rule for duplicates (advisor M2) |
| 7 | **Invariant:** the user can never store a literal `<hidden>`. Every `<hidden>` is a placeholder, so it either restores a server value or blocks locally (`UnmatchedPlaceholder`) before any request. A placeholder restored through the index fallback from an item whose other fields changed shows as **`<hidden, moved>`** in the diff, plus a warning in Checks. It is restored, but never silently | makes the index fallback safe: a reordered unnamed list cannot quietly attach a secret value to the wrong item |
| 8 | **Secret `data` / `stringData` are locked** (the maps equal the base's). Other Secret fields can be edited, and the restored `PUT` carries the server's data | 0016 has no YAML reveal; a `PUT` without `data` would wipe the Secret |
| 9 | Env values: the editor starts `Hidden`; the toggle refetches and is enabled only while the text is unchanged. With `Shown`, the diff shows env literals (the user's choice). `EditPreview` never reaches the audit or a notification | toggling would discard edits; previews stay on screen |
| 10 | Diff sides are masked like the editor and built with `header: None`. A masked value whose raw sides differ reads `<hidden, changed>` | otherwise a changed literal looks unchanged; a header line would show as a diff line |
| 11 | Raw objects (fresh GET, response) never leave the crate. They are masked, then dropped at the end of `write`, never traced, never in `Debug`, and errors use fixed texts | 0007 decisions 2 and 13; the 0016 transport ceiling |

## Editor

| # | Decision | Rationale |
|---|---|---|
| 12 | Hidden from the editor: `status`, `metadata.{managedFields, resourceVersion, uid, generation, creationTimestamp, deletionTimestamp, deletionGracePeriodSeconds, selfLink}` (W10 note 1). If the user types them back, they are stripped | the server owns them |
| 13 | `apiVersion`, `kind`, `metadata.name`, and `metadata.namespace` cannot change (local error) | a rename is a create plus a delete |
| 14 | `managedFields` is always hidden; no W10 toggle | it would be stripped anyway |
| 15 | Two presses: Ctrl S / `Apply…` runs the preview and shows the Diff; a second press opens the 0030 dialog, which dry-runs again. Ctrl S while a dry-run runs does nothing | W10 note 1; no queued double submit |
| 16 | The editor replaces the table and drawer (W10 layout). One edit at a time; leaving with changes asks to discard | W10 draws a workspace screen |
| 17 | `Format` re-serializes with the 0007 serializer (sorted keys) | W10 button; reuse |
| 18 | Checks: dry-run, rollout impact (Deployment, StatefulSet, DaemonSet template changes), stale last-applied (decision 1), `<hidden, moved>` (decision 7), and leading-zero numbers. The parser follows YAML 1.2, so an unquoted `0755` reads as decimal 755, while kubectl (YAML 1.1) reads octal 493. Lines with an unquoted number starting with `0` get the warning `0755 is read as 755; write 493 or 0o755` | `defaultMode: 0755` is common and would silently set the wrong mode |
| 19 | No YAML LSP; the server dry-run validates (C6 proposal) | no Node process |
| 20 | Line diff: `similar` 3 (defaults `std`, `text`), on the background executor | C6; one package |

## Conflicts, errors, audit, RBAC

| # | Decision | Rationale |
|---|---|---|
| 21 | **Rebase by paths, not merge patch** (advisor M1). For each path of `field_paths(old.masked, edited)` (name-matched), set or remove that path in `new.masked`. A path whose parent is gone on the server is listed in the banner: `{path}: no longer exists on the server`. The side panel also lists `paths changed on the server since you opened it` (`field_paths(old.masked, new.masked)`) | a whole-list merge patch would revert concurrent changes to other list items. Paths touch only what the user changed |
| 22 | 422 → side panel `The change is invalid`, showing `details.causes[].field` **verbatim** | server paths can use another notation; rewriting them could mislead |
| 23 | Audit: action `Edit YAML`, the changed **paths** of `ObjectEdit`, never values | credentials in env, ConfigMaps, annotations |
| 24 | RBAC: `AccessCheck::Update(kind)` is **lazy per kind**. It runs when a screen of that kind is first shown and is cached in the session report. Before it answers, the gate reads `Checking permissions…`. 0033 adds `Delete(kind)` to the same lazy set | no session-start burst of write checks; the dry-run stays the precise check |
| 25 | Editable kinds: Pod, Deployment, StatefulSet, DaemonSet, CronJob, Service, Ingress, NetworkPolicy, ConfigMap, HorizontalPodAutoscaler, ResourceQuota, PodDisruptionBudget, Secret (0016), the 0015 RBAC kinds once they exist | W7 "Edit YAML" / "Edit" items |
| 26 | Steps 0–3 send no mutating verb; the dry-run `PUT` ships with the commit in step 4 | C3: a dry-run is still a mutating-verb request (advisor M3) |
