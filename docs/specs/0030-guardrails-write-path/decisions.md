# 0030 · Decisions

[Back to index](README.md). Architect defaults; items marked (user) are the user's. C3 approved by the user on 2026-10-02 (one approval for all mutating specs).

| # | Decision | Rationale |
|---|---|---|
| 1 | All mutating requests go through `ClusterConnection::write` in `object_write.rs`; the app has no `kube` dependency | one reviewable choke point; C3 allow-list replaces the 0001 grep |
| 2 | The allow-list is the `WriteOperation` enum; one variant per allow-listed call, each with its `AccessCheck` and a pinned request shape test | a new write is a visible enum change plus a table row, never a stray call |
| 3 | `WriteRequest::new` returns `None` when the kind does not fit the operation | invalid requests cannot be built (design.md) |
| 4 | Field manager `k8sboard` on every patch, create, and replace; deletes carry none (`DeleteOptions` has no `fieldManager`) and put `dryRun` in the body (0033). JSON merge patch for single-field actions; YAML edits replace with a `resourceVersion` (0031). No SSA and no Force path (amended C8 note) | C8; the API shape of each verb |
| 5 | `dryRun=All` before every commit, same request; the only operations without it are the connect verbs (0035/0036/0037) and 0037 `DeleteNodeShellPod` (commit only: it deletes the app's own pod under a `uid` precondition and must fit the shutdown window) | C8; the dry-run is also the precise per-object RBAC check |
| 6 | The lock is per session, starts from `profile.read_only` (PROD on), and toggles session-only; the W2 switch is the stored default | W2 "Open as read-only" names the open state; a toggle that rewrote settings would surprise |
| 7 | Unlocking asks the cluster's confirm tier; locking is immediate | removing a guard deserves the same friction as a change |
| 8 | One gate in `action_availability`, order: shipped → permission state → SSAR → lock | the first failing reason is the actionable one; menus, keys, palette agree |
| 9 | (user, 2026-10-02) Two tiers: `TypeName` (PROD) and `Click` (STG, DEV, LOCAL; an unknown environment is STG). Every guarded action opens a confirm dialog, for every tier, risk, and trigger (pointer, key, palette); nothing runs without it. The Enter, one-click-without-dialog, and None tiers are removed, and with them `Trigger`, `ConfirmStep::Run`, and `ClickOnly`. Privileged (0037) always types the name | user decision; one rule everywhere, and "no destructive action from one key" holds by construction |
| 10 | Typed confirm matches the cluster display name by default; an action may require its object name (drain) | W10 vs W6 |
| 11 | `confirm` per cluster, `None` = env default, edited in Clusters › Safety | 0024 reserved key; W2 "Confirm changes by" |
| 12 | The badge is a toggle button with an env-colored dashed border, `Read-only` / `Unlocked` | W1 `.lock`; 0024 left the toggle to 0030 |
| 13 | Write errors: 403 Denied (RBAC form only; other 403s are Invalid), 404 NotFound, 409 Conflict, 422 Invalid (with field paths), 429 TooManyRequests (offered Retry like Conflict), commit timeout OutcomeUnknown, the rest `ClusterError` | each needs a different user answer |
| 14 | Single-field patches send no `resourceVersion`; replace edits (0031) carry it; deletes and evictions carry a `uid` precondition | idempotent intent vs. edits of a seen version vs. name reuse |
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
| 27 | clippy `disallowed-methods` for raw `kube::Client` requests and mutating `kube::Api` methods, with named exceptions kept in one canonical table (write-path.md): 0030 owns SSAR (`access_review.rs`), `object_write.rs`, and the read-only kubelet GETs (`kubelet_stats.rs`); 0035–0037 add one row each (0038 `helm_command.rs` is deferred) | compile-time guard instead of a grep; the kubelet exception exists because 0011 already uses `request_text`/`request_stream` for GETs |
| 28 | (2026-10-02) Enter stays a keyboard path inside the dialog: `Click` focuses the confirm button and Enter activates it; `TypeName` focuses the input and Enter confirms once the name matches. A key-down handler confirms only when `!is_held`, so a held or repeated Enter is ignored; the kit Enter bindings are suppressed there with `NoAction` | keyboard accessibility; gpui runs bindings before key listeners, so only a key handler can see `is_held`; a held Enter from the menu or palette must not confirm |
| 29 | Secret redaction runs on the final `WriteError`, any variant, and on the audit `error` field | one place, no variant forgotten |
| 30 | (amendment, 0032) `checked_write(WriteStep { intent, generation, mode, note })` is the `GuardedKind::Write` branch of `run_guarded` steps 5–6 and the only caller of `ClusterConnection::write`; `Connect` keeps its callback | bulk and drain repeat commits; `commit_block` and the audit line stay unskippable |
| 31 | (amendment) App `CommitMode { DryRun, Commit { confirmed: Confirmed } }` (named apart from `cluster::WriteMode`, which stays); `Confirmed` is built only when the confirm step is satisfied (dry-run passed, typed name matches) | a commit cannot be expressed without a confirmation |
| 32 | (amendment) `GuardedIntent.warnings: Vec<SharedString>`, rendered by `run_guarded`'s dialog (replaces a per-spec warning field) | 0031, 0032, 0033, 0034 add non-blocking context lines |
| 33 | (amendment) `WriteOutcome.effect: WriteEffect` and `created_name: Option<String>` | notices and audit name what happened and what was created |
| 34 | (amendment, 0032) `GuardedKind::Batch(BatchPlan { items, skipped, extras: BatchExtras })` is the one bulk mechanism: ≤ 50 items, sequential dry-runs, all must pass, each commit through `checked_write` | one bulk path for 0032, 0033, 0034 |
| 35 | (amendment) `action_availability(action, guard)` stays two-argument; `ResourceAction::gate()` reads the kind carried by kind-dependent variants (`Scale(kind)`, `RestartRollout(kind)`) | per-resource SSAR without touching every caller (0036 unchanged) |
| 36 | (amendment, 0034) A 429 refusal is not audited; a drain writes one summary line per node (outcomes `drained`, `stuck`, `cancelled`, `stopped`) | nothing changed on a refusal; retries would flood the log |
