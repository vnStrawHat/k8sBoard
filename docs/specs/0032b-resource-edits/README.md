# 0032b — Resource edits (HPA limits, PVC expand, default StorageClass)

Status: **built (steps 1 to 4, 2026-10-03)**, see [as-built.md](as-built.md); refreshed 2026-10-03 against main `2c7dc08`. The "object actions" of the roadmap 0032 entry, split out of 0032. Builds strictly on 0030 as amended (decisions 30–36: `run_guarded`, `checked_write`, `GuardedIntent.warnings`, `WriteEffect::Patched`, gate) and on 0032 (`GuardedKind::Batch`, `ValuePopover`, the `RowAction` key layer and one entry point). Prerequisites: merged 0013 (HPAs), 0014 (PVCs, StorageClasses), 0027, 0028, 0030 steps 1/2a/3; 0030 steps 2b + 4 (in flight) and 0032 (2a-i, 2a-ii, 2b) before step 2. Step 1 is crate-only and can run any time after 0032 step 1 (both edit `object_write.rs`). Lane W1, last spec. Roadmap: gap plan 0032; C3, C8, C10; R2. Wireframe: W7 kind data for HPAs (`Edit min / max…`, top `Edit limits`), PVCs (`Expand…`, top `Expand`), StorageClasses (`Set as default`, top `Set default`).

**User approval (C3):** Approved by the user on 2026-10-02 (one approval for all mutating specs). Step 1 sends nothing (fake transport); steps 2, 3, and 4 each add the first real commit of one operation. Debug builds still block writes unless `K8SBOARD_ALLOW_WRITES=1` (agents never set it); UAT checks stay denied-path-only.

## Goal

- `SetHpaReplicaRange`: HPA `Edit min / max…` (one row) and `Edit min / max` (ticked rows, `Batch`).
- `ExpandClaim`: PVC `Expand…` and `Expand` (ticked rows); irreversible, needs `allowVolumeExpansion`.
- `SetDefaultStorageClass`: `Set as default` / `Set default`, the first two-object write (set the new default, then unset the old one) as one `Batch`.

## Non-goals

- Certificate `Renew now` (W7 Certificates): moved to the custom-resource scope, recorded as 0018 open item 5 (cert-manager status patch, like `cmctl renew`).
- HPA metric or behavior edits (Edit YAML, 0031); PVC shrink (the API forbids it); StorageClass create or edit; "no default" as an explicit action.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Cluster crate: three operations, three `AccessCheck`s, fake-transport tests | 1–4 |
| 2 | `ValueForm::ReplicaRange`, HPA Edit min / max (menu and selection bar). Approved by the user on 2026-10-02 (one approval for all mutating specs). | 1, 2, 5, 6, 9 |
| 3 | PVC Expand and bulk Expand. Approved by the user on 2026-10-02 (one approval for all mutating specs). | 1, 2, 5, 7, 9 |
| 4 | StorageClass Set default (two-object `Batch`); UAT denied path; ui-verifier. Approved by the user on 2026-10-02 (one approval for all mutating specs). | 1, 2, 5, 8–10 |

## Files

| File | Contents |
|---|---|
| [operations.md](operations.md) | wire format, RBAC, dry-run, risk, audit per operation; validation in `WriteRequest::new` |
| [actions-ui.md](actions-ui.md) | menus, `ValuePopover` forms, row-state reasons, warnings, bulk, the two-object plan |
| [decisions.md](decisions.md) | numbered decisions with rationale |
| [files-to-touch.md](files-to-touch.md) | files per step, doc updates |
| [test-plan.md](test-plan.md) | fake transport, unit and window tests, UAT denied path, ui-verifier |
| [as-built.md](as-built.md) | where the code differs from the text above, behavior notes, tests, UAT |

## Acceptance criteria

- [x] 1. Quality gate passes, plus the screenshot-feature clippy; no new `#[allow]`; `Cargo.lock` gains no package.
- [x] 2. Every test in [test-plan.md](test-plan.md) exists and passes offline; no test talks to a cluster.
- [x] 3. `allow_list_matches_the_operations` pins method, path, query, content type, and body of the three operations.
- [x] 4. `WriteRequest::new` refuses `min = 0`, `min > max`, and an unparsable or zero storage quantity.
- [x] 5. Every commit goes through `run_guarded` / `checked_write`; each action is gated by its own `AccessCheck`, then `row_block`.
- [x] 6. HPA edits warn when the new range moves the current replica count; bulk applies one range to every ticked HPA.
- [x] 7. Expand: risk `Change` with the irreversible warning; the quantity is trimmed; only bound, non-terminating claims; the new size must be larger than the current request; a class without expansion disables it when the class list is loaded; otherwise the dry-run reports the admission refusal.
- [x] 8. Set default: one `Batch` (`BatchFailure::Stop`) that sets the new default first, then unsets every other default (GA and beta annotation); every dry-run must pass; a failed set sends nothing more; a partial failure names the class in use (newest by `creationTimestamp`).
- [x] 9. Audit: one line per object; values are numbers, quantities, and `true`/`false` only.
- [x] 10. UAT (debug build): every item disabled with `Not permitted: …`; trace shows only GETs and SSAR POSTs. ui-verifier: `--screen hpa-range-popover`, `--screen expand-confirm`, `--screen default-class-confirm`, no high-severity defect.

## Open items

1. The Expand class check uses the StorageClasses list only when it is loaded; a one-shot class GET could make it exact (no new list call in 0032b).
2. Two defaults exist briefly between the two commits; the API server uses the newest by `creationTimestamp` meanwhile (Kubernetes 1.26+). A failed unset leaves two defaults; the notice names the class in use and offers Retry. A failed set sends nothing more (`BatchFailure::Stop`).
3. R2: no write-capable cluster; commits are proven by fake transport only.
