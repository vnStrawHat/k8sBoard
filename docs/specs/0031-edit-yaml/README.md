# 0031 — Edit YAML (W10)

Status: draft, amended after the advisor review (M1–M3, S1–S8, N1–N4), HEAD `1c859ae`. **Mutating.** Steps 0–3 send no mutating request: no `PUT`, not even a dry-run. **Step 4 ships Edit YAML, the dry-run preview, and the first real `update`. C3: Approved by the user on 2026-10-02 (one approval for all mutating specs).** Debug builds still block writes unless `K8SBOARD_ALLOW_WRITES=1` (agents never set it); UAT checks stay denied-path-only. **Decision 1 (replace vs SSA, C8) is pending a user decision (research requested 2026-10-02).** Requires 0030 with the one-line amendments the 0032 architect owns (`GuardedIntent.warnings`, `WriteOutcome.effect` / `WriteEffect`, `ObjectKind::{ALL, resource}`, "no SSA / no Force" for C8), the 0036 `run_guarded`, 0028 (E key), and 0007 (masking, kit editor). Secret edits wait for 0016; 0027 is optional. Roadmap: gap plan 0031; C1, C3, C6, C8; R1, R2.

## Goal

- **Edit YAML** for the W7 kinds that offer it: the kit code editor (`EditorState`, tree-sitter YAML) in the workspace, as W10 draws it.
- **Diff vs cluster** from a server dry-run (`PUT ?dryRun=All`), with a semantic change list and checks (rollout impact, stale last-applied, leading-zero numbers).
- **Apply** through the 0030 core: gate, tier, dry-run, typed name, audit. Concurrency uses `resourceVersion` (replace, not server-side apply).
- **Masked values round-trip safely**: `<hidden>` means "keep the server's value". The crate restores it from a fresh GET; a placeholder never reaches the server, and Secret data is never editable.
- 409 → refresh and rediff, rebasing the user's changed paths. 422 → the field paths are listed verbatim.

## Non-goals

Revision history tab, pre-apply snapshot and rollback (W10 note 5), "New" from templates, YAML LSP or schema, quota check, field-manager ownership warnings, server-side apply and Force, editing Secret values, jump-to-line, multi-object edit, Nodes (taints and labels are 0034).

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 0 | Cluster refactor: `object_yaml.rs` splits into `mask_object` / `to_yaml_text` / `get_object`; a golden test proves the 0007 output is unchanged | 1, 2, 14 |
| 1 | Cluster model: `object_edit.rs`, `edit_placeholders.rs` (check), `edit_preview.rs` (`FieldPath`, `field_paths`), `ObjectKind::is_editable`, `AccessCheck::Update(kind)`, lazy per-kind write checks (crate and session) | 1, 2, 3, 4, 13 |
| 2 | Cluster write: `ReplaceObject`, restore with the `moved` marker, `EditPreview`, rollout and last-applied checks. No app caller yet | 1–5 |
| 3 | App view with no server write: `yaml_edit.rs`, `yaml_diff.rs`, `format_yaml`, local validation, discard prompt, `--screen edit-yaml-diff`. Entry points are wired, but `EditYaml` stays unshipped (`Comes in a later version`) | 1, 2, 6, 10 |
| 4 | Approved by the user on 2026-10-02 (one approval for all mutating specs). `EditYaml` shipped, `preview_write` (dry-run `PUT`), Apply via `run_guarded` + `on_commit`, conflict banner and `rebase`, 422 panel, audit | 1, 2, 7, 8, 9, 11, 12 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions and rationale |
| [edit-model.md](edit-model.md) | steps 0–1: refactor, `EditBase`, `ObjectEdit`, placeholders, rebase, lazy write checks |
| [edit-preview.md](edit-preview.md) | steps 1–2: paths, changes, preview build, checks |
| [write-path.md](write-path.md) | step 2: `ReplaceObject`, request shape, errors |
| [editor-view.md](editor-view.md) | steps 3–4: view, tabs, diff, side panel, flows, keys |
| [files-to-touch.md](files-to-touch.md) · [test-plan.md](test-plan.md) | files per step; tests and checks |

## Acceptance criteria

- [ ] 1. Quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`. `Cargo.lock` gains exactly `similar` (step 3).
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists and passes offline. No test talks to a real cluster.
- [ ] 3. No recorded request body contains `<hidden>`, `<hidden, changed>`, `<hidden, moved>`, or the header comment. An unmatched placeholder blocks locally and names its path.
- [ ] 4. Changes to Secret `data` / `stringData` are refused locally. A Secret `PUT` carries the server's data unchanged.
- [ ] 5. Request shape: `PUT {path}/{name}?dryRun=All&fieldManager=k8sboard` (a commit has no `dryRun`). JSON body: the edited object with the **base** `resourceVersion` and base `uid`, and no `status`, `managedFields`, or other server-owned metadata.
- [ ] 6. The Diff tab shows the dry-run result against the current object, both masked the same way, built with no header. Changed masked values read `<hidden, changed>`, position-matched ones `<hidden, moved>`. Unchanged runs fold.
- [ ] 7. A 409 shows the banner. "Reload and keep my changes" re-applies the user's changed paths to the new object, keeps a concurrent change to another list item, and lists unreachable paths and server-side changes.
- [ ] 8. 422 field paths are listed verbatim; Secret messages are redacted (0030).
- [ ] 9. Edit YAML is enabled only when shipped, the lazy `update {resource}` check allows it, and the row's cluster is unlocked. Menus, E, and the palette agree. Tier, typed name, and audit come from `WriteIntent.cluster`.
- [ ] 10. Leaving with changes asks before discarding. Ctrl S while a dry-run is running does nothing.
- [ ] 11. One audit line per commit: action `Edit YAML`, changed **paths only**. `EditPreview` never reaches the audit or a notification.
- [ ] 12. UAT (debug build): Edit YAML is disabled with `Not permitted: update {resource}` (or the probe's real answer). A trace shows only GETs and SSAR POSTs. The ui-verifier finds no high-severity defect in `edit-yaml-diff` (W10).
- [ ] 13. No `tracing::` call in `object_edit.rs`, `edit_placeholders.rs`, `edit_preview.rs`, or `yaml_edit.rs`. `EditBase`, `ObjectEdit`, and `EditPreview` have only a manual `Debug` (counts). No raw object leaves the crate.
- [ ] 14. Step 0: `masked_yaml_output_is_unchanged` passes on the 0007 fixtures, and every 0007 test passes unmodified.

## Open items

1. R2: no write-capable cluster. The commit is proven only by fake-transport tests.
2. 0030 open item 5 applies to the lazy checks too: with scope All, namespace-only `update` rights show as denied.
3. Deferred W10 parts (non-goals) need their own spec. Snapshots of Secrets and ConfigMaps need a C1 decision first.
4. (user) C8, decision 1 (replace vs SSA): Pending user decision (research requested 2026-10-02).
