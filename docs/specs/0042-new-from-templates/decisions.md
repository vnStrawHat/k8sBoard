# 0042 · Decisions

[Back to index](README.md)

| # | Decision | Rationale |
|---|---|---|
| 1 | **Five kinds, a closed list** (`ObjectKind::is_creatable`): Namespace, ConfigMap, ResourceQuota, PodDisruptionBudget, RoleBinding | The kinds W7 gives `New`. A closed list keeps the create allow-list narrow (C3); a new kind is a reviewed code change. |
| 2 | **YAML template in an editor**, no per-kind form | W7 draws buttons only. The 0031 editor, keys, and slot already exist; a form per kind is five UIs for no drawn need. |
| 3 | A **separate small view** (`ObjectCreateView`), not a mode of `YamlEditView` | `YamlEditView` is built around a server base (`EditBase`, placeholders, rebase, 409 reload). A create has no base; mixing both would add `Option`s to every path. The new view reuses the kit editor setup, the `YAML_EDIT` key context (`ApplyEdit` on Ctrl S), the edit slot (`OpenEdit`), the discard prompt, and the 0031 panel helpers. |
| 4 | **POST with `fieldManager=k8sboard`**, no server-side apply | C8: no SSA anywhere; a create has no concurrency to guard, and the server answers 409 if the name exists. |
| 5 | **Validation twice**: `ObjectDraft::new` (user-facing errors) and `checked_operation` (re-check of the built draft) | The task asks for explicit validation in `checked_operation`; the draft type also makes a bad request unrepresentable from the app. |
| 6 | 409 on create → `Invalid { "{Kind} {name} already exists", ["metadata.name"] }` | `Conflict` reads "changed since it was read" and offers a reload that has no meaning for a create. No new `WriteError` variant. |
| 7 | **Draft warnings** (non-blocking, in the confirm): a RoleBinding whose `roleRef` is ClusterRole `cluster-admin` or `admin`; a subject Group `system:authenticated`, `system:unauthenticated`, or `system:serviceaccounts` | Granting broad rights by mistake is the main risk of this feature. The API server's escalation check (needs `bind` or the rights already) still applies; the warning makes the grant visible before it is sent. |
| 8 | Namespace in the template: the **single scoped namespace**, else `default` | The user is usually looking at that namespace; the text stays editable. |
| 9 | Name rules: Namespace a DNS-1123 **label**; ConfigMap, ResourceQuota, PDB a DNS subdomain; RoleBinding the path-segment rule (0031 decision 27) | The API server's own rules; checked locally so the error names the field before a request. |
| 10 | Server-owned fields are **refused**, not stripped | A pasted `uid` or `resourceVersion` means the text came from another object; refusing says so. `status` likewise. |
| 11 | After commit: **close and reveal** | W7 lists the new object at once; the user sees it in place. The reveal waits for the watch to show the row (existing `reveal` behavior). |
| 12 | No Retry after `OutcomeUnknown` | A second POST could create a duplicate where the server allows it (it does not for named objects, but the message would be a confusing 409); the user checks the list first. |
| 13 | **Secrets deferred** | Not in W7's `New` list. 0047 rules (write-only masked values) do not fit a YAML text editor. |
