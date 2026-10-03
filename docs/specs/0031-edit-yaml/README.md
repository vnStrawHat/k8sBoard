# 0031 — Edit YAML (W10)

Status: draft, amended after the advisor review (M1–M3, S1–S8, N1–N4); **refreshed 2026-10-03 against main `2c7dc08`** (0027, 0028, 0029, 0030 steps 1/2a/3 merged). **Mutating.** Steps 0–3 send no mutating request: no `PUT`, not even a dry-run. **Step 4 ships Edit YAML, the dry-run preview, and the first real `update`. C3: Approved by the user on 2026-10-02 (one approval for all mutating specs).** Debug builds still block writes unless `K8SBOARD_ALLOW_WRITES=1` (agents never set it); UAT checks stay denied-path-only. **Decision 1 (replace vs SSA, C8): replace, decided by the user on 2026-10-02.** Roadmap: gap plan 0031; C1, C3, C6, C8; R1, R2.

**Prerequisites.** Merged: 0007, 0015, 0016, 0027, 0028, 0029, 0030 steps 1/2a/3 (`object_write.rs`, `write_guard.rs`, `audit_log.rs`). In flight: 0030 steps 2b + 4 (`run_guarded`, `checked_write`, `GuardedIntent.warnings`, `CommitMode`, `confirm_dialog.rs`, session `lock`/`generation`): steps 3–4 wait for them. Step 3 also needs the `RowAction` key layer ([0032 actions-ui.md](../0032-workload-actions/actions-ui.md); whichever of 0031 step 3 / 0032 step 2a-i lands first builds it). Lane W1 (see [files-to-touch.md](files-to-touch.md)).

## Goal

- **Edit YAML** for the editable kinds: the kit code editor (`EditorState`, tree-sitter YAML) in the workspace, as W10 draws it.
- **Diff vs cluster** from a server dry-run (`PUT ?dryRun=All`), with a semantic change list and checks (rollout impact, stale last-applied, leading-zero numbers).
- **Apply** through the 0030 core: gate, tier, dry-run, typed name, audit, always on the **row's own cluster** (0027). Concurrency uses `resourceVersion` (replace, not server-side apply).
- **Masked values round-trip safely**: `<hidden>` means "keep the server's value". The crate restores it from a fresh GET; a placeholder never reaches the server, and Secret data is never editable.
- 409 → refresh and rediff, rebasing the user's changed paths. 422 → the field paths are listed verbatim.

## Non-goals

Revision history tab, pre-apply snapshot and rollback (W10 note 5), "New" from templates, YAML LSP or schema, quota check, field-manager ownership warnings, server-side apply and Force, editing Secret values, jump-to-line, multi-object edit, Nodes (taints and labels are 0034), custom resources (0018: no `ObjectKind`), Helm releases (0038, deferred).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 0 | Cluster refactor: `object_yaml.rs` splits `mask_to_yaml` into `mask_object` + the existing `yaml_text`/`with_hidden_header`, and `object_yaml` into `get_object`; a golden test proves the 0007 output is unchanged | 1, 2, 14 |
| 1 | Cluster only: `object_edit.rs`, `edit_placeholders.rs` (check), `edit_preview.rs` (`FieldPath`, `field_paths`), `ObjectKind::{ALL, resource, is_editable}`, `AccessCheck::Update(kind)`, `review_access_for(checks, scope)` | 1, 2, 3, 4, 13 |
| 2 | Cluster write: `ReplaceObject`, restore with the `moved` marker, `EditPreview`, `WriteEffect::Replaced`, rollout and last-applied checks. No app caller yet | 1–5 |
| 3 | App, no server write (after 0030 2b): session `kind_access` and the lazy gate, `EditYaml(ObjectKind)`, `yaml_edit.rs`, `yaml_diff.rs`, `format_yaml`, local validation, discard prompt (release hook), `--screen edit-yaml-diff`. Entry points wired; `EditYaml` stays unshipped (`Comes in a later version`) | 1, 2, 6, 9, 10 |
| 4 | Approved by the user on 2026-10-02 (one approval for all mutating specs). `EditYaml` shipped, `preview_write` (dry-run `PUT`), Apply via `run_guarded` + `on_commit`, conflict banner and `rebase`, 422 panel, audit | 1, 2, 7, 8, 9, 11, 12 |

