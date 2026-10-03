# 0047 — Edit ConfigMap and Secret values

Status: draft, 2026-10-03, against main `52f8f87`. Amended 2026-10-03 after the opus security review (newline-safe Secret fields, write-only values, `owner=helm` refusal, zeroize ceiling). **Mutating.** User decision 2026-10-03: "Có làm sửa configmap/secret" (build it). C3: covered by the one approval of 2026-10-02 for all mutating specs. Debug builds block writes unless `K8SBOARD_ALLOW_WRITES=1` (agents never set it); UAT checks stay denied-path-only. Builds on 0016 (masking rules; the drawer keeps Reveal and Copy value), 0030 (write path), 0031 (edit slot, discard prompt, conflict banner), 0033 (Helm release refusal), 0046 (one cluster). **Amends 0031 decision 25** (decision 9, coordinator decision 2026-10-03): on ConfigMaps and Secrets, E opens Edit values. Roadmap: gap audit row "W7 Secrets · Edit values", plan item 11; C1, C3, C8, C10; R1, R2. Wireframes: W7 ConfigMaps and Secrets drawers (`Edit`, Data section), W10 notes 3–4 (dry-run, tiered confirm, audit).

## Goal

- A key/value editor for ConfigMap `data` / `binaryData` and Secret `data`: add a key, change a value, remove a key.
- Every Apply goes through the 0030 path: lazy `patch` gate, lock, confirm tier, server dry-run first, commit, audit.
- The write is a **JSON merge patch carrying the base `resourceVersion`** ([decisions.md](decisions.md) 1): only changed keys travel, and a concurrent change is a 409.
- Secret values are write-only here: never fetched or shown by the editor, typed or pasted into masked fields, and never in logs, audit, notices, or the confirm dialog ([secret-safety.md](secret-safety.md)).

## Non-goals

- **Snapshots and rollback of values** (user decision 2026-10-03: dropped).
- Renaming a key (add + remove does it); editing binary values or uploading a file; editing `stringData` as a field (decision 2).
- Line diffs of values; "Compare with previous" (W7 ConfigMaps); restart actions after a change.
- Labels, annotations, `type`, `immutable` (Edit YAML, 0031); creating ConfigMaps or Secrets ("New", templates).
- Viewing or copying a current Secret value in the editor (the drawer does it, 0016); setting a Secret value to `""` (an empty field means "keep", decision 16).
- Helm release records (type `helm.sh/release.v1`, or label `owner=helm` on a ConfigMap or Secret), service-account-token Secrets, immutable objects: refused.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Cluster only: `config_values.rs` (`ValuesBase`, `values_base`, `KeyChange`, `ValuesEdit`, patch body), `WriteOperation::SetDataValues`, `AccessCheck::Patch(kind)`, allow-list row, fake-transport tests. No app caller | 1–4, 6, 7, 9, 18 |
| 2 | App: `ResourceAction::EditValues`, lazy `patch` check, menus and palette, E on ConfigMaps and Secrets (`ValuesScreen` key context), `values_edit.rs` view in the shared edit slot, Apply via `start_write`, conflict reload, discard and leaving prompts, `--screen values-edit` | 1, 2, 5–17, 19, 20, 22, 23 |
| 3 | Live: UAT denied path, request trace, ui-verifier on `values-edit` | 21 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | numbered decisions; patch vs replace |
| [cluster-api.md](cluster-api.md) | step 1: types, base read, validation, operation, body, errors |
| [editor-view.md](editor-view.md) | step 2: entry, gate, view, flows, confirm, audit, 409 |
| [secret-safety.md](secret-safety.md) | where plaintext lives, rules, ceilings |
| [files-to-touch.md](files-to-touch.md) · [test-plan.md](test-plan.md) | files per step; tests and checks |

## Acceptance criteria

