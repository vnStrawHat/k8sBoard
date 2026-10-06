# 0047 · Secret safety (C1 applied to edits)

[Back to index](README.md) · All steps. Reviewer checklist; each rule maps to an AC and a test in [test-plan.md](test-plan.md). Extends [0016 secret-safety.md](../0016-secrets-tls/secret-safety.md). Amended after the opus security review (2026-10-03).

## Where Secret plaintext exists

| Place | Holds | Lifetime | Wiped? |
|---|---|---|---|
| `values_base` GET body (`secret_text`) | every value of the Secret | one call | yes (`Zeroizing<String>`) |
| decoded `Secret` in `values_base` | data, annotations | one call | yes: data and annotations zeroized before drop |
| kit `TextareaState` of a Secret field: `Rope`, undo history | typed or pasted text | until the view drops | **no** (kit-owned) |
| grown or reallocated kit buffers while typing | earlier partial copies | until reused by the allocator | **no** |
| `value()` copies (`Arc<str>`) | the whole text | until the last clone drops | **no**; the view never calls `value()` on a Secret field, the kit may |
| `text_for_range` answers to the OS IME | ranges of the text | OS-owned | **no** (outside our control) |
| `Paste` clipboard read | the pasted text | one call, then inserted | yes (read into `Zeroizing<String>`) |
| Ctrl V in a shown field | the kit's own paste: it reads the clipboard and inserts the text itself | until the view drops | **no** (kit-owned; only the Paste button of a masked field wipes its own read, the `Zeroizing` stays the owner and the kit gets a borrowed copy) |
| Apply copy (rope chunks → `String::with_capacity(len)`) | one changed value | until the `NewValue` drops | yes (`Zeroizing`) |
| `NewValue` in `ValuesEdit` / `WriteRequest` / `WriteIntent` | new values only | until the dialog closes and the intent drops | yes |
| `values_patch` body `serde_json::Value`, request bytes | new values, base64 | one request | no (freed) |
| PATCH response (`DynamicObject`) | every value of the Secret | inside `send`, dropped at once | no (freed; the 0016 log filter covers a decode failure) |
| GPUI paint of an unmasked field | text | ≤ 30 s, frames after re-mask | no (outside our control) |

**Ceiling, stated honestly.** `zeroize` (already a dependency) covers only k8sBoard's own short-lived copies: the GET body, the decoded Secret, the Paste button read, the Apply copy, and `NewValue`. The kit's own Ctrl V paste in a shown field, The kit's text storage, undo stack, grown buffers and `value()` copies, the IME, GPUI text caches, serde and hyper buffers, and the server response are freed, not wiped. The design limits **how long** and **where**: the editor never fetches a current value, only changed keys travel, and every buffer goes with the view.

## Rules (each is a review item)

1. **Masked by default** (AC 8, 12). Secret fields start masked and empty. Unmask is per field, by the editor's own eye button, for ≤ 30 s. The kit mask and `Input::mask_toggle()` are never used. No "unmask all", no launch option that unmasks.
2. **Write-only values** (AC 8). The editor never fetches or shows a current Secret value; there is no "Load current value". The drawer's 0016 Reveal and Copy value are the only way to see one.
3. **No logging or Debug** (AC 9). No `tracing::` call in `config_values.rs`, `values_edit.rs`, `values_edit_flow.rs`. `NewValue` and `ValueKey` have no `Debug`. `ValuesBase`, `ValuesEdit`, and `KeyChange` have a manual `Debug` (kinds, names, counts). `FieldChange` derives `Debug`, which prints only `field` and `KeyChange`'s manual `Debug`. `WriteOperation`'s `Debug` stays the variant name.
4. **No crash text** (AC 9). No `unwrap`, `expect`, `panic!`, `assert!` with a value in these modules; error `Display`s are fixed text plus at most a key name.
5. **Audit: names and markers** (AC 10). Fields are `data[KEY] added|value changed|removed`; `recordable_fields` still drops every value for `Secret` and `ConfigMap`. A typed value equal to the current one is still `value changed` (decision 16). The note is the user's own text.
6. **Confirm dialog: names and markers** (AC 11). It renders `changed_fields()` only; no old or new value for a Secret. A ConfigMap shows its text before and after (`confirm_lines`, UX round 3, see as-built.md); the audit never does. Warnings name keys, never values.
7. **Notices** (AC 9). Success: count and object name. Failure: `write_error_text` of a redacted error (Secret targets: reason code and field paths only, 0030 decision 29).
8. **Base64 for `data`** (AC 4). Encoding happens in the cluster crate just before the body is built; the app never sees base64.
9. **Binary is never text** (AC 13). Binary values are never decoded, shown, or loaded into an input.
10. **Refusals fail-closed** (AC 6). Helm release records (type `helm.sh/release.v1`, or label `owner=helm` on a ConfigMap or Secret), service-account-token Secrets, and immutable objects never produce a `ValuesBase`. There is no other constructor, and the `resourceVersion` precondition ties the commit to the checked object.
11. **Clipboard** (AC 14). Copy and Cut in a Secret field do nothing, masked or not. The editor has no Copy button. Paste is allowed, and `Paste` works while masked.
12. **One widget, newlines kept** (AC 23). A Secret field is one `TextareaState` for its whole life. Masking swaps only the rendered element, so `\n` and `\r\n` survive paste and re-mask.
13. **Drop on leave** (AC 15, 16). Cancel, success, a discard, a cluster switch, and a namespace or screen change drop the view. No value is kept in `AppShell`, settings, or any file.
14. **Screenshots** (AC 17). `value_access` blocks the eye; the ui-verifier never types a real value. A visible value in any capture is a high-severity defect: report it without the image and delete the file from `.tmp/`.
15. **No new persistence** (AC 1). This spec writes no file besides the existing audit line.

## ConfigMaps

ConfigMap values are shown as text in the editor, as the drawer already shows them. The editor never puts them into the audit, the confirm dialog, a notice, or a log (rules 3–6 apply; audit `PATH_ONLY_KINDS`). The server's own 422 and webhook messages for a ConfigMap may quote a value and are shown as sent. That is acceptable because the same value is visible in the drawer. Secret messages stay redacted (0030 `redact_error`).

## Live checks

- Agents: UAT denied path only (AC 21). The request count comes only from the app's own debug log, run with `RUST_LOG=kube_client::client::builder=debug`. That directive logs the `HTTP` span with method and URL only, and the 0016 guard keeps `kube_client::client=error` for bodies. Count `PATCH`/`PUT`/`DELETE` (must be 0); also grep the log for the fixture value (must be absent). If the directive cannot be used, a code review of the call sites stands in, as in 0026.
- User (manual, optional, on a write-capable test cluster with `K8SBOARD_ALLOW_WRITES=1`): edit a throwaway Secret, paste a multi-line value, check the stored value is unchanged, check `audit.jsonl` holds key names only, and check that Win+V history lists nothing from the editor.
