# 0017 · Helm safety (C1 applied)

[Back to index](README.md) · All steps. Reviewer's checklist; each rule maps to a test or a review item in [test-plan.md](test-plan.md). 0016 [secret-safety.md](../0016-secrets-tls/secret-safety.md) rules (log guard, `request_text`, no cache, private clipboard) apply.

## What a release payload holds

`data.release` = base64(gzip(JSON)), base64-encoded again by the Secret. The JSON holds `config` (user values: may hold passwords), `chart.values` (defaults), `chart.templates`, `manifest` (rendered YAML, **including Secret manifests with values**), `info.notes` (rendered text, may hold passwords), status and chart metadata.

## Where plaintext exists, and for how long

| Place | Holds | Lifetime | Wiped? |
|---|---|---|---|
| kube/hyper/serde buffers of the release watch | current revisions in scope, ≤ 10 per list page | one event or page | no (freed) |
| `release_json` output | decompressed JSON (≤ 64 MiB), one `Zeroizing<Vec<u8>>` reserved once from ISIZE | one call | **wiped**; regrowth after a wrong ISIZE frees unwiped copies |
| summarizer decode | kept fields only | one call | freed |
| detail GET body | `Zeroizing<String>` from `request_text` | one call | **wiped** |
| detail decode | `serde_json::Value` trees of config and defaults; serde scratch buffers for `manifest`, `notes` strings | one call | freed, not wiped |
| `HelmText` (masked or revealed) | YAML, notes, diff values | while the view exists; revealed ≤ 30 s | **wiped on drop** |
| editor rope, GPUI shaped-line caches | the shown text | until replaced, a few frames | no |
| copy of revealed text | the selection, via `Zeroizing<String>` → `write_private_text` | ≤ 30 s on the clipboard, then cleared if unchanged | buffer wiped; clipboard cleared |

The design limits **where** and **how long**: summaries hold no value, plaintext exists only while a Reveal is active, every tab change hides, every subject change or drawer close drops the view.

## Rules (each is a review item)

1. **Summaries hold no value and are never traced**: `HelmReleaseSummary` and `HelmRevision` keep names, numbers, status, timestamps, chart metadata, and `description`; tests prove the distinctive config value, manifest Secret value, and notes are absent from `format!("{:?}")`. No code logs or traces a summary with `{:?}` (or at all): `description` can quote a value (S6).
2. **`HelmText`**: private `Zeroizing<String>`; no `Debug`, `Display`, `Clone`, `Serialize`, `PartialEq`. `HelmReleaseDetail`, `HelmRevealed`, `HelmValuesDiff`, `ValueChange` hold it and derive none of these.
3. **No logging**: no `tracing::` call in `helm_release.rs`, `helm_release_detail.rs`, `helm_values_diff.rs` (cluster), `helm_rows.rs`, `helm_release_view.rs` (app). Decode errors are fixed texts; serde, flate2, base64, and serde-saphyr errors are dropped (they can quote content).
4. **GET path**: detail GETs use 0016's extracted `secret_text` (`Request::get` + `request_text`); never `Api::get` or `Client::request`.
5. **Masked by default**: values, the Overview diff, and notes open masked; `HelmReleaseDetail` carries no notes text (only `notes_lines`); revealed text comes only from `helm_revealed` / a `Revealed` diff after an explicit Reveal; manifests are never revealed.
6. **Reveal lifetime**: `hides_at` = first revealed arrival + `REVEAL_DURATION` (30 s); a 1 s ticker drops revealed texts; any tab change hides at once; subject, revision, screen, scope, context change, or drawer close drop the view.
7. **Private copy** (M2): while revealed, the editor's `Copy` and `Cut` actions are captured (`capture_action`), propagation stops, the selection goes into `Zeroizing<String>` → 0016 `write_private_text` → `SecretCopied(mark)` (AppShell arms the 30 s clear). Clipboard unavailable → nothing is written, inline error (fail closed, never the normal clipboard). Revealed Diff rows and Notes text are plain elements that do **not** opt into text selection. Masked text copies normally (it holds no value).
8. **No cache**: a new view refetches; nothing survives a dropped view.
9. **No persistence**: nothing is written to disk; nothing reaches settings or the audit log (C10).
10. **Screenshots**: `AppShell.secret_value_access == Blocked` disables every Reveal (Overview, Values, Notes); no launch option reveals. ui-verifier never clicks Reveal or Env values.
11. **Probe**: `--helm` prints counts, statuses, line counts, hidden counts, `{ns}/{name}`, revision numbers, and `description {n} chars`; never values, manifest text, notes, or description text.
12. **Size**: decompression stops at `RELEASE_SIZE_LIMIT` (64 MiB) with a fixed error; manifests parse under the decision 13 budget.

## Screenshot rules for ui-verifier

- Allowed screens: `releases`, `releases-drawer`, `releases-values`, `releases-manifest` (masked state only; code blocks reveal under `--screenshot`).
- Drawer screens use `--filter <first release name from the probe>`; if UAT has no release, capture the empty state of `releases` only.
- Any string or number value other than `<hidden>` in Values or the Overview diff, any notes text, or any Secret `data` value in the Manifest is a **high-severity defect**: report it without attaching the image to another agent and delete the file from `.tmp/`.

## Live checks (coder, step 3)

On UAT with the first release: Overview diff and Values open masked; Reveal shows values, `Hides in Ns` counts down, re-masked after 30 s; switching tabs hides at once. Notes: placeholder, Reveal shows text, hides after 30 s. Copy a revealed **boolean or key line** (Ctrl+C): `wc -c < /dev/clipboard` is non-zero (never print the clipboard), Win+V history does not list it, and the count is 0 after 30 s. Manifest: every `kind: Secret` document shows `<hidden>` data. No screenshots of revealed states; never paste revealed text anywhere.
