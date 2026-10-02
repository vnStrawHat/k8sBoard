# 0038 · Decisions

[Back to index](README.md). Architect defaults; items marked (user) need confirmation.

## Native vs `helm` CLI (C12)

What a native **rollback** must reproduce (Helm `action.Rollback`): read the current and target records; write a `pending-rollback` record (gzip + base64 JSON Secret, `helm.sh/release.v1`); map both manifests to resources (discovery for kind → resource and scope); per resource, create the missing, **three-way merge** the existing (Helm 3: client-side strategic merge from original, target, and live, with Go patch metadata that Rust lacks; Helm 4: server-side apply as manager `helm`), delete those the target dropped unless `helm.sh/resource-policy: keep`; run pre/post-rollback hooks by weight with delete policies and Job waits; mark the old record `superseded`, the new `deployed` or `failed`; prune history past 10. A native **uninstall**: pre/post-delete hooks, deletes in reverse kind order, keep policy, record deletion or `uninstalled`.

| Criterion | Native | `helm` CLI |
|---|---|---|
| Correctness | a port of thousands of lines of Go; a mismatch corrupts a production release record | Helm is its own reference |
| Hooks, keep policy, pruning, Helm 3 vs 4 apply | all to rebuild | free |
| A shortcut (SSA with manager `k8sboard`) | wrong: fields the target dropped stay, ownership moves away from Helm | n/a |
| Install footprint | none | user installs `helm` (none here) |
| Fits the 0030 write path | yes, many new `WriteOperation`s | one named process exception beside `object_write.rs` |
| Dry-run | `dryRun=All` per resource | release record checks only, both majors (decision 7) |
| Testability here | fake transport | argv, env, and output classification only |
| Effort, risk | very high, high | moderate, moderate |

**Recommendation: the CLI, for rollback and uninstall only.** The wireframe shows exactly these two actions; native is the larger and riskier path for no visible gain.

## Scope

| # | Decision | Rationale |
|---|---|---|
| 1 | Roll back and Uninstall only; no upgrade | W7 shows `Roll back…`, `Uninstall release…`, History `Roll back`, top `Rollback`; no upgrade anywhere |
| 2 | No upgrade even with stored values: the release Secret holds the chart but not its source (repo or OCI ref) | `helm upgrade` needs a chart reference; rebuilding a chart from stored templates is fragile |
| 3 | No action options; helm defaults (`--timeout 5m0s`, no `--wait`, hooks on, history removed on uninstall) | the wireframe draws none; defaults are what `helm` users expect |

## CLI

| # | Decision | Rationale |
|---|---|---|
| 4 | `helm` resolved by walking `PATH` (`split_paths`): `helm` on Unix, `helm.exe` on Windows, never `.bat`/`.cmd`, never the exe dir or CWD; the absolute path is stored and used for every spawn; version by `helm version --template {{.Version}}` (5 s), once per app run; major 3 or 4 | no setting the wireframe lacks; CVE-2024-24576 (batch-file argument injection); no search-order surprises |
| 5 | Context by `--kube-context=`; kubeconfig by `KUBECONFIG` = exactly the files k8sBoard loaded for that cluster | the same files and context name as the app. Ceiling: client-go and kube-rs can still differ on which contexts they open (an ambient proxy without `proxy-url`, a file 0024 skipped as `Incompatible`); helm's own error then shows in the dry-run |
| 6 | Environment on every spawn, detection included: inherit (exec auth plugins need theirs), set `HELM_DRIVER=secret` and `HELM_NO_PLUGINS=1`, remove `HELM_KUBE*`, `HELM_NAMESPACE`, `HELM_DEBUG`; remove proxy variables unless the kubeconfig sets `proxy-url` | parity with `connection.rs`; nothing redirects helm away from the shown context; no plugin code runs |
| 7 | Dry-run first, on both majors: rollback `--dry-run` (v3) / `--dry-run=client` (v4), uninstall `--dry-run`. It checks **only the release record** (exists, revision exists, no pending operation); it proves nothing about RBAC or admission webhooks on the chart's resources. Helm 4 `--dry-run=server` on rollback also returns before any Kubernetes call, so it is not used | 0030 "dry-run before every commit", stated honestly in the dialog |
| 8 | Kill switch: `WritePolicy::Blocked` → `WritesBlocked` before spawning | C3; agents run debug builds |
| 9 | Process: stdin null, `output()` drains both pipes concurrently, each truncated to 64 KiB afterwards; Windows `CREATE_NO_WINDOW` as an inline const on the spawn call | a bounded read of one pipe deadlocks helm on the other; no console flash; no `windows-sys` |
| 10 | Detection (5 s) and DryRun (2 min) use `kill_on_drop(true)`. Commit uses **`kill_on_drop(false)`**: at the 15 min cap it returns `OutcomeUnknown` (0030 audit `outcome: unknown`) without killing, and a detached task keeps draining the pipes until helm exits. A non-zero exit after a commit started → `Failed` with "some resources may have changed" | killing helm mid-commit leaves a `pending-rollback`/`uninstalling` record that blocks later operations; closed pipes would kill it too. Ceiling: helm's `--timeout` applies per operation, so a commit can outlive the cap |
| 11 | stderr is shown once (helm 3 `Error:` line or helm 4 slog `error="…"`, ≤ 500 chars, control characters stripped) and never logged, traced, persisted, or audited | it can echo field values (open item 4) |
| 12 | Only `helm_command.rs` spawns processes (clippy `disallowed-methods`, one named exception) | the allow-list stays a short reviewable list |

## App

| # | Decision | Rationale |
|---|---|---|
| 13 | Rollback is `ActionRisk::Change`; Uninstall is `Destructive` with the **release name** as the typed name | 0030 tiers; uninstall deletes every resource |
| 14 | Default rollback target: the highest earlier revision whose status is `superseded` or `deployed` | W7: rev 4 failed → rev 3; failed revisions are poor targets |
| 15 | Disabled while the release is `pending-*` or `uninstalling` | helm refuses "another operation is in progress" |
| 16 | Gate SSARs (release namespace): rollback `create`, `update`, and `delete secrets`; uninstall `delete secrets` | helm writes its records there; every new record prunes history past 10 (`Storage.Create` → `removeLeastRecent`), and a failed prune delete aborts the rollback; per-resource rights show only in the helm result |
| 17 | Audit object kind `HelmRelease`; fields `revision` (`4 → 3`) or none (uninstall); error field = the error class, never stderr; `OutcomeUnknown` → 0030 `outcome: unknown` | C10, C1, 0030 decision 25 |
| 18 | The row updates from the 0017 watches; no optimistic UI | one source of truth |
