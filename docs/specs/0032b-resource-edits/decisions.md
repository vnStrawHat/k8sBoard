# 0032b · Decisions

[Back to index](README.md). Architect defaults; items marked (user) need confirmation.

| # | Decision | Rationale |
|---|---|---|
| 1 | Three new `WriteOperation`s, all JSON merge patches without `resourceVersion` | 0030 decisions 2, 14; each sets whole fields |
| 2 | `WriteRequest::new` validates the values (`1 ≤ min ≤ max`, parsable non-zero quantity) | invalid requests cannot be built (0030 decision 3) |
| 3 | HPA patches `minReplicas` and `maxReplicas` together | one intent, one dialog line pair; the API validates the pair |
| 4 | Expand is `Change` with the warning `A volume cannot shrink; this cannot be undone` (decided) | consistent with 0032 decision 13: irreversibility is told by a warning, not by a harder confirm; nothing is taken down |
| 5 | Unsetting a default also clears the beta annotation | 0014 treats either key as default; a beta `true` left behind would keep the class default |
| 6 | Three static `AccessCheck`s (`patch` on the resource), not the 0031 lazy per-kind checks | three fixed resources; SSAR at session start like 0030 `PatchNodes` |
| 7 | The 0032 popover (`ValuePopover`, named so from the start) gains forms; each form owns its inputs | one input surface for number and quantity edits; no mixed homes for input state |
| 8 | No rename commit: 0032 already ships `ValuePopover` / `ValueForm::Replicas` | the generalization costs nothing later |
| 9 | The new PVC size must exceed `max(requested, capacity)` | the API forbids less than the previous value; equal is a no-op |
| 10 | The class-expansion check reads the classes the PVCs screen joins in (round 3: a StorageClasses companion list); the dry-run is the backstop for a claim read elsewhere | the admission plugin's 403 names the cause |
| 11 | Warnings: HPA range moves current replicas; expand is irreversible, resize in progress, file-system resize pending; set default changes new claims only | non-blocking context the dry-run cannot give |
| 12 | Bulk HPA and Expand apply one value to every ticked row; rows already there are skipped | W7 top buttons act on selected rows (W7 note 1); 0032 `Batch` |
| 13 | Set default is one `Batch` with `BatchFailure::Stop`: set the new default first, then unset the others | never zero defaults; two at once is allowed since 1.26 (the newest by `creationTimestamp` wins); a failed set must not unset the old default |
| 14 | A partial failure uses the generic `Stop` notice plus Retry; the "both are marked default" text comes from `resource_edits.rs`; Retry re-plans, omits the set when the target is already default, and bypasses `row_block` | the state left behind is safe but must be visible; a repeated `true` patch (watch latency) is harmless |
| 15 | Set default from the selection bar needs exactly one ticked class | there is only one default |
| 16 | (user) Certificate `Renew now` moves to the custom-resource scope as 0018 open item 5 | it is a cert-manager status patch on a CR (like `cmctl renew`), not a built-in field edit; it needs 0018's custom-object model and a cert-manager check |
