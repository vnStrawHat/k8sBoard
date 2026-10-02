# 0030 · Decisions

[Back to index](README.md). Architect defaults; items marked (user) need confirmation.

| # | Decision | Rationale |
|---|---|---|
| 1 | All mutating requests go through `ClusterConnection::write` in `object_write.rs`; the app has no `kube` dependency | one reviewable choke point; C3 allow-list replaces the 0001 grep |
| 2 | The allow-list is the `WriteOperation` enum; one variant per allow-listed call, each with its `AccessCheck` and a pinned request shape test | a new write is a visible enum change plus a table row, never a stray call |
| 3 | `WriteRequest::new` returns `None` when the kind does not fit the operation | invalid requests cannot be built (design.md) |
| 4 | Field manager `k8sboard` on every write; JSON merge patch for single-field actions; SSA for YAML edits (0031) | C8 |
| 5 | `dryRun=All` before every commit, same request; connect verbs (0035/0036) are the only operations without it | C8; the dry-run is also the precise per-object RBAC check |
| 6 | The lock is per session, starts from `profile.read_only` (PROD on), and toggles session-only; the W2 switch is the stored default | W2 "Open as read-only" names the open state; a toggle that rewrote settings would surprise |
| 7 | Unlocking asks the cluster's confirm tier; locking is immediate | removing a guard deserves the same friction as a change |
| 8 | One gate in `action_availability`, order: shipped → permission state → SSAR → lock | the first failing reason is the actionable one; menus, keys, palette agree |
| 9 | (user, proposed reading, asked before step 4) Tiers: TypeName and Enter always show a dialog; a key never runs a change without an Enter dialog; for the pointer, Click shows a click-only dialog and None runs at once (None differs from Click only for the pointer); Destructive always gets a dialog | wireframe env rows plus the keyboard rule "no destructive action from one key" |
| 10 | Typed confirm matches the cluster display name by default; an action may require its object name (drain) | W10 vs W6 |
| 11 | `confirm` per cluster, `None` = env default, edited in Clusters › Safety | 0024 reserved key; W2 "Confirm changes by" |
| 12 | The badge is a toggle button with an env-colored dashed border, `Read-only` / `Unlocked` | W1 `.lock`; 0024 left the toggle to 0030 |
| 13 | Write errors: 403 Denied, 404 NotFound, 409 Conflict, 422 Invalid (with field paths), commit timeout OutcomeUnknown, the rest `ClusterError` | each needs a different user answer |
| 14 | Single-field patches send no `resourceVersion`; SSA edits carry it; deletes carry a `uid` precondition | idempotent intent vs. edits of a seen version vs. name reuse |
| 15 | Tests use a fake transport (`tower::service_fn` into `kube::Client::new`); no test or agent run writes to UAT | UAT is read-only (R2); deterministic tests |
| 16 | A started commit is never cancelled by closing the dialog; it runs to the audit line | a half-observed write is worse than a late notice |
| 17 | The lock is re-checked immediately before the commit in `write_flow.rs` | the dialog may stay open across a lock toggle |
| 18 | First consumer: Cordon / Uncordon (moved from 0034) | one reversible field, cluster-scoped, dry-run capable, existing menu item and key |
| 19 | Audit log is local JSON lines at `<config>/audit.jsonl`, append only: commits and lock toggles | W10 note 4; C10; lock lines give the module a user before any write and record who unlocked PROD |
| 20 | Audit values: Secret targets never; ConfigMap data never; paths always | C1; ConfigMaps often hold credentials |
| 21 | Audit identity = kubeconfig user entry name | no extra API call without approval |
| 22 | An audit append failure warns but does not block or undo | the write already happened |
| 23 | Every guardrail input (session access, lock, environment, confirm tier, typed name) comes from the row's cluster through `ClusterGuard`; `WriteIntent` carries its `ClusterRef`; no API reads the primary | 0027: several live sessions; a PROD row must never be gated by a DEV primary (or the reverse) |
| 24 | Crate-level kill switch: `write` returns `WritesBlocked` before building a request when the connection's `WritePolicy` is `Blocked`; debug builds are `Blocked` unless `K8SBOARD_ALLOW_WRITES=1`; tests inject the policy | every agent and screenshot run is a debug build; no stray write even with a bug upstream |
| 25 | A commit error after the request may have left the client (timeout, transport, service, unreadable response) is `OutcomeUnknown`, audited as `unknown` | claiming "failed" when the server may have applied it would mislead |
| 26 | A webhook that rejects dry-run blocks the commit; no escape in 0030 | C8 "dry-run before every apply" stays absolute; a later spec may add an audited escape |
| 27 | clippy `disallowed-methods` for raw `kube::Client` requests and mutating `kube::Api` methods, with three named exceptions: SSAR (`access_review.rs`), `object_write.rs`, and the read-only kubelet GETs (`kubelet_stats.rs`) | compile-time guard instead of a grep; the kubelet exception exists because 0011 already uses `request_text`/`request_stream` for GETs |
| 28 | Enter in the confirm dialog is a key-down handler that ignores `is_held`; the kit Enter bindings are suppressed there with `NoAction` | gpui runs bindings before key listeners, so only a key handler can see `is_held`; a held Enter from the menu must not confirm |
| 29 | Secret redaction runs on the final `WriteError`, any variant, and on the audit `error` field | one place, no variant forgotten |
