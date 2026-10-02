# 0032 · Decisions

[Back to index](README.md). Architect defaults; items marked (user) need confirmation. Amended after the advisor review (M1–M3, S1–S9, nice-to-haves).

| # | Decision | Rationale |
|---|---|---|
| 1 | One `WriteOperation` per action; no generic "patch" variant | 0030 decision 2: each write is a visible enum change plus an allow-list row |
| 2 | Scale uses the `scale` subresource with a JSON merge patch `{"spec":{"replicas":n}}`, checked as `patch {resource}/scale` | kubectl `scale` parity; RBAC can grant scale without full patch |
| 3 | Restart sets `kubectl.kubernetes.io/restartedAt` by merge patch; `kube::Api::restart` is forbidden by clippy | kube-rs writes `kube.kubernetes.io/restartedAt` (`kube-core util.rs:28`); kubectl and dashboards read the kubectl key |
| 4 | The restart timestamp is fixed (whole seconds, UTC) when the intent is built | the dry-run and the commit must be the same request (0030 decision 5) |
| 5 | Roll back is JSON Patch (`test` uid + `replace /spec/template`); enables kube feature `jsonpatch` | `rollout undo` replaces the template; a merge patch keeps stale map keys. `json-patch 4.2.0` is already locked (kube-runtime) |
| 6 | Roll back, Trigger now, and Re-run read the full object inside `object_write.rs` at send time | summaries never hold templates (C1: env literals) |
| 7 | Roll back verifies owner uid and revision before sending (`NotFound` otherwise) and refuses a target equal to the current template (`Invalid`) | a stale drawer must not roll back to a foreign, reused, or identical revision |
| 8 | Trigger now and Re-run use `generateName`; the server name returns as `WriteOutcome.created_name` (S8) and appears in the notice and the audit | repeated runs never collide; the user sees what was created |
| 9 | Trigger now copies kubectl (`instantiate: manual`, `controller: true`) **without `blockOwnerDeletion`** (M1); Re-run is standalone (no owner, selector and controller labels stripped, `suspend: false`) | `blockOwnerDeletion` needs `update cronjobs/finalizers`; a re-run must not inherit a selector or a suspended state |
| 10 | Kind-dependent actions carry their kind (`Scale(ObjectKind)`, `RestartRollout(ObjectKind)`); `action_availability` stays two-argument (0030 decision 35) | per-resource SSAR; no ripple into 0036 |
| 11 | Row-state reasons come after the 0030 gate (`row_block`) | the 0030 gate order stays intact |
| 12 | Restart and Roll back of a paused Deployment are disabled | kubectl refuses both |
| 13 | Risk: Scale to 0 is `Destructive`; everything else `Change`; scaling down adds a warning | zero takes a workload down; a smaller drop deserves a visible line, not a harder confirm |
| 14 | No drawer replicas input (S1); one Scale popover serves the menu, ⇧S, the palette fallback, and the bulk button | W7 note 4: no action buttons in the drawer; one input surface |
| 15 | Palette inline argument on `Ctrl ⏎`; plain ⏎ on Scale falls back to the popover; `secondary-enter` bound in `CommandPalette` only | W9 shows `Ctrl ⏎` on Scale; 0029 decision 18 reserved it |
| 16 | Palette `Roll back to rev {n}` only from loaded revisions | 0029 data rule: no new list calls |
| 17 | Warnings use 0030 `GuardedIntent.warnings`: HPA, scale-down, OnDelete, CronJob concurrency (S9 texts) | non-blocking context the dry-run cannot give |
| 18 | The HPA warning uses the HPA list only when already loaded | no new watch for a hint (open item 2) |
| 19 | `checked_write` is the single caller of `ClusterConnection::write` (0030 decision 30) | one place keeps `commit_block` and the audit unskippable |
| 20 | Batch = one cluster, ≤ 50 items, always a dialog (even tier None) | 0027 non-goal "cross-cluster bulk"; a click must never fan out unseen |
| 21 | Batch dry-runs and commits run one at a time | gentle on the API server; no limiter code |
| 22 | **Every** batch item must pass its dry-run before Apply (B) | one rule for 0032, 0033, 0034; a partial apply is a surprise |
| 23 | `BatchPlan.on_failure`: `Continue` (0032, 0033, 0034) lets the next item run after a failed commit; `Stop` (0032b Set default) sends nothing more. A `Blocked` result always stops the rest | independent objects vs an ordered plan where a later step depends on an earlier one; a lock or session change is a global stop |
| 24 | Batch Roll back stays disabled | each Deployment needs its own revision choice |
| 25 | `BatchExtras` carries per-action dialog extras; 0032 uses `None`; 0033 adds `Delete` (NotFound = already gone), which may rebuild items and rerun dry-runs | one bulk mechanism instead of a per-action kind |
| 26 | (user) The roadmap's object actions moved to 0032b; Certificate Renew moved to 0018 | W7 shows only labels for them; Renew is a cert-manager CR action |
| 27 | 422 on Roll back is `Conflict` (a failed `test` op means the Deployment was replaced) (S7) | the user needs "reload", not "invalid" |
| 28 | 422 on Trigger now and Re-run shows field paths only | the server text can quote env literals of the template |
| 29 | Implementation is split into 2a-i (gate, simple actions, `checked_write` use), 2a-ii (Scale popover, palette), 2a-iii (Roll back), 2b (Batch) (S5) | each step reviewable and gated on its own |
