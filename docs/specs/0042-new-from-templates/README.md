# 0042 — New from templates

Status: draft, 2026-10-04, against main `4d99faa`. **Mutating**: one new allow-listed operation, `CreateObject` (POST of a new object). C3: covered by the user's one approval of 2026-10-02 for all mutating specs. Debug builds block writes unless `K8SBOARD_ALLOW_WRITES=1` (agents never set it); UAT checks stay denied-path-only. Builds on 0030 (write path, gate, tiers, audit), 0031 (editor, edit slot, discard prompt, `ApplyEdit`), 0046 (one cluster). Wireframes: **W7** `top` buttons `New namespace` (Namespaces), `New` (ConfigMaps, ResourceQuotas, PDBs, RoleBindings); W10 notes 3–4 (dry-run, tiered confirm, audit). Roadmap: gap audit row "W7 5 kinds · New", plan item 5; C3, C8; R1, R2.

## Goal

- A `New` header button on the five W7 screens opens a **YAML editor pre-filled with a template** of that kind (namespace from the session scope).
- Ctrl S runs a **server dry-run create**; a second Ctrl S (or `Create…`) opens the 0030 confirm (tier, typed name on PROD), commits, writes one audit line, and reveals the new row.
- One **narrow write**: `WriteOperation::CreateObject` accepts only the five kinds, validated again in `checked_operation`, with a lazy `create {resource}` gate.

## Non-goals

- **Secrets**: W7 Secrets has no `New` (its header is `Reveal all`). Deferred; a Secret create would need 0047's rules (write-only masked values, base64 in `data`, no `stringData`, never in logs, audit, or dialog), not a YAML editor.
- A form UI per kind: W7 draws only the buttons; the YAML template is the form.
- Other kinds (Deployments, Services, …), multi-document YAML, `generateName`, import from a file, a template library or user templates, server defaults diff, a palette command or key for New.
- Snapshot or undo of a create (snapshots dropped by the user, 2026-10-03); Delete (0033) removes a mistake.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Cluster only: `object_create.rs` (`ObjectDraft`, `DraftError`, `DraftWarning`), `ObjectKind::is_creatable`, `WriteOperation::CreateObject`, `AccessCheck::Create(kind)` (lazy), 409 mapping, allow-list row, fake-transport tests. No app caller | 1–5, 13 |
| 2 | App: `object_templates.rs`, `ObjectCreateView` in the edit slot (`OpenEdit::Create`), `New` header buttons and gate, dry-run and confirm flow, reveal after commit, discard and leaving prompts, `--screen new-config-map` | 1, 2, 6–12, 14 |
| 3 | Live: UAT denied path and request trace; ui-verifier | 15 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions with rationale |
| [create-path.md](create-path.md) | step 1: draft, validation, operation, request, errors, allow-list row |
| [create-view.md](create-view.md) | step 2: buttons, gate, templates, view, flow, confirm, audit |
| [files-to-touch.md](files-to-touch.md) · [test-plan.md](test-plan.md) | files per step; tests and checks |

## Acceptance criteria

- [ ] 1. Quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]` in production code; `Cargo.lock` unchanged.
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists under that name and passes offline; no test talks to a real cluster.
- [ ] 3. Request shape: `POST {collection path}?dryRun=All&fieldManager=k8sboard` (a commit has no `dryRun`), `application/json`, body = the draft with no `status` and no server metadata. Namespace: `POST /api/v1/namespaces`.
- [ ] 4. `WriteRequest::new(_, CreateObject(_))` is `None` for any kind outside Namespace, ConfigMap, ResourceQuota, PodDisruptionBudget, RoleBinding, a target that differs from the draft, or an unsafe name; `checked_operation` re-checks the draft (kind, `apiVersion`, name, no server fields).
- [ ] 5. A 409 `AlreadyExists` reads `{Kind} {name} already exists` (field `metadata.name`), not "changed since it was read".
- [ ] 6. (W7) Namespaces shows `New namespace`; ConfigMaps, ResourceQuotas, PDBs, RoleBindings show `New`. Disabled with the gate reason (`Checking permissions…`, `Not permitted: create {resource}`, `{cluster} is read-only`).
- [ ] 7. The editor opens with the kind's template, `metadata.namespace` = the single scoped namespace, else `default`. Local errors (syntax, wrong kind or `apiVersion`, missing or invalid name, missing namespace, `generateName`, server fields, `status`, `<hidden>` text) block before any request and name the field.
- [ ] 8. Ctrl S or `Create…` runs the dry-run; a passed dry-run for the current text enables the confirm; a changed text needs a new dry-run; Ctrl S during a dry-run does nothing.
- [ ] 9. The confirm uses the active cluster's guard and tier (PROD types the cluster name); it lists the name, namespace, and, for a RoleBinding, `roleRef` and subjects; it shows the draft warnings (decision 7).
- [ ] 10. After a commit the editor closes, the notice reads `Created {Kind} {namespace}/{name}`, and the cursor moves to the new row once the list shows it. One audit line, action `Create`, fields from `changed_fields()` (ConfigMap values never; path-only rule).
- [ ] 11. Cancel, a screen or namespace change, and a cluster switch ask before discarding a changed text (`Discard the new {Kind}?`); an unchanged template closes without a prompt.
- [ ] 12. A commit timeout says the object may have been created and keeps the editor; Retry is not offered (the user checks the list).
- [ ] 13. No `tracing::` call with draft content; `ObjectDraft` has a manual `Debug` (kind, namespace, name).
- [ ] 14. Screenshot builds send nothing (`block-writes`); `--screen new-config-map` shows the template, a passed dry-run line, and no connection call.
- [ ] 15. UAT (debug build, `readonly@Monitor`): every `New` is disabled with `Not permitted: create {resource}` (or the probe's real answer); the trace shows only GETs, LISTs, watches, and SSAR POSTs. ui-verifier: `new-config-map` and the five screens' headers, light and dark, no high-severity defect against W7 and W10.

## Open items

1. R2: no write-capable cluster; the commit is proven by fake-transport tests only.
2. 0030 open item 5 applies: with scope All, the lazy `create` check is cluster-wide, so namespace-only rights read as denied; the dry-run is the precise check.
3. Secrets `New` is deferred (non-goal); add it only with a 0047-style write-only form if the user asks.