- [ ] 1. Quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`; `Cargo.lock` unchanged (`zeroize` and `base64` are already dependencies).
- [ ] 2. Every test of the step in [test-plan.md](test-plan.md) exists and passes offline; no test talks to a real cluster.
- [ ] 3. Request shape: `PATCH {path}/{name}?dryRun=All&fieldManager=k8sboard` (a commit has no `dryRun`), `application/merge-patch+json`. Body: `metadata.resourceVersion` = base, and only the changed keys under `data` / `binaryData`; a removal is `null`. Nothing else.
- [ ] 4. Secret text values are sent base64 (standard alphabet) in `data`; `stringData` is never sent. ConfigMap text values go to `data` as typed.
- [ ] 5. Apply runs the 0030 flow: gate (`patch {resource}`, lock), confirm tier (PROD types the name), dry-run before the confirm button enables, commit, one audit line. A failed dry-run blocks Apply.
- [ ] 6. **Fail-closed refusals**: a Secret of type `helm.sh/release.v1` or `kubernetes.io/service-account-token`, any ConfigMap or Secret labelled `owner=helm` (Helm release records, both storage drivers), and any immutable ConfigMap or Secret never yields a `ValuesBase`, so no `ValuesEdit` and no write request exists. The menu item is disabled with the reason; Helm release rows never offer it.
- [ ] 7. A 409 shows the conflict banner; "Reload and keep my changes" re-reads the base and re-applies changes by key, listing dropped ones. No request is ever sent without the base `resourceVersion`.
- [ ] 8. **Masked and write-only**: Secret value fields start masked and empty (empty = keep). The editor never fetches or shows a current Secret value; there is no "Load current value" and no Copy button.
- [ ] 9. **No value leaks**: no `tracing::` call and no `unwrap`/`expect`/`panic!`/`assert!` carrying a value in `config_values.rs`, `values_edit.rs`, `values_edit_flow.rs`; `NewValue` and `ValueKey` have no `Debug`; `ValuesBase`, `ValuesEdit`, `KeyChange` have a manual one (names and counts); `FieldChange` derives it over `KeyChange`'s. A test proves a distinctive value and its base64 are absent from the `Debug` of `ValuesBase`, `ValuesEdit`, `KeyChange`, `WriteRequest`, from `changed_fields()`, the audit line, every notice text, and every error text.
- [ ] 10. **Audit**: one line per commit, action `Edit values`, fields `data[KEY] added|value changed|removed` (or `binaryData[KEY] removed`); never a value. `PATH_ONLY_KINDS` is unchanged.
- [ ] 11. **Confirm dialog** lists key names with `added` / `value changed` / `removed` only; it never shows an old or new value, for Secrets and ConfigMaps alike.
- [ ] 12. **Unmask per key**: only an explicit click on the editor's own eye button unmasks one field; its own timer re-masks it after 30 s, and Apply and close re-mask too. While masked, a `•••• N chars` placeholder is rendered instead of the textarea. The kit mask (`masked`, `set_masked`) and `Input::mask_toggle()` are never used. No "unmask all".
- [ ] 13. **Binary values** read `binary, N bytes`, are not editable as text, and can only be removed.
- [ ] 14. **Clipboard**: Copy and Cut inside a Secret field do nothing, masked or not. `Paste` works while masked and does not show the text. ConfigMap fields copy normally.
- [ ] 15. **Buffers**: dirtiness uses `InputEvent::Change` and `text().len()`; the view never calls `value()` on a Secret field. Apply copies each changed value once, from the rope chunks into a `Zeroizing` `String::with_capacity(len)`. Cancel, a successful Apply, a cluster switch, and a namespace or screen change (after the discard prompt) drop the view and every buffer it owns.
- [ ] 16. **One cluster** (0046): the editor belongs to the active cluster; a switch with unsaved changes asks (`leaving_work`), then closes it. Tier, typed name, connection, and audit come from that cluster.
- [ ] 17. Screenshot runs: the eye is disabled (0016 `value_access`); `--screen values-edit` shows fixture keys only, masked.
- [ ] 18. Local checks before any request: key name rule (`[-._a-zA-Z0-9]+`, ≤ 253, not `.`/`..`), duplicates across `data`/`binaryData`, unknown keys, value ≤ 1 MiB, estimated object ≤ 1 MiB (`ObjectTooLarge`), at least one change. A typed Secret value equal to the current one is sent and audited as `value changed` (decision 16).
- [ ] 19. Server errors of a Secret target are redacted (0030 `redact_error`); a 422 lists field paths verbatim. ConfigMap server messages are shown as sent (they may quote a value the drawer already shows).
- [ ] 20. ConfigMap values show and edit as plain text (inline cap 128 KiB, larger keys can only be removed); same flow, confirm, and audit rules.
- [ ] 21. UAT (debug build, `readonly@Monitor`): Edit values is disabled with `Not permitted: patch secrets` / `patch configmaps` (or the probe's real answer); the app's own debug log with `RUST_LOG=kube_client::client::builder=debug` (method and URL only) shows only GETs and SSAR POSTs, or a code review stands in, as in 0026 ([secret-safety.md](secret-safety.md)). The ui-verifier finds no high-severity defect in `values-edit` (W7) and no visible secret value.
- [ ] 22. **Keys** (decision 9): on the ConfigMaps and Secrets screens E opens Edit values (same gate as the menu), and the menus show `E` on `Edit values…` only; `Edit YAML` stays in their menu and palette with no hint and still opens. Every other editable kind keeps E = Edit YAML; on Releases E does nothing. Menus, E, and the palette reach the one `run_available_row_key` entry.
- [ ] 23. **Newlines survive**: each Secret value field is one `TextareaState` for its whole life. Pasting `a\nb\n` or `a\r\nb`, masking, and unmasking leaves the text unchanged, and the `NewValue` (and its base64) carries it byte for byte.

## Open items

1. Closed (coordinator decision 2026-10-03): follow W7; E opens Edit values on ConfigMaps and Secrets, Edit YAML stays there without a key (decision 9, amends 0031 decision 25).
2. R2: no write-capable cluster; commits are proven by fake-transport tests only.
