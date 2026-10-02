# 0038 · Release actions in the app

[Back to index](README.md) · Step 2 · Modules: `helm_rows.rs` (0017), `helm_release_view.rs` (0017), `resource_actions.rs`, `write_flow.rs` (0030), `confirm_dialog.rs` (0030), `audit_log.rs` (0030), `app_shell.rs`, `screenshot.rs`, `launch_options.rs`. Decisions 13–18. Wireframe: W7 Releases.

## Helm CLI state

`AppShell.helm_cli: HelmCliState` plus `Detecting`, filled once by `ClusterRuntime::spawn(detect_helm())` the first time the Releases screen opens (not at app start). Read by the gate; never refreshed in a run (README open item 6).

## Gate (`release_action_availability(action, guard, release, history, helm_cli)`)

Built on 0030 `action_availability` for rows 1–3 and 6; rows 4–5 are subject checks:

| # | Check | Reason |
|---|---|---|
| 1 | shipped | `Comes in a later version` (until step 2) |
| 2 | permissions checking / unknown | 0030 texts |
| 3 | SSAR: rollback `create`, `update`, `delete secrets` (history pruning, decision 16); uninstall `delete secrets` | `Not permitted: create secrets` (first missing) |
| 4 | helm CLI | `Looking for helm…` · `Needs the helm CLI (v3 or v4) on PATH` · `helm {version} is not supported; use v3 or v4` |
| 5 | release state | `pending-*` / `uninstalling` → `Another Helm operation is in progress ({status})`; rollback with no earlier `superseded`/`deployed` revision → `No earlier revision to roll back to`; uninstall of `uninstalled` → `Already uninstalled` |
| 6 | lock | `{cluster} is read-only` |

`ResourceAction::{HelmRollback, HelmUninstall}` are new (`mutates: true`); 0017's disabled `Read-only mode` items and decision 24 there are replaced.

## Entry points (W7)

| Where | Item | Runs |
|---|---|---|
| Release ⋯ menu and row menu | `Roll back…` | options dialog |
| Release ⋯ menu and row menu, last, danger | `Uninstall release…` | guarded uninstall |
| W7 top button `Rollback` (selected row) | same as `Roll back…` | options dialog |
| History row (0017 Overview) | `Roll back` on rows other than the current revision | guarded rollback to that revision (no options dialog) |

`View values` and `View manifest` stay 0017's.

## Roll back options dialog (kit `Dialog`, width 420)

- Title `Roll back {release}`; muted `{namespace} · current revision {from} ({status})`.
- `Revision` select: every History revision below `from`, newest first, `rev {n} · {status} · {age}`; default per decision 14.
- `Continue` → guarded flow; `Cancel`.

## Guarded flow (0030 `run_guarded`)

```rust
pub(crate) enum GuardedKind { /* 0030, 0035, 0036, 0037 variants */
    Helm { cli: HelmCli, request: HelmRequest } }               // 0038
```

`GuardedIntent { cluster, action, label, risk, expected_name, kind: Helm { .. } }`:

| Action | `label` | `risk` | `expected_name` |
|---|---|---|---|
| Roll back | `Roll back {release} to revision {to}` | `Change` | `None` (cluster name on TypeName) |
| Uninstall | `Uninstall {release}` | `Destructive` | `Some(release)` |

- Dry-run branch: `connection.helm(&cli, &request, DryRun)` → `DryRunState::Passed` / `Failed(text)`; commit: `helm(.., Commit)`. Everything else (gate re-check, lock, generation, typed name, held Enter) is 0030's.
- `ClusterConnection::helm` has no other caller (AC 3).

## Confirm dialog content (0030 `confirm_dialog.rs`, Helm variant)

| Part | Content |
|---|---|
| Object row | `Hm` icon text, `{namespace}/{release}` mono, `Helm release` |
| Changes | rollback: `revision {from} ({status}) → {to}`; uninstall: `Deletes every resource of the release and its history. Resources annotated helm.sh/resource-policy: keep stay.` (danger token). Both, muted: `The dry-run checks only the release record; it proves nothing about permissions or admission webhooks on the chart's resources.` |
| Command | muted mono line: `helm_preview(&request.commit_args(cli, context))` (joins with `resource_actions::shell_quote`), e.g. `helm rollback strimzi 3 --namespace=kafka --kube-context=prod-eu-1`; a copy icon copies it (plain clipboard; names only) |
| Dry-run line | `Helm dry-run…` · `Helm dry-run passed · {ms} ms` + muted `(checks the release only)` on both majors · `Helm dry-run failed: {message}` |
| helm line | muted `helm {version} · {path}` |
| Typed name, note, buttons | 0030 (`Roll back` primary; `Uninstall` danger) |

## After the commit

- Success: notification `Rolled back {release} to revision {to}` / `Uninstalled {release}` (success token). The 0017 release and history watches update the row; an uninstalled release leaves the list (history deleted) on the next event.
- Failure: 0030 surfaces with the `HelmError` text; `OutcomeUnknown` and commit `Failed` both say the release may have changed and suggest checking History.

## Audit (0030 `audit_entry`)

| Key | Rollback | Uninstall |
|---|---|---|
| `action` | `Roll back release` | `Uninstall release` |
| `object` | `{ kind: "HelmRelease", namespace, name }` | same |
| `fields` | `[{ path: "revision", value: "4 → 3" }]` | `[]` |
| `error` | error class only (`denied`, `not found`, `busy`, `failed (exit 1)`, `unknown`), never stderr | same |
| `outcome` | `applied` / `failed`; `OutcomeUnknown` → `unknown` (0030) | same |

## Screenshot (`--screen helm-rollback-confirm`, `screenshot` feature only)

The confirm dialog for a fixture release `kafka/strimzi` rev 4 failed → 3, fixture `HelmCli` v3.15.2, `DryRunState::Passed { 640 ms }`, TypeName tier on a seeded PROD entry; no process, no connection call. Listed in `USAGE`.
