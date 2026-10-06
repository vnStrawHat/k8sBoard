# 0047 · As built

[Back to index](README.md) · Steps 1-3, 2026-10-03, on main `09a7a4c`. All ACs hold; the deviations are listed below.

## What was built

- **Cluster crate.** `config_values.rs`: `ValuesBase` (from `values_base`), `ValuesEdit` (from `ValuesBase::edit`), `KeyChange`, `NewValue` (a `Zeroizing<String>`), `values_patch`. `WriteOperation::SetDataValues`, `AccessCheck::Patch(kind)`, the 0030 allow-list row. The Secret body is read with `secret_text` and wiped (data and annotations) before `values_base` returns; the PATCH answer is dropped inside `send`.
- **App.** `values_edit.rs` (state, commands, rebase) and `values_edit_panels.rs` (drawing), `values_edit_flow.rs` (open, commit result, notice). `OpenEdit` (`Yaml` | `Values`) is the one edit slot. E is `EditValues` under the `ValuesScreen` key context (`keyboard_navigation::shell_key_context`); `EditYaml` is bound in `WORKSPACE && !ValuesScreen`.
- **Fixture.** `--screen values-edit` (screenshot builds): a Secret `payments/api-db-credentials`, a pending Remove of `DB_USER` and Add of `DB_PORT`, every field masked, no cluster.

## Deviations from the spec text

1. **`FieldChange` is `DataFieldChange`.** `cluster` already exports `edit_preview::FieldChange`; the new type has the same shape (`field`, `change`) and a derived `Debug` over `KeyChange`'s manual one.
2. **Add key.** The Add line takes the name only; the value is typed (or pasted) in the new row, whose Secret field starts masked like every other (AC 12). A new Secret key with an empty field is a local error under its key.
3. **Object size estimate.** Stored sizes are compared: base64 length for a Secret and for `binaryData`, bytes for ConfigMap text, plus key names. The spec said base sizes plus base64 for new values; mixing would under-count a Secret.
4. **Error types derive `Debug`.** `ValuesBaseError` and `ValuesEditError` need it for `thiserror`; their `Display` is fixed text plus at most a key name, so the grep of the test plan lists them next to `DataField`, `BaseNotes`, and `DataFieldChange`.
5. **`#[allow(clippy::disallowed_methods)]` on the new test module** `object_write_values_tests` in `object_write.rs`: every `object_write_*_tests` module carries it (0030 AC 9). It is the only added `#[allow]`; the production code adds none.
6. **A debug line.** `finish_kind_access` logs `is_patch_allowed` next to `is_delete_allowed` (a bool, false for kinds that do not ask), so the live check can read the real answer from the app's own log.
7. **Menu item.** `Edit values…` is a `KindAction::keyed` of ConfigMaps and Secrets, built by `row_action_item` like the other kind actions: it has no click handler and dispatches the key action, which acts on the cursor row and re-reads the gate. `values_edit_block` gives the row reasons to the menu, the palette, and the open call.

## Live check (UAT, `readonly@Monitor`, writes blocked)

The app's own debug log (`RUST_LOG=kube_client::client::builder=debug`) of `--screen secrets-menu` and `--screen configmaps-menu`: GET and SelfSubjectAccessReview POST only, **0 PATCH, 0 PUT, 0 DELETE**. The kind review logged `is_patch_allowed=false` for Secret and ConfigMap, so `Edit values…` reads `Not permitted: patch secrets` / `patch configmaps` (the gate text is tested offline). No value is involved on UAT; the fixture value never appears in the log. No UAT image was kept. The fixture screenshots (`v94-values-edit-light|dark.png`) were read by the coder; the ui-verifier was not run.

## Not covered by a test

- The rendered text of the confirm dialog: it draws `changed_fields()` and the warnings, which are tested (names and markers only).
- The 1 s ticker with a clock: `expire_reveals` and `tick` take the time as a parameter and are tested with it.
- The R2 gap stays: no write-capable cluster, so commits are proven by fake-transport tests only.

## UX follow-up (walk H9)

A Secret row whose text ends with `\n` or `\r\n` (usually a paste) shows `ends with a line break` and a `Trim` button (`ValueRow::ends_with_line_break`, `trim_line_break`). Trim removes the trailing `\r`/`\n`; the warning never blocks Apply. ConfigMap text is not flagged: a trailing newline is normal there.

## UX round 3 (M8, M9)

- **The confirm shows the ConfigMap text before and after** (decision of 2026-10-06, replacing rule 6 of `secret-safety.md` for ConfigMaps only): `data[LOG_LEVEL] value changed: info → debug`, `data[NEW] added: x`, `data[OLD] removed: gone`, each value on one line and cut at 80 characters. `ValuesEdit::confirm_lines` builds them from the base text (`old_texts`, kept for ConfigMap text keys only); the dialog draws them in place of `changed_fields()`. A Secret still shows the path and marker only, and `changed_fields()`, the audit line, notices, and logs never carry a value for either kind.
- **Edit YAML ends like Edit values** for a ConfigMap or Secret: the notice reads `Saved ConfigMap web-config` and carries the same `Restart N consumers` button (`env_source_kind`). Restart consumers names its source: `Restart 1 deployment that reads web-config`, with the Helm line for a Helm-managed workload and the OnDelete line, and a paused Deployment is skipped. The workload is read from the rows the session lists (the Deployments and DaemonSets feeds, the shown screen's kind); a consumer no list holds (a StatefulSet while its screen is not shown) is restarted unchecked and the dialog says its state is not loaded.
