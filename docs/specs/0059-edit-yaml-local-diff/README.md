# 0059 — Edit YAML: local Diff, Dry-run button

Status: **draft 2026-10-08** against main `80a6b5a`. Amends [0031](../0031-edit-yaml/README.md) (Edit YAML, W10) and supersedes its "Diff opens with its check" (O16). User request: "When I edit and then restore the original value, Diff vs cluster still shows a change. I want the diff to show only the actual diff. The dry-run becomes a separate button and runs only when the user clicks it." Crates: `crates/app` (view), `crates/cluster` (deletion only). No new Kubernetes call, no new dependency.

**Root cause (verified):** `PreviewState::Passed` keeps the rows of the last dry-run; `refresh_dirty` never drops them, and `render_diff` lists the rows of any `Passed` preview whatever its `for_text`. The footer says `Changed since the last check`, the Diff tab still shows the old rows. `show_tab(Diff)` also sends a dry-run by itself.

## Goal

- The **Diff** tab is computed locally from the editor text against the opened object, in serializer form: an edit that is undone, or differs only in whitespace, key order, or comments, shows `No changes`. It never sends a request and can never show rows for another text.
- **Dry-run** is a footer button and the only way to send the server dry-run.
- **Apply… / Ctrl S** need a passed dry-run for the current text; the two-press Apply goes away.

## Non-goals

A key binding for Dry-run (open item 1); changes to the confirm dialog, its own dry-run, the audit, `rebase`, the 422 panel, the quota line, or the Revision history tab; Edit values (0047) and the form editors.

## Decisions (continuing 0031's numbering)

| # | Decision |
|---|---|
| 31 | Diff tab = `diff_rows` of the base body vs `format_yaml(text)` body, header comment lines dropped on both sides. No request |
| 32 | Computed on `cx.background_executor()`; refreshed from `refresh_dirty` (every text change) while the Diff tab is shown, and by `show_tab(Diff)`. Rows render only when `for_text` equals the current text |
| 33 | A text that `format_yaml` refuses: the Diff tab shows the error text (danger), no rows |
| 34 | `Dry-run` footer button (secondary, left of `Apply…`), the only caller of `run_preview`; the tab does not change |
| 35 | `Apply…` / Ctrl S only with a passed dry-run for the current text, else `Run the dry-run first`; held-key guard stays |
| 36 | `Reload and keep my changes` no longer reruns the check (the user asked for click-only dry-runs) |
| 37 | Tab title stays `Diff vs cluster` (the base is the cluster's object); `· {n}` only for a passed check of the current text |
| 38 | The side panel lists changes and the quota line only for a check of the current text; a stale one reads like `NotChecked` |
| 39 | `PassedPreview.rows` goes; `EditPreview.before/after` are deleted from the cluster crate (no other reader) |

Rationale and edge cases: [decisions.md](decisions.md).

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | rationale per decision, Secret and leading-zero notes, 0031 parts superseded |
| [view-design.md](view-design.md) | `LocalDiff` state, button reasons, render rules, flows table |
| [files-to-touch.md](files-to-touch.md) | per file: functions to change, add, delete |
| [test-plan.md](test-plan.md) | new, changed, and deleted tests |

## Acceptance criteria

- [ ] 1. Quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. No new `#[allow]`, no new dependency.
- [ ] 2. Every test of [test-plan.md](test-plan.md) exists under its name and passes offline.
- [ ] 3. Edit, dry-run, restore the base text, open Diff: no rows, `No changes`, tab title `Diff vs cluster`, side panel `No changes checked yet`.
- [ ] 4. Opening the Diff tab never sends a request (fake transport: zero PUTs).
- [ ] 5. A whitespace-only or key-order-only edit shows `No changes`; a text that does not parse shows its error in the Diff tab.
- [ ] 6. `Dry-run` sends exactly one dry-run `PUT` and keeps the current tab; it is disabled with the reasons of [view-design.md](view-design.md).
- [ ] 7. `Apply…` is disabled (`Run the dry-run first`) and Ctrl S does nothing until a dry-run passes for the current text; then one fresh press opens the confirm dialog; a held Ctrl S never does.
- [ ] 8. `grep -rn "\.before\|\.after" crates/cluster/src/edit_preview*.rs` finds no `EditPreview` field; `PassedPreview` has no `rows`.
- [ ] 9. `--screen edit-yaml-diff` shows the two changed lines as rows and the passed preview in the side panel; the ui-verifier finds no high-severity defect.

## Open items

1. (user) A key for Dry-run (e.g. Ctrl Shift S). Default: none; the button only.
2. (user) Whether `Dry-run` should stay enabled after it passed for the current text. Default: enabled; a click asks again (harmless, and it picks up a server change).
