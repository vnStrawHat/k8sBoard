# 0047 · Secret safety (C1 applied to edits)

[Back to index](README.md) · All steps. Reviewer checklist; each rule maps to an AC and a test in [test-plan.md](test-plan.md). Extends [0016 secret-safety.md](../0016-secrets-tls/secret-safety.md).

## Where Secret plaintext exists

| Place | Holds | Lifetime | Wiped? |
|---|---|---|---|
| `values_base` GET body (`secret_text`) | every value of the Secret | one call | yes (`Zeroizing<String>`) |
| decoded `Secret` in `values_base` | data, annotations | one call | yes: data and annotations zeroized before drop |
| `Load current value` (`secret_values`) | every value, one kept | one call; the kept one until the view drops | yes (`SecretValue`, then `Zeroizing<String>`) |
| kit input of a Secret field | typed or loaded text | until the view drops | **no**: kit `Rope`, undo history (ceiling) |
| `NewValue` in `ValuesEdit` / `WriteRequest` / `WriteIntent` | new values only | until the dialog closes and the intent drops | yes |
| `values_patch` body `serde_json::Value`, request bytes | new values, base64 | one request | no (freed) |
| PATCH response (`DynamicObject`) | every value of the Secret | inside `send`, dropped at once | no (freed; the 0016 log filter covers a decode failure) |
| GPUI paint of an unmasked field | text | ≤ 30 s, frames after re-mask | no (outside our control) |
| OS clipboard after the row's Copy | one value | ≤ 30 s, cleared if unchanged | cleared (0016) |

**Ceiling, stated honestly.** `zeroize` (already a dependency) wipes only buffers k8sBoard owns. The kit's text storage and undo stack, GPUI text caches, serde and hyper buffers, and the server response are freed, not wiped. The design limits **how long** and **where**: values are not loaded until asked, only changed keys travel, every buffer goes with the view.

## Rules (each is a review item)

1. **Masked by default** (AC 8, 12). Secret fields start masked and empty. Unmask is per field, on a click, for ≤ 30 s. No "unmask all", no launch option that unmasks.
2. **Load on demand only** (AC 8). Opening the editor fetches no value. `Load current value` is per key and explicit; the other keys of the GET are dropped in the same call.
3. **No logging or Debug** (AC 9). No `tracing::` call in `config_values.rs`, `values_edit.rs`, `values_edit_flow.rs`. `NewValue` and `ValueKey` have no `Debug`; `ValuesBase`, `ValuesEdit`, `KeyChange`, `FieldChange` print kinds, names, and counts. `WriteOperation`'s `Debug` stays the variant name.
4. **No crash text** (AC 9). No `unwrap`, `expect`, `panic!`, `assert!` with a value in these modules; error `Display`s are fixed text plus at most a key name.
5. **Audit: names and markers** (AC 10). Fields are `data[KEY] added|value changed|removed`; `recordable_fields` still drops every value for `Secret` and `ConfigMap`. The note is the user's own text.
6. **Confirm dialog: names and markers** (AC 11). It renders `changed_fields()` only; no old or new value, for either kind. Warnings name keys, never values.
7. **Notices** (AC 9). Success: count and object name. Failure: `write_error_text` of a redacted error (Secret targets: reason code and field paths only, 0030 decision 29).
8. **Base64 for `data`** (AC 4). Encoding happens in the cluster crate just before the body is built; the app never sees base64.
9. **Binary is never text** (AC 13). Binary values are never decoded, shown, or loaded into an input.
10. **Refusals fail-closed** (AC 6). Helm release, service-account-token, and immutable objects never produce a `ValuesBase`; there is no other constructor; the `resourceVersion` precondition ties the commit to the checked object.
11. **Clipboard** (AC 14). Copy and Cut in a Secret field do nothing; the row's Copy is the 0016 private write (Windows: off history and cloud) with the 30 s clear. Paste is allowed.
12. **Drop on leave** (AC 15, 16). Cancel, success, a discard, a cluster switch, and a namespace or screen change drop the view. No value is kept in `AppShell`, settings, or any file.
13. **Screenshots** (AC 17). `value_access` blocks unmask, load, and copy; the ui-verifier never types a real value. A visible value in any capture is a high-severity defect: report it without the image and delete the file from `.tmp/`.
14. **No new persistence** (AC 1). This spec writes no file besides the existing audit line.

## ConfigMaps

ConfigMap values are shown as text (the drawer already shows them). They still never reach the audit, the confirm dialog, a notice, or a log: the same rules 3–7 apply, because a ConfigMap often holds credentials (audit `PATH_ONLY_KINDS`).

## Live checks

- Agents: UAT denied path only (AC 21); the request log is grepped for `PATCH`/`PUT`/`DELETE` (none) and for any value (none).
- User (manual, optional, on a write-capable test cluster with `K8SBOARD_ALLOW_WRITES=1`): edit a throwaway Secret, check `audit.jsonl` holds key names only, and that Win+V history does not list a copied value.