Steps 0–2 touch only `crates/cluster` and can run while 0030 steps 2b/4 are in flight.

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions and rationale |
| [edit-model.md](edit-model.md) | steps 0–1: refactor, `EditBase`, `ObjectEdit`, placeholders, rebase, lazy write checks |
| [edit-preview.md](edit-preview.md) | steps 1–2: paths, changes, preview build, checks |
| [write-path.md](write-path.md) | step 2: `ReplaceObject`, request shape, errors |
| [editor-view.md](editor-view.md) | steps 3–4: entry, view, tabs, diff, side panel, flows, keys, release hook |
| [files-to-touch.md](files-to-touch.md) · [test-plan.md](test-plan.md) | files per step, parallel lanes; tests and checks |

## Acceptance criteria

- [ ] 1. (steps 0-2: the gate and both clippy runs pass, no new `#[allow]`; `similar` and its `Cargo.lock` entry wait for step 3) Quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`. `Cargo.lock` gains exactly `similar` (step 3).
- [ ] 2. (steps 0-2 done; steps 3-4 pending) Every test of the step in [test-plan.md](test-plan.md) exists and passes offline. No test talks to a real cluster.
- [x] 3. No recorded request body contains `<hidden>`, `<hidden, changed>`, `<hidden, moved>`, or the header comment. An unmatched placeholder blocks locally and names its path.
- [x] 4. Changes to Secret `data` / `stringData` are refused locally. A Secret `PUT` carries the server's data unchanged.
- [x] 5. Request shape: `PUT {path}/{name}?dryRun=All&fieldManager=k8sboard` (a commit has no `dryRun`). JSON body: the edited object with the **base** `resourceVersion` and base `uid`, and no `status`, `managedFields`, or other server-owned metadata.
- [ ] 6. The Diff tab shows the dry-run result against the current object, both masked the same way, built with no header. Changed masked values read `<hidden, changed>`, position-matched ones `<hidden, moved>`. Unchanged runs fold.
- [ ] 7. A 409 shows the banner. "Reload and keep my changes" re-applies the user's changed paths to the new object, keeps a concurrent change to another list item, and lists unreachable paths and server-side changes.
- [ ] 8. (crate half done: `server_422_lists_fields_verbatim`, `secret_errors_are_redacted`; the side panel waits for step 4) 422 field paths are listed verbatim; Secret messages are redacted (0030 `redact_error`).
- [ ] 9. Edit YAML is offered only on editable kinds, enabled only when shipped, the lazy `update {resource}` check allows it, and the row's cluster is unlocked. Menus, E, and the palette agree (one entry: the `EditYaml` arm of `run_available_row_key`). Tier, typed name, connection, and audit come from the subject's `ClusterObject.cluster`, never the primary.
- [ ] 10. Leaving with changes asks before discarding, including a cluster switch or a slot release of the edited cluster. Ctrl S while a dry-run is running does nothing.
- [ ] 11. One audit line per commit: action `Edit YAML`, changed **paths only**. `EditPreview` never reaches the audit or a notification.
- [ ] 12. UAT (debug build): Edit YAML is disabled with `Not permitted: update {resource}` (or the probe's real answer). A trace shows only GETs and SSAR POSTs. The ui-verifier finds no high-severity defect in `edit-yaml-diff` (W10).
- [ ] 13. (crate half done; `yaml_edit.rs` waits for step 3) No `tracing::` call in `object_edit.rs`, `edit_placeholders.rs`, `edit_preview.rs`, or `yaml_edit.rs`. `EditBase`, `ObjectEdit`, and `EditPreview` have only a manual `Debug` (counts). No raw object leaves the crate.
- [x] 14. Step 0: `masked_yaml_output_is_unchanged` passes on the 0007 fixtures, and every 0007 test passes unmodified.

## Open items

1. R2: no write-capable cluster. The commit is proven only by fake-transport tests.
2. 0030 open item 5 applies to the lazy checks too: with scope All, namespace-only `update` rights show as denied.
3. Deferred W10 parts (non-goals) need their own spec. Snapshots of Secrets and ConfigMaps need a C1 decision first.
4. Closed: C8, decision 1 (replace vs SSA), user, 2026-10-02.
5. Closed (architect default, 2026-10-03; the user may override): E is offered only on editable kinds (decision 25), elsewhere it does nothing (`NotOffered`). Every editable kind's menu gets `Edit YAML` (E hint); the ConfigMaps `Edit` `KindAction` becomes that item.
6. (user) RBAC names such as `system:aggregate-to-admin` are not DNS-1123 subdomains, so the merged `WriteRequest::new` refuses them. Default: the RBAC kinds accept a path-segment name (decision 27); the alternative is to leave such objects uneditable.
