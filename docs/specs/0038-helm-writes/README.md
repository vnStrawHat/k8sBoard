# 0038 — Helm write actions (roll back, uninstall)

Status: draft, HEAD `1c859ae`. **Mutating: needs the user's approval of C12 (CLI) before step 1, and the first real rollback or uninstall needs it again (C3).** Lands after 0017 (releases, history, statuses), 0030 (gate, tiers, dry-run, audit, `run_guarded`, kill switch), 0024 (kubeconfig sources). Wireframes: W7 Releases — top button `Rollback`, ⋯ `Roll back…`, `View values`, `View manifest`, `Uninstall release…` (danger), History rows `Roll back`; phase 2 card "Helm releases: view values, diff, rollback".

## Goal

- **Roll back** a release to an earlier revision and **Uninstall** a release, exactly as `helm rollback` / `helm uninstall` do, by running the user's `helm` CLI (v3 or v4) with the same kubeconfig files and context (decision C12: CLI, not native).
- Command preview, helm dry-run before every commit, the 0030 gate, tier, lock re-check, and audit line; no values cross the process boundary.

## Non-goals

**Upgrade with values** (not in the wireframe; release Secrets do not record the chart source, so an upgrade needs a chart k8sBoard does not have; see [decisions.md](decisions.md)); a native Helm engine; install; rollback/uninstall options (`--wait`, `--no-hooks`, `--keep-history`, `--cleanup-on-fail`, `--force`); a custom helm path setting; bundling or downloading helm; Helm 2.

## Implementation steps

| Step | Scope | ACs |
|---|---|---|
| 1 | Cluster crate: `helm_command.rs` (detect, argv per major, env, run seam, classify), `ClusterConnection::helm`, `AccessCheck`s, clippy process rule; pure tests | 1–5 |
| 2 | App: gate rows, `GuardedKind::Helm`, Roll back options dialog, confirm content, History `Roll back`, top `Rollback`, Uninstall, audit, `--screen helm-rollback-confirm`; UAT denied path. **Needs user approval** | 1, 2, 6–10 |

## Files

| File | Contents |
|---|---|
| [decisions.md](decisions.md) | native vs CLI weighed, scope, numbered decisions |
| [helm-command.md](helm-command.md) | cluster API, detection, argv, environment, process, errors, secrets |
| [release-actions.md](release-actions.md) | gate, dialogs, guarded flow, audit, menus, screenshot |
| [files-to-touch.md](files-to-touch.md) · [test-plan.md](test-plan.md) | files per step; tests and checks |

## Acceptance criteria

- [ ] 1. Quality gate passes, plus `cargo clippy -p k8sboard --features screenshot --all-targets -- -D warnings`. `Cargo.lock` gains no package (tokio `process` feature only).
- [ ] 2. Every test in [test-plan.md](test-plan.md) exists under its name and passes offline; no test spawns `helm` or talks to a cluster.
- [ ] 3. `helm_command.rs` is the only process spawn in the workspace (clippy `disallowed-methods` on `std::process::Command::new` and `tokio::process::Command::new`, one named exception); `ClusterConnection::helm` is called only from `write_flow.rs`.
- [ ] 4. argv per action, mode, and helm major matches the table in [helm-command.md](helm-command.md) exactly (`--flag=value` tokens); it never contains `--debug`, `--set`, `--values`, a token, or a file path; helm is the absolute path resolved from `PATH` (`helm`/`helm.exe` only).
- [ ] 5. Debug builds without `K8SBOARD_ALLOW_WRITES=1` spawn no helm process for dry-run or commit (`WritesBlocked`).
- [ ] 6. Every commit is preceded by a helm dry-run of the same command; a failed dry-run blocks it.
- [ ] 7. Gate order: shipped → permissions → `Not permitted: …` → helm CLI missing/unsupported → release state (pending, no earlier revision, already uninstalled) → `{cluster} is read-only`.
- [ ] 8. Each commit appends one audit line (action, release, `revision` from → to; `OutcomeUnknown` → `unknown`); helm stderr is never logged, traced, or audited (test `helm_errors_are_never_traced` over `write_flow.rs` and `confirm_dialog.rs`, plus no `tracing::` in `helm_command.rs` that takes stdout or stderr).
- [ ] 9. On UAT the SSAR answers of `create`, `update`, `delete secrets` are recorded; Roll back… and Uninstall release… show their reason; **no helm process with rollback or uninstall is ever spawned** (trace, process list).
- [ ] 10. ui-verifier: `--screen helm-rollback-confirm` matches the W10-style confirm with the command preview; no high-severity defect.

## Open items

1. (user) C12: the questions below.
2. No `helm` binary on this machine and R2 (no write-capable cluster): the commit path is pure-tested; the first real run is user-run with helm installed.
3. Helm 4 flags and stderr format (slog `error="…"`) are taken from the 4.3 docs and source; confirm with `helm rollback --help` and one failing call on the user's install before step 2.
4. helm stderr can quote a Kubernetes validation error that echoes a field value; it is shown once in the dialog or notification and never stored. Accept, or show only the error class.
5. A later upgrade spec must pass values through a 0600 temp file in the config dir, never `--set` (process lists show argv).
6. helm is detected once per run when Releases first opens; a helm installed later needs an app restart. Add a "Look again" action if users ask.

## Questions for the user (C12; yes/no)

1. Use the `helm` CLI rather than a native port?
2. Scope is Roll back and Uninstall only, with no options and no custom helm path?
3. Resolve helm from `PATH` only (`helm` / `helm.exe`, absolute path, never `.bat`/`.cmd`)?
4. Environment rules: inherit, set `KUBECONFIG`, `HELM_DRIVER=secret`, `HELM_NO_PLUGINS=1`, drop `HELM_KUBE*` and proxies unless `proxy-url`?
5. Accept a dry-run that checks only the release record (no RBAC or webhook proof on chart resources)?
6. A commit is never killed; past 15 min it is reported as unknown?
7. helm stderr is shown once and never logged, traced, persisted, or audited?
8. Uninstall asks to type the release name?
9. You verify the helm 4 flags and error format on your own install before step 2?
10. The first real run is yours, on a disposable cluster with `K8SBOARD_ALLOW_WRITES=1`?
11. Add the line "processes run only through `helm_command.rs`" to `CLAUDE.md` and project-rules?
